//! HotkeyService（tech/04 4.4、tech/05 5.3.1）：全局快捷键注册与录音控制事件出口。
//!
//! - 消费 `platform/hotkey/` 的原始按键事件（Pressed / Released）；
//! - 按配置模式映射为录音控制命令（F-INPUT-02：按住 / 切换），经 mpsc 通道发给
//!   AudioService（tech/04 4.2 录音控制通道，容量 8）；
//! - 监听 `config:changed` 的 hotkey 节，平滑重注册：先注销再注册，失败回滚旧
//!   快捷键并广播 `hotkey:error` 事件（错误编号 E-HOTKEY-001，T-004 SPEC 步骤 4/5）；
//! - 快捷键冲突检测（F-SYS-03 / T-543）：应用内重复校验（`validate_hotkey_set`，
//!   保存被拒绝并指出冲突项）、注册失败归因（被占用 E-HOTKEY-001 / 平台级
//!   E-HOTKEY-004）与 OS 常用组合预警表（`os_preset_warnings`，仅警告允许保存）；
//! - 快捷键事件只驱动录音状态机，不做任何 UI 操作（tech/05 5.3.1）。
//!
//! 已知平台限制（tech/09 9.2.1）：
//! - Windows：单个修饰键作触发键由低级键盘钩子覆盖（platform/hotkey/windows_llhook），
//!   PRD 3.1.1 规则 3（组合键抑制）与规则 4（0.3 s 误触门）仅在钩子路径生效；
//!   组合键 / 功能键仍走 RegisterHotKey，无该门控；
//! - X11：修饰键事件依赖 global-hotkey XRecord 路径，"按了其他键则不触发"
//!   （PRD 3.1.1 规则 3）未实现，组合键场景可能误触发；
//! - Wayland：协议限制无法全局监听（v1.0 提供替代方案）。

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tracing::info;
use tracing::warn;

use crate::error::AppError;
use crate::events::AppEvent;
use crate::platform::hotkey::{
    normalize_accelerator, os_preset_conflicts, HotkeyBackend, HotkeyError, RawHotkeyEvent,
};
use crate::services::config_store::{ConfigSection, ConfigStore, HotkeyConfig, HotkeyMode};

/// 快捷键注册失败错误编号（T-004 SPEC 步骤 5；T-543 归因：被占用 / 系统保留）。
pub const ERR_REGISTER_FAILED: u32 = 1;
/// 快捷键串非法（组合格式无法解析，E-HOTKEY-002）。
pub const ERR_INVALID_SPEC: u32 = 2;
/// 应用内快捷键冲突（重复，保存被拒绝，E-HOTKEY-003，T-543）。
pub const ERR_DUPLICATE: u32 = 3;
/// 平台级注册失败（低级钩子安装失败 / 系统拒绝等，非占用归因，E-HOTKEY-004，T-543）。
pub const ERR_PLATFORM_REGISTER: u32 = 4;

// 错误编号速查（tech/04 4.6：编号出现在用户可见提示，映射文案在 src/i18n/）：
// - E-HOTKEY-001 注册失败：已被其他应用占用 / 系统保留（三平台注册冲突路径，
//   见 platform::hotkey::classify_register_error 归因）；
// - E-HOTKEY-002 快捷键串非法（含鼠标侧键暂不支持）；
// - E-HOTKEY-003 应用内快捷键重复：保存被拒绝并指出冲突项（T-543）；
// - E-HOTKEY-004 平台级注册失败：非占用的平台错误，保留原始信息供诊断（T-543）。

/// 录音控制命令（T-004 SPEC 步骤 1）：快捷键 / 命令层 / 小窗统一出口。
///
/// 由 AudioService 消费（tech/05 5.2 状态机）；`Cancel` 由 T-010 消费路径接入。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordCommand {
    Start,
    Stop,
    Toggle,
    Cancel,
}

/// 触发模式映射器（按住 / 切换的状态推导，纯逻辑可单测）。
///
/// - 按住（Hold）：Pressed → Start，Released → Stop；
/// - 切换（Toggle）：Pressed 翻转内部状态，首次 → Start、再次 → Stop；
///   Released 忽略。
#[derive(Debug, Clone)]
pub struct ModeMapper {
    mode: HotkeyMode,
    /// 切换模式下当前是否处于"已按下开始"状态。
    toggle_armed: bool,
}

