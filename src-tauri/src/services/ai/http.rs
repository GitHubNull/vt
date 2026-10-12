//! 供应商 HTTP 公共设施（tech/06 6.1.3）：统一超时 / 重试策略、状态码 → 稳定
//! 错误编号映射、响应体脱敏截断。
//!
//! 安全（NFR-S-01/02）：任何进入错误信息的响应体片段都会先经 [`redact_secret`]
//! 脱敏（Key 由调用方传入），防止供应商回显 Key 时泄露。

use std::time::Duration;

use crate::error::AppError;

use super::provider::{
    net_error, DEFAULT_CONNECT_TIMEOUT_SECS, DEFAULT_TIMEOUT_SECS, ERR_NET_BAD_REQUEST,
    ERR_NET_REDIRECT, ERR_NET_SERVER, ERR_NET_TIMEOUT, ERR_NET_UNREACHABLE,
};

/// 重试次数（首次 + 2 次重试）。仅重试"连接未建立 / 限流 / 服务端异常"，
/// 不重试读超时与 4xx——避免补全类请求重复计费 / 重复生成。
pub const RETRY_ATTEMPTS: u32 = 3;
/// 重试退避基数（指数：300 ms → 600 ms）。
pub const RETRY_BACKOFF: Duration = Duration::from_millis(300);
/// 进入错误信息的响应体片段上限（字符），防止超大错误页刷屏。
pub const ERROR_BODY_SNIPPET_CHARS: usize = 200;

/// 默认 HTTP 客户端：总超时 15 s（tech/06 6.1.4）+ 连接超时 10 s，rustls。
pub fn default_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .connect_timeout(Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECS))
        .build()
        .unwrap_or_default()
}

/// 发送请求（带统一重试）：连接失败 / 429 / 5xx 按指数退避重试，
/// 其余错误立即返回。请求由 `build` 闭包按次构造（body 复用同一份字节；
/// 客户端经闭包捕获，调用方共享同一连接池）。
pub async fn send_with_retry(
    build: impl Fn() -> reqwest::RequestBuilder,
) -> Result<reqwest::Response, AppError> {
    let mut last_err: Option<(u32, String)> = None;
    for attempt in 0..RETRY_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(RETRY_BACKOFF * (1u32 << (attempt - 1))).await;
        }
        match build().send().await {
            Ok(response) => {
                let status = response.status().as_u16();
                if !should_retry_status(status) {
                    return Ok(response);
                }
                last_err = Some(status_error_parts(status, ""));
            }
            Err(err) => {
                let parts = map_request_error_parts(&err);
                if !err.is_connect() {
                    return Err(net_error(parts.0, parts.1));
                }
                last_err = Some(parts);
            }
        }
    }
    let (code, message) =
        last_err.unwrap_or((ERR_NET_SERVER, "供应商请求失败（未知原因）".to_string()));
    Err(net_error(
        code,
        format!("{message}（已重试 {} 次）", RETRY_ATTEMPTS - 1),
    ))
}

/// 可重试状态码：限流与服务端异常。
fn should_retry_status(status: u16) -> bool {
    status == 429 || status >= 500
}

/// reqwest 请求阶段错误 → （稳定编号，描述）。
fn map_request_error_parts(err: &reqwest::Error) -> (u32, String) {
    if err.is_timeout() {
        (
            ERR_NET_TIMEOUT,
            format!("供应商请求超时（{DEFAULT_TIMEOUT_SECS} s）: {err}"),
        )
    } else if err.is_connect() {
        (
            ERR_NET_UNREACHABLE,
            format!("无法连接供应商（断网或地址不可达）: {err}"),
        )
    } else if err.is_redirect() {
        (ERR_NET_REDIRECT, format!("供应商重定向异常: {err}"))
    } else {
        (ERR_NET_UNREACHABLE, format!("供应商请求失败: {err}"))
    }
}

/// reqwest 请求阶段错误 → 稳定编号错误（断网 / DNS / 拒连 → E-NET-101）。
pub fn map_request_error(err: &reqwest::Error) -> AppError {
    let (code, message) = map_request_error_parts(err);
    net_error(code, message)
}

/// HTTP 状态码 → （稳定编号，描述）（PRD 3.7：测试连接须给出可读错误原因）。
///
/// `detail` 为响应体片段（调用方已脱敏），可为空。
fn status_error_parts(status: u16, detail: &str) -> (u32, String) {
    let (code, reason) = match status {
        401 => (
            super::provider::ERR_NET_UNAUTHORIZED,
            "API Key 无效或已过期",
        ),
        403 => (super::provider::ERR_NET_FORBIDDEN, "无权限访问该模型或服务"),
        429 => (
            super::provider::ERR_NET_RATE_LIMITED,
            "请求被限流或额度不足",
        ),
        500..=599 => (ERR_NET_SERVER, "供应商服务端异常"),
        _ => (ERR_NET_BAD_REQUEST, "供应商拒绝了该请求"),
    };
    let mut message = format!("HTTP {status}: {reason}");
    if !detail.is_empty() {
        message.push_str("；供应商返回: ");
        message.push_str(detail);
    }
    (code, message)
}

/// HTTP 状态码 → 稳定编号错误。
pub fn status_error(status: u16, detail: &str) -> AppError {
    let (code, message) = status_error_parts(status, detail);
    net_error(code, message)
}

/// 读取响应体全文（读失败映射为稳定编号错误）。
pub async fn read_body(response: reqwest::Response) -> Result<String, AppError> {
    response
        .text()
        .await
        .map_err(|err| net_error(ERR_NET_SERVER, format!("读取供应商响应失败: {err}")))
}

/// 按字符数截断（避免把多字节字符截成非法 UTF-8）。
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_status_only_for_rate_limit_and_server_errors() {
        assert!(should_retry_status(429));
        assert!(should_retry_status(500));
        assert!(should_retry_status(503));
        assert!(!should_retry_status(401));
        assert!(!should_retry_status(400));
        assert!(!should_retry_status(200));
    }

    #[test]
    fn status_codes_map_to_stable_codes() {
        assert_eq!(status_error(401, "").code(), "E-NET-103");
        assert_eq!(status_error(403, "").code(), "E-NET-104");
        assert_eq!(status_error(429, "").code(), "E-NET-105");
        assert_eq!(status_error(400, "").code(), "E-NET-106");
        assert_eq!(status_error(500, "").code(), "E-NET-107");
        assert!(status_error(401, "invalid key")
            .message()
            .contains("invalid key"));
    }

    #[test]
    fn truncate_is_char_safe() {
        let text = "中文".repeat(150);
        let cut = truncate_chars(&text, 10);
        assert_eq!(cut.chars().count(), 11); // 10 + 省略号
        assert!(cut.ends_with('…'));
        assert_eq!(truncate_chars("short", 10), "short");
    }

    #[test]
    fn request_errors_map_to_unreachable_or_timeout() {
        // 连接失败：指向不存在端口的本地地址。
        let client = default_client();
        let url = "http://127.0.0.1:9/nope";
        let err = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { client.get(url).send().await.unwrap_err() });
        assert_eq!(map_request_error(&err).code(), "E-NET-101");
    }
}
