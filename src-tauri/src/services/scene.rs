//! 场景识别与规则引擎（T-501 识别，SPEC: doc/specs/v0.5/T-501-scene-detection.md；
//! T-502 规则引擎，SPEC: doc/specs/v0.5/T-502-scene-rules.md）。
//!
//! 把当前前台应用分类为 PRD 定义的场景（IDE / 终端 / AI 对话 / 浏览器 / 沟通文档 / 其他），
//! 并解析该场景生效的规则条目（`SceneEntry`：标点 / 空格 / 口令 / 自动回车 / AI 模板），
//! 供编程口令（T-511 ~ T-516）与终端安全保护（T-515）消费。
//! 识别信息（窗口标题等）不写入日志（NFR-S-02）。
//!
//! 场景分类优先级（PRD 3.2.1 业务规则 1 + SPEC 验收标准）：
//! 1. 手动 override（`override_scene`，识别受限环境 / 临时锁定兜底，最高优先）；
//! 2. 场景规则条目（T-502 `SceneConfig.entries`，内置五场景 + 自定义）：
//!    带"窗口标题关键字"的规则 > 仅按应用匹配的规则，同级按列表顺序；
//! 3. 用户自定义规则（`SceneConfig.custom_rules`，T-501 简化入口，兼容保留，
//!    应用标识 → 场景，优先于内置规则库）；
//! 4. 内置标题关键字规则（浏览器中的 AI 对话页等按窗口标题细分）；
//! 5. 内置应用规则（进程名 / bundle id / 应用名 → 场景，数据驱动 JSON，见
//!    [`BUILTIN_RULES_JSON`]，可随版本更新）；
//! 6. 未匹配 → `Other`；前台应用不可枚举（识别受限 / 空信息）→ `Unknown`。
//!
//! 规则条目（T-502）：
//! - 内置五场景默认规则以 PRD 3.2.1 表格为准（[`builtin_default_entries`]）；
//!   内置可编辑不可删除、可恢复默认；`entries` 为空表示使用内置默认；
//! - 条目匹配给出的场景优先于内置规则库：把应用加入某条目的匹配列表即完成
//!   "应用归类"，T-501 识别结果随之变化；
//! - 录音开始时经 [`SceneService::snapshot_for_recording`] 快照当前场景与生效
//!   条目（开录后改配置不影响进行中的会话），后处理管线经
//!   [`SceneService::session_snapshot`] 装入 `PipelineContext`（T-511 Stage 消费）。
//!
//! 缓存与变更：前台窗口查询带短缓存（[`SCENE_CACHE_TTL`]），后台轮询任务每
//! [`SCENE_POLL_INTERVAL`] 刷新一次并在场景变化时广播 `scene:changed`
//! （events.rs `AppEvent::SceneChanged`，前端事件名 `scene:changed`）。

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::error::AppError;
use crate::events::AppEvent;
use crate::platform::window_info::{AppInfo, WindowInfoProvider};
use crate::services::config_store::{
    CjkSpace, ConfigSection, ConfigStore, PunctStyle, SceneConfig, SceneEntry, SceneRule,
    TrailingPunct,
};

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

/// 场景规则条目数量上限（含内置五条，防配置膨胀）。
pub const MAX_SCENE_ENTRIES: usize = 50;
/// 单条目匹配应用数量上限。
pub const MAX_ENTRY_APPS: usize = 50;
/// 场景名称最大长度（字符数）。
pub const MAX_ENTRY_NAME_LEN: usize = 30;
/// 窗口标题关键字最大长度（字符数）。
pub const MAX_TITLE_KEYWORD_LEN: usize = 100;
/// 会话快照保留条数上限（防内存膨胀；识别链路远快于新会话开始，8 条足够）。
pub const MAX_SESSION_SNAPSHOTS: usize = 8;

/// 错误编号：自定义规则数量已达上限。
pub const ERR_RULE_LIMIT: u32 = 1;
/// 错误编号：自定义规则非法（应用标识为空 / 过长 / 重复 / 目标场景为 Unknown）。
pub const ERR_BAD_RULE: u32 = 2;
/// 错误编号：场景规则条目数量已达上限。
pub const ERR_ENTRY_LIMIT: u32 = 3;
/// 错误编号：场景规则条目非法（名称 / 颜色 / 场景 / 应用列表 / 关键字等）。
pub const ERR_BAD_ENTRY: u32 = 4;
/// 错误编号：内置场景不允许该操作（删除内置条目 / 恢复非内置条目的默认值）。
pub const ERR_BUILTIN_ENTRY: u32 = 5;
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

/// 内置五场景条目 ID（稳定契约：`SceneEntry.id`；顺序即默认排序）。
pub const BUILTIN_ENTRY_IDS: [&str; 5] = ["ide", "terminal", "ai_chat", "docs", "default"];

/// 内置场景默认规则（T-502，PRD 3.2.1 表格逐条落地）。
///
/// - 编码：半角标点；删除结尾标点；编程口令开启；中英文间不自动加空格；
/// - 终端：半角标点；删除结尾标点；编程口令开启；**禁止自动回车**（NFR-S-05）；
/// - AI 对话：跟随识别标点（保留中文标点）；保留结尾标点；中英文间自动加空格；
///   口令关闭；默认 AI 模板 `None`（T-533 提供模板后可选，默认关闭）；
/// - 文档沟通：全角标点；保留结尾标点；中英文间自动加空格；口令关闭（PRD 3.2.2 规则 1）；
/// - 默认：全部跟随"通用 › 文本输出"设置（`Follow`），口令关闭，允许回车。
///
/// 内置条目的匹配应用 / 标题关键字默认为空——应用归类由 T-501 内置规则库完成；
/// 用户往条目的匹配列表中增删应用即得到显式的归类规则（优先于内置规则库）。
pub fn builtin_default_entries() -> Vec<SceneEntry> {
    BUILTIN_ENTRY_IDS
        .iter()
        .filter_map(|id| builtin_default_entry(id))
        .collect()
}