impl ModeMapper {
    pub fn new(mode: HotkeyMode) -> Self {
        Self {
            mode,
            toggle_armed: false,
        }
    }

    /// 模式热切换时调用：重置内部状态，避免残留 armed 造成首按即 Stop。
    pub fn set_mode(&mut self, mode: HotkeyMode) {
        if self.mode != mode {
            self.mode = mode;
            self.toggle_armed = false;
        }
    }

    /// 映射原始按键事件为录音控制命令。
    pub fn map(&mut self, event: RawHotkeyEvent) -> Option<RecordCommand> {
        match self.mode {
            HotkeyMode::Hold => match event {
                RawHotkeyEvent::Pressed => Some(RecordCommand::Start),
                RawHotkeyEvent::Released => Some(RecordCommand::Stop),
            },
            HotkeyMode::Toggle => match event {
                RawHotkeyEvent::Pressed => {
                    self.toggle_armed = !self.toggle_armed;
                    Some(if self.toggle_armed {
                        RecordCommand::Start
                    } else {
                        RecordCommand::Stop
                    })
                }
                RawHotkeyEvent::Released => None,
            },
        }
    }
}

/// 应用内快捷键槽位标识：录音触发键（配置 `hotkey.trigger`）。
pub const SLOT_TRIGGER: &str = "trigger";
/// 应用内快捷键槽位标识：取消录音键（配置 `hotkey.cancel_trigger`）。
pub const SLOT_CANCEL: &str = "cancel_trigger";

/// 待校验的快捷键槽位（T-543 应用内冲突校验的通用入参）。
///
/// 录音触发键 / 取消键之外，将来的模板快捷键（T-534）等以追加槽位的方式
/// 进入同一套校验，无需新增错误类型或校验入口。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeySlot {
    /// 槽位标识（`trigger` / `cancel_trigger`；模板键接入后扩展 `template:<id>`）。
    pub id: String,
    /// 快捷键串（录入组件契约格式，如 `"ControlLeft+ShiftLeft+Space"`）。
    pub spec: String,
}

impl HotkeySlot {
    pub fn new(id: &str, spec: &str) -> Self {
        Self {
            id: id.to_string(),
            spec: spec.to_string(),
        }
    }
}

/// 一组应用内冲突：≥ 2 个槽位归一化后为同一组合。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HotkeyConflictGroup {
    /// 归一化后的重复组合（左右修饰键等价、修饰键顺序无关，见
    /// `platform::hotkey::normalize_accelerator`）。
    pub normalized: String,
    /// 冲突槽位 id（按槽位表顺序，≥ 2 个）。
    pub ids: Vec<String>,
}

/// 应用内快捷键冲突：保存被拒绝（E-HOTKEY-003，PRD 3.1.1 规则 7）。
#[derive(Debug, Clone, PartialEq)]
pub struct DuplicateHotkeyError {
    /// 全部冲突组（可能多组同时重复）。
    pub conflicts: Vec<HotkeyConflictGroup>,
}

impl std::fmt::Display for DuplicateHotkeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 指出冲突项（NFR：用户可见错误必须可定位）："a、b 使用了相同组合（X）"。
        write!(f, "应用内快捷键重复：")?;
        for (index, group) in self.conflicts.iter().enumerate() {
            if index > 0 {
                write!(f, "；")?;
            }
            write!(
                f,
                "{} 使用了相同组合（{}）",
                group.ids.join("、"),
                group.normalized
            )?;
        }
        Ok(())
    }
}

/// OS 常用组合预警（命中仅警告、允许保存，PRD 3.1.1 规则 7）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OsPresetWarning {
    /// 命中的槽位 id。
    pub id: String,
    /// 归一化组合。
    pub normalized: String,
    /// 用途标识（i18n 键后缀：`hotkey.conflict.osPresets.<usage>`）。
    pub usage: String,
}

/// 冲突校验报告：应用内冲突（保存被拒绝）+ OS 常用组合预警（允许保存）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HotkeyValidationReport {
    pub conflicts: Vec<HotkeyConflictGroup>,
    pub warnings: Vec<OsPresetWarning>,
}

