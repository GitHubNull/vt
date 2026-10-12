//! 场景识别服务（T-501，SPEC: doc/specs/v0.5/T-501-scene-detection.md）。
//!
//! 把当前前台应用分类为 PRD 定义的场景（IDE / 终端 / AI 对话 / 浏览器 / 沟通文档 / 其他），
//! 供场景规则（T-502）与终端安全保护（T-515）消费。
//! 识别信息（窗口标题等）不写入日志（NFR-S-02）。
//!
//! 分类优先级（PRD 3.2.1 业务规则 1 + SPEC 验收标准）：
//! 1. 手动 override（`override_scene`，识别受限环境 / 临时锁定兜底，最高优先）；
//! 2. 用户自定义规则（`SceneConfig.custom_rules`，应用标识 → 场景，优先于内置规则库）；
//! 3. 内置标题关键字规则（浏览器中的 AI 对话页等按窗口标题细分）；
//! 4. 内置应用规则（进程名 / bundle id / 应用名 → 场景，数据驱动 JSON，见
//!    [`BUILTIN_RULES_JSON`]，可随版本更新）；
//! 5. 未匹配 → `Other`；前台应用不可枚举（识别受限 / 空信息）→ `Unknown`。
//!
//! 缓存与变更：前台窗口查询带短缓存（[`SCENE_CACHE_TTL`]），后台轮询任务每
//! [`SCENE_POLL_INTERVAL`] 刷新一次并在场景变化时广播 `scene:changed`
//! （events.rs `AppEvent::SceneChanged`，前端事件名 `scene:changed`）。

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::error::AppError;
use crate::events::AppEvent;
use crate::platform::window_info::{AppInfo, WindowInfoProvider};
use crate::services::config_store::{ConfigSection, ConfigStore, SceneConfig, SceneRule};

/// 应用场景（PRD 3.2.1 五大类 + 其他/未知）。
///
/// 本类型被 `config_store`（SceneConfig 自定义规则 / override）引用，
/// 变体名与序列化值是跨层契约，修改须同步前端 `src/api/types.ts`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneKind {
    /// 集成开发环境（VS Code、JetBrains 系等）。
    Ide,
    /// 终端（Windows Terminal、iTerm2、GNOME Terminal 等）。
    Terminal,
    /// AI 对话页（ChatGPT、Claude、Kimi 等）。
    AiChat,
    /// 浏览器（通用网页场景）。
    Browser,
    /// 沟通与文档（IM、邮件、Office、笔记等）。
    Docs,
    /// 其他已知应用（不匹配以上分类）。
    Other,
    /// 未知：识别受限（Wayland / 权限缺失）或前台应用不可枚举。
    #[default]
    Unknown,
}

impl SceneKind {
    /// 场景标识字符串（稳定契约，前端展示与配置持久化共用）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ide => "ide",
            Self::Terminal => "terminal",
            Self::AiChat => "ai_chat",
            Self::Browser => "browser",
            Self::Docs => "docs",
            Self::Other => "other",
            Self::Unknown => "unknown",
        }
    }
}

/// 场景查询短缓存 TTL：该窗口内的重复查询不触发 OS 前台窗口 API
/// （录音开始时注入场景的查询与前端轮询共享，OS 查询本身毫秒级）。
pub const SCENE_CACHE_TTL: Duration = Duration::from_millis(300);

/// 后台场景轮询间隔（PRD 3.2.1：场景在录音开始时确定；轮询只为驱动
/// `scene:changed` 广播与设置页"当前识别到"展示，500 ms 对人感知即时）。
pub const SCENE_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// 自定义规则条数上限（与替换词典同一量级，防配置膨胀）。
pub const MAX_CUSTOM_RULES: usize = 100;

/// 错误编号：自定义规则数量已达上限。
pub const ERR_RULE_LIMIT: u32 = 1;
/// 错误编号：自定义规则非法（应用标识为空 / 过长 / 重复 / 目标场景为 Unknown）。
pub const ERR_BAD_RULE: u32 = 2;
/// 应用标识最大长度（字符数）。
pub const MAX_RULE_APP_LEN: usize = 100;

