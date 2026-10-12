//! OpenAI 兼容协议适配器（tech/06 6.1.3，F-AI-06/07/08）。
//!
//! 覆盖多数云端供应商（OpenAI、DeepSeek、通义千问、智谱、豆包、硅基流动、Kimi、
//! Gemini 兼容层、龙猫、小米、Mercury、Cerebras 等）与 LM Studio 本地服务。
//! 端点：`POST {base}/chat/completions` 补全、`GET {base}/models` 模型清单。
//!
//! 安全（NFR-S-01/02）：Key 只经 `Authorization: Bearer` 头外发，出现的任何
//! 错误信息都经 [`redact_secret`] 脱敏；Key 永不写日志。

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

use super::http::{
    read_body, send_with_retry, status_error, truncate_chars, ERROR_BODY_SNIPPET_CHARS,
};
use super::provider::{net_error, normalize_base_url};
use super::provider::{
    redact_secret, AiProvider, CompletionRequest, CompletionResponse, CompletionUsage,
    ConnectionTest, ModelInfo, ERR_NET_BAD_RESPONSE, ERR_NET_NO_KEY,
};

/// OpenAI 兼容适配器。
///
/// - `requires_key = true`：云端供应商，无 Key 时补全 / 测试直接返回 E-NET-109；
/// - `requires_key = false`：LM Studio 等本地服务，不携带 Authorization 头。
pub struct OpenAiCompatibleProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    requires_key: bool,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        http: reqwest::Client,
        base_url: impl Into<String>,
        api_key: Option<String>,
        requires_key: bool,
    ) -> Self {
        Self {
            http,
            base_url: normalize_base_url(&base_url.into()),
            api_key,
            requires_key,
        }
    }

    /// 取 Key（云端供应商强制要求；本地服务放行）。
    fn require_key(&self) -> Result<Option<&str>, AppError> {
        if !self.requires_key {
            return Ok(None);
        }
        let key = self
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        match key {
            Some(key) => Ok(Some(key)),
            None => Err(net_error(
                ERR_NET_NO_KEY,
                "该供应商尚未配置 API Key，请先在设置中填写",
            )),
        }
    }

    fn auth_header(&self) -> Option<String> {
        self.api_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|key| format!("Bearer {key}"))
    }
}

/// chat/completions 请求体。
#[derive(Serialize)]
struct ChatWireBody {
    model: String,
    messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
}

#[derive(Serialize)]
struct WireMessage {
    role: &'static str,
    content: String,
}

/// chat/completions 响应体（仅取所需字段）。
#[derive(Deserialize)]
struct ChatWireResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<WireChoice>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireChoice {
    #[serde(default)]
    message: Option<WireMessageOut>,
}

#[derive(Deserialize)]
struct WireMessageOut {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize, Default)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
}

/// /models 响应体。
#[derive(Deserialize)]
struct ModelsWireResponse {
    #[serde(default)]
    data: Vec<WireModel>,
}

#[derive(Deserialize)]
struct WireModel {
    id: String,
    #[serde(default)]
    owned_by: Option<String>,
}

/// 解析非 2xx 响应为稳定编号错误（响应体片段脱敏后附入）。
async fn error_from_response(response: reqwest::Response, secret: Option<&str>) -> AppError {
    let status = response.status().as_u16();
    let snippet = read_body(response)
        .await
        .map(|text| redact_secret(&truncate_chars(&text, ERROR_BODY_SNIPPET_CHARS), secret))
        .unwrap_or_default();
    status_error(status, &snippet)
}