/// 应用内冲突校验（T-543 SPEC 步骤 1）：快捷键集合（录音触发键、取消键，将来模板键）
/// 经归一化等价比较（左右修饰键等价、修饰键顺序无关）后不得重复。
///
/// 空串语义为"未设置"，与非法串（鼠标侧键、`Unidentified`、主键位出现修饰键等）
/// 一并**不参与**重复比较——非法串由注册路径的 E-HOTKEY-002 专门拒绝。
pub fn validate_hotkey_set(slots: &[HotkeySlot]) -> Result<(), DuplicateHotkeyError> {
    let mut groups: Vec<HotkeyConflictGroup> = Vec::new();
    for slot in slots {
        let Some(normalized) = normalize_accelerator(&slot.spec) else {
            continue;
        };
        match groups
            .iter_mut()
            .find(|group| group.normalized == normalized)
        {
            Some(group) => group.ids.push(slot.id.clone()),
            None => groups.push(HotkeyConflictGroup {
                normalized,
                ids: vec![slot.id.clone()],
            }),
        }
    }
    let conflicts: Vec<HotkeyConflictGroup> = groups
        .into_iter()
        .filter(|group| group.ids.len() > 1)
        .collect();
    if conflicts.is_empty() {
        Ok(())
    } else {
        Err(DuplicateHotkeyError { conflicts })
    }
}

/// 校验配置节中的全部快捷键（录音触发键 + 取消键）无重复。
///
/// `commands/config.rs` 保存 hotkey 节前的权威校验入口（E-HOTKEY-003 拒绝）；
/// 模板快捷键（T-534）接入后在其保存路径按同一函数追加槽位。
pub fn validate_hotkey_config(config: &HotkeyConfig) -> Result<(), DuplicateHotkeyError> {
    validate_hotkey_set(&[
        HotkeySlot::new(SLOT_TRIGGER, &config.trigger),
        HotkeySlot::new(SLOT_CANCEL, &config.cancel_trigger),
    ])
}

/// 对快捷键集合做 OS 常用组合预警匹配（T-543 SPEC 步骤 3；三平台清单见
/// `platform::hotkey::os_presets`）。只提示不拒绝：保存不受影响。
pub fn os_preset_warnings(slots: &[HotkeySlot]) -> Vec<OsPresetWarning> {
    let mut warnings = Vec::new();
    for slot in slots {
        let Some(normalized) = normalize_accelerator(&slot.spec) else {
            continue;
        };
        for usage in os_preset_conflicts(&normalized) {
            warnings.push(OsPresetWarning {
                id: slot.id.clone(),
                normalized: normalized.clone(),
                usage: usage.to_string(),
            });
        }
    }
    warnings
}

/// 通用校验入口（`validate_hotkeys` 命令与配置保存共用）：
/// 一次调用同时给出应用内冲突组与逐槽位 OS 预警。
pub fn validate_hotkeys(slots: &[HotkeySlot]) -> HotkeyValidationReport {
    let conflicts = validate_hotkey_set(slots)
        .err()
        .map(|err| err.conflicts)
        .unwrap_or_default();
    let warnings = os_preset_warnings(slots);
    HotkeyValidationReport {
        conflicts,
        warnings,
    }
}

/// HotkeyService：持有后端与注册状态，事件循环在 `start()` 中运行。
pub struct HotkeyService {
    backend: Arc<dyn HotkeyBackend>,
    events: broadcast::Sender<AppEvent>,
    record_tx: mpsc::Sender<RecordCommand>,
    /// 当前已注册快捷键与模式（重注册 / 回滚的依据）。
    inner: Arc<tokio::sync::Mutex<Inner>>,
}

struct Inner {
    accelerator: String,
    mapper: ModeMapper,
}

impl HotkeyService {
    pub fn new(
        backend: Arc<dyn HotkeyBackend>,
        events: broadcast::Sender<AppEvent>,
        record_tx: mpsc::Sender<RecordCommand>,
        initial: &HotkeyConfig,
    ) -> Self {
        Self {
            backend,
            events,
            record_tx,
            inner: Arc::new(tokio::sync::Mutex::new(Inner {
                accelerator: initial.trigger.clone(),
                mapper: ModeMapper::new(initial.mode),
            })),
        }
    }

