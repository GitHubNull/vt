//! ConfigStore 服务（tech/08 8.2、tech/04 4.4）：JSON 配置的原子化持久化与变更广播。
//!
//! - 写入（NFR-R-05）：序列化 → 写 `config.json.tmp` → `fsync` → 原子 `rename`；
//!   成功后刷新 `.bak` 上次良好副本，任何时刻磁盘上不存在半写状态；
//! - 读取：JSON 解析失败 → 尝试 `config.json.bak` → 仍失败则默认配置启动，
//!   并广播 `config:recovered` 提示事件（tech/08 8.2 损坏回退）；
//! - 并发：全部读写经本单例（`RwLock`），前端多窗口不直接碰文件（tech/08 8.2）；
//! - 隐私（NFR-S-02）：日志只出现节名与操作结果，绝不出现配置值正文。
//!
//! 与前端 `src/api/types.ts` 手工保持同步（tech/03 3.5），修改字段须同步前端类型。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::error::AppError;
use crate::events::{AppEvent, RecoverySource};

/// 配置文件名（配置目录下）。
pub const CONFIG_FILE_NAME: &str = "config.json";

/// v0.1 配置节名（T-003 SPEC 1.2），`set_config` 的节名枚举校验。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSection {
    General,
    Hotkey,
    Audio,
    Output,
    Dict,
    Asr,
    About,
    Log,
}

impl ConfigSection {
    /// 从字符串解析节名（命令层参数校验）。
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "general" => Some(Self::General),
            "hotkey" => Some(Self::Hotkey),
            "audio" => Some(Self::Audio),
            "output" => Some(Self::Output),
            "dict" => Some(Self::Dict),
            "asr" => Some(Self::Asr),
            "about" => Some(Self::About),
            "log" => Some(Self::Log),
            _ => None,
        }
    }
}

/// 界面语言（v0.1 仅简体中文可用，其余选项预留，F-SET-01）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    System,
    ZhCn,
    EnUs,
    ZhTw,
}

/// 界面主题（v0.1 浅色 / 跟随系统，深色 v1.0 置灰，F-SET-01）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// 录音快捷键触发模式（F-INPUT-02）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyMode {
    /// 按住说话：Pressed 开始 / Released 结束。
    #[default]
    Hold,
    /// 切换模式：Pressed 翻转开始 / 结束。
    Toggle,
}

/// 悬浮球位置（PRD 3.8.2）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FloatBallPosition {
    #[default]
    Cursor,
    BottomCenter,
    Fixed,
}

/// 界面设置（F-SET-01、F-SET-05）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub language: Language,
    pub theme: Theme,
    /// 开机自启动（PRD 3.8.6，F-SET-05；默认关。平台自启注册随实现任务接入，
    /// 当前仅持久化开关）。
    pub launch_at_login: bool,
    /// 保存语音数据集（PRD 3.8.6，默认关，隐私优先：开启才可能留存音频样本）。
    pub save_voice_dataset: bool,
    /// 诊断模式（debug 日志级别，T-013 设置页开关）。
    pub diagnostic_mode: bool,
    /// 悬浮球上次拖动位置（T-008 位置记忆；None = 未拖动过，按落点策略初始放置）。
    #[serde(default)]
    pub float_ball_xy: Option<[i32; 2]>,
}