/// 内置规则库（数据驱动 JSON，tech/09 9.2.3；可随版本更新——版本号见 `version` 字段）。
///
/// 覆盖 PRD 3.2.1 五大类代表应用（识别键 = 进程名 / bundle id / 应用名，标题 =
/// 窗口标题包含关键字；均大小写不敏感）。更新本 JSON 即可扩充识别清单，无需改代码。
pub const BUILTIN_RULES_JSON: &str = r#"{
  "version": 1,
  "app_rules": [
    { "process": "code", "scene": "ide" },
    { "process": "code-insiders", "scene": "ide" },
    { "process": "code-oss", "scene": "ide" },
    { "process": "codium", "scene": "ide" },
    { "process": "cursor", "scene": "ide" },
    { "process": "windsurf", "scene": "ide" },
    { "process": "devenv", "scene": "ide" },
    { "process": "sublime_text", "scene": "ide" },
    { "process": "zed", "scene": "ide" },
    { "process": "neovide", "scene": "ide" },
    { "process": "helix", "scene": "ide" },
    { "process": "emacs", "scene": "ide" },
    { "process": "emacsclient", "scene": "ide" },
    { "process": "nvim", "scene": "ide" },
    { "process": "vim", "scene": "ide" },
    { "process": "idea", "scene": "ide" },
    { "process": "idea64", "scene": "ide" },
    { "process": "webstorm", "scene": "ide" },
    { "process": "webstorm64", "scene": "ide" },
    { "process": "pycharm", "scene": "ide" },
    { "process": "pycharm64", "scene": "ide" },
    { "process": "clion", "scene": "ide" },
    { "process": "clion64", "scene": "ide" },
    { "process": "goland", "scene": "ide" },
    { "process": "goland64", "scene": "ide" },
    { "process": "rider", "scene": "ide" },
    { "process": "rider64", "scene": "ide" },
    { "process": "phpstorm", "scene": "ide" },
    { "process": "phpstorm64", "scene": "ide" },
    { "process": "rubymine", "scene": "ide" },
    { "process": "rubymine64", "scene": "ide" },
    { "process": "datagrip", "scene": "ide" },
    { "process": "datagrip64", "scene": "ide" },
    { "process": "studio", "scene": "ide" },
    { "process": "studio64", "scene": "ide" },
    { "process": "com.microsoft.VSCode", "scene": "ide" },
    { "process": "com.microsoft.VSCodeInsiders", "scene": "ide" },
    { "process": "com.apple.dt.Xcode", "scene": "ide" },
    { "process": "com.google.android.studio", "scene": "ide" },
    { "process": "com.jetbrains.intellij", "scene": "ide" },
    { "process": "com.jetbrains.pycharm", "scene": "ide" },
    { "process": "com.jetbrains.webstorm", "scene": "ide" },
    { "process": "com.jetbrains.clion", "scene": "ide" },
    { "process": "com.jetbrains.goland", "scene": "ide" },
    { "process": "com.jetbrains.rider", "scene": "ide" },
    { "process": "com.jetbrains.datagrip", "scene": "ide" },

    { "process": "windowsterminal", "scene": "terminal" },
    { "process": "cmd", "scene": "terminal" },
    { "process": "powershell", "scene": "terminal" },
    { "process": "pwsh", "scene": "terminal" },
    { "process": "conhost", "scene": "terminal" },
    { "process": "mintty", "scene": "terminal" },
    { "process": "putty", "scene": "terminal" },
    { "process": "alacritty", "scene": "terminal" },
    { "process": "wezterm", "scene": "terminal" },
    { "process": "wezterm-gui", "scene": "terminal" },
    { "process": "hyper", "scene": "terminal" },
    { "process": "tabby", "scene": "terminal" },
    { "process": "warp", "scene": "terminal" },
    { "process": "kitty", "scene": "terminal" },
    { "process": "ghostty", "scene": "terminal" },
    { "process": "konsole", "scene": "terminal" },
    { "process": "xterm", "scene": "terminal" },
    { "process": "uxterm", "scene": "terminal" },
    { "process": "urxvt", "scene": "terminal" },
    { "process": "rxvt", "scene": "terminal" },
    { "process": "st", "scene": "terminal" },
    { "process": "terminator", "scene": "terminal" },
    { "process": "tilix", "scene": "terminal" },
    { "process": "guake", "scene": "terminal" },
    { "process": "gnome-terminal-server", "scene": "terminal" },
    { "process": "ptyxis", "scene": "terminal" },
    { "process": "kgx", "scene": "terminal" },
    { "process": "qterminal", "scene": "terminal" },
    { "process": "mate-terminal", "scene": "terminal" },
    { "process": "xfce4-terminal", "scene": "terminal" },
    { "process": "lxterminal", "scene": "terminal" },
    { "process": "sakura", "scene": "terminal" },
    { "process": "deepin-terminal", "scene": "terminal" },
    { "process": "terminal", "scene": "terminal" },
    { "process": "iterm2", "scene": "terminal" },
    { "process": "com.apple.Terminal", "scene": "terminal" },
    { "process": "com.googlecode.iterm2", "scene": "terminal" },
    { "process": "net.kovidgoyal.kitty", "scene": "terminal" },
    { "process": "io.alacritty", "scene": "terminal" },
    { "process": "com.mitchellh.ghostty", "scene": "terminal" },
    { "process": "com.github.wez.wezterm", "scene": "terminal" },
    { "process": "dev.warp.Warp-Stable", "scene": "terminal" },
    { "process": "co.zeit.hyper", "scene": "terminal" },

    { "process": "chatgpt", "scene": "ai_chat" },
    { "process": "com.openai.chat", "scene": "ai_chat" },
    { "process": "claude", "scene": "ai_chat" },
    { "process": "com.anthropic.claudefordesktop", "scene": "ai_chat" },
    { "process": "chatbox", "scene": "ai_chat" },
    { "process": "cherry-studio", "scene": "ai_chat" },

    { "process": "chrome", "scene": "browser" },
    { "process": "chrome-dev", "scene": "browser" },
    { "process": "chromium", "scene": "browser" },
    { "process": "chromium-browser", "scene": "browser" },
    { "process": "msedge", "scene": "browser" },
    { "process": "firefox", "scene": "browser" },
    { "process": "firefox-esr", "scene": "browser" },
    { "process": "waterfox", "scene": "browser" },
    { "process": "librewolf", "scene": "browser" },
    { "process": "floorp", "scene": "browser" },
    { "process": "brave", "scene": "browser" },
    { "process": "brave-browser", "scene": "browser" },
    { "process": "vivaldi", "scene": "browser" },
    { "process": "vivaldi-stable", "scene": "browser" },
    { "process": "opera", "scene": "browser" },
    { "process": "arc", "scene": "browser" },
    { "process": "orion", "scene": "browser" },
    { "process": "safari", "scene": "browser" },
    { "process": "360se", "scene": "browser" },
    { "process": "360chrome", "scene": "browser" },
    { "process": "qqbrowser", "scene": "browser" },
    { "process": "com.google.Chrome", "scene": "browser" },
    { "process": "com.microsoft.edgemac", "scene": "browser" },
    { "process": "org.mozilla.firefox", "scene": "browser" },
    { "process": "com.brave.Browser", "scene": "browser" },
    { "process": "com.vivaldi.Vivaldi", "scene": "browser" },
    { "process": "com.operasoftware.Opera", "scene": "browser" },
    { "process": "company.thebrowser.Browser", "scene": "browser" },
    { "process": "com.apple.Safari", "scene": "browser" },

    { "process": "wechat", "scene": "docs" },
    { "process": "weixin", "scene": "docs" },
    { "process": "wxwork", "scene": "docs" },
    { "process": "dingtalk", "scene": "docs" },
    { "process": "feishu", "scene": "docs" },
    { "process": "lark", "scene": "docs" },
    { "process": "telegram", "scene": "docs" },
    { "process": "slack", "scene": "docs" },
    { "process": "discord", "scene": "docs" },
    { "process": "qq", "scene": "docs" },
    { "process": "teams", "scene": "docs" },
    { "process": "msteams", "scene": "docs" },
    { "process": "zoom", "scene": "docs" },
    { "process": "whatsapp", "scene": "docs" },
    { "process": "signal", "scene": "docs" },
    { "process": "element", "scene": "docs" },
    { "process": "thunderbird", "scene": "docs" },
    { "process": "outlook", "scene": "docs" },
    { "process": "mailmaster", "scene": "docs" },
    { "process": "winword", "scene": "docs" },
    { "process": "excel", "scene": "docs" },
    { "process": "powerpnt", "scene": "docs" },
    { "process": "onenote", "scene": "docs" },
    { "process": "notepad", "scene": "docs" },
    { "process": "notepad++", "scene": "docs" },
    { "process": "wps", "scene": "docs" },
    { "process": "wpspdf", "scene": "docs" },
    { "process": "wpp", "scene": "docs" },
    { "process": "et", "scene": "docs" },
    { "process": "wpsoffice", "scene": "docs" },
    { "process": "soffice", "scene": "docs" },
    { "process": "typora", "scene": "docs" },
    { "process": "obsidian", "scene": "docs" },
    { "process": "notion", "scene": "docs" },
    { "process": "marktext", "scene": "docs" },
    { "process": "zettlr", "scene": "docs" },
    { "process": "logseq", "scene": "docs" },
    { "process": "com.tencent.xinWeChat", "scene": "docs" },
    { "process": "com.apple.mail", "scene": "docs" },
    { "process": "com.apple.Notes", "scene": "docs" },
    { "process": "com.apple.TextEdit", "scene": "docs" },
    { "process": "com.apple.iWork.Pages", "scene": "docs" },
    { "process": "com.apple.iWork.Numbers", "scene": "docs" },
    { "process": "com.apple.iWork.Keynote", "scene": "docs" },
    { "process": "md.obsidian", "scene": "docs" },
    { "process": "notion.id", "scene": "docs" },
    { "process": "com.microsoft.Outlook", "scene": "docs" },
    { "process": "com.microsoft.teams2", "scene": "docs" }
  ],
  "title_rules": [
    { "pattern": "chatgpt", "scene": "ai_chat" },
    { "pattern": "claude", "scene": "ai_chat" },
    { "pattern": "kimi", "scene": "ai_chat" },
    { "pattern": "deepseek", "scene": "ai_chat" },
    { "pattern": "gemini", "scene": "ai_chat" },
    { "pattern": "通义千问", "scene": "ai_chat" },
    { "pattern": "豆包", "scene": "ai_chat" },
    { "pattern": "腾讯元宝", "scene": "ai_chat" },
    { "pattern": "文心一言", "scene": "ai_chat" },
    { "pattern": "智谱清言", "scene": "ai_chat" },
    { "pattern": "chatglm", "scene": "ai_chat" },
    { "pattern": "copilot", "scene": "ai_chat" },
    { "pattern": "perplexity", "scene": "ai_chat" },
    { "pattern": "grok", "scene": "ai_chat" }
  ]
}"#;