impl OpenAiCompatibleProvider {
    /// 实现 complete 的异步主体（trait 方法包装为 BoxFuture）。
    async fn complete_impl(&self, req: &CompletionRequest) -> Result<CompletionResponse, AppError> {
        if req.model.trim().is_empty() {
            return Err(net_error(
                super::provider::ERR_NET_NO_MODEL,
                "未选择模型，请先在供应商设置中指定模型",
            ));
        }
        // 云端供应商：无 Key 直接返回稳定编号错误；本地服务放行。
        self.require_key()?;
        let auth = self.auth_header();
        let mut messages = Vec::new();
        if let Some(system) = req.system.as_deref().filter(|s| !s.trim().is_empty()) {
            messages.push(WireMessage {
                role: "system",
                content: system.to_string(),
            });
        }
        messages.push(WireMessage {
            role: "user",
            content: req.prompt.clone(),
        });
        let body = serde_json::to_vec(&ChatWireBody {
            model: req.model.trim().to_string(),
            messages,
            temperature: req.temperature,
            stream: false,
        })
        .map_err(|err| net_error(ERR_NET_BAD_RESPONSE, format!("请求体序列化失败: {err}")))?;
        let url = format!("{}/chat/completions", self.base_url);
        let http = self.http.clone();
        let url_clone = url.clone();
        let response = send_with_retry(|| {
            let mut request = http
                .post(&url_clone)
                .header(reqwest::header::CONTENT_TYPE, "application/json");
            if let Some(auth) = &auth {
                request = request.header(reqwest::header::AUTHORIZATION, auth.clone());
            }
            request.body(body.clone())
        })
        .await?;
        if !response.status().is_success() {
            return Err(error_from_response(response, self.api_key.as_deref()).await);
        }
        let text = read_body(response).await?;
        let wire: ChatWireResponse = serde_json::from_str(&text).map_err(|err| {
            let snippet = redact_secret(
                &truncate_chars(&text, ERROR_BODY_SNIPPET_CHARS),
                self.api_key.as_deref(),
            );
            net_error(
                ERR_NET_BAD_RESPONSE,
                format!("补全响应解析失败: {err}；响应片段: {snippet}"),
            )
        })?;
        let content = wire
            .choices
            .first()
            .and_then(|choice| choice.message.as_ref())
            .and_then(|message| message.content.clone())
            .unwrap_or_default();
        Ok(CompletionResponse {
            text: content,
            model: wire.model.unwrap_or_else(|| req.model.clone()),
            usage: wire.usage.map(|usage| CompletionUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
            }),
        })
    }

    /// 拉取模型清单（GET /models）。
    async fn list_models_impl(&self) -> Result<Vec<ModelInfo>, AppError> {
        self.require_key()?;
        let url = format!("{}/models", self.base_url);
        let auth = self.auth_header();
        let http = self.http.clone();
        let url_clone = url.clone();
        let response = send_with_retry(|| {
            let mut request = http.get(&url_clone);
            if let Some(auth) = &auth {
                request = request.header(reqwest::header::AUTHORIZATION, auth.clone());
            }
            request
        })
        .await?;
        if !response.status().is_success() {
            return Err(error_from_response(response, self.api_key.as_deref()).await);
        }
        let text = read_body(response).await?;
        let wire: ModelsWireResponse = serde_json::from_str(&text).map_err(|err| {
            let snippet = redact_secret(
                &truncate_chars(&text, ERROR_BODY_SNIPPET_CHARS),
                self.api_key.as_deref(),
            );
            net_error(
                ERR_NET_BAD_RESPONSE,
                format!("模型清单解析失败: {err}；响应片段: {snippet}"),
            )
        })?;
        Ok(wire
            .data
            .into_iter()
            .map(|model| ModelInfo {
                display_name: model.owned_by.unwrap_or_else(|| model.id.clone()),
                id: model.id,
            })
            .collect())
    }
}