/// 输入与录音设置（F-INPUT-01/02、PRD 3.8.2）。
///
/// 快捷键键名使用全局快捷键库的 Code 字符串（如 `"AltRight"`、`"F2"`、
/// `"ControlLeft+ShiftLeft+Space"` 形式由 HotkeyService 解析，见 T-004）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// 录音快捷键键名（默认右 Alt，macOS 右 Option 同为 `AltRight`）。
    pub trigger: String,
    /// 触发模式：按住说话（默认）/ 切换。
    pub mode: HotkeyMode,
    /// 取消录音快捷键（PRD 3.8.2，默认 Esc）。
    pub cancel_trigger: String,
    /// 最长录音时长（分钟，1–10，PRD 3.1.1 规则 5）。
    pub max_recording_minutes: u32,
    /// 录音时显示悬浮球（PRD 3.1.6）。
    pub show_float_ball: bool,
    /// 悬浮球位置。
    pub float_ball_position: FloatBallPosition,
    /// 开始 / 结束录音提示音（默认关，PRD 3.8.2）。
    pub sound_hint: bool,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            trigger: "AltRight".to_string(),
            mode: HotkeyMode::default(),
            cancel_trigger: "Escape".to_string(),
            max_recording_minutes: 5,
            show_float_ball: true,
            float_ball_position: FloatBallPosition::default(),
            sound_hint: false,
        }
    }
}

/// 录音环境设置（F-SET-03、PRD 3.8.4）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// 首选麦克风设备 ID（`None` = 系统默认；设备 ID 来自 T-005 设备枚举）。
    pub input_device_id: Option<String>,
    /// 录音时静音系统扬声器（默认开）。
    pub mute_speaker_while_recording: bool,
    /// 麦克风常驻预热（默认开，NFR-P-01 / NFR-PR-07，T-005 消费）。
    pub keep_warm: bool,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input_device_id: None,
            mute_speaker_while_recording: true,
            keep_warm: true,
        }
    }
}

/// 文本输出设置（F-SET-02、PRD 3.8.3）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    /// 智能数字转换（"一百二十" → "120"）。
    pub smart_number: bool,
    /// 过滤语气词（"嗯、啊"等）。
    pub filter_fillers: bool,
    /// 输出繁体中文。
    pub traditional_chinese: bool,
    /// 删除结尾标点。
    pub remove_trailing_punct: bool,
    /// 使用复制粘贴输入（默认开；关闭则模拟键入）。
    pub paste_input: bool,
    /// 保留识别文本到剪贴板（默认关，上屏后恢复原剪贴板）。
    pub keep_text_in_clipboard: bool,
    /// 已知"模拟键入响应异常"的应用清单：命中时强制走剪贴板路径（T-007 机制，
    /// 前台应用识别接入前（v0.5 T-501）恒为空命中）。
    #[serde(default)]
    pub compat_clipboard_apps: Vec<String>,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            smart_number: true,
            filter_fillers: true,
            traditional_chinese: false,
            remove_trailing_punct: true,
            paste_input: true,
            keep_text_in_clipboard: false,
            compat_clipboard_apps: Vec::new(),
        }
    }
}

/// 词典设置（F-DICT-01 总开关，词条本体由 T-011 DictService 管理）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DictConfig {
    /// 替换词典启用开关。
    pub enabled: bool,
}

impl Default for DictConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// 模型下载源（T-017，PRD 3.8.7 v1.5：默认镜像源，两源均 SHA-256 校验）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    /// 国内镜像（hf-mirror.com，作者官方同步仓库）。
    #[default]
    Mirror,
    /// 官方源（GitHub Release 归档）。
    Official,
}

/// 识别引擎设置（F-SET-06 当前模型，T-006；下载体验 T-017）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AsrConfig {
    /// 当前激活模型 ID（`None` = 自动选择首个可用模型）。
    pub current_model_id: Option<String>,
    /// 模型存储目录（`None` = 默认：安装目录旁 `models/`，不可写时回退数据目录）。
    pub models_dir: Option<String>,
    /// 模型下载代理（如 `http://127.0.0.1:7890`；`None`/空 = 直连；只作用于模型下载）。
    pub download_proxy: Option<String>,
    /// 模型下载源（默认镜像源）。
    pub model_source: ModelSource,
}