/// 内置规则库运行时形态（`BUILTIN_RULES_JSON` 解析结果，进程内单例）。
#[derive(Debug, Clone, Deserialize)]
pub struct BuiltinRules {
    /// 规则库版本（随版本更新时递增）。
    #[allow(dead_code)]
    pub version: u32,
    /// 应用规则：进程名 / bundle id / 应用名 → 场景（等值匹配，大小写不敏感）。
    pub app_rules: Vec<BuiltinAppRule>,
    /// 标题规则：窗口标题包含关键字 → 场景（子串匹配，大小写不敏感）。
    pub title_rules: Vec<BuiltinTitleRule>,
}

/// 内置应用规则。
#[derive(Debug, Clone, Deserialize)]
pub struct BuiltinAppRule {
    /// 识别键：Windows / Linux 为进程可执行名，macOS 为 bundle id，
    /// 亦可填应用显示名（匹配时与 `AppInfo::process` / `AppInfo::name` 双字段比对）。
    pub process: String,
    /// 目标场景。
    pub scene: SceneKind,
}

/// 内置标题关键字规则。
#[derive(Debug, Clone, Deserialize)]
pub struct BuiltinTitleRule {
    /// 窗口标题包含关键字（应为小写；匹配时对标题做小写化后子串比对）。
    pub pattern: String,
    /// 目标场景。
    pub scene: SceneKind,
}

/// 内置规则库单例（启动后首次访问解析；解析失败视为程序缺陷，回退空规则库并 warn
/// ——不 panic，识别降级为 Other 场景）。
fn builtin_rules() -> &'static BuiltinRules {
    static BUILTIN: OnceLock<BuiltinRules> = OnceLock::new();
    BUILTIN.get_or_init(|| {
        match serde_json::from_str(BUILTIN_RULES_JSON) {
            Ok(rules) => rules,
            Err(err) => {
                // NFR-S-02：日志只记录解析错误，不记录规则内容。
                tracing::warn!(error = %err, "内置场景规则库解析失败，回退空规则库");
                BuiltinRules {
                    version: 0,
                    app_rules: Vec::new(),
                    title_rules: Vec::new(),
                }
            }
        }
    })
}