    /// 按配置注册快捷键；失败时回滚旧快捷键并返回错误（调用方广播错误事件）。
    pub async fn apply_config(&self, config: &HotkeyConfig) -> Result<(), AppError> {
        // 先注销再注册（T-004 SPEC 步骤 4）；注册失败回滚旧配置。
        self.backend
            .unregister_all()
            .map_err(|err| AppError::Hotkey(ERR_REGISTER_FAILED, err.to_string()))?;
        if let Err(err) = self.backend.register(&config.trigger) {
            // 回滚：重新注册旧快捷键。
            let old = self.inner.lock().await.accelerator.clone();
            warn!(error = %err, "快捷键注册失败，回滚旧快捷键");
            if let Err(rollback_err) = self.backend.register(&old) {
                warn!(error = %rollback_err, "回滚旧快捷键也失败");
            }
            return Err(match err {
                // 串非法 / 鼠标侧键等输入侧问题：错误编号区分，前端给引导提示。
                HotkeyError::InvalidAccelerator(_) | HotkeyError::MouseKeyUnsupported(_) => {
                    AppError::Hotkey(ERR_INVALID_SPEC, err.to_string())
                }
                // 平台级失败（低级钩子安装失败 / 系统拒绝等），保留原始信息供诊断（E-HOTKEY-004）。
                HotkeyError::Platform(details) => AppError::Hotkey(
                    ERR_PLATFORM_REGISTER,
                    format!("快捷键 {} 注册失败：{details}", config.trigger),
                ),
                // 被其他应用占用 / 系统保留（三平台注册冲突路径的统一归因，E-HOTKEY-001）。
                HotkeyError::RegisterFailed => AppError::Hotkey(
                    ERR_REGISTER_FAILED,
                    format!(
                        "快捷键 {} 注册失败：可能已被其他应用占用或被系统保留，请更换按键",
                        config.trigger
                    ),
                ),
            });
        }
        let mut inner = self.inner.lock().await;
        inner.accelerator = config.trigger.clone();
        inner.mapper.set_mode(config.mode);
        info!("快捷键已注册");
        Ok(())
    }