/// 关于页设置（F-ABOUT-01）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AboutConfig {
    /// 自动检查更新（默认开，每天最多一次，只发送版本号 / OS / 架构，NFR-PR-06）。
    pub auto_check_update: bool,
    /// 更新源地址（v0.1 更新服务器未就绪，默认占位地址；tech/08 8.8）。
    #[serde(default = "default_update_feed_url")]
    pub update_feed_url: String,
}

fn default_update_feed_url() -> String {
    "https://example.com/vibetype/releases.json".to_string()
}

impl Default for AboutConfig {
    fn default() -> Self {
        Self {
            auto_check_update: true,
            update_feed_url: default_update_feed_url(),
        }
    }
}

/// 日志记录目标（F-LOG-02）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogTarget {
    /// 按天滚动文本文件。
    File,
    /// 本地 SQLite 数据库（支撑应用内搜索）。
    Database,
}

/// 日志设置（F-LOG-02：记录目标，至少一项，默认文本 + 数据库全开）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    /// 启用的记录目标（全空非法：apply 拒绝并提示至少保留一项）。
    pub targets: Vec<LogTarget>,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            targets: vec![LogTarget::File, LogTarget::Database],
        }
    }
}

/// 应用配置根（按节分模块；各节带 `#[serde(default)]` 保证升级新增字段不破坏旧配置，
/// 即 NFR-R-06"升级保留配置"的读取侧保证）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub hotkey: HotkeyConfig,
    pub audio: AudioConfig,
    pub output: OutputConfig,
    pub dict: DictConfig,
    pub asr: AsrConfig,
    pub about: AboutConfig,
    pub log: LogConfig,
}

/// 配置持久化错误编号（用户可见提示中的稳定编号）。
const ERR_IO: u32 = 1;
const ERR_BAD_JSON: u32 = 2;
const ERR_BAD_VALUE: u32 = 4;

/// ConfigStore 单例：配置内存镜像 + 原子落盘 + 变更广播。
///
/// 句柄可克隆（`Arc`），多窗口命令共享同一实例。
#[derive(Clone)]
pub struct ConfigStore {
    path: Arc<PathBuf>,
    config: Arc<RwLock<AppConfig>>,
    events: Arc<tokio::sync::broadcast::Sender<AppEvent>>,
}

impl ConfigStore {
    /// 加载配置：解析失败 → `.bak` → 默认配置；损坏回退时广播 `config:recovered`。
    pub fn load(
        config_dir: &Path,
        events: tokio::sync::broadcast::Sender<AppEvent>,
    ) -> Result<Self, AppError> {
        let path = config_dir.join(CONFIG_FILE_NAME);
        let mut recovered = None;
        let mut config = match fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<AppConfig>(&content) {
                Ok(config) => {
                    info!("配置加载成功");
                    config
                }
                Err(err) => {
                    // NFR-S-02：日志只记录解析错误与操作结果，不记录配置正文。
                    warn!(error = %err, "配置文件解析失败，尝试 .bak 回退");
                    match Self::read_backup(config_dir) {
                        Some(backup) => {
                            recovered = Some(RecoverySource::Backup);
                            backup
                        }
                        None => {
                            warn!("配置 .bak 回退不可用，使用默认配置");
                            recovered = Some(RecoverySource::Default);
                            AppConfig::default()
                        }
                    }
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                info!("配置文件不存在，使用默认配置");
                AppConfig::default()
            }
            Err(err) => {
                return Err(AppError::Config(ERR_IO, format!("读取配置文件失败: {err}")));
            }
        };
        // F-LOG-02：手工编辑成空的记录目标按默认（双开）归一化，与 apply 校验口径一致。
        if config.log.targets.is_empty() {
            config.log = LogConfig::default();
        }

        let store = Self {
            path: Arc::new(path),
            config: Arc::new(RwLock::new(config)),
            events: Arc::new(events),
        };
        if let Some(source) = recovered {
            // 提示事件：前端据此弹出"配置已恢复"提示（tech/08 8.2）。
            let _ = store.events.send(AppEvent::ConfigRecovered { source });
        }
        Ok(store)
    }