/// 应用 → 场景分类（纯函数，单测覆盖 PRD 3.2.1 / SPEC 验收用例）。
///
/// 优先级：自定义规则 > 内置标题规则 > 内置应用规则 > Other；
/// 前台应用信息为空（识别受限）→ `Unknown`。自定义规则的应用标识与
/// `AppInfo::process` / `AppInfo::name` 做大小写不敏感等值匹配。
pub fn classify(info: &AppInfo, custom_rules: &[SceneRule]) -> SceneKind {
    if info.name.is_empty() && info.process.is_empty() && info.title.is_empty() {
        return SceneKind::Unknown;
    }
    // 1. 自定义规则（列表顺序即优先级，首个命中返回）。
    for rule in custom_rules {
        let key = rule.app.trim().to_lowercase();
        if key.is_empty() {
            continue;
        }
        if info.process.to_lowercase() == key || info.name.to_lowercase() == key {
            return rule.scene;
        }
    }
    let builtin = builtin_rules();
    // 2. 内置标题关键字规则（浏览器中的 AI 对话页等，PRD 3.2.1 业务规则 1：
    //    标题关键字规则优先于应用规则）。
    let title_lower = info.title.to_lowercase();
    for rule in &builtin.title_rules {
        if !rule.pattern.is_empty() && title_lower.contains(&rule.pattern) {
            return rule.scene;
        }
    }
    // 3. 内置应用规则（进程名 / bundle id / 应用名等值匹配，大小写不敏感）。
    for rule in &builtin.app_rules {
        if info.process.eq_ignore_ascii_case(&rule.process)
            || info.name.eq_ignore_ascii_case(&rule.process)
        {
            return rule.scene;
        }
    }
    // 4. 未匹配 → 其他已知应用。
    SceneKind::Other
}

/// 场景判定来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneSource {
    /// 手动指定（override，识别受限环境 / 临时锁定兜底）。
    Override,
    /// 自动识别（内置规则库 + 自定义规则）。
    Auto,
}

/// 当前场景状态（`scene_get_current` 返回载荷，前端"当前识别到：×× → ××场景"
/// 展示数据源；`app` / `process` 为展示用途，服务端绝不写入日志，NFR-S-02）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SceneStatus {
    /// 当前生效场景。
    pub scene: SceneKind,
    /// 判定来源。
    pub source: SceneSource,
    /// 前台应用显示名（识别受限 / override 时为空）。
    pub app: String,
    /// 前台应用进程名 / bundle id（识别受限 / override 时为空）。
    pub process: String,
}

/// 场景查询短缓存条目（TTL 见 [`SCENE_CACHE_TTL`]）。
#[derive(Debug, Clone)]
struct CacheEntry {
    at: Instant,
    scene: SceneKind,
    info: AppInfo,
}

/// SceneService：前台应用 → 场景分类引擎（内置规则库 + 自定义规则 + override）。
///
/// `Clone` 句柄共享缓存与广播通道；`start` 启动后台轮询（场景变化广播
/// `scene:changed`），命令层经 `AppState` 共享同一实例。
#[derive(Clone)]
pub struct SceneService {
    config: ConfigStore,
    provider: Arc<dyn WindowInfoProvider>,
    events: Arc<tokio::sync::broadcast::Sender<AppEvent>>,
    cache: Arc<Mutex<Option<CacheEntry>>>,
    /// 最近一次广播的场景（None = 尚未广播）。
    last_broadcast: Arc<Mutex<Option<SceneKind>>>,
}

impl SceneService {
    /// 创建场景服务（`provider` 由平台工厂 `platform::window_info::default_provider()`
    /// 提供；测试注入 mock）。
    pub fn new(
        config: ConfigStore,
        provider: Arc<dyn WindowInfoProvider>,
        events: tokio::sync::broadcast::Sender<AppEvent>,
    ) -> Self {
        Self {
            config,
            provider,
            events: Arc::new(events),
            cache: Arc::new(Mutex::new(None)),
            last_broadcast: Arc::new(Mutex::new(None)),
        }
    }

