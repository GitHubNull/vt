//! AI 供应商抽象：trait、请求 / 响应模型与稳定错误编号（tech/06 6.1.3，F-AI-06/08）。
//!
//! 错误编号约定：云端供应商的网络 / 协议错误统一使用 `E-NET-1xx` 段（tech/04 4.6
//! 错误编号前缀表），本地供应商（Ollama / LM Studio）的同类错误也走同一编号段，
//! 便于前端统一处理；编号稳定，文案在 i18n 中映射。

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// 默认请求总超时（tech/06 6.1.4：单次 AI 调用 15 s，可配）。
pub const DEFAULT_TIMEOUT_SECS: u64 = 15;
/// 连接建立超时（断网时快速失败，先于总超时触发）。
pub const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 10;

/// E-NET 编号：断网 / DNS / 连接被拒（云端供应商不可达）。
pub const ERR_NET_UNREACHABLE: u32 = 101;
/// E-NET 编号：请求超时（默认 15 s，tech/06 6.1.4）。
pub const ERR_NET_TIMEOUT: u32 = 102;
/// E-NET 编号：云端供应商返回 401——API Key 无效。
pub const ERR_NET_UNAUTHORIZED: u32 = 103;
/// E-NET 编号：云端供应商返回 403——无权限访问该模型 / 服务。
pub const ERR_NET_FORBIDDEN: u32 = 104;
/// E-NET 编号：云端供应商返回 429——限流或额度不足。
pub const ERR_NET_RATE_LIMITED: u32 = 105;
/// E-NET 编号：云端供应商返回 400——请求参数错误（通常是模型标识错误）。
pub const ERR_NET_BAD_REQUEST: u32 = 106;
/// E-NET 编号：云端供应商返回 5xx——供应商服务端异常。
pub const ERR_NET_SERVER: u32 = 107;
/// E-NET 编号：响应解析失败（协议不匹配或返回体损坏）。
pub const ERR_NET_BAD_RESPONSE: u32 = 108;
/// E-NET 编号：云端供应商未配置 API Key。
pub const ERR_NET_NO_KEY: u32 = 109;
/// E-NET 编号：未选择模型。
pub const ERR_NET_NO_MODEL: u32 = 110;
/// E-NET 编号：重定向次数超限或重定向目标非法。
pub const ERR_NET_REDIRECT: u32 = 111;

/// E-CONFIG 编号：供应商配置不合法（重名 / 地址非法等，与 ConfigStore 的
/// ERR_BAD_VALUE 同号段语义）。
pub const ERR_AI_BAD_VALUE: u32 = 4;

/// 供应商协议类型（配置 `protocol` 字段的合法值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolKind {
    /// OpenAI 兼容协议（chat/completions + /models）。
    OpenAiCompatible,
    /// Ollama 本地协议（/api/chat + /api/tags）。
    Ollama,
    /// LM Studio 本地协议（OpenAI 兼容子集，无需 Key）。
    LmStudio,
}

impl ProtocolKind {
    /// 解析配置中的协议字符串；非法值返回 `None`。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "openai_compatible" => Some(Self::OpenAiCompatible),
            "ollama" => Some(Self::Ollama),
            "lm_studio" => Some(Self::LmStudio),
            _ => None,
        }
    }

    /// 配置序列化值。
    pub fn as_config_str(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai_compatible",
            Self::Ollama => "ollama",
            Self::LmStudio => "lm_studio",
        }
    }

    /// 该协议是否为本机协议（无外发，断网不影响）。
    pub fn is_local(self) -> bool {
        matches!(self, Self::Ollama | Self::LmStudio)
    }
}

/// 文本补全请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// 模型标识（供应商侧 ID，如 `deepseek-chat`）。
    pub model: String,
    /// 用户提示词。
    pub prompt: String,
    /// 系统提示词（可选）。
    #[serde(default)]
    pub system: Option<String>,
    /// 采样温度（可选，默认交给供应商）。
    #[serde(default)]
    pub temperature: Option<f32>,
}

/// 文本补全响应。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// 补全文本。
    pub text: String,
    /// 实际响应的模型标识。
    pub model: String,
    /// 用量（供应商返回则透传）。
    #[serde(default)]
    pub usage: Option<CompletionUsage>,
}

/// Token 用量。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionUsage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

/// 模型信息（F-AI-08 拉取模型）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// 模型标识（供应商侧 ID）。
    pub id: String,
    /// 显示名称（供应商未提供时回退为 id）。
    pub display_name: String,
}