/// 单个内置场景默认条目（按 ID；非内置 ID 返回 `None`）。
pub fn builtin_default_entry(id: &str) -> Option<SceneEntry> {
    let entry = match id {
        "ide" => SceneEntry {
            id: "ide".to_string(),
            name: "编码场景".to_string(),
            icon_color: "#3b82f6".to_string(),
            scene: SceneKind::Ide,
            apps: Vec::new(),
            title_keyword: String::new(),
            punct_style: PunctStyle::HalfWidth,
            trailing_punct: TrailingPunct::Remove,
            cjk_space: CjkSpace::None,
            commands: true,
            auto_enter: true,
            ai_template: None,
            builtin: true,
        },
        "terminal" => SceneEntry {
            id: "terminal".to_string(),
            name: "终端场景".to_string(),
            icon_color: "#22c55e".to_string(),
            scene: SceneKind::Terminal,
            apps: Vec::new(),
            title_keyword: String::new(),
            punct_style: PunctStyle::HalfWidth,
            trailing_punct: TrailingPunct::Remove,
            cjk_space: CjkSpace::None,
            commands: true,
            // 产品原则 4 / NFR-S-05：终端场景默认禁止自动回车。
            auto_enter: false,
            ai_template: None,
            builtin: true,
        },
        "ai_chat" => SceneEntry {
            id: "ai_chat".to_string(),
            name: "AI 对话场景".to_string(),
            icon_color: "#8b5cf6".to_string(),
            scene: SceneKind::AiChat,
            apps: Vec::new(),
            title_keyword: String::new(),
            punct_style: PunctStyle::Follow,
            trailing_punct: TrailingPunct::Keep,
            cjk_space: CjkSpace::Auto,
            commands: false,
            auto_enter: true,
            ai_template: None,
            builtin: true,
        },
        "docs" => SceneEntry {
            id: "docs".to_string(),
            name: "文档沟通场景".to_string(),
            icon_color: "#f59e0b".to_string(),
            scene: SceneKind::Docs,
            apps: Vec::new(),
            title_keyword: String::new(),
            punct_style: PunctStyle::FullWidth,
            trailing_punct: TrailingPunct::Keep,
            cjk_space: CjkSpace::Auto,
            commands: false,
            auto_enter: true,
            ai_template: None,
            builtin: true,
        },
        "default" => SceneEntry {
            id: "default".to_string(),
            name: "默认场景".to_string(),
            icon_color: "#64748b".to_string(),
            // 默认场景归属 `Other`：浏览器 / 其他 / 未知场景统一回退到本条目的规则。
            scene: SceneKind::Other,
            apps: Vec::new(),
            title_keyword: String::new(),
            punct_style: PunctStyle::Follow,
            trailing_punct: TrailingPunct::Follow,
            cjk_space: CjkSpace::Follow,
            commands: false,
            auto_enter: true,
            ai_template: None,
            builtin: true,
        },
        _ => return None,
    };
    Some(entry)
}

/// 生效条目清单：配置为空 = 使用内置默认；非空时补齐缺失的内置条目（手工改坏
/// 配置的防御性归一），并按 ID 修正内置标记（内置 ID 恒为 true，其余恒为 false）。
pub fn effective_entries(entries: &[SceneEntry]) -> Vec<SceneEntry> {
    if entries.is_empty() {
        return builtin_default_entries();
    }
    let mut list: Vec<SceneEntry> = entries.to_vec();
    for entry in &mut list {
        entry.builtin = BUILTIN_ENTRY_IDS.contains(&entry.id.as_str());
    }
    for default in builtin_default_entries() {
        if !list.iter().any(|entry| entry.id == default.id) {
            list.push(default);
        }
    }
    list
}

/// 应用标识与前台应用匹配（进程名或显示名，大小写不敏感等值）。
fn app_matches(info: &AppInfo, apps: &[String]) -> bool {
    apps.iter().any(|app| {
        let key = app.trim().to_lowercase();
        !key.is_empty() && (info.process.to_lowercase() == key || info.name.to_lowercase() == key)
    })
}

/// 按 PRD 3.2.1 业务规则 1 匹配规则条目：
/// 带"窗口标题关键字"的规则 > 仅按应用匹配的规则，同级按列表顺序。
///
/// - 标题规则：条目关键字非空且窗口标题包含关键字；条目匹配列表非空时还要求
///   应用也命中（关键字用于在已匹配应用内细分页面，如浏览器 PR 页）；
/// - 应用规则：条目关键字为空且应用命中匹配列表；
/// - 归属 `Unknown` 的条目（手工改坏配置）不参与匹配。
pub fn match_entry(info: &AppInfo, entries: &[SceneEntry]) -> Option<SceneEntry> {
    // 1. 标题关键字规则（同级按列表顺序）。
    for entry in entries {
        if entry.scene == SceneKind::Unknown {
            continue;
        }
        let keyword = entry.title_keyword.trim();
        if keyword.is_empty() {
            continue;
        }
        if !info.title.to_lowercase().contains(&keyword.to_lowercase()) {
            continue;
        }
        if !entry.apps.is_empty() && !app_matches(info, &entry.apps) {
            continue;
        }
        return Some(entry.clone());
    }
    // 2. 仅按应用匹配的规则（同级按列表顺序）。
    for entry in entries {
        if entry.scene == SceneKind::Unknown || !entry.title_keyword.trim().is_empty() {
            continue;
        }
        if app_matches(info, &entry.apps) {
            return Some(entry.clone());
        }
    }
    None
}

/// 场景类型 → 生效条目：取列表中首个归属该场景的条目（列表顺序即优先级）；
/// 浏览器 / 未知场景回退到 `Other`（默认场景条目）。
pub fn entry_for_scene(entries: &[SceneEntry], scene: SceneKind) -> Option<SceneEntry> {
    let target = match scene {
        SceneKind::Browser | SceneKind::Unknown => SceneKind::Other,
        other => other,
    };
    entries.iter().find(|entry| entry.scene == target).cloned()
}

/// 自动模式解析：规则条目匹配优先，未命中回退 T-501 内置分类（`classified`，
/// 已含 custom_rules 与内置规则库），再按场景取生效条目。
fn resolve_auto(
    info: &AppInfo,
    entries: &[SceneEntry],
    classified: SceneKind,
) -> (SceneKind, Option<SceneEntry>) {
    if let Some(matched) = match_entry(info, entries) {
        let scene = matched.scene;
        return (scene, Some(matched));
    }
    (classified, entry_for_scene(entries, classified))
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
    /// 生效场景规则条目 ID（T-502；None = 无匹配条目）。
    pub entry_id: Option<String>,
    /// 生效场景规则条目名称（T-502，展示用）。
    pub entry_name: Option<String>,
}