    /// 启动后台场景轮询：每 [`SCENE_POLL_INTERVAL`] 刷新一次，场景变化时广播
    /// `scene:changed`（tech/04 4.2 广播通道 → main.rs 事件桥转 Tauri Event）。
    pub fn start(self: &Arc<Self>) {
        let service = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(SCENE_POLL_INTERVAL);
            // 轮询偶发延迟（前台查询阻塞）时不补偿式连发，按固定间隔顺延。
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                service.refresh_and_broadcast().await;
            }
        });
    }

    /// 当前场景状态（override 优先；自动识别走短缓存，前端查询不触发 OS API 风暴）。
    pub async fn status(&self) -> SceneStatus {
        let config = self.config.get().await;
        if let Some(override_scene) = config.scene.override_scene {
            return SceneStatus {
                scene: override_scene,
                source: SceneSource::Override,
                app: String::new(),
                process: String::new(),
            };
        }
        let (scene, info) = self.detect_auto(&config.scene.custom_rules).await;
        SceneStatus {
            scene,
            source: SceneSource::Auto,
            app: info.name,
            process: info.process,
        }
    }

    /// 设置手动场景指定（PRD 3.2.1 业务规则 3 临时锁定；Wayland 等识别受限兜底，
    /// SPEC T-501 步骤 4）。持久化到 `scene.override_scene` 并即时广播场景变化。
    pub async fn set_override(&self, scene: SceneKind) -> Result<(), AppError> {
        if scene == SceneKind::Unknown {
            return Err(AppError::Config(
                ERR_BAD_RULE,
                "不能锁定为未知场景".to_string(),
            ));
        }
        self.update_scene_config(|scene_config| {
            scene_config.override_scene = Some(scene);
        })
        .await?;
        self.refresh_and_broadcast().await;
        info!(scene = scene.as_str(), "场景已手动锁定");
        Ok(())
    }

    /// 清除手动指定，恢复自动识别；即时广播场景变化。
    pub async fn clear_override(&self) -> Result<(), AppError> {
        self.update_scene_config(|scene_config| {
            scene_config.override_scene = None;
        })
        .await?;
        self.refresh_and_broadcast().await;
        info!("场景手动锁定已清除，恢复自动识别");
        Ok(())
    }

    /// 自定义规则清单（`SceneConfig.custom_rules` 镜像）。
    pub async fn list_custom_rules(&self) -> Vec<SceneRule> {
        self.config.get().await.scene.custom_rules
    }

    /// 新增自定义规则（应用标识 → 场景；优先于内置规则库）。
    pub async fn add_custom_rule(
        &self,
        app: String,
        scene: SceneKind,
    ) -> Result<SceneRule, AppError> {
        let app = app.trim().to_string();
        validate_custom_rule(&app, scene)?;
        self.update_scene_config(|scene_config| {
            // 大小写不敏感去重（同一应用标识只保留一条）。
            let key = app.to_lowercase();
            scene_config
                .custom_rules
                .retain(|rule| rule.app.trim().to_lowercase() != key);
            let rule = SceneRule {
                app: app.clone(),
                scene,
            };
            scene_config.custom_rules.push(rule);
        })
        .await?;
        self.refresh_and_broadcast().await;
        let count = self.list_custom_rules().await.len();
        info!(count, "场景自定义规则已保存");
        Ok(SceneRule { app, scene })
    }

    /// 更新自定义规则（按应用标识匹配原规则，整体替换目标场景）。
    pub async fn update_custom_rule(&self, rule: SceneRule) -> Result<(), AppError> {
        let app = rule.app.trim().to_string();
        validate_custom_rule(&app, rule.scene)?;
        let key = app.to_lowercase();
        // 先确认规则存在，再做合法修改（避免空写盘与多余广播）。
        if !self
            .config
            .get()
            .await
            .scene
            .custom_rules
            .iter()
            .any(|existing| existing.app.trim().to_lowercase() == key)
        {
            return Err(AppError::Config(ERR_BAD_RULE, "规则不存在".to_string()));
        }
        self.update_scene_config(|scene_config| {
            for existing in &mut scene_config.custom_rules {
                if existing.app.trim().to_lowercase() == key {
                    existing.scene = rule.scene;
                    break;
                }
            }
        })
        .await?;
        self.refresh_and_broadcast().await;
        Ok(())
    }

    /// 删除自定义规则（按应用标识，大小写不敏感）。
    pub async fn delete_custom_rule(&self, app: &str) -> Result<(), AppError> {
        let key = app.trim().to_lowercase();
        // 先确认规则存在，再做合法删除（避免空写盘与多余广播）。
        if !self
            .config
            .get()
            .await
            .scene
            .custom_rules
            .iter()
            .any(|existing| existing.app.trim().to_lowercase() == key)
        {
            return Err(AppError::Config(ERR_BAD_RULE, "规则不存在".to_string()));
        }
        self.update_scene_config(|scene_config| {
            scene_config
                .custom_rules
                .retain(|rule| rule.app.trim().to_lowercase() != key);
        })
        .await?;
        self.refresh_and_broadcast().await;
        let count = self.list_custom_rules().await.len();
        info!(count, "场景自定义规则已保存");
        Ok(())
    }

    /// 刷新场景判定并在变化时广播 `scene:changed`（轮询任务与规则 / override
    /// 变更后共用；变化 = 与上次广播值不同）。
    ///
    /// 日志只记录场景标识字符串，绝不记录应用名 / 窗口标题（NFR-S-02）。
    pub async fn refresh_and_broadcast(&self) -> SceneKind {
        // 规则 / override 可能已变化：绕过缓存强制重判。
        self.invalidate_cache();
        let status = self.status().await;
        let changed = {
            let mut last = match self.last_broadcast.lock() {
                Ok(last) => last,
                Err(_) => return status.scene,
            };
            if last.as_ref() == Some(&status.scene) {
                false
            } else {
                *last = Some(status.scene);
                true
            }
        };
        if changed {
            let _ = self.events.send(AppEvent::SceneChanged {
                scene: status.scene.as_str().to_string(),
            });
            info!(scene = status.scene.as_str(), "当前场景已变更");
        }
        status.scene
    }

    /// 自动识别（override 已由 `status` 处理）：短缓存命中直接返回，否则查询
    /// 前台窗口提供者并按规则库分类。
    async fn detect_auto(&self, custom_rules: &[SceneRule]) -> (SceneKind, AppInfo) {
        if let Some((scene, info)) = self.cached() {
            return (scene, info);
        }
        // 平台查询为毫秒级只读 OS 调用（见 platform/window_info 各实现的最小权限
        // 说明），同步执行即可，不值得为它引入 spawn_blocking 的任务开销。
        let info = self.provider.active_app();
        let scene = classify(&info, custom_rules);
        if let Ok(mut cache) = self.cache.lock() {
            *cache = Some(CacheEntry {
                at: Instant::now(),
                scene,
                info: info.clone(),
            });
        }
        (scene, info)
    }

    /// 读取未过期的缓存条目。
    fn cached(&self) -> Option<(SceneKind, AppInfo)> {
        let entry = self.cache.lock().ok()?.clone()?;
        (entry.at.elapsed() < SCENE_CACHE_TTL).then_some((entry.scene, entry.info))
    }

    /// 清空缓存（规则 / override 变更后强制重判）。
    fn invalidate_cache(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            *cache = None;
        }
    }

    /// 修改 `SceneConfig` 并持久化（原子落盘 + `config:changed` 广播由 ConfigStore 完成）。
    async fn update_scene_config(
        &self,
        mutate: impl FnOnce(&mut SceneConfig),
    ) -> Result<(), AppError> {
        let mut config = self.config.get().await;
        mutate(&mut config.scene);
        // 自定义规则上限校验（写入侧统一拦截，覆盖 add / update / 导入等入口）。
        if config.scene.custom_rules.len() > MAX_CUSTOM_RULES {
            return Err(AppError::Config(
                ERR_RULE_LIMIT,
                format!("场景自定义规则最多 {MAX_CUSTOM_RULES} 条，已达上限"),
            ));
        }
        let value = serde_json::to_value(&config.scene)
            .map_err(|err| AppError::Config(ERR_BAD_RULE, format!("场景配置序列化失败: {err}")))?;
        self.config.set_section(ConfigSection::Scene, value).await
    }
}