/// 连通性测试结果（F-AI-08；`ok=false` 时 `error_code` / `detail` 给前端展示）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionTest {
    pub ok: bool,
    /// 往返延迟（毫秒）；失败时为 0。
    pub latency_ms: u64,
    /// 失败时的稳定错误编号（成功为 None）。
    #[serde(default)]
    pub error_code: Option<String>,
    /// 失败原因（已脱敏，不含 Key 本体）。
    #[serde(default)]
    pub detail: Option<String>,
}

/// 供应商抽象（tech/06 6.1.3）。
///
/// 三个方法均为异步；实现不得在任何返回的错误信息 / 日志中泄露 API Key。
/// 返回类型统一为 `BoxFuture` 以保持对象安全（`Box<dyn AiProvider>`，服务层
/// 经 trait 对象持有不同协议的适配器）。
pub trait AiProvider: Send + Sync {
    /// 文本补全（F-AI-07）。输出 future 生命周期与 `&self` / `req` 一致，
    /// 便于适配器直接 `Box::pin` 内部 async fn。
    fn complete<'a>(
        &'a self,
        req: &'a CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionResponse, AppError>>;

    /// 拉取供应商可用模型（F-AI-08）。
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<ModelInfo>, AppError>>;

    /// 连通性测试（F-AI-08）：成功返回延迟，失败返回带稳定编号的错误。
    fn test_connection(&self) -> BoxFuture<'_, Result<ConnectionTest, AppError>>;
}

/// 构造带稳定编号的网络类错误（E-NET 段，编号 → 文案映射在 i18n）。
pub fn net_error(code: u32, message: impl Into<String>) -> AppError {
    AppError::Network(code, message.into())
}

/// 规范化 Base URL：去首尾空白与结尾 `/`（拼接端点路径前统一处理）。
pub fn normalize_base_url(base_url: &str) -> String {
    base_url.trim().trim_end_matches('/').to_string()
}

/// 防御性脱敏：若 `text` 中含秘密本体（API Key），替换为掩码。
///
/// 用于供应商错误响应体 / reqwest 错误串的二次兜底（NFR-S-01/02：任何错误
/// 信息不得出现 Key 明文）。Key 为空时原样返回。
pub fn redact_secret(text: &str, secret: Option<&str>) -> String {
    let Some(secret) = secret.filter(|s| !s.is_empty()) else {
        return text.to_string();
    };
    if let Some(index) = text.find(secret) {
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..index]);
        out.push_str(&mask_secret(secret));
        out.push_str(&text[index + secret.len()..]);
        out
    } else {
        text.to_string()
    }
}

/// 掩码显示（NFR-S-01 界面掩码约定的服务端侧：`sk-ab****…****yz`）。
pub fn mask_secret(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= 8 {
        return "****".to_string();
    }
    format!(
        "{}-****…****-{}",
        chars[..3].iter().collect::<String>(),
        chars[chars.len() - 2..].iter().collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_parse_roundtrip() {
        for kind in [
            ProtocolKind::OpenAiCompatible,
            ProtocolKind::Ollama,
            ProtocolKind::LmStudio,
        ] {
            assert_eq!(ProtocolKind::parse(kind.as_config_str()), Some(kind));
        }
        assert_eq!(ProtocolKind::parse("gemini_native"), None);
        assert!(ProtocolKind::Ollama.is_local());
        assert!(ProtocolKind::LmStudio.is_local());
        assert!(!ProtocolKind::OpenAiCompatible.is_local());
    }

    #[test]
    fn redact_secret_strips_key_from_error_text() {
        // 构造性验证（验收标准：日志与错误信息中不出现 API Key）。
        let key = "sk-test-9f8e7d6c5b4a3210";
        let server_message = format!("invalid key {key} rejected by gateway");
        let redacted = redact_secret(&server_message, Some(key));
        assert!(!redacted.contains(key), "脱敏后不得含 Key: {redacted}");
        assert!(redacted.contains("****"), "脱敏后应含掩码: {redacted}");
        // 不含 Key 的文本原样返回。
        assert_eq!(redact_secret("timeout", Some(key)), "timeout");
        // 无 Key（本地供应商）时原样返回。
        assert_eq!(redact_secret(key, None), key);
    }

    #[test]
    fn mask_secret_hides_middle() {
        let masked = mask_secret("sk-test-9f8e7d6c5b4a3210");
        assert!(!masked.contains("9f8e7d6c"));
        assert!(masked.starts_with("sk-"));
        assert!(masked.ends_with("10"));
        assert_eq!(mask_secret("short"), "****");
    }
}