    /// 启动事件循环：原始按键事件 → 模式映射 → 录音控制通道；
    /// 同时订阅 `config:changed` 做热重注册。
    pub fn start(
        self: Arc<Self>,
        config: ConfigStore,
        mut raw_rx: mpsc::Receiver<RawHotkeyEvent>,
        mut config_rx: broadcast::Receiver<AppEvent>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(raw) = raw_rx.recv() => {
                        let cmd = {
                            let mut inner = self.inner.lock().await;
                            inner.mapper.map(raw)
                        };
                        if let Some(cmd) = cmd {
                            // 控制类通道满时返回错误而非丢弃（tech/04 4.2 背压原则）。
                            if self.record_tx.send(cmd).await.is_err() {
                                warn!("录音控制通道接收端已关闭");
                                break;
                            }
                        }
                    }
                    changed = config_rx.recv() => {
                        match changed {
                            Ok(AppEvent::ConfigChanged { section: ConfigSection::Hotkey }) => {
                                let hotkey = config.get().await.hotkey;
                                if let Err(err) = self.apply_config(&hotkey).await {
                                    // 注册失败：错误编号经事件通知前端（T-004 SPEC 验收 4）。
                                    let _ = self.events.send(AppEvent::HotkeyError {
                                        code: err.code(),
                                        message: err.message().to_string(),
                                    });
                                }
                            }
                            Ok(_) => {} // 其他节的配置变更与本服务无关
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                warn!("配置变更事件滞后，热更新可能延迟");
                            }
                            Err(_) => break, // 发送端全部关闭
                        }
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::hotkey::MockHotkeyBackend;

    #[test]
    fn hold_mode_maps_press_and_release() {
        let mut mapper = ModeMapper::new(HotkeyMode::Hold);
        assert_eq!(
            mapper.map(RawHotkeyEvent::Pressed),
            Some(RecordCommand::Start)
        );
        assert_eq!(
            mapper.map(RawHotkeyEvent::Released),
            Some(RecordCommand::Stop)
        );
    }

    #[test]
    fn toggle_mode_flips_on_each_press() {
        let mut mapper = ModeMapper::new(HotkeyMode::Toggle);
        assert_eq!(
            mapper.map(RawHotkeyEvent::Pressed),
            Some(RecordCommand::Start)
        );
        // 切换模式下 Released 不产生命令。
        assert_eq!(mapper.map(RawHotkeyEvent::Released), None);
        assert_eq!(
            mapper.map(RawHotkeyEvent::Pressed),
            Some(RecordCommand::Stop)
        );
        assert_eq!(
            mapper.map(RawHotkeyEvent::Pressed),
            Some(RecordCommand::Start)
        );
    }

    #[test]
    fn mode_switch_resets_toggle_state() {
        let mut mapper = ModeMapper::new(HotkeyMode::Toggle);
        assert_eq!(
            mapper.map(RawHotkeyEvent::Pressed),
            Some(RecordCommand::Start)
        );
        mapper.set_mode(HotkeyMode::Hold);
        mapper.set_mode(HotkeyMode::Toggle);
        // 重置后首按应为 Start 而非 Stop。
        assert_eq!(
            mapper.map(RawHotkeyEvent::Pressed),
            Some(RecordCommand::Start)
        );
    }

    fn test_service(
        backend: Arc<MockHotkeyBackend>,
    ) -> (Arc<HotkeyService>, mpsc::Receiver<RecordCommand>) {
        let (record_tx, record_rx) = mpsc::channel(8);
        let (events, _rx) = broadcast::channel(16);
        let service = Arc::new(HotkeyService::new(
            backend,
            events,
            record_tx,
            &HotkeyConfig::default(),
        ));
        (service, record_rx)
    }

    #[tokio::test]
    async fn apply_config_registers_new_accelerator() {
        let backend = Arc::new(MockHotkeyBackend::new());
        let (service, _rx) = test_service(backend.clone());
        let config = HotkeyConfig {
            trigger: "ControlLeft+ShiftLeft+Space".to_string(),
            mode: HotkeyMode::Toggle,
            ..Default::default()
        };
        service.apply_config(&config).await.unwrap();
        assert_eq!(
            backend.current().as_deref(),
            Some("ControlLeft+ShiftLeft+Space")
        );
        // 内部模式已同步：切换模式映射生效。
        let cmd = {
            let mut inner = service.inner.lock().await;
            inner.mapper.map(RawHotkeyEvent::Pressed)
        };
        assert_eq!(cmd, Some(RecordCommand::Start));
    }

    #[tokio::test]
    async fn register_failure_rolls_back_old_accelerator() {
        let backend = Arc::new(MockHotkeyBackend::new());
        let (service, _rx) = test_service(backend.clone());
        // 初始注册右 Alt。
        service
            .apply_config(&HotkeyConfig::default())
            .await
            .unwrap();
        // 新键被占用：注册失败，回滚旧键。
        backend
            .fail_next
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let config = HotkeyConfig {
            trigger: "F13".to_string(),
            ..Default::default()
        };
        let err = service.apply_config(&config).await.unwrap_err();
        assert_eq!(err.code(), "E-HOTKEY-001");
        // 注销后被清空，再回滚重新注册旧键（默认 Alt+R）。
        assert_eq!(backend.current().as_deref(), Some("AltLeft+KeyR"));
    }

    #[tokio::test]
    async fn config_change_event_triggers_reregistration() {
        let backend = Arc::new(MockHotkeyBackend::new());
        let (service, mut record_rx) = test_service(backend.clone());
        let (config_dir, _keep) = {
            let dir = std::env::temp_dir().join(format!("vt-hotkey-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            (dir, ())
        };
        let (tx, _rx) = crate::events::channel();
        let config = ConfigStore::load(&config_dir, tx.clone()).unwrap();
        let (raw_tx, raw_rx) = mpsc::channel(8);
        let handle = service
            .clone()
            .start(config.clone(), raw_rx, tx.subscribe());

        // 初始注册。
        service
            .apply_config(&HotkeyConfig::default())
            .await
            .unwrap();
        // 模拟按键 → 录音控制命令出口。
        raw_tx.send(RawHotkeyEvent::Pressed).await.unwrap();
        assert_eq!(record_rx.recv().await, Some(RecordCommand::Start));

        // 修改配置（热更新）→ 服务自动重注册 F2。
        let mut value = serde_json::to_value(config.get().await.hotkey).unwrap();
        value["trigger"] = serde_json::json!("F2");
        config
            .set_section(ConfigSection::Hotkey, value)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(backend.current().as_deref(), Some("F2"));

        handle.abort();
        let _ = std::fs::remove_dir_all(&config_dir);
    }

    #[tokio::test]
    async fn platform_register_failure_reports_e_hotkey_004() {
        let backend = Arc::new(MockHotkeyBackend::new());
        let (service, _rx) = test_service(backend.clone());
        service
            .apply_config(&HotkeyConfig::default())
            .await
            .unwrap();
        // 平台级失败（非占用归因，如低级钩子安装失败）→ E-HOTKEY-004 且保留诊断信息。
        *backend.next_error.lock().unwrap() = Some(HotkeyError::Platform(
            "钩子线程启动失败: thread pool exhausted".to_string(),
        ));
        let config = HotkeyConfig {
            trigger: "F13".to_string(),
            ..Default::default()
        };
        let err = service.apply_config(&config).await.unwrap_err();
        assert_eq!(err.code(), "E-HOTKEY-004");
        assert!(err.message().contains("钩子线程启动失败"), "保留诊断信息");
        // 回滚仍生效：旧键（默认 Alt+R）重新注册。
        assert_eq!(backend.current().as_deref(), Some("AltLeft+KeyR"));
    }

    #[tokio::test]
    async fn occupied_register_failure_keeps_e_hotkey_001_with_guidance() {
        let backend = Arc::new(MockHotkeyBackend::new());
        let (service, _rx) = test_service(backend.clone());
        service
            .apply_config(&HotkeyConfig::default())
            .await
            .unwrap();
        backend
            .fail_next
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let config = HotkeyConfig {
            trigger: "F13".to_string(),
            ..Default::default()
        };
        let err = service.apply_config(&config).await.unwrap_err();
        // 被占用归因不变（E-HOTKEY-001），提示含"占用"引导。
        assert_eq!(err.code(), "E-HOTKEY-001");
        assert!(err.message().contains("占用"));
    }

    fn slot(id: &str, spec: &str) -> HotkeySlot {
        HotkeySlot::new(id, spec)
    }

    #[test]
    fn validate_accepts_distinct_hotkeys() {
        let slots = [
            slot(SLOT_TRIGGER, "AltLeft+KeyR"),
            slot(SLOT_CANCEL, "Escape"),
        ];
        assert!(validate_hotkey_set(&slots).is_ok());
        // 默认配置（触发键 + Esc）天然无冲突。
        assert!(validate_hotkey_config(&HotkeyConfig::default()).is_ok());
    }

    #[test]
    fn validate_rejects_duplicates_and_names_conflicts() {
        let slots = [
            slot(SLOT_TRIGGER, "ControlLeft+ShiftLeft+Space"),
            slot(SLOT_CANCEL, "ControlLeft+ShiftLeft+Space"),
        ];
        let err = validate_hotkey_set(&slots).unwrap_err();
        assert_eq!(err.conflicts.len(), 1);
        let group = &err.conflicts[0];
        assert_eq!(group.normalized, "Control+Shift+Space");
        assert_eq!(group.ids, vec![SLOT_TRIGGER, SLOT_CANCEL]);
        // Display 指出冲突项（NFR：错误可定位）。
        let text = err.to_string();
        assert!(text.contains(SLOT_TRIGGER) && text.contains(SLOT_CANCEL));
        assert!(text.contains("Control+Shift+Space"));
    }

    #[test]
    fn validate_equates_left_right_modifiers_and_modifier_order() {
        // 左右修饰键等价（平台不区分修饰位，tech/09 9.2.1）。
        let slots = [
            slot(SLOT_TRIGGER, "AltLeft+KeyR"),
            slot(SLOT_CANCEL, "AltRight+KeyR"),
        ];
        assert_eq!(
            validate_hotkey_set(&slots).unwrap_err().conflicts[0].normalized,
            "Alt+KeyR"
        );
        // 修饰键顺序无关。
        let slots = [
            slot(SLOT_TRIGGER, "ShiftLeft+ControlLeft+Space"),
            slot(SLOT_CANCEL, "ControlRight+ShiftRight+Space"),
        ];
        assert!(validate_hotkey_set(&slots).is_err());
        // 单修饰键作触发键：主键区分左右（物理键不同），同侧才判重。
        let slots = [
            slot(SLOT_TRIGGER, "AltRight"),
            slot(SLOT_CANCEL, "AltRight"),
        ];
        assert!(validate_hotkey_set(&slots).is_err());
        let slots = [slot(SLOT_TRIGGER, "AltRight"), slot(SLOT_CANCEL, "AltLeft")];
        assert!(validate_hotkey_set(&slots).is_ok());
    }

    #[test]
    fn validate_ignores_empty_and_invalid_specs() {
        // 空键 = 未设置；多槽位同为空不构成冲突。
        let slots = [slot(SLOT_TRIGGER, ""), slot(SLOT_CANCEL, "")];
        assert!(validate_hotkey_set(&slots).is_ok());
        // 非法串（鼠标侧键 / Unidentified / 修饰键占主键位）不参与比较，由注册路径拒绝。
        let slots = [
            slot(SLOT_TRIGGER, "MouseBack"),
            slot(SLOT_CANCEL, "MouseBack"),
        ];
        assert!(validate_hotkey_set(&slots).is_ok());
        let slots = [
            slot(SLOT_TRIGGER, "AltLeft+Unidentified"),
            slot(SLOT_CANCEL, "AltRight+Unidentified"),
        ];
        assert!(validate_hotkey_set(&slots).is_ok());
        let slots = [
            slot(SLOT_TRIGGER, "KeyC+KeyD"),
            slot(SLOT_CANCEL, "KeyC+KeyD"),
        ];
        assert!(validate_hotkey_set(&slots).is_ok());
    }

    #[test]
    fn validate_dedupes_repeated_modifiers_before_compare() {
        // ControlLeft+ControlRight+KeyC 与 ControlRight+KeyC 同一位集合。
        let slots = [
            slot(SLOT_TRIGGER, "ControlLeft+ControlRight+KeyC"),
            slot(SLOT_CANCEL, "ControlRight+KeyC"),
        ];
        assert_eq!(
            validate_hotkey_set(&slots).unwrap_err().conflicts[0].normalized,
            "Control+KeyC"
        );
    }

    #[test]
    fn validate_reports_multiple_conflict_groups() {
        let slots = [
            slot("a", "KeyA"),
            slot("b", "KeyB"),
            slot("c", "KeyA"),
            slot("d", "KeyB"),
            slot("e", "KeyC"),
        ];
        let err = validate_hotkey_set(&slots).unwrap_err();
        assert_eq!(err.conflicts.len(), 2);
    }

    #[test]
    fn validate_hotkey_config_rejects_trigger_equal_cancel() {
        let config = HotkeyConfig {
            cancel_trigger: "AltLeft+KeyR".to_string(),
            ..Default::default()
        };
        let err = validate_hotkey_config(&config).unwrap_err();
        assert_eq!(err.conflicts[0].ids, vec![SLOT_TRIGGER, SLOT_CANCEL]);
    }

    #[test]
    fn os_preset_warnings_flag_common_combos_only() {
        // 平台相关：取当前平台表中的第一条做正例（表条目在三平台各自有维护）。
        let preset = &crate::platform::hotkey::presets()[0];
        let slots = [
            HotkeySlot::new(SLOT_TRIGGER, preset.normalized),
            slot(SLOT_CANCEL, "Escape"),
        ];
        let warnings = os_preset_warnings(&slots);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].id, SLOT_TRIGGER);
        assert_eq!(warnings[0].normalized, preset.normalized);
        assert_eq!(warnings[0].usage, preset.usage);

        // 非常用组合（默认键）与空 / 非法串均不告警。
        let slots = [
            slot(SLOT_TRIGGER, "AltLeft+KeyR"),
            slot(SLOT_CANCEL, ""),
            slot("bad", "MouseBack"),
        ];
        assert!(os_preset_warnings(&slots).is_empty());
    }

    #[test]
    fn validate_hotkeys_report_combines_conflicts_and_warnings() {
        // 平台相关构造：第一条预警条目 + 一组应用内重复（避开预警组合）。
        let preset = &crate::platform::hotkey::presets()[0];
        let slots = [
            HotkeySlot::new(SLOT_TRIGGER, preset.normalized),
            HotkeySlot::new(SLOT_CANCEL, preset.normalized),
        ];
        let report = validate_hotkeys(&slots);
        assert_eq!(report.conflicts.len(), 1);
        // 同一组合同时命中预警（冲突与预警正交：冲突拒绝保存，预警不拦截）；
        // 两个槽位都是该组合 → 各自产出一条预警。
        assert_eq!(report.warnings.len(), 2);
        assert_eq!(report.warnings[0].usage, preset.usage);
        assert_eq!(report.warnings[1].usage, preset.usage);
    }
}
