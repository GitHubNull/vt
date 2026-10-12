//! 测试专用本地 HTTP mock 服务器（仅 `#[cfg(test)]` 编译）。
//!
//! 用极简 HTTP/1.1 实现（tokio TcpListener，零新增依赖）模拟供应商端点：
//! 按路径返回预制响应序列（可模拟重试场景），并记录收到的请求（方法 / 路径 /
//! Authorization 头 / 请求体）供适配器请求构造断言。响应统一 `Connection: close`，
//! 客户端每个请求新建连接，路由按次序消费。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// 单个预制响应。
#[derive(Debug, Clone)]
pub struct MockResponse {
    pub status: u16,
    pub body: String,
}

/// 按路径匹配的路由（同一路径的响应按请求次序消费，耗尽后重复最后一个）。
#[derive(Debug, Clone)]
pub struct MockRoute {
    pub path: &'static str,
    /// 每个响应写出前的延迟（毫秒，模拟慢响应）。
    pub delay_ms: u64,
    pub responses: Vec<MockResponse>,
}

/// 被记录的请求（断言请求构造用）。
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    /// Authorization 头原值（断言 `Bearer <key>`；本地供应商应为 None）。
    pub authorization: Option<String>,
    pub body: String,
}

/// 运行中的 mock 服务器（Drop 时中止 accept 任务）。
pub struct MockServer {
    pub base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    handle: JoinHandle<()>,
}

impl MockServer {
    /// 在 127.0.0.1 随机端口启动 mock 服务器。
    pub async fn spawn(routes: Vec<MockRoute>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock 服务器绑定 127.0.0.1:0");
        let addr = listener.local_addr().expect("读取本地端口");
        let requests: Arc<Mutex<Vec<RecordedRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let routes: Arc<Vec<MockRoute>> = Arc::new(routes);
        let counters: Arc<Mutex<HashMap<String, usize>>> = Arc::new(Mutex::new(HashMap::new()));
        let requests_clone = Arc::clone(&requests);
        let handle = tokio::spawn(async move {
            // 测试结束监听器被 Drop，accept 报错即退出。
            while let Ok((stream, _)) = listener.accept().await {
                let routes = Arc::clone(&routes);
                let counters = Arc::clone(&counters);
                let requests = Arc::clone(&requests_clone);
                tokio::spawn(async move {
                    serve_connection(stream, routes, counters, requests).await;
                });
            }
        });
        Self {
            base_url: format!("http://{addr}"),
            requests,
            handle,
        }
    }

    /// 全部已记录请求（快照）。
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().expect("请求记录锁").clone()
    }

    /// 最近一条请求（无请求时 panic，测试断言用）。
    pub fn last_request(&self) -> RecordedRequest {
        self.requests()
            .pop()
            .expect("mock 服务器应已收到至少一个请求")
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// 处理单条连接：读一个完整请求 → 选路由响应 → 关闭（Connection: close）。
async fn serve_connection(
    mut stream: TcpStream,
    routes: Arc<Vec<MockRoute>>,
    counters: Arc<Mutex<HashMap<String, usize>>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
) {
    let Some(request) = read_request(&mut stream).await else {
        return;
    };
    let route = routes.iter().find(|route| {
        // 去掉 query 后按路径匹配。
        request.path.split('?').next() == Some(route.path)
    });
    let (status, body, delay_ms) = match route {
        Some(route) => {
            let mut counter = counters.lock().expect("路由计数锁");
            let index = counter.entry(route.path.to_string()).or_insert(0);
            let response_index = (*index).min(route.responses.len().saturating_sub(1));
            *index += 1;
            drop(counter);
            match route.responses.get(response_index) {
                Some(response) => (response.status, response.body.clone(), route.delay_ms),
                None => (500, String::from("no scripted response"), 0),
            }
        }
        None => (404, String::from(r#"{"error":"mock: no route"}"#), 0),
    };
    if delay_ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
    }
    requests.lock().expect("请求记录锁").push(request);
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "Server Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// 读取一个完整 HTTP 请求（头 + Content-Length 定长 body）。
async fn read_request(stream: &mut TcpStream) -> Option<RecordedRequest> {
    let mut buffer: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(index) = find_subsequence(&buffer, b"\r\n\r\n") {
            break index;
        }
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None; // 连接在请求完成前关闭
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next()?.to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut content_length = 0usize;
    let mut authorization = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        if name == "content-length" {
            content_length = value.parse().unwrap_or(0);
        } else if name == "authorization" {
            authorization = Some(value.to_string());
        }
    }
    let body_start = header_end + 4;
    while buffer.len() < body_start + content_length {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = buffer
        .get(body_start..(body_start + content_length).min(buffer.len()))
        .map(|bytes| String::from_utf8_lossy(bytes).to_string())
        .unwrap_or_default();
    Some(RecordedRequest {
        method,
        path,
        authorization,
        body,
    })
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn serves_scripted_responses_in_order() {
        let server = MockServer::spawn(vec![MockRoute {
            path: "/ping",
            delay_ms: 0,
            responses: vec![
                MockResponse {
                    status: 500,
                    body: "a".to_string(),
                },
                MockResponse {
                    status: 200,
                    body: r#"{"ok":true}"#.to_string(),
                },
            ],
        }])
        .await;
        let client = reqwest::Client::new();
        let first = client
            .get(format!("{}/ping", server.base_url))
            .send()
            .await
            .unwrap();
        assert_eq!(first.status(), 500);
        let second = client
            .get(format!("{}/ping", server.base_url))
            .header("Authorization", "Bearer tok")
            .send()
            .await
            .unwrap();
        assert_eq!(second.status(), 200);
        assert_eq!(second.text().await.unwrap(), r#"{"ok":true}"#);
        // 未匹配路由 → 404。
        let missing = client
            .get(format!("{}/none", server.base_url))
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), 404);

        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[1].method, "GET");
        assert_eq!(requests[1].authorization.as_deref(), Some("Bearer tok"));
    }
}
