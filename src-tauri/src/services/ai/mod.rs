//! AI 供应商抽象与服务（T-531，SPEC: doc/specs/v0.5/T-531-ai-providers.md）。
//!
//! - [`provider`]：`AiProvider` trait（complete / list_models / test_connection，
//!   tech/06 6.1.3）、请求 / 响应模型与稳定错误编号；
//! - [`openai_compatible`] / [`ollama`]：协议适配器（LM Studio 走 OpenAI 兼容子集）；
//! - [`presets`]：PRD 3.7 预置供应商清单（数据化）与配置校验；
//! - [`service`]：AiState（供应商 CRUD、Key 保管、适配器构造）；
//! - [`http`]：统一超时 / 重试的 HTTP 公共设施。
//!
//! 密钥只进 OS 凭据存储（NFR-S-01），永不进日志、配置文件与命令返回值。

pub mod http;
pub mod ollama;
pub mod openai_compatible;
pub mod presets;
pub mod provider;
pub mod service;
pub mod templates;

pub use ollama::OllamaProvider;
pub use openai_compatible::OpenAiCompatibleProvider;
pub use presets::{MergeResult, PresetProvider, CATALOG};
pub use provider::{
    redact_secret, AiProvider, CompletionRequest, CompletionResponse, CompletionUsage,
    ConnectionTest, ModelInfo, ProtocolKind,
};
pub use service::{AiState, ProviderView, RestoreReport};

/// 测试专用 mock HTTP 服务器（仅测试编译，不进生产构建）。
#[cfg(test)]
pub(crate) mod mock_http;