    /// 读取 `.bak` 上次良好副本；不存在或同样损坏时返回 `None`。
    fn read_backup(config_dir: &Path) -> Option<AppConfig> {
        let backup_path = backup_path(config_dir);
        let content = fs::read_to_string(backup_path).ok()?;
        let config = serde_json::from_str::<AppConfig>(&content).ok()?;
        info!("配置已从 .bak 副本回退");
        Some(config)
    }

    /// 获取配置镜像（读取侧一律走内存，不碰磁盘）。
    pub async fn get(&self) -> AppConfig {
        self.config.read().await.clone()
    }

    /// 更新指定节并原子落盘；写盘成功后广播 `config:changed`（携带节名）。
    pub async fn set_section(
        &self,
        section: ConfigSection,
        value: serde_json::Value,
    ) -> Result<(), AppError> {
        // 校验：值必须能反序列化为该节的 Schema。
        let mut config = self.config.write().await;
        Self::apply(&mut config, section, value)?;
        self.save_locked(&config).await?;
        // 先落盘再广播：多窗口以广播为准做镜像刷新（tech/08 8.2）。
        let _ = self.events.send(AppEvent::ConfigChanged { section });
        info!(section = ?section, "配置已保存");
        Ok(())
    }

    /// 把节值应用到内存镜像（Schema 校验失败时镜像保持不变）。
    fn apply(
        config: &mut AppConfig,
        section: ConfigSection,
        value: serde_json::Value,
    ) -> Result<(), AppError> {
        let apply_err = |err: serde_json::Error| {
            AppError::Config(ERR_BAD_VALUE, format!("配置节值不合法: {err}"))
        };
        // 逐节校验后再整体赋值，避免半更新状态。
        match section {
            ConfigSection::General => {
                config.general = serde_json::from_value(value).map_err(apply_err)?;
            }
            ConfigSection::Hotkey => {
                let hotkey: HotkeyConfig = serde_json::from_value(value).map_err(apply_err)?;
                if !(1..=10).contains(&hotkey.max_recording_minutes) {
                    return Err(AppError::Config(
                        ERR_BAD_VALUE,
                        "最长录音时长须在 1–10 分钟之间".to_string(),
                    ));
                }
                config.hotkey = hotkey;
            }
            ConfigSection::Audio => {
                config.audio = serde_json::from_value(value).map_err(apply_err)?;
            }
            ConfigSection::Output => {
                config.output = serde_json::from_value(value).map_err(apply_err)?;
            }
            ConfigSection::Dict => {
                config.dict = serde_json::from_value(value).map_err(apply_err)?;
            }
            ConfigSection::Asr => {
                let asr: AsrConfig = serde_json::from_value(value).map_err(apply_err)?;
                // T-017：代理只接受 http(s) 代理地址，格式非法直接拒绝（避免下载器报错延迟暴露）。
                if let Some(proxy) = asr.download_proxy.as_deref() {
                    if !proxy.is_empty()
                        && !(proxy.starts_with("http://") || proxy.starts_with("https://"))
                    {
                        return Err(AppError::Config(
                            ERR_BAD_VALUE,
                            "下载代理须为 http(s):// 地址（留空表示直连）".to_string(),
                        ));
                    }
                }
                config.asr = asr;
            }
            ConfigSection::About => {
                config.about = serde_json::from_value(value).map_err(apply_err)?;
            }
            ConfigSection::Log => {
                let log: LogConfig = serde_json::from_value(value).map_err(apply_err)?;
                // F-LOG-02：至少保留一种记录方式，全空直接拒绝。
                if log.targets.is_empty() {
                    return Err(AppError::Config(
                        ERR_BAD_VALUE,
                        "至少选择一种日志记录方式（文本文件或数据库）".to_string(),
                    ));
                }
                config.log = log;
            }
        }
        Ok(())
    }

