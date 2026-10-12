//! Ollama 本地适配器（tech/06 6.1.3，F-AI-06/07/08）。
//!
//! 端点：`POST {base}/api/chat`（stream=false 补全）、`GET {base}/api/tags` 模型清单。
//! 本地 HTTP 服务无 Key（`local = true`，数据不出本机）；断网不影响本地供应商调用。

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

use super::http::{
    read_body, send_with_retry, status_error, truncate_chars, ERROR_BODY_SNIPPET_CHARS,
};
use super::provider::{net_error, normalize_base_url};
use super::provider::{
    AiProvider, CompletionRequest, CompletionResponse, CompletionUsage, ConnectionTest, ModelInfo,
    ERR_NET_BAD_RESPONSE,
};

/// Ollama 适配器（base_url 默认 `http://127.0.0.1:11434`）。
pub struct OllamaProvider {
    http: reqwest::Client,
    base_url: String,
}

impl OllamaProvider {
    pub fn new(http: reqwest::Client, base_url: impl Into<String>) -> Self {
        Self {
            http,
            base_url: normalize_base_url(&base_url.into()),
        }
    }
}

/// /api/chat 请求体。
#[derive(Serialize)]
struct ChatWireBody {
    model: String,
    messages: Vec<WireMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Serialize)]
struct WireMessage {
    role: &'static str,
    content: String,
}

/// /api/chat 响应体（stream=false）。
#[derive(Deserialize)]
struct ChatWireResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    message: Option<WireMessageOut>,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    #[serde(default)]
    eval_count: Option<u64>,
}

#[derive(Deserialize)]
struct WireMessageOut {
    #[serde(default)]
    content: Option<String>,
}

/// /api/tags 响应体。
#[derive(Deserialize)]
struct TagsWireResponse {
    #[serde(default)]
    models: Vec<WireTagModel>,
}

#[derive(Deserialize)]
struct WireTagModel {
    name: String,
}

impl OllamaProvider {
    async fn complete_impl(&self, req: &CompletionRequest) -> Result<CompletionResponse, AppError> {
        if req.model.trim().is_empty() {
            return Err(net_error(
                super::provider::ERR_NET_NO_MODEL,
                "未选择模型，请先在供应商设置中指定模型",
            ));
        }
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
            stream: false,
            temperature: req.temperature,
        })
        .map_err(|err| net_error(ERR_NET_BAD_RESPONSE, format!("请求体序列化失败: {err}")))?;
        let url = format!("{}/api/chat", self.base_url);
        let http = self.http.clone();
        let url_clone = url.clone();
        let response = send_with_retry(|| {
            http.post(&url_clone)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.clone())
        })
        .await?;
        let status = response.status().as_u16();
        let text = read_body(response).await?;
        if !(200..300).contains(&status) {
            return Err(status_error(
                status,
                &truncate_chars(&text, ERROR_BODY_SNIPPET_CHARS),
            ));
        }
        let wire: ChatWireResponse = serde_json::from_str(&text).map_err(|err| {
            net_error(
                ERR_NET_BAD_RESPONSE,
                format!(
                    "补全响应解析失败: {err}；响应片段: {}",
                    truncate_chars(&text, ERROR_BODY_SNIPPET_CHARS)
                ),
            )
        })?;
        let content = wire
            .message
            .and_then(|message| message.content)
            .unwrap_or_default();
        // 用量映射：prompt_eval_count / eval_count → prompt / completion tokens。
        let usage = match (wire.prompt_eval_count, wire.eval_count) {
            (Some(prompt), Some(completion)) => Some(CompletionUsage {
                prompt_tokens: prompt,
                completion_tokens: completion,
                total_tokens: prompt + completion,
            }),
            _ => None,
        };
        Ok(CompletionResponse {
            text: content,
            model: wire.model.unwrap_or_else(|| req.model.clone()),
            usage,
        })
    }

    async fn list_models_impl(&self) -> Result<Vec<ModelInfo>, AppError> {
        let url = format!("{}/api/tags", self.base_url);
        let http = self.http.clone();
        let url_clone = url.clone();
        let response = send_with_retry(|| http.get(&url_clone)).await?;
        let status = response.status().as_u16();
        let text = read_body(response).await?;
        if !(200..300).contains(&status) {
            return Err(status_error(
                status,
                &truncate_chars(&text, ERROR_BODY_SNIPPET_CHARS),
            ));
        }
        let wire: TagsWireResponse = serde_json::from_str(&text).map_err(|err| {
            net_error(
                ERR_NET_BAD_RESPONSE,
                format!(
                    "模型清单解析失败: {err}；响应片段: {}",
                    truncate_chars(&text, ERROR_BODY_SNIPPET_CHARS)
                ),
            )
        })?;
        Ok(wire
            .models
            .into_iter()
            .map(|model| ModelInfo {
                display_name: model.name.clone(),
                id: model.name,
            })
            .collect())
    }
}

