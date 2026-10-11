//! 场景识别服务（T-501，SPEC: doc/specs/v0.5/T-501-scene-detection.md）。
//!
//! 把当前前台应用分类为 PRD 定义的场景（IDE / 终端 / AI 对话 / 浏览器 / 沟通文档 / 其他），
//! 供场景规则（T-502）与终端安全保护（T-515）消费。
//! 识别信息（窗口标题等）不写入日志（NFR-S-02）。

use serde::{Deserialize, Serialize};

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