    /// 原子写入（NFR-R-05）：tmp → fsync → rename → 刷新 `.bak`。
    ///
    /// 调用者必须已持有写锁。
    async fn save_locked(&self, config: &AppConfig) -> Result<(), AppError> {
        write_json_atomic(&self.path, config)?;
        // 成功后留下本次良好副本 config.json.bak（与 config.json 不同 inode，
        // config.json 被外部写坏时可回退；tech/08 8.2 损坏回退）。
        let backup = backup_path_of(&self.path);
        if let Err(err) = fs::copy(self.path.as_ref(), &backup) {
            warn!(error = %err, "刷新配置 .bak 副本失败");
        }
        Ok(())
    }
}

/// 原子 JSON 落盘（NFR-R-05）：序列化 → 写 `<path>.tmp` → fsync → rename → 同步父目录。
///
/// 复用方（如 T-011 词典持久化）仅需提供目标路径与可序列化值；
/// 任何时刻磁盘上不存在半写状态，崩溃最多残留 `.tmp`。
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), AppError> {
    let content = serde_json::to_string_pretty(value)
        .map_err(|err| AppError::Config(ERR_BAD_JSON, format!("JSON 序列化失败: {err}")))?;
    // 父目录可能尚不存在（如词典首写时的 dict/）：先创建再落盘。
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_err)?;
    }
    let tmp = tmp_path(path);
    let mut file = fs::File::create(&tmp).map_err(io_err)?;
    use std::io::Write;
    file.write_all(content.as_bytes()).map_err(io_err)?;
    file.sync_all().map_err(io_err)?;
    drop(file);
    fs::rename(&tmp, path).map_err(io_err)?;
    if let Some(dir) = path.parent() {
        if let Ok(dir_file) = fs::File::open(dir) {
            let _ = dir_file.sync_all();
        }
    }
    Ok(())
}

fn io_err(err: std::io::Error) -> AppError {
    AppError::Config(ERR_IO, format!("配置文件写入失败: {err}"))
}

fn tmp_path(path: &Path) -> PathBuf {
    path.with_extension("json.tmp")
}

fn backup_path_of(path: &Path) -> PathBuf {
    path.with_extension("json.bak")
}