/// 单次录音会话的场景快照（T-502）：录音开始时捕获，贯穿识别 → 后处理链路。
///
/// 语义（SPEC T-502 验收：规则修改不影响进行中的录音会话）：开录后修改场景 /
/// 规则配置不改变本次会话的生效条目；下一次录音使用新配置。
#[derive(Debug, Clone, PartialEq)]
pub struct SceneSnapshot {
    /// 本次会话生效场景。
    pub scene: SceneKind,
    /// 判定来源（override = 锁定）。
    pub source: SceneSource,
    /// 本次会话生效的场景规则条目（None = 无匹配条目，回退通用设置 / 内置默认）。
    pub entry: Option<SceneEntry>,
}

impl SceneSnapshot {
    /// 生效条目 ID（无条目时 None；等价 `entry.id`）。
    pub fn entry_id(&self) -> Option<&str> {
        self.entry.as_ref().map(|entry| entry.id.as_str())
    }
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
    /// 录音会话场景快照（T-502，key = session_id，按时间淘汰旧条目）。
    snapshots: Arc<Mutex<VecDeque<(u64, SceneSnapshot)>>>,
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
            snapshots: Arc::new(Mutex::new(VecDeque::new())),
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
        self.resolve_current().await.0
    }

    /// 解析当前状态与生效规则条目（`status` 与录音快照共用）。
    ///
    /// override（锁定）优先：场景取锁定值，条目取该场景的首个条目（列表顺序）；
    /// 自动模式：条目规则匹配（标题关键字 > 应用，T-502）优先，未命中回退
    /// T-501 内置分类（custom_rules + 内置规则库），再按场景取生效条目。
    async fn resolve_current(&self) -> (SceneStatus, Option<SceneEntry>) {
        let config = self.config.get().await;
        let entries = effective_entries(&config.scene.entries);
        if let Some(override_scene) = config.scene.override_scene {
            let entry = entry_for_scene(&entries, override_scene);
            return (
                SceneStatus {
                    scene: override_scene,
                    source: SceneSource::Override,
                    app: String::new(),
                    process: String::new(),
                    entry_id: entry.as_ref().map(|entry| entry.id.clone()),
                    entry_name: entry.as_ref().map(|entry| entry.name.clone()),
                },
                entry,
            );
        }
        let (classified, info) = self.detect_auto(&config.scene.custom_rules).await;
        let (scene, entry) = resolve_auto(&info, &entries, classified);
        (
            SceneStatus {
                scene,
                source: SceneSource::Auto,
                app: info.name,
                process: info.process,
                entry_id: entry.as_ref().map(|entry| entry.id.clone()),
                entry_name: entry.as_ref().map(|entry| entry.name.clone()),
            },
            entry,
        )
    }

    /// 录音开始时快照当前场景与生效规则条目（T-502 核心语义：场景在录音开始时
    /// 确定，录音期间切窗口 / 改配置不改变本次会话）。
    ///
    /// 绕过短缓存重新判定（一次只读 OS 前台查询，毫秒级）；快照按 session_id
    /// 存入进程内表（最多 [`MAX_SESSION_SNAPSHOTS`] 条，淘汰最旧），供后处理
    /// 管线经 [`SceneService::session_snapshot`] 装入 `PipelineContext`。
    pub async fn snapshot_for_recording(&self, session_id: u64) -> SceneSnapshot {
        self.invalidate_cache();
        let (status, entry) = self.resolve_current().await;
        let snapshot = SceneSnapshot {
            scene: status.scene,
            source: status.source,
            entry,
        };
        if let Ok(mut store) = self.snapshots.lock() {
            store.retain(|(id, _)| *id != session_id);
            store.push_back((session_id, snapshot.clone()));
            while store.len() > MAX_SESSION_SNAPSHOTS {
                store.pop_front();
            }
        }
        // 隐私（NFR-S-02）：只记录会话与场景，不记录应用名 / 标题 / 文本。
        info!(
            session_id,
            scene = snapshot.scene.as_str(),
            "录音场景快照已捕获"
        );
        snapshot
    }

    /// 查询会话场景快照（录音开始时捕获；后处理管线装入 `PipelineContext`，
    /// T-511 Stage 消费）。会话无快照（如未接入场景服务的测试链路）返回 None。
    pub fn session_snapshot(&self, session_id: u64) -> Option<SceneSnapshot> {
        self.snapshots
            .lock()
            .ok()?
            .iter()
            .find(|(id, _)| *id == session_id)
            .map(|(_, snapshot)| snapshot.clone())
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

    /// 生效场景条目清单（配置为空 = 内置五场景默认；内置条目始终在列）。
    pub async fn list_entries(&self) -> Vec<SceneEntry> {
        effective_entries(&self.config.get().await.scene.entries)
    }

    /// 新增自定义场景条目（SPEC T-502 步骤 4：应用归类自定义）。
    ///
    /// ID 由服务端生成（`custom-N`），内置标记恒为 false；名称 / 应用列表 /
    /// 关键字先归一（裁剪、去重）再校验与落盘。
    pub async fn add_entry(&self, entry: SceneEntry) -> Result<SceneEntry, AppError> {
        let existing = self.list_entries().await;
        let mut entry = entry;
        entry.id = generate_entry_id(&existing);
        entry.builtin = false;
        sanitize_entry(&mut entry);
        validate_entry(&entry)?;
        let saved = entry.clone();
        self.update_scene_config(|scene_config| {
            let mut list = effective_entries(&scene_config.entries);
            list.push(saved.clone());
            scene_config.entries = list;
        })
        .await?;
        self.refresh_and_broadcast().await;
        let count = self.list_entries().await.len();
        info!(count, "场景条目已新增");
        Ok(entry)
    }

    /// 更新场景条目（内置条目可编辑；ID 与内置标记由服务端保持不变）。
    pub async fn update_entry(&self, entry: SceneEntry) -> Result<(), AppError> {
        let existing = self.list_entries().await;
        let Some(current) = existing.iter().find(|item| item.id == entry.id) else {
            return Err(AppError::Config(
                ERR_BAD_ENTRY,
                "场景条目不存在".to_string(),
            ));
        };
        let mut entry = entry;
        // 归一 + 校验；内置标记与内置条目的归属场景以服务端为准（前端表单不改这两项）。
        entry.builtin = current.builtin;
        if entry.builtin {
            entry.scene = current.scene;
        }
        sanitize_entry(&mut entry);
        validate_entry(&entry)?;
        let saved = entry.clone();
        self.update_scene_config(|scene_config| {
            let mut list = effective_entries(&scene_config.entries);
            if let Some(slot) = list.iter_mut().find(|item| item.id == saved.id) {
                *slot = saved.clone();
            }
            scene_config.entries = list;
        })
        .await?;
        self.refresh_and_broadcast().await;
        let count = self.list_entries().await.len();
        info!(count, "场景条目已保存");
        Ok(())
    }

    /// 删除场景条目（仅自定义条目；内置条目返回 [`ERR_BUILTIN_ENTRY`]）。
    pub async fn delete_entry(&self, entry_id: &str) -> Result<(), AppError> {
        let existing = self.list_entries().await;
        let Some(current) = existing.iter().find(|item| item.id == entry_id) else {
            return Err(AppError::Config(
                ERR_BAD_ENTRY,
                "场景条目不存在".to_string(),
            ));
        };
        if current.builtin {
            return Err(AppError::Config(
                ERR_BUILTIN_ENTRY,
                "内置场景不可删除，可恢复默认".to_string(),
            ));
        }
        let id = entry_id.to_string();
        self.update_scene_config(|scene_config| {
            let list = effective_entries(&scene_config.entries);
            scene_config.entries = list.into_iter().filter(|item| item.id != id).collect();
        })
        .await?;
        self.refresh_and_broadcast().await;
        let count = self.list_entries().await.len();
        info!(count, "场景条目已删除");
        Ok(())
    }

    /// 恢复内置场景条目为 PRD 默认规则（保持列表位置——排序即匹配优先级）。
    ///
    /// 仅内置条目支持恢复默认；`entries` 为空（从未编辑）时直接返回默认值不写盘。
    pub async fn restore_entry_default(&self, entry_id: &str) -> Result<SceneEntry, AppError> {
        let Some(default) = builtin_default_entry(entry_id) else {
            return Err(AppError::Config(
                ERR_BUILTIN_ENTRY,
                "仅内置场景支持恢复默认".to_string(),
            ));
        };
        if self.config.get().await.scene.entries.is_empty() {
            return Ok(default);
        }
        let restored = default.clone();
        self.update_scene_config(|scene_config| {
            let mut list = effective_entries(&scene_config.entries);
            if let Some(slot) = list.iter_mut().find(|item| item.id == restored.id) {
                *slot = restored.clone();
            }
            scene_config.entries = list;
        })
        .await?;
        self.refresh_and_broadcast().await;
        info!(entry = entry_id, "场景条目已恢复默认");
        Ok(default)
    }

    /// 持久化条目排序（SPEC T-502 步骤 4：拖动排序）。
    ///
    /// `entry_ids` 须为当前全部条目 ID 的一个排列（长度与去重集合一致），
    /// 否则拒绝（E-CONFIG-004）——排序影响同级规则的匹配优先级，不静默丢弃。
    pub async fn reorder_entries(&self, entry_ids: Vec<String>) -> Result<(), AppError> {
        let existing = self.list_entries().await;
        let current: HashSet<&str> = existing.iter().map(|item| item.id.as_str()).collect();
        let wanted: HashSet<&str> = entry_ids.iter().map(|id| id.as_str()).collect();
        if entry_ids.len() != existing.len() || wanted != current {
            return Err(AppError::Config(
                ERR_BAD_ENTRY,
                "排序清单与场景条目不一致".to_string(),
            ));
        }
        self.update_scene_config(|scene_config| {
            let list = effective_entries(&scene_config.entries);
            let mut ordered = Vec::with_capacity(list.len());
            for id in &entry_ids {
                if let Some(position) = list.iter().position(|item| item.id == *id) {
                    ordered.push(list[position].clone());
                }
            }
            scene_config.entries = ordered;
        })
        .await?;
        self.refresh_and_broadcast().await;
        info!(count = entry_ids.len(), "场景条目排序已保存");
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
        // 场景条目统一校验（数量 / ID 唯一性 / 字段合法性）。
        validate_entries(&config.scene.entries)?;
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

/// 条目写入前归一（用户输入宽容，存储严格）：去除首尾空白、应用列表剔除空串
/// 并按大小写不敏感去重（保序）。
fn sanitize_entry(entry: &mut SceneEntry) {
    entry.id = entry.id.trim().to_string();
    entry.name = entry.name.trim().to_string();
    entry.icon_color = entry.icon_color.trim().to_lowercase();
    entry.title_keyword = entry.title_keyword.trim().to_string();
    let mut apps = Vec::with_capacity(entry.apps.len());
    let mut seen = HashSet::new();
    for app in std::mem::take(&mut entry.apps) {
        let app = app.trim().to_string();
        if app.is_empty() {
            continue;
        }
        if !seen.insert(app.to_lowercase()) {
            continue;
        }
        apps.push(app);
    }
    entry.apps = apps;
}

/// 场景条目合法性校验（写入侧统一拦截；错误编号 E-CONFIG-004）。
fn validate_entry(entry: &SceneEntry) -> Result<(), AppError> {
    let bad = |message: String| Err(AppError::Config(ERR_BAD_ENTRY, message));
    if entry.id.is_empty() {
        return bad("场景条目 ID 不能为空".to_string());
    }
    if entry.name.is_empty() {
        return bad("场景名称不能为空".to_string());
    }
    if entry.name.chars().count() > MAX_ENTRY_NAME_LEN {
        return bad(format!("场景名称过长（最多 {MAX_ENTRY_NAME_LEN} 字）"));
    }
    if entry.scene == SceneKind::Unknown {
        return bad("不能把场景条目归属为未知场景".to_string());
    }
    if !entry.icon_color.is_empty() && !is_hex_color(&entry.icon_color) {
        return bad("图标颜色须为 #RRGGBB 格式".to_string());
    }
    if entry.apps.len() > MAX_ENTRY_APPS {
        return bad(format!("单个场景最多匹配 {MAX_ENTRY_APPS} 个应用"));
    }
    for app in &entry.apps {
        if app.is_empty() || app.chars().count() > MAX_RULE_APP_LEN {
            return bad(format!("应用标识非法（非空且最多 {MAX_RULE_APP_LEN} 字）"));
        }
    }
    if entry.title_keyword.chars().count() > MAX_TITLE_KEYWORD_LEN {
        return bad(format!("标题关键字过长（最多 {MAX_TITLE_KEYWORD_LEN} 字）"));
    }
    Ok(())
}

/// 条目清单整体校验：数量上限 + ID 唯一 + 逐条合法。
fn validate_entries(entries: &[SceneEntry]) -> Result<(), AppError> {
    if entries.len() > MAX_SCENE_ENTRIES {
        return Err(AppError::Config(
            ERR_ENTRY_LIMIT,
            format!("场景最多 {MAX_SCENE_ENTRIES} 条，已达上限"),
        ));
    }
    let mut ids = HashSet::new();
    for entry in entries {
        if !ids.insert(entry.id.as_str()) {
            return Err(AppError::Config(
                ERR_BAD_ENTRY,
                "场景条目 ID 重复".to_string(),
            ));
        }
        validate_entry(entry)?;
    }
    Ok(())
}

/// `#RRGGBB` 颜色格式校验（大小写不限；`#` 后恰 6 位十六进制）。
fn is_hex_color(value: &str) -> bool {
    let Some(hex) = value.strip_prefix('#') else {
        return false;
    };
    hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())
}

/// 生成新条目 ID（`custom-N`，取未被占用的最小序号；上限内必有空位）。
fn generate_entry_id(existing: &[SceneEntry]) -> String {
    for n in 1..=MAX_SCENE_ENTRIES {
        let id = format!("custom-{n}");
        if !existing.iter().any(|entry| entry.id == id) {
            return id;
        }
    }
    format!("custom-{}", existing.len() + 1)
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

    // ---------- 规则引擎（T-502）：内置默认条目 ----------

    fn entry_draft(id: &str, scene: SceneKind) -> SceneEntry {
        SceneEntry {
            id: id.to_string(),
            name: format!("{id} 场景"),
            scene,
            ..SceneEntry::default()
        }
    }

    /// SPEC T-502 验收：各场景默认规则与 PRD 3.2.1 表格一致。
    #[test]
    fn builtin_default_entries_match_prd_table() {
        let entries = builtin_default_entries();
        assert_eq!(entries.len(), 5);
        let by_id = |id: &str| entries.iter().find(|entry| entry.id == id).unwrap().clone();

        let ide = by_id("ide");
        assert_eq!(ide.scene, SceneKind::Ide);
        assert_eq!(ide.punct_style, PunctStyle::HalfWidth, "编码：半角标点");
        assert_eq!(
            ide.trailing_punct,
            TrailingPunct::Remove,
            "编码：删除结尾标点"
        );
        assert_eq!(ide.cjk_space, CjkSpace::None, "编码：中英文间不自动加空格");
        assert!(ide.commands, "编码：编程口令开启");
        assert!(ide.auto_enter);
        assert!(ide.builtin);

        let terminal = by_id("terminal");
        assert_eq!(terminal.scene, SceneKind::Terminal);
        assert_eq!(terminal.punct_style, PunctStyle::HalfWidth);
        assert_eq!(terminal.trailing_punct, TrailingPunct::Remove);
        assert!(terminal.commands);
        assert!(!terminal.auto_enter, "终端：默认禁止自动回车（NFR-S-05）");

        let ai = by_id("ai_chat");
        assert_eq!(ai.scene, SceneKind::AiChat);
        assert_eq!(ai.punct_style, PunctStyle::Follow, "AI 对话：保留中文标点");
        assert_eq!(ai.cjk_space, CjkSpace::Auto, "AI 对话：中英文间自动加空格");
        assert!(!ai.commands, "AI 对话：口令默认关闭");
        assert!(ai.ai_template.is_none(), "默认 AI 模板默认不指定");

        let docs = by_id("docs");
        assert_eq!(docs.scene, SceneKind::Docs);
        assert_eq!(
            docs.punct_style,
            PunctStyle::FullWidth,
            "文档沟通：全角标点"
        );
        assert_eq!(
            docs.trailing_punct,
            TrailingPunct::Keep,
            "文档沟通：保留结尾标点"
        );
        assert_eq!(docs.cjk_space, CjkSpace::Auto);
        assert!(!docs.commands, "文档沟通：口令默认关闭（PRD 3.2.2 规则 1）");

        let default = by_id("default");
        assert_eq!(default.scene, SceneKind::Other);
        assert_eq!(
            default.punct_style,
            PunctStyle::Follow,
            "默认：跟随通用设置"
        );
        assert_eq!(default.trailing_punct, TrailingPunct::Follow);
        assert_eq!(default.cjk_space, CjkSpace::Follow);

        // 内置 ID 契约：五条都可恢复默认；非内置 ID 无默认值。
        for id in BUILTIN_ENTRY_IDS {
            assert!(builtin_default_entry(id).is_some());
        }
        assert!(builtin_default_entry("custom-1").is_none());
    }

    #[test]
    fn effective_entries_materializes_and_fills_builtins() {
        // 空 = 内置默认。
        assert_eq!(effective_entries(&[]).len(), 5);
        // 非空：补齐缺失的内置条目并修正内置标记（手工改坏配置的防御性归一）。
        let mut custom = entry_draft("custom-1", SceneKind::Docs);
        custom.builtin = true; // 自定义条目被标为内置 → 归一为 false
        let list = effective_entries(&[custom]);
        assert_eq!(list.len(), 6, "缺失的内置条目应补齐");
        assert_eq!(list[0].id, "custom-1");
        assert!(!list[0].builtin, "非内置 ID 恒为自定义条目");
        assert!(list
            .iter()
            .any(|entry| entry.id == "default" && entry.builtin));
    }

    // ---------- 规则引擎（T-502）：匹配优先级 ----------

    /// SPEC T-502 验收 / PRD 3.2.1 业务规则 1：带标题关键字规则 > 仅按应用匹配 > 默认。
    #[test]
    fn match_entry_priority_keyword_over_app() {
        let mut app_rule = entry_draft("custom-1", SceneKind::Docs);
        app_rule.apps = vec!["chrome".to_string()];
        let mut keyword_rule = entry_draft("custom-2", SceneKind::Terminal);
        keyword_rule.title_keyword = "Pull Request".to_string();
        let entries = vec![app_rule, keyword_rule];
        // 标题规则优先（即使位于列表后方）。
        let pr_page = app("Google Chrome", "chrome", "PR #42 · Pull Request review");
        assert_eq!(match_entry(&pr_page, &entries).unwrap().id, "custom-2");
        // 标题不匹配 → 应用规则。
        let home_page = app("Google Chrome", "chrome", "新标签页");
        assert_eq!(match_entry(&home_page, &entries).unwrap().id, "custom-1");
        // 应用也不匹配 → 无条目（回退默认场景）。
        let firefox = app("Firefox", "firefox", "");
        assert!(match_entry(&firefox, &entries).is_none());
    }

    /// 同级按列表顺序（SPEC：支持拖动排序，排序即优先级）。
    #[test]
    fn match_entry_same_tier_uses_list_order() {
        let mut first = entry_draft("custom-1", SceneKind::Docs);
        first.apps = vec!["chrome".to_string()];
        let mut second = entry_draft("custom-2", SceneKind::Terminal);
        second.apps = vec!["chrome".to_string()];
        let info = app("Google Chrome", "chrome", "");
        assert_eq!(
            match_entry(&info, &[first.clone(), second.clone()])
                .unwrap()
                .id,
            "custom-1"
        );
        assert_eq!(match_entry(&info, &[second, first]).unwrap().id, "custom-2");

        // 标题规则同级同样按列表顺序。
        let mut first = entry_draft("custom-1", SceneKind::Docs);
        first.title_keyword = "PR".to_string();
        let mut second = entry_draft("custom-2", SceneKind::Terminal);
        second.title_keyword = "PR".to_string();
        let info = app("Google Chrome", "chrome", "PR 页面");
        assert_eq!(match_entry(&info, &[first, second]).unwrap().id, "custom-1");
    }

    /// 标题关键字是条目的收窄条件：关键字未命中时该条目整体不生效
    /// （PRD 验收：PR 规则生效，其他网页不受影响）。
    #[test]
    fn keyword_rule_requires_keyword_and_app() {
        let mut rule = entry_draft("custom-1", SceneKind::Docs);
        rule.apps = vec!["chrome".to_string()];
        rule.title_keyword = "Pull Request".to_string();
        let entries = vec![rule];
        // 关键字命中但应用不符 → 不命中（约束在指定应用内）。
        let in_firefox = app("Firefox", "firefox", "Pull Request #7");
        assert!(match_entry(&in_firefox, &entries).is_none());
        // 应用命中且关键字命中 → 命中。
        let in_chrome_pr = app("Google Chrome", "chrome", "Pull Request #7");
        assert_eq!(match_entry(&in_chrome_pr, &entries).unwrap().id, "custom-1");
        // 应用命中但关键字未命中 → 不命中。
        let other_page = app("Google Chrome", "chrome", "今日新闻");
        assert!(match_entry(&other_page, &entries).is_none());
        // 关键字大小写不敏感。
        let lower = app("Google Chrome", "chrome", "fix parser · pull request #8");
        assert_eq!(match_entry(&lower, &entries).unwrap().id, "custom-1");
    }

    #[test]
    fn entry_for_scene_falls_back_to_default_entry() {
        let entries = builtin_default_entries();
        assert_eq!(
            entry_for_scene(&entries, SceneKind::Docs).unwrap().id,
            "docs"
        );
        // 浏览器 / 其他 / 未知 → 默认场景条目（PRD：未匹配到规则的应用用默认场景）。
        assert_eq!(
            entry_for_scene(&entries, SceneKind::Browser).unwrap().id,
            "default"
        );
        assert_eq!(
            entry_for_scene(&entries, SceneKind::Other).unwrap().id,
            "default"
        );
        assert_eq!(
            entry_for_scene(&entries, SceneKind::Unknown).unwrap().id,
            "default"
        );
    }

    #[test]
    fn resolve_auto_prefers_entry_match_then_classification() {
        let entries = builtin_default_entries();
        let (scene, entry) = resolve_auto(
            &app("Visual Studio Code", "code", ""),
            &entries,
            SceneKind::Ide,
        );
        assert_eq!(scene, SceneKind::Ide);
        assert_eq!(entry.unwrap().id, "ide");

        // 条目匹配覆盖内置分类（chrome 归到终端）。
        let mut terminal = builtin_default_entry("terminal").unwrap();
        terminal.apps = vec!["chrome".to_string()];
        let entries = effective_entries(&[terminal]);
        let (scene, entry) = resolve_auto(
            &app("Google Chrome", "chrome", ""),
            &entries,
            SceneKind::Browser,
        );
        assert_eq!(scene, SceneKind::Terminal);
        assert_eq!(entry.unwrap().id, "terminal");
    }

    // ---------- 规则引擎（T-502）：条目 CRUD / 恢复默认 / 排序 ----------

    #[tokio::test]
    async fn status_reports_matched_entry() {
        let dir = TestDir::new("status-entry");
        let (service, _mock) = make_service(&dir);
        let status = service.status().await;
        assert_eq!(status.entry_id.as_deref(), Some("ide"));
        assert_eq!(status.entry_name.as_deref(), Some("编码场景"));
    }

    /// SPEC T-502 验收：自定义应用归类生效（T-501 识别结果随之变化）。
    #[tokio::test]
    async fn entry_app_rule_changes_detection() {
        let dir = TestDir::new("entry-classify");
        let (service, mock) = make_service(&dir);
        let entries = service.list_entries().await;
        let mut terminal = entries.iter().find(|e| e.id == "terminal").unwrap().clone();
        terminal.apps = vec!["chrome".to_string()];
        service.update_entry(terminal).await.unwrap();

        mock.set_app(app("Google Chrome", "chrome", ""));
        service.invalidate_cache();
        let status = service.status().await;
        assert_eq!(status.scene, SceneKind::Terminal);
        assert_eq!(status.entry_id.as_deref(), Some("terminal"));
    }

    /// SPEC T-502 验收（PRD 3.2.1 验收标准 2）：标题关键字规则生效范围。
    #[tokio::test]
    async fn prd_acceptance_title_keyword_rule() {
        let dir = TestDir::new("acceptance-keyword");
        let (service, mock) = make_service(&dir);
        let mut draft = entry_draft("x", SceneKind::Docs);
        draft.name = "PR 审阅".to_string();
        draft.title_keyword = "Pull Request".to_string();
        let added = service.add_entry(draft).await.unwrap();

        // 浏览器 PR 页面 → 文档沟通场景（新建规则生效）。
        mock.set_app(app(
            "Google Chrome",
            "chrome",
            "Improve parser by dev · Pull Request #42",
        ));
        service.invalidate_cache();
        let status = service.status().await;
        assert_eq!(status.scene, SceneKind::Docs);
        assert_eq!(status.entry_id.as_deref(), Some(added.id.as_str()));

        // 其他网页不受影响（回退内置识别：浏览器 → 默认场景）。
        mock.set_app(app("Google Chrome", "chrome", "今日新闻"));
        service.invalidate_cache();
        let status = service.status().await;
        assert_eq!(status.scene, SceneKind::Browser);
        assert_eq!(status.entry_id.as_deref(), Some("default"));
    }

    /// T-501 简化入口兼容：custom_rules 仍生效，但 T-502 条目规则更优先。
    #[tokio::test]
    async fn legacy_custom_rules_still_classify_with_entries() {
        let dir = TestDir::new("legacy-compat");
        let (service, mock) = make_service(&dir);
        service
            .add_custom_rule("chrome".to_string(), SceneKind::Docs)
            .await
            .unwrap();
        mock.set_app(app("Google Chrome", "chrome", ""));
        service.invalidate_cache();
        assert_eq!(service.status().await.scene, SceneKind::Docs);

        // 条目应用规则（新模型）优先于简化入口。
        let mut terminal = service
            .list_entries()
            .await
            .into_iter()
            .find(|e| e.id == "terminal")
            .unwrap();
        terminal.apps = vec!["chrome".to_string()];
        service.update_entry(terminal).await.unwrap();
        service.invalidate_cache();
        let status = service.status().await;
        assert_eq!(status.scene, SceneKind::Terminal);
        assert_eq!(status.entry_id.as_deref(), Some("terminal"));
    }

    #[tokio::test]
    async fn entries_crud_and_validation() {
        let dir = TestDir::new("entries-crud");
        let (service, _mock) = make_service(&dir);

        // 初始：内置五条（空配置生效内置默认）。
        assert_eq!(service.list_entries().await.len(), 5);

        // 新增：ID 由服务端生成，builtin = false，输入归一（名称裁剪 / 应用去重）。
        let mut draft = entry_draft("ignored-id", SceneKind::Docs);
        draft.name = "  我的场景  ".to_string();
        draft.builtin = true; // 调用方指定内置标记被忽略
        draft.apps = vec![" Chrome ".to_string(), "chrome".to_string(), String::new()];
        let added = service.add_entry(draft).await.unwrap();
        assert_eq!(added.id, "custom-1");
        assert_eq!(added.name, "我的场景");
        assert_eq!(
            added.apps,
            vec!["Chrome".to_string()],
            "应用去重（大小写不敏感）"
        );
        assert!(!added.builtin);

        // 更新：改名 + 换归属场景（自定义条目允许）。
        let mut updated = added.clone();
        updated.name = "改名".to_string();
        updated.scene = SceneKind::Terminal;
        service.update_entry(updated).await.unwrap();
        let found = service
            .list_entries()
            .await
            .into_iter()
            .find(|e| e.id == "custom-1")
            .unwrap();
        assert_eq!(found.name, "改名");
        assert_eq!(found.scene, SceneKind::Terminal);

        // 更新不存在的条目报错。
        let mut missing = added.clone();
        missing.id = "nope".to_string();
        assert!(service.update_entry(missing).await.is_err());

        // 内置条目可编辑，但内置标记与归属场景由服务端裁决保持。
        let mut ide = service
            .list_entries()
            .await
            .into_iter()
            .find(|e| e.id == "ide")
            .unwrap();
        ide.name = "前端编码".to_string();
        ide.builtin = false;
        ide.scene = SceneKind::Docs;
        ide.punct_style = PunctStyle::FullWidth;
        service.update_entry(ide).await.unwrap();
        let ide = service
            .list_entries()
            .await
            .into_iter()
            .find(|e| e.id == "ide")
            .unwrap();
        assert_eq!(ide.name, "前端编码");
        assert!(ide.builtin);
        assert_eq!(ide.scene, SceneKind::Ide, "内置条目归属场景不可改");
        assert_eq!(
            ide.punct_style,
            PunctStyle::FullWidth,
            "内置条目输出规则可编辑"
        );

        // 内置不可删除；自定义可删除。
        let err = service.delete_entry("ide").await.unwrap_err();
        assert_eq!(err.code(), format!("E-CONFIG-{ERR_BUILTIN_ENTRY:03}"));
        service.delete_entry("custom-1").await.unwrap();
        assert!(!service
            .list_entries()
            .await
            .iter()
            .any(|e| e.id == "custom-1"));
        assert!(
            service.delete_entry("custom-1").await.is_err(),
            "重复删除报错"
        );
    }

    #[tokio::test]
    async fn entry_validation_rejects_bad_input() {
        let dir = TestDir::new("entry-validate");
        let (service, _mock) = make_service(&dir);
        let bad_code = format!("E-CONFIG-{ERR_BAD_ENTRY:03}");

        let mut bad = entry_draft("x", SceneKind::Docs);
        bad.name = "   ".to_string();
        assert_eq!(service.add_entry(bad).await.unwrap_err().code(), bad_code);

        let mut bad = entry_draft("x", SceneKind::Docs);
        bad.name = "名".repeat(MAX_ENTRY_NAME_LEN + 1);
        assert_eq!(service.add_entry(bad).await.unwrap_err().code(), bad_code);

        let mut bad = entry_draft("x", SceneKind::Unknown);
        bad.name = "未知".to_string();
        assert_eq!(service.add_entry(bad).await.unwrap_err().code(), bad_code);

        let mut bad = entry_draft("x", SceneKind::Docs);
        bad.name = "颜色".to_string();
        bad.icon_color = "red".to_string();
        assert_eq!(service.add_entry(bad).await.unwrap_err().code(), bad_code);

        let mut bad = entry_draft("x", SceneKind::Docs);
        bad.name = "应用太多".to_string();
        bad.apps = (0..=MAX_ENTRY_APPS).map(|i| format!("app-{i}")).collect();
        assert_eq!(service.add_entry(bad).await.unwrap_err().code(), bad_code);

        let mut bad = entry_draft("x", SceneKind::Docs);
        bad.name = "关键字太长".to_string();
        bad.title_keyword = "k".repeat(MAX_TITLE_KEYWORD_LEN + 1);
        assert_eq!(service.add_entry(bad).await.unwrap_err().code(), bad_code);

        // 合法输入（含合法颜色）通过。
        let mut ok = entry_draft("x", SceneKind::Docs);
        ok.name = "合法".to_string();
        ok.icon_color = "#a1B2c3".to_string();
        assert!(service.add_entry(ok).await.is_ok());
    }

    #[tokio::test]
    async fn entry_limit_enforced() {
        let dir = TestDir::new("entry-limit");
        let (service, _mock) = make_service(&dir);
        while service.list_entries().await.len() < MAX_SCENE_ENTRIES {
            let mut draft = entry_draft("x", SceneKind::Docs);
            draft.name = format!("场景{}", service.list_entries().await.len());
            service.add_entry(draft).await.unwrap();
        }
        let mut draft = entry_draft("x", SceneKind::Docs);
        draft.name = "溢出".to_string();
        let err = service.add_entry(draft).await.unwrap_err();
        assert_eq!(err.code(), format!("E-CONFIG-{ERR_ENTRY_LIMIT:03}"));
        assert_eq!(service.list_entries().await.len(), MAX_SCENE_ENTRIES);
    }

    #[tokio::test]
    async fn restore_entry_default_restores_and_keeps_position() {
        let dir = TestDir::new("entry-restore");
        let (service, _mock) = make_service(&dir);
        // 从未编辑（entries 为空）：恢复默认直接返回内置默认值。
        let restored = service.restore_entry_default("terminal").await.unwrap();
        assert_eq!(restored, builtin_default_entry("terminal").unwrap());

        // 编辑内置条目后恢复：字段还原、列表位置保持（排序即匹配优先级）。
        let mut terminal = service
            .list_entries()
            .await
            .into_iter()
            .find(|e| e.id == "terminal")
            .unwrap();
        terminal.name = "我的终端".to_string();
        terminal.punct_style = PunctStyle::FullWidth;
        terminal.auto_enter = true;
        service.update_entry(terminal).await.unwrap();
        let before: Vec<String> = service
            .list_entries()
            .await
            .iter()
            .map(|e| e.id.clone())
            .collect();
        let restored = service.restore_entry_default("terminal").await.unwrap();
        assert_eq!(restored, builtin_default_entry("terminal").unwrap());
        let after: Vec<String> = service
            .list_entries()
            .await
            .iter()
            .map(|e| e.id.clone())
            .collect();
        assert_eq!(before, after, "恢复默认保持列表位置");

        // 非内置条目不支持恢复默认（内置不可删除、只能恢复；自定义只能删除）。
        let err = service.restore_entry_default("custom-1").await.unwrap_err();
        assert_eq!(err.code(), format!("E-CONFIG-{ERR_BUILTIN_ENTRY:03}"));
    }

    /// SPEC T-502 步骤 4 / 验收：拖动排序持久化与校验。
    #[tokio::test]
    async fn reorder_entries_persists_and_validates() {
        let dir = TestDir::new("entry-reorder");
        let (service, _mock) = make_service(&dir);
        let ids: Vec<String> = service
            .list_entries()
            .await
            .iter()
            .map(|e| e.id.clone())
            .collect();
        // 把 default 移到首位（其余顺延）。
        let mut reordered = ids.clone();
        reordered.rotate_right(1);
        service.reorder_entries(reordered.clone()).await.unwrap();
        assert_eq!(
            service
                .list_entries()
                .await
                .iter()
                .map(|e| e.id.clone())
                .collect::<Vec<_>>(),
            reordered
        );
        // 持久化：重新加载配置后顺序保持。
        let (service2, _mock) = make_service(&dir);
        assert_eq!(
            service2
                .list_entries()
                .await
                .iter()
                .map(|e| e.id.clone())
                .collect::<Vec<_>>(),
            reordered
        );
        // 清单不一致拒绝（缺条目 / 重复 / 位置数量不符）。
        assert!(service
            .reorder_entries(vec!["ide".to_string()])
            .await
            .is_err());
        assert!(service
            .reorder_entries(vec![
                "ide".to_string(),
                "ide".to_string(),
                "terminal".to_string(),
                "ai_chat".to_string(),
                "docs".to_string(),
            ])
            .await
            .is_err());
        assert!(service
            .reorder_entries({
                let mut extra = reordered.clone();
                extra.push("extra".to_string());
                extra
            })
            .await
            .is_err());
    }

    #[tokio::test]
    async fn entries_persist_across_restart() {
        let dir = TestDir::new("entry-persist");
        {
            let (service, _mock) = make_service(&dir);
            let mut draft = entry_draft("x", SceneKind::Docs);
            draft.name = "持久化场景".to_string();
            service.add_entry(draft).await.unwrap();
        }
        let (service, _mock) = make_service(&dir);
        let entries = service.list_entries().await;
        assert_eq!(entries.len(), 6);
        assert!(entries.iter().any(|e| e.name == "持久化场景"));
    }

    // ---------- 快照语义（T-502）----------

    /// SPEC T-502 验收：规则修改不影响进行中的录音会话（快照语义）。
    #[tokio::test]
    async fn snapshot_freezes_entry_for_session() {
        let dir = TestDir::new("snapshot");
        let (service, _mock) = make_service(&dir);

        // 会话 1：VS Code → 编码场景（默认半角规则）。
        let snap1 = service.snapshot_for_recording(1).await;
        assert_eq!(snap1.scene, SceneKind::Ide);
        assert_eq!(snap1.source, SceneSource::Auto);
        assert_eq!(snap1.entry_id(), Some("ide"));
        assert_eq!(
            snap1.entry.as_ref().unwrap().punct_style,
            PunctStyle::HalfWidth
        );

        // 开录后修改配置：编码条目改全角。
        let mut ide = service
            .list_entries()
            .await
            .into_iter()
            .find(|e| e.id == "ide")
            .unwrap();
        ide.punct_style = PunctStyle::FullWidth;
        service.update_entry(ide).await.unwrap();

        // 进行中的会话 1 快照不受影响。
        let frozen = service.session_snapshot(1).unwrap();
        assert_eq!(frozen, snap1, "开录后改配置不影响进行中会话");
        // 下一次录音使用新配置。
        let snap2 = service.snapshot_for_recording(2).await;
        assert_eq!(
            snap2.entry.as_ref().unwrap().punct_style,
            PunctStyle::FullWidth
        );
        // 未知会话无快照。
        assert!(service.session_snapshot(99).is_none());
    }

    #[tokio::test]
    async fn snapshot_respects_override_and_store_is_bounded() {
        let dir = TestDir::new("snapshot-lock");
        let (service, _mock) = make_service(&dir);
        service.set_override(SceneKind::Terminal).await.unwrap();
        let snap = service.snapshot_for_recording(7).await;
        assert_eq!(snap.scene, SceneKind::Terminal);
        assert_eq!(snap.source, SceneSource::Override);
        assert_eq!(snap.entry_id(), Some("terminal"));
        assert!(
            !snap.entry.as_ref().unwrap().auto_enter,
            "锁定终端时的快照携带禁回车规则"
        );

        // 快照表有界：超过上限的旧快照被淘汰。
        let last = 10 + MAX_SESSION_SNAPSHOTS as u64 + 1;
        for id in 10..=last {
            service.snapshot_for_recording(id).await;
        }
        assert!(service.session_snapshot(7).is_none(), "旧快照应被淘汰");
        assert!(service.session_snapshot(last).is_some());
    }
}