/// 自定义规则合法性校验（应用标识非空、不超长、目标场景非 Unknown）。
fn validate_custom_rule(app: &str, scene: SceneKind) -> Result<(), AppError> {
    if app.is_empty() {
        return Err(AppError::Config(
            ERR_BAD_RULE,
            "应用标识不能为空".to_string(),
        ));
    }
    if app.chars().count() > MAX_RULE_APP_LEN {
        return Err(AppError::Config(
            ERR_BAD_RULE,
            format!("应用标识过长（最多 {MAX_RULE_APP_LEN} 字）"),
        ));
    }
    if scene == SceneKind::Unknown {
        return Err(AppError::Config(
            ERR_BAD_RULE,
            "不能把应用指定为未知场景".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::window_info::MockWindowInfoProvider;

    fn app(name: &str, process: &str, title: &str) -> AppInfo {
        AppInfo {
            name: name.to_string(),
            process: process.to_string(),
            title: title.to_string(),
        }
    }

    fn custom(app: &str, scene: SceneKind) -> SceneRule {
        SceneRule {
            app: app.to_string(),
            scene,
        }
    }

    // ---------- classify：内置规则库 ----------

    #[test]
    fn empty_app_info_is_unknown() {
        assert_eq!(classify(&AppInfo::default(), &[]), SceneKind::Unknown);
    }

    /// SPEC 验收标准 1（可自动化部分）：PRD 五大类代表应用各至少 2 个识别正确；
    /// 三平台手动验证清单见任务报告。
    #[test]
    fn builtin_rules_cover_prd_five_categories() {
        let cases = [
            // IDE：VS Code / JetBrains。
            app("Visual Studio Code", "code", "main.rs - vibetype"),
            app("IntelliJ IDEA", "idea64", "project – App.java"),
            app("Visual Studio Code", "com.microsoft.VSCode", "main.rs"),
            // 终端：Windows Terminal / iTerm2 / GNOME Terminal。
            app("Windows Terminal", "WindowsTerminal", "pwsh"),
            app("iTerm2", "com.googlecode.iterm2", "ssh dev@example.com"),
            app("终端", "gnome-terminal-server", "wgh@dev: ~"),
            // 浏览器。
            app("Google Chrome", "chrome", "新标签页"),
            app("Firefox", "firefox", "Mozilla Firefox"),
            // AI 对话（独立应用 + 浏览器内对话页按标题）。
            app("ChatGPT", "com.openai.chat", "ChatGPT"),
            app("Google Chrome", "chrome", "ChatGPT - 对话"),
            app(
                "Microsoft Edge",
                "msedge",
                "DeepSeek - Enter into a new era of AI",
            ),
            // 沟通文档：IM / 文档工具。
            app("微信", "WeChat", "微信"),
            app("飞书", "Feishu", "飞书"),
            app("Obsidian", "obsidian", "笔记"),
        ];
        for info in &cases {
            assert_ne!(
                classify(info, &[]),
                SceneKind::Other,
                "应识别 {info:?} 的场景"
            );
        }
        assert_eq!(classify(&cases[0], &[]), SceneKind::Ide);
        assert_eq!(classify(&cases[1], &[]), SceneKind::Ide);
        assert_eq!(classify(&cases[3], &[]), SceneKind::Terminal);
        assert_eq!(classify(&cases[4], &[]), SceneKind::Terminal);
        assert_eq!(classify(&cases[5], &[]), SceneKind::Terminal);
        assert_eq!(classify(&cases[6], &[]), SceneKind::Browser);
        assert_eq!(classify(&cases[8], &[]), SceneKind::AiChat);
        assert_eq!(classify(&cases[9], &[]), SceneKind::AiChat);
        assert_eq!(classify(&cases[10], &[]), SceneKind::AiChat);
        assert_eq!(classify(&cases[11], &[]), SceneKind::Docs);
    }

    #[test]
    fn unmatched_app_is_other() {
        assert_eq!(
            classify(&app("计算器", "gnome-calculator", "计算器"), &[]),
            SceneKind::Other
        );
    }

    #[test]
    fn builtin_match_is_case_insensitive_and_uses_name_fallback() {
        assert_eq!(classify(&app("Code", "Code", ""), &[]), SceneKind::Ide);
        assert_eq!(classify(&app("Terminal", "", ""), &[]), SceneKind::Terminal);
    }

    // ---------- classify：优先级 ----------

    #[test]
    fn custom_rule_beats_builtin_title_and_app_rules() {
        // 自定义应用规则优先于内置标题规则：firefox 命中内置 ai_chat 标题，
        // 但用户显式指定为 browser。
        let info = app("Firefox", "firefox", "ChatGPT - 对话");
        assert_eq!(classify(&info, &[]), SceneKind::AiChat);
        assert_eq!(
            classify(&info, &[custom("firefox", SceneKind::Browser)]),
            SceneKind::Browser
        );
        // 自定义规则优先于内置应用规则：code 被指定为 docs。
        let info = app("Visual Studio Code", "code", "");
        assert_eq!(
            classify(&info, &[custom("code", SceneKind::Docs)]),
            SceneKind::Docs
        );
    }

    #[test]
    fn builtin_title_rule_beats_builtin_app_rule() {
        // PRD 3.2.1 业务规则 1：带标题关键字的规则 > 仅按应用匹配的规则。
        let info = app("Google Chrome", "chrome", "腾讯元宝");
        assert_eq!(classify(&info, &[]), SceneKind::AiChat);
    }

    #[test]
    fn custom_rule_matches_process_or_name_case_insensitively() {
        // 应用标识按进程名匹配（大小写不敏感）。
        let info = app("Visual Studio Code", "code", "");
        assert_eq!(
            classify(&info, &[custom("  CODE  ", SceneKind::Docs)]),
            SceneKind::Docs
        );
        // 应用标识按应用显示名匹配。
        let info = app("Telegram Desktop", "telegram", "");
        assert_eq!(
            classify(&info, &[custom("telegram desktop", SceneKind::Docs)]),
            SceneKind::Docs
        );
    }

    #[test]
    fn custom_rule_can_map_app_to_any_scene() {
        let info = app("Code", "code", "");
        for scene in [
            SceneKind::Ide,
            SceneKind::Terminal,
            SceneKind::Browser,
            SceneKind::Docs,
            SceneKind::AiChat,
            SceneKind::Other,
        ] {
            assert_eq!(classify(&info, &[custom("code", scene)]), scene);
        }
    }

    // ---------- 内置规则库数据完整性 ----------

    #[test]
    fn builtin_rules_json_is_valid_and_consistent() {
        let rules = builtin_rules();
        assert_eq!(rules.version, 1);
        assert!(!rules.app_rules.is_empty(), "内置应用规则不应为空");
        assert!(!rules.title_rules.is_empty(), "内置标题规则不应为空");
        for rule in &rules.app_rules {
            assert!(!rule.process.trim().is_empty(), "识别键不应为空");
            assert_ne!(rule.scene, SceneKind::Unknown, "内置规则不应指向 Unknown");
        }
        for rule in &rules.title_rules {
            assert!(!rule.pattern.trim().is_empty(), "标题关键字不应为空");
            assert_ne!(rule.scene, SceneKind::Unknown, "内置规则不应指向 Unknown");
        }
        // 同一识别键不重复登记。
        let mut keys: Vec<&str> = rules.app_rules.iter().map(|r| r.process.as_str()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), rules.app_rules.len(), "内置应用规则识别键重复");
    }

    #[test]
    fn builtin_rules_parse_is_lazy_singleton() {
        // 首次访问解析成功后不再重复解析（同一实例）。
        let first = builtin_rules();
        let second = builtin_rules();
        assert!(std::ptr::eq(first, second));
    }

    // ---------- SceneService：缓存 / override / 自定义规则 / 广播 ----------

    /// 临时目录（进程 + 纳秒唯一，析构清理）。
    struct TestDir(std::path::PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "vt-scene-test-{}-{}-{:?}",
                name,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    type TestService = (SceneService, Arc<MockWindowInfoProvider>);

    fn make_service(dir: &TestDir) -> TestService {
        let (tx, _rx) = crate::events::channel();
        let config = ConfigStore::load(&dir.0, tx.clone()).unwrap();
        let mock = Arc::new(MockWindowInfoProvider::with_app(app(
            "Visual Studio Code",
            "code",
            "",
        )));
        let service = SceneService::new(config, mock.clone(), tx);
        (service, mock)
    }

    #[tokio::test]
    async fn status_classifies_and_reports_source() {
        let dir = TestDir::new("status");
        let (service, _mock) = make_service(&dir);
        let status = service.status().await;
        assert_eq!(status.scene, SceneKind::Ide);
        assert_eq!(status.source, SceneSource::Auto);
        assert_eq!(status.app, "Visual Studio Code");
        assert_eq!(status.process, "code");
    }

    #[tokio::test]
    async fn cache_avoids_repeated_provider_calls() {
        let dir = TestDir::new("cache");
        let (service, mock) = make_service(&dir);
        service.status().await;
        service.status().await;
        assert_eq!(mock.call_count(), 1, "TTL 内重复查询应命中缓存");
        // 过期后重新查询。
        tokio::time::sleep(SCENE_CACHE_TTL + Duration::from_millis(20)).await;
        service.status().await;
        assert_eq!(mock.call_count(), 2, "缓存过期后应重新查询前台应用");
        // 切换前台应用后分类随之变化。
        mock.set_app(app("终端", "gnome-terminal-server", ""));
        tokio::time::sleep(SCENE_CACHE_TTL + Duration::from_millis(20)).await;
        assert_eq!(service.status().await.scene, SceneKind::Terminal);
    }

    #[tokio::test]
    async fn override_wins_and_clear_restores_auto() {
        let dir = TestDir::new("override");
        let (service, mock) = make_service(&dir);
        assert_eq!(service.status().await.scene, SceneKind::Ide);

        // 锁定为终端：无论前台是什么都返回终端。
        service.set_override(SceneKind::Terminal).await.unwrap();
        mock.set_app(app("Google Chrome", "chrome", ""));
        let status = service.status().await;
        assert_eq!(status.scene, SceneKind::Terminal);
        assert_eq!(status.source, SceneSource::Override);
        assert_eq!(status.app, "", "override 时不需要前台应用信息");

        // 清除后恢复自动识别（chrome → browser）。
        service.clear_override().await.unwrap();
        let status = service.status().await;
        assert_eq!(status.scene, SceneKind::Browser);
        assert_eq!(status.source, SceneSource::Auto);
    }

    #[tokio::test]
    async fn override_persists_across_restart() {
        let dir = TestDir::new("override-persist");
        {
            let (service, _mock) = make_service(&dir);
            service.set_override(SceneKind::AiChat).await.unwrap();
        }
        // 重新加载配置（模拟重启）后 override 保持。
        let (service, _mock) = make_service(&dir);
        let status = service.status().await;
        assert_eq!(status.scene, SceneKind::AiChat);
        assert_eq!(status.source, SceneSource::Override);
    }

    #[tokio::test]
    async fn override_unknown_scene_rejected() {
        let dir = TestDir::new("override-unknown");
        let (service, _mock) = make_service(&dir);
        let err = service.set_override(SceneKind::Unknown).await.unwrap_err();
        assert_eq!(err.code(), format!("E-CONFIG-{ERR_BAD_RULE:03}"));
        assert_eq!(service.status().await.source, SceneSource::Auto);
    }

    #[tokio::test]
    async fn custom_rule_crud_and_priority() {
        let dir = TestDir::new("crud");
        let (service, mock) = make_service(&dir);

        // 新增自定义规则：chrome → docs。
        let rule = service
            .add_custom_rule("chrome".to_string(), SceneKind::Docs)
            .await
            .unwrap();
        assert_eq!(rule.app, "chrome");
        assert_eq!(service.list_custom_rules().await.len(), 1);
        mock.set_app(app("Google Chrome", "chrome", ""));
        // 前台应用已切换：短缓存为模拟切换前的判定，先失效再断言。
        service.invalidate_cache();
        assert_eq!(service.status().await.scene, SceneKind::Docs);

        // 自定义规则优先于内置标题规则（标题含 chatgpt 也保持 docs）。
        mock.set_app(app("Google Chrome", "chrome", "ChatGPT - 对话"));
        service.invalidate_cache();
        assert_eq!(service.status().await.scene, SceneKind::Docs);

        // 更新规则：chrome → terminal。
        service
            .update_custom_rule(custom("chrome", SceneKind::Terminal))
            .await
            .unwrap();
        service.invalidate_cache();
        assert_eq!(service.status().await.scene, SceneKind::Terminal);

        // 删除规则后回到内置规则（恢复无标题的 chrome → browser）。
        mock.set_app(app("Google Chrome", "chrome", ""));
        service.delete_custom_rule("Chrome").await.unwrap();
        assert!(service.list_custom_rules().await.is_empty());
        service.invalidate_cache();
        assert_eq!(service.status().await.scene, SceneKind::Browser);

        // 删除 / 更新不存在的规则报错。
        assert!(service.delete_custom_rule("chrome").await.is_err());
        assert!(service
            .update_custom_rule(custom("chrome", SceneKind::Ide))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn custom_rule_validation_and_dedup() {
        let dir = TestDir::new("validate");
        let (service, _mock) = make_service(&dir);
        // 空应用标识。
        assert!(service
            .add_custom_rule("   ".to_string(), SceneKind::Ide)
            .await
            .is_err());
        // 超长应用标识。
        assert!(service
            .add_custom_rule("x".repeat(MAX_RULE_APP_LEN + 1), SceneKind::Ide)
            .await
            .is_err());
        // 目标场景 Unknown。
        assert!(service
            .add_custom_rule("code".to_string(), SceneKind::Unknown)
            .await
            .is_err());
        // 重复添加（大小写不敏感去重，覆盖式更新）。
        service
            .add_custom_rule("code".to_string(), SceneKind::Ide)
            .await
            .unwrap();
        service
            .add_custom_rule("CODE".to_string(), SceneKind::Browser)
            .await
            .unwrap();
        let rules = service.list_custom_rules().await;
        assert_eq!(rules.len(), 1, "同一应用标识应去重");
        assert_eq!(rules[0].scene, SceneKind::Browser, "重复添加按覆盖处理");
    }

    #[tokio::test]
    async fn custom_rule_limit_enforced() {
        let dir = TestDir::new("limit");
        let (service, _mock) = make_service(&dir);
        for i in 0..MAX_CUSTOM_RULES {
            service
                .add_custom_rule(format!("app-{i}"), SceneKind::Ide)
                .await
                .unwrap();
        }
        let err = service
            .add_custom_rule("one-more".to_string(), SceneKind::Ide)
            .await
            .unwrap_err();
        assert_eq!(err.code(), format!("E-CONFIG-{ERR_RULE_LIMIT:03}"));
        assert_eq!(service.list_custom_rules().await.len(), MAX_CUSTOM_RULES);
    }

    #[tokio::test]
    async fn scene_change_broadcast_on_refresh() {
        let dir = TestDir::new("broadcast");
        let (tx, mut rx) = crate::events::channel();
        let config = ConfigStore::load(&dir.0, tx.clone()).unwrap();
        let mock = Arc::new(MockWindowInfoProvider::with_app(app(
            "Visual Studio Code",
            "code",
            "",
        )));
        let service = SceneService::new(config, mock.clone(), tx);

        // 首次刷新：确立初始场景并广播（前端启动晚于服务时经命令查询兜底）。
        assert_eq!(service.refresh_and_broadcast().await, SceneKind::Ide);
        assert!(matches!(
            rx.try_recv().unwrap(),
            AppEvent::SceneChanged { ref scene } if scene == "ide"
        ));

        // 同一场景重复刷新：不重复广播。
        service.refresh_and_broadcast().await;
        assert!(rx.try_recv().is_err(), "场景未变化不应广播");

        // 切换前台应用 → 广播新场景。
        mock.set_app(app("iTerm2", "com.googlecode.iterm2", ""));
        assert_eq!(service.refresh_and_broadcast().await, SceneKind::Terminal);
        assert!(matches!(
            rx.try_recv().unwrap(),
            AppEvent::SceneChanged { ref scene } if scene == "terminal"
        ));

        // 手动锁定 → 广播锁定场景；清除 → 广播回自动识别结果。
        // 注意：override 持久化经 ConfigStore 先发 ConfigChanged，SceneChanged 在其后。
        service.set_override(SceneKind::AiChat).await.unwrap();
        assert_eq!(next_scene_changed(&mut rx), Some("ai_chat".to_string()));
        service.clear_override().await.unwrap();
        assert_eq!(next_scene_changed(&mut rx), Some("terminal".to_string()));
    }

    /// 从事件通道取出下一条 `SceneChanged`（跳过配置变更等旁路事件）。
    fn next_scene_changed(rx: &mut tokio::sync::broadcast::Receiver<AppEvent>) -> Option<String> {
        loop {
            match rx.try_recv() {
                Ok(AppEvent::SceneChanged { scene }) => return Some(scene),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    }

    #[tokio::test]
    async fn scene_change_skips_duplicate_broadcast_after_refresh() {
        let dir = TestDir::new("broadcast-dup");
        let (service, _mock) = make_service(&dir);
        service.refresh_and_broadcast().await;
        // 缓存被外部清空（模拟规则无变化的重判）后同一场景不重复广播。
        service.invalidate_cache();
        service.refresh_and_broadcast().await;
        // 通过事件通道无新消息验证（无订阅者时 send 返回 Err 也算"未广播成功"，
        // 这里直接断言 last_broadcast 状态不变）。
        let last = *service.last_broadcast.lock().unwrap();
        assert_eq!(last, Some(SceneKind::Ide));
    }
}