impl AiProvider for OpenAiCompatibleProvider {
    fn complete<'a>(
        &'a self,
        req: &'a CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionResponse, AppError>> {
        Box::pin(self.complete_impl(req))
    }

    fn list_models(&self) -> BoxFuture<'_, Result<Vec<ModelInfo>, AppError>> {
        Box::pin(self.list_models_impl())
    }

    fn test_connection(&self) -> BoxFuture<'_, Result<ConnectionTest, AppError>> {
        Box::pin(async move {
            // 连通性测试走 GET /models：不消耗补全 token 即可同时验证
            // 地址可达与 Key 有效（PRD 3.7：失败显示可读错误原因）。
            let start = std::time::Instant::now();
            match self.list_models_impl().await {
                Ok(models) => Ok(ConnectionTest {
                    ok: true,
                    latency_ms: start.elapsed().as_millis() as u64,
                    error_code: None,
                    detail: Some(format!("连接正常，共 {} 个可用模型", models.len())),
                }),
                Err(err) => Ok(ConnectionTest {
                    ok: false,
                    latency_ms: 0,
                    error_code: Some(err.code()),
                    detail: Some(redact_secret(err.message(), self.api_key.as_deref())),
                }),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::ai::http::default_client;
    use crate::services::ai::mock_http::{MockResponse, MockRoute, MockServer};

    const KEY: &str = "sk-test-9f8e7d6c5b4a3210";

    fn provider(base: &str) -> OpenAiCompatibleProvider {
        OpenAiCompatibleProvider::new(default_client(), base, Some(KEY.to_string()), true)
    }

    fn completion_request() -> CompletionRequest {
        CompletionRequest {
            model: "deepseek-chat".to_string(),
            prompt: "把这句话改写为敬语".to_string(),
            system: Some("你是中文润色助手".to_string()),
            temperature: Some(0.2),
        }
    }

    #[tokio::test]
    async fn complete_builds_expected_request_and_parses_response() {
        let server = MockServer::spawn(vec![MockRoute {
            path: "/v1/chat/completions",
            delay_ms: 0,
            responses: vec![MockResponse {
                status: 200,
                body: r#"{"id":"x","model":"deepseek-chat","choices":[
                    {"index":0,"message":{"role":"assistant","content":"您好，请问有什么可以帮您？"}}],
                    "usage":{"prompt_tokens":10,"completion_tokens":8,"total_tokens":18}}"#
                    .to_string(),
            }],
        }])
        .await;
        let provider = provider(&format!("{}/v1", server.base_url));
        let response = provider.complete(&completion_request()).await.unwrap();
        assert_eq!(response.text, "您好，请问有什么可以帮您？");
        assert_eq!(response.model, "deepseek-chat");
        assert_eq!(response.usage.unwrap().total_tokens, 18);

        let request = server.last_request();
        assert_eq!(request.method, "POST");
        assert_eq!(
            request.authorization.as_deref(),
            Some(format!("Bearer {KEY}").as_str())
        );
        let body: serde_json::Value = serde_json::from_str(&request.body).unwrap();
        assert_eq!(body["model"], "deepseek-chat");
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["stream"], false);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "你是中文润色助手");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "把这句话改写为敬语");
    }

    #[tokio::test]
    async fn list_models_parses_data_array() {
        let server = MockServer::spawn(vec![MockRoute {
            path: "/v1/models",
            delay_ms: 0,
            responses: vec![MockResponse {
                status: 200,
                body: r#"{"object":"list","data":[
                    {"id":"deepseek-chat","object":"model","owned_by":"deepseek"},
                    {"id":"deepseek-coder","object":"model","owned_by":"deepseek"}]}"#
                    .to_string(),
            }],
        }])
        .await;
        let provider = provider(&format!("{}/v1", server.base_url));
        let models = provider.list_models().await.unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "deepseek-chat");
        assert_eq!(models[0].display_name, "deepseek");
    }

    #[tokio::test]
    async fn test_connection_reports_latency_or_code() {
        let ok_server = MockServer::spawn(vec![MockRoute {
            path: "/v1/models",
            delay_ms: 0,
            responses: vec![MockResponse {
                status: 200,
                body: r#"{"data":[{"id":"m1"}]}"#.to_string(),
            }],
        }])
        .await;
        let ok = provider(&format!("{}/v1", ok_server.base_url))
            .test_connection()
            .await
            .unwrap();
        assert!(ok.ok);
        assert!(ok.error_code.is_none());

        let bad_server = MockServer::spawn(vec![MockRoute {
            path: "/v1/models",
            delay_ms: 0,
            responses: vec![MockResponse {
                status: 401,
                body: format!("invalid api key {KEY}"),
            }],
        }])
        .await;
        let bad = provider(&format!("{}/v1", bad_server.base_url))
            .test_connection()
            .await
            .unwrap();
        assert!(!bad.ok);
        assert_eq!(bad.error_code.as_deref(), Some("E-NET-103"));
        // NFR-S-01/02：错误原因中不得出现 Key 明文。
        assert!(
            !bad.detail.as_deref().unwrap().contains(KEY),
            "连通性测试错误信息不得泄露 Key: {:?}",
            bad.detail
        );
    }

    #[tokio::test]
    async fn missing_key_returns_stable_code() {
        let provider =
            OpenAiCompatibleProvider::new(default_client(), "http://127.0.0.1:1", None, true);
        let err = provider.complete(&completion_request()).await.unwrap_err();
        assert_eq!(err.code(), "E-NET-109");
    }

    #[tokio::test]
    async fn missing_model_returns_stable_code() {
        let server = MockServer::spawn(vec![]).await;
        let provider = provider(&format!("{}/v1", server.base_url));
        let req = CompletionRequest {
            model: "  ".to_string(),
            prompt: "hi".to_string(),
            system: None,
            temperature: None,
        };
        assert_eq!(
            provider.complete(&req).await.unwrap_err().code(),
            "E-NET-110"
        );
    }

    #[tokio::test]
    async fn retriable_5xx_eventually_succeeds() {
        // 统一重试策略（tech/06 6.1.3）：前两次 500，第三次成功。
        let server = MockServer::spawn(vec![MockRoute {
            path: "/v1/models",
            delay_ms: 0,
            responses: vec![
                MockResponse {
                    status: 500,
                    body: "boom".to_string(),
                },
                MockResponse {
                    status: 500,
                    body: "boom".to_string(),
                },
                MockResponse {
                    status: 200,
                    body: r#"{"data":[]}"#.to_string(),
                },
            ],
        }])
        .await;
        let provider = provider(&format!("{}/v1", server.base_url));
        let models = provider.list_models().await.unwrap();
        assert!(models.is_empty());
        assert_eq!(server.requests().len(), 3, "应当重试至第三次成功");
    }

    #[tokio::test]
    async fn offline_cloud_fails_with_unreachable_code() {
        // 断网模拟：指向无监听端口 → E-NET-101（验收标准 4）。
        let provider = OpenAiCompatibleProvider::new(
            default_client(),
            "http://127.0.0.1:1/v1",
            Some(KEY.to_string()),
            true,
        );
        let err = provider.list_models().await.unwrap_err();
        assert_eq!(err.code(), "E-NET-101");
    }
}