impl AiProvider for OllamaProvider {
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
            let start = std::time::Instant::now();
            match self.list_models_impl().await {
                Ok(models) => Ok(ConnectionTest {
                    ok: true,
                    latency_ms: start.elapsed().as_millis() as u64,
                    error_code: None,
                    detail: Some(format!("连接正常，共 {} 个已安装模型", models.len())),
                }),
                Err(err) => Ok(ConnectionTest {
                    ok: false,
                    latency_ms: 0,
                    error_code: Some(err.code()),
                    detail: Some(err.message().to_string()),
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

    fn provider(base: &str) -> OllamaProvider {
        OllamaProvider::new(default_client(), base)
    }

    #[tokio::test]
    async fn complete_posts_to_api_chat_without_stream() {
        let server = MockServer::spawn(vec![MockRoute {
            path: "/api/chat",
            delay_ms: 0,
            responses: vec![MockResponse {
                status: 200,
                body: r#"{"model":"qwen3:8b","created_at":"2026-01-01T00:00:00Z",
                    "message":{"role":"assistant","content":"函数名用小驼峰。"},
                    "done":true,"prompt_eval_count":9,"eval_count":5}"#
                    .to_string(),
            }],
        }])
        .await;
        let provider = provider(&server.base_url);
        let request = CompletionRequest {
            model: "qwen3:8b".to_string(),
            prompt: "给 getUserName 起个中文命名建议".to_string(),
            system: None,
            temperature: None,
        };
        let response = provider.complete(&request).await.unwrap();
        assert_eq!(response.text, "函数名用小驼峰。");
        assert_eq!(response.model, "qwen3:8b");
        let usage = response.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 9);
        assert_eq!(usage.completion_tokens, 5);
        assert_eq!(usage.total_tokens, 14);

        let recorded = server.last_request();
        assert_eq!(recorded.method, "POST");
        assert!(recorded.authorization.is_none(), "本地供应商不携带 Key");
        let body: serde_json::Value = serde_json::from_str(&recorded.body).unwrap();
        assert_eq!(body["model"], "qwen3:8b");
        assert_eq!(body["stream"], false);
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[tokio::test]
    async fn list_models_reads_api_tags() {
        let server = MockServer::spawn(vec![MockRoute {
            path: "/api/tags",
            delay_ms: 0,
            responses: vec![MockResponse {
                status: 200,
                body: r#"{"models":[
                    {"name":"qwen3:8b","model":"qwen3:8b","modified_at":"x","size":1},
                    {"name":"deepseek-r1:7b","model":"deepseek-r1:7b","modified_at":"x","size":2}]}"#
                    .to_string(),
            }],
        }])
        .await;
        let models = provider(&server.base_url).list_models().await.unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[1].id, "deepseek-r1:7b");
    }

    #[tokio::test]
    async fn test_connection_and_error_snippet() {
        let ok_server = MockServer::spawn(vec![MockRoute {
            path: "/api/tags",
            delay_ms: 0,
            responses: vec![MockResponse {
                status: 200,
                body: r#"{"models":[{"name":"qwen3:8b"}]}"#.to_string(),
            }],
        }])
        .await;
        let ok = provider(&ok_server.base_url)
            .test_connection()
            .await
            .unwrap();
        assert!(ok.ok);

        let err_server = MockServer::spawn(vec![MockRoute {
            path: "/api/chat",
            delay_ms: 0,
            responses: vec![MockResponse {
                status: 404,
                body: r#"{"error":"model not found"}"#.to_string(),
            }],
        }])
        .await;
        let bad = provider(&err_server.base_url)
            .test_connection()
            .await
            .unwrap();
        assert!(!bad.ok);
        assert_eq!(bad.error_code.as_deref(), Some("E-NET-106"));
    }
}