fn backup_path(config_dir: &Path) -> PathBuf {
    backup_path_of(&config_dir.join(CONFIG_FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static DIR_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// 极简临时目录（tech/02 未登记 tempfile crate，测试内自建）。
    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let n = DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir().join(format!(
                "vt-config-test-{}-{}-{}",
                name,
                std::process::id(),
                n
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn test_store(dir: &TestDir) -> (ConfigStore, tokio::sync::broadcast::Receiver<AppEvent>) {
        let (tx, rx) = crate::events::channel();
        let store = ConfigStore::load(dir.path(), tx).unwrap();
        (store, rx)
    }

    async fn section_value(store: &ConfigStore, section: ConfigSection) -> serde_json::Value {
        let config = store.get().await;
        match section {
            ConfigSection::General => serde_json::to_value(&config.general).unwrap(),
            ConfigSection::Hotkey => serde_json::to_value(&config.hotkey).unwrap(),
            ConfigSection::Audio => serde_json::to_value(&config.audio).unwrap(),
            ConfigSection::Output => serde_json::to_value(&config.output).unwrap(),
            ConfigSection::Dict => serde_json::to_value(&config.dict).unwrap(),
            ConfigSection::Asr => serde_json::to_value(&config.asr).unwrap(),
            ConfigSection::About => serde_json::to_value(&config.about).unwrap(),
            ConfigSection::Log => serde_json::to_value(&config.log).unwrap(),
        }
    }

    #[tokio::test]
    async fn normal_read_write_roundtrip() {
        let dir = TestDir::new("roundtrip");
        let (store, _rx) = test_store(&dir);
        assert_eq!(store.get().await.hotkey.trigger, "AltRight");
        assert_eq!(store.get().await.hotkey.max_recording_minutes, 5);

        let mut value = section_value(&store, ConfigSection::Hotkey).await;
        value["max_recording_minutes"] = serde_json::json!(10);
        store
            .set_section(ConfigSection::Hotkey, value)
            .await
            .unwrap();

        // 磁盘内容正确，且无 .tmp 残留（NFR-R-05）。
        let raw = fs::read_to_string(dir.path().join(CONFIG_FILE_NAME)).unwrap();
        let on_disk: AppConfig = serde_json::from_str(&raw).unwrap();
        assert_eq!(on_disk.hotkey.max_recording_minutes, 10);
        assert!(!tmp_path(&dir.path().join(CONFIG_FILE_NAME)).exists());

        // .bak 副本为本次写入前的良好配置留存（与 config.json 内容一致，独立 inode）。
        let bak: AppConfig =
            serde_json::from_str(&fs::read_to_string(backup_path(dir.path())).unwrap()).unwrap();
        assert_eq!(bak.hotkey.max_recording_minutes, 10);

        // 重新加载得到新值（持久化生效）。
        let (store2, _rx) = test_store(&dir);
        assert_eq!(store2.get().await.hotkey.max_recording_minutes, 10);
    }

    #[tokio::test]
    async fn log_targets_default_both_and_reject_empty() {
        let dir = TestDir::new("log-targets");
        let (store, _rx) = test_store(&dir);
        // 默认：文本 + 数据库全开（F-LOG-02）。
        assert_eq!(
            store.get().await.log.targets,
            vec![LogTarget::File, LogTarget::Database]
        );
        // 允许只留一项。
        let mut value = section_value(&store, ConfigSection::Log).await;
        value["targets"] = serde_json::json!(["file"]);
        store.set_section(ConfigSection::Log, value).await.unwrap();
        assert_eq!(store.get().await.log.targets, vec![LogTarget::File]);
        // 全空拒绝（E-CONFIG-004），镜像保持不变。
        let mut value = section_value(&store, ConfigSection::Log).await;
        value["targets"] = serde_json::json!([]);
        let err = store
            .set_section(ConfigSection::Log, value)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "E-CONFIG-004");
        assert_eq!(store.get().await.log.targets, vec![LogTarget::File]);
        // 手工编辑成空：加载归一化为默认。
        drop(store);
        let mut config: AppConfig = {
            let raw = fs::read_to_string(dir.path().join(CONFIG_FILE_NAME)).unwrap();
            serde_json::from_str(&raw).unwrap()
        };
        config.log.targets = Vec::new();
        fs::write(
            dir.path().join(CONFIG_FILE_NAME),
            serde_json::to_string(&config).unwrap(),
        )
        .unwrap();
        let (store2, _rx) = test_store(&dir);
        assert_eq!(
            store2.get().await.log.targets,
            vec![LogTarget::File, LogTarget::Database]
        );
    }

    #[tokio::test]
    async fn corrupted_config_falls_back_to_backup() {
        let dir = TestDir::new("corrupt-bak");
        let (store, _rx) = test_store(&dir);
        let mut value = section_value(&store, ConfigSection::Output).await;
        value["paste_input"] = serde_json::json!(false);
        store
            .set_section(ConfigSection::Output, value)
            .await
            .unwrap();
        drop(store);

        // 手工写坏 config.json（非法 JSON）。
        fs::write(dir.path().join(CONFIG_FILE_NAME), "{ not valid json !!!").unwrap();
        let (_store2, mut rx) = test_store(&dir);
        let recovered = rx.try_recv().unwrap();
        match recovered {
            AppEvent::ConfigRecovered {
                source: RecoverySource::Backup,
            } => {}
            other => panic!("期望 ConfigRecovered(Backup)，实际 {other:?}"),
        }
        // 回退到 .bak 中的值。
        let store3 = {
            let (tx, _rx) = crate::events::channel();
            ConfigStore::load(dir.path(), tx).unwrap()
        };
        assert!(!store3.get().await.output.paste_input);
    }

    #[tokio::test]
    async fn corrupted_config_and_backup_falls_back_to_default() {
        let dir = TestDir::new("corrupt-default");
        fs::write(dir.path().join(CONFIG_FILE_NAME), "broken").unwrap();
        fs::write(backup_path(dir.path()), "also broken").unwrap();
        let (store, mut rx) = test_store(&dir);
        match rx.try_recv().unwrap() {
            AppEvent::ConfigRecovered {
                source: RecoverySource::Default,
            } => {}
            other => panic!("期望 ConfigRecovered(Default)，实际 {other:?}"),
        }
        assert_eq!(store.get().await.hotkey.trigger, "AltRight");
    }

    #[tokio::test]
    async fn leftover_tmp_does_not_affect_load() {
        let dir = TestDir::new("tmp-leftover");
        let (store, _rx) = test_store(&dir);
        let mut value = section_value(&store, ConfigSection::General).await;
        value["language"] = serde_json::json!("zh_cn");
        store
            .set_section(ConfigSection::General, value)
            .await
            .unwrap();

        // 模拟写入中断：磁盘上残留 .tmp 文件。
        fs::write(tmp_path(&dir.path().join(CONFIG_FILE_NAME)), "partial").unwrap();
        let (store2, _rx) = test_store(&dir);
        assert_eq!(store2.get().await.general.language, Language::ZhCn);
    }

    #[tokio::test]
    async fn concurrent_writes_are_serialized_and_last_wins() {
        let dir = TestDir::new("concurrent");
        let (store, mut rx) = test_store(&dir);
        let mut handles = Vec::new();
        for i in 0..8u32 {
            let store = store.clone();
            handles.push(tokio::spawn(async move {
                let mut value = serde_json::to_value(HotkeyConfig::default()).unwrap();
                value["max_recording_minutes"] = serde_json::json!(1 + i.min(9));
                store.set_section(ConfigSection::Hotkey, value).await
            }));
        }
        for handle in handles {
            handle.await.unwrap().unwrap();
        }
        // 磁盘上的配置必是某个完整写入（原子性），且与内存镜像一致。
        let raw = fs::read_to_string(dir.path().join(CONFIG_FILE_NAME)).unwrap();
        let on_disk: AppConfig = serde_json::from_str(&raw).unwrap();
        let in_memory = store.get().await;
        assert_eq!(on_disk, in_memory);
        assert!((1..=10).contains(&on_disk.hotkey.max_recording_minutes));
        // 每次成功写盘都广播 config:changed。
        let mut changed_count = 0;
        while rx.try_recv().is_ok() {
            changed_count += 1;
        }
        assert_eq!(changed_count, 8);
    }

    #[tokio::test]
    async fn invalid_section_value_rejected_without_mutation() {
        let dir = TestDir::new("invalid-value");
        let (store, _rx) = test_store(&dir);
        // 字段类型不合法（trigger 应为字符串）→ 拒绝且不改动镜像。
        let err = store
            .set_section(ConfigSection::Hotkey, serde_json::json!({ "trigger": 123 }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), format!("E-CONFIG-{ERR_BAD_VALUE:03}"));
        assert_eq!(store.get().await.hotkey.trigger, "AltRight");

        // 超出范围的最长录音时长被拒绝（PRD 3.1.1 规则 5：1–10 分钟）。
        let mut value = serde_json::to_value(HotkeyConfig::default()).unwrap();
        value["max_recording_minutes"] = serde_json::json!(99);
        let err = store
            .set_section(ConfigSection::Hotkey, value)
            .await
            .unwrap_err();
        assert_eq!(err.code(), format!("E-CONFIG-{ERR_BAD_VALUE:03}"));
        assert_eq!(store.get().await.hotkey.max_recording_minutes, 5);
    }
}
