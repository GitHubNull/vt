//! 模型下载器（T-017，PRD 3.8.7 v1.5）：多线程分段并发 + 断点续传。
//!
//! - 分段：按文件大小切 N 个 Range 段并发下载（服务端不支持 Range 时回退单流）；
//! - 续传：每段独立 `.partN` 文件保留已下载字节，应用退出 / 失败后重下按段继续；
//! - 进度：调用方持有共享 `AtomicU64`，自行节流折算速度并上报（事件层在 asr.rs）；
//!   本模块保证进度单调不减（绝不重置计数器）、单文件维度不重复计数（封顶 100%）；
//! - 代理：`build_client` 按配置构造 reqwest Client，只作用于模型下载；
//! - 重定向：客户端禁用自动跳转，下载请求手动跟随 3xx 并每跳重带 Range 头
//!   （GitHub 302 到对象存储后 Range 可能丢失，丢失即整包追加污染段文件）；
//! - 源解析：镜像源直传散文件（免解包），官方源整包归档由 asr.rs 解包。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tracing::warn;

use super::asr::{ModelMeta, MODEL_ONNX_NAME, TOKENS_NAME};
use crate::error::AppError;

/// 下载错误编号（与 asr.rs 的 E-ASR-006 一致：下载失败）。
const ERR_DOWNLOAD: u32 = 6;

/// 默认分段并发数（镜像源单连接实测 ~12 MB/s，4 段约 40 MB/s 量级）。
pub const DEFAULT_THREADS: usize = 4;
/// 最小分段大小：文件小于 2 × 该值时退化为单流（分段开销不划算）。
pub const MIN_PART_BYTES: u64 = 8 * 1024 * 1024;
/// 单段网络读取超时。
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// 单段数据停滞兜底超时（续传场景重新发起即可）。
const STALL_TIMEOUT: Duration = Duration::from_secs(300);
/// 手动跟随 3xx 重定向的跳数上限（防重定向环）。
const MAX_REDIRECTS: usize = 5;

/// 镜像源根地址（hf-mirror.com，csukuangfj 官方同步仓库；2026-10-10 实测单连接 ~12 MB/s、支持 Range）。
const MIRROR_ROOT: &str =
    "https://hf-mirror.com/csukuangfj/sherpa-onnx-paraformer-zh-2023-03-28/resolve/main";

/// 一段下载区间（含端点）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartRange {
    pub start: u64,
    pub end: u64,
}

impl PartRange {
    /// 段长度（字节）。
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start) + 1
    }

    /// 段是否为空（total = 0 的退化场景）。
    pub fn is_empty(&self) -> bool {
        self.end < self.start
    }
}

/// 按总大小与线程数切分段计划；过小文件退化为单段。
pub fn plan_parts(total: u64, threads: usize, min_part: u64) -> Vec<PartRange> {
    if total == 0 || threads <= 1 || total < min_part.saturating_mul(2) {
        return if total == 0 {
            Vec::new()
        } else {
            vec![PartRange {
                start: 0,
                end: total - 1,
            }]
        };
    }
    let threads = threads.min((total / min_part).max(1) as usize);
    let part_len = total / threads as u64;
    let mut parts = Vec::with_capacity(threads);
    for i in 0..threads {
        let start = i as u64 * part_len;
        let end = if i == threads - 1 {
            total - 1
        } else {
            start + part_len - 1
        };
        parts.push(PartRange { start, end });
    }
    parts
}

/// 镜像源直传文件清单（模型文件 + 词表，逐文件下载无需解包）。
pub struct MirrorFile {
    pub url: String,
    /// 落入模型目录的文件名（与官方解包后命名一致）。
    pub dest_name: String,
}

/// 解析镜像源文件清单：模型主文件按档位取归档内同名成员，词表固定。
pub fn mirror_files(meta: &ModelMeta) -> Vec<MirrorFile> {
    let file_name = meta.model_file.rsplit('/').next().unwrap_or("model.onnx");
    let tokens_name = meta.tokens_file.rsplit('/').next().unwrap_or("tokens.txt");
    vec![
        MirrorFile {
            url: format!("{MIRROR_ROOT}/{file_name}"),
            dest_name: MODEL_ONNX_NAME.to_string(),
        },
        MirrorFile {
            url: format!("{MIRROR_ROOT}/{tokens_name}"),
            dest_name: TOKENS_NAME.to_string(),
        },
    ]
}

/// 构造下载客户端：直连或按配置走代理（代理只作用于模型下载，T-017）。
///
/// 禁用自动重定向：下载请求统一走 `send_with_redirects` 手动跟随 3xx，
/// 保证 Range 头在每一跳都重新附带（GitHub 302 后自动跳转可能丢 Range，
/// 导致续传偏移错乱或整包追加污染段文件）。
pub fn build_client(proxy: Option<&str>) -> Result<reqwest::Client, AppError> {
    let mut builder = reqwest::Client::builder()
        .read_timeout(READ_TIMEOUT)
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(proxy) = proxy.filter(|p| !p.is_empty()) {
        builder = builder.proxy(
            reqwest::Proxy::all(proxy)
                .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("代理地址无效: {err}")))?,
        );
    }
    builder
        .build()
        .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("下载客户端构造失败: {err}")))
}

/// 发送请求并手动跟随 3xx 重定向（最多 `MAX_REDIRECTS` 跳）。
///
/// 每一跳都重新附带 `range` 头（若有）：302/303 的目标地址（如 GitHub 对象存储
/// 签名 URL）不保证继承原请求头，Range 丢失会让续传拿到整包 200 响应。
/// `location` 支持相对地址（相对上一跳 URL 解析）。
async fn send_with_redirects(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    range: Option<&str>,
) -> Result<reqwest::Response, AppError> {
    let mut url = url.to_string();
    for _hop in 0..=MAX_REDIRECTS {
        let mut request = client.request(method.clone(), &url);
        if let Some(range) = range {
            request = request.header(reqwest::header::RANGE, range);
        }
        let response = request
            .send()
            .await
            .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("连接下载源失败({url}): {err}")))?;
        if !response.status().is_redirection() {
            return Ok(response);
        }
        let status = response.status();
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                AppError::Asr(ERR_DOWNLOAD, format!("下载源重定向缺少 Location: {status}"))
            })?;
        url = match reqwest::Url::parse(location) {
            Ok(absolute) => absolute.to_string(),
            Err(_) => {
                let base = reqwest::Url::parse(&url)
                    .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("下载源地址无效: {err}")))?;
                base.join(location)
                    .map_err(|err| {
                        AppError::Asr(ERR_DOWNLOAD, format!("下载源重定向地址无效: {err}"))
                    })?
                    .to_string()
            }
        };
    }
    Err(AppError::Asr(
        ERR_DOWNLOAD,
        format!("下载源重定向次数超过上限({MAX_REDIRECTS})"),
    ))
}

/// 探测远端文件大小：优先 HEAD，被拒不支持时退回 1 字节 Range 探针读 Content-Range。
pub(crate) async fn probe_size(client: &reqwest::Client, url: &str) -> Option<u64> {
    if let Ok(resp) = send_with_redirects(client, reqwest::Method::HEAD, url, None).await {
        let len = resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        if let Some(len) = len.filter(|l| *l > 0) {
            return Some(len);
        }
    }
    if let Ok(resp) =
        send_with_redirects(client, reqwest::Method::GET, url, Some("bytes=0-0")).await
    {
        if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            return resp
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit('/').next())
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|l| *l > 0);
        }
    }
    None
}

/// 续传 Range 头：段起点 + 已落盘字节，保证请求偏移与已写前缀严格一致。
fn resume_range_value(part: PartRange, existing: u64) -> String {
    format!("bytes={}-{}", part.start + existing, part.end)
}

/// 下载单文件到 `dest`（多段并发，段级断点续传），进度累计进 `downloaded`。
///
/// - `expected_total` 为期望大小（HEAD 失败时的进度兜底），实际以远端探测为准；
/// - 服务端不支持 Range 时退化为单流整文件下载，或分段模式下按整流截取本段区间；
/// - 进度只累计新写入 `dest` / 段文件的字节（另有跨会话续传前缀补偿），
///   任一路径都不会重置计数器，保证单调不减、单文件维度不超额（封顶 100%）；
/// - 任一段失败即整体失败（其余段被取消），已下载 `.partN` 保留供下次续传。
pub async fn download_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected_total: u64,
    threads: usize,
    downloaded: Arc<AtomicU64>,
) -> Result<u64, AppError> {
    let total = probe_size(client, url)
        .await
        .unwrap_or(expected_total)
        .max(expected_total);
    let parts = plan_parts(total, threads, MIN_PART_BYTES);
    if parts.len() <= 1 {
        download_single(client, url, dest, total, downloaded).await?;
    } else {
        download_parts(client, url, dest, &parts, downloaded).await?;
    }
    Ok(total)
}

/// 流式落盘：跳过流前缀 `skip` 字节，最多写入 `remaining` 字节并计入进度。
///
/// 服务器多发（206 超出请求区间 / 200 整流）时提前结束读取，写入与进度
/// 都以 `remaining` 为限——杜绝进度超 100% 与脏数据落盘。流提前结束
/// （`remaining` 未写满）视为下载中断，报错带期望/实际值。
async fn stream_to_file(
    response: reqwest::Response,
    file: &mut tokio::fs::File,
    mut skip: u64,
    mut remaining: u64,
    downloaded: &AtomicU64,
    context: &str,
) -> Result<(), AppError> {
    let expected = remaining;
    let mut stream = response.bytes_stream();
    let mut last_activity = Instant::now();
    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        let mut buf: &[u8] = &bytes;
                        if skip > 0 {
                            let drop_len = (skip as usize).min(buf.len());
                            buf = &buf[drop_len..];
                            skip -= drop_len as u64;
                        }
                        let take = (remaining as usize).min(buf.len());
                        if take > 0 {
                            file.write_all(&buf[..take])
                                .await
                                .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("{context}: {err}")))?;
                            downloaded.fetch_add(take as u64, Ordering::Relaxed);
                            remaining -= take as u64;
                        }
                        if remaining == 0 {
                            break;
                        }
                        last_activity = Instant::now();
                    }
                    Some(Err(err)) => {
                        return Err(AppError::Asr(ERR_DOWNLOAD, format!("模型下载中断: {err}")));
                    }
                    None => break,
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(250)) => {
                if last_activity.elapsed() > STALL_TIMEOUT {
                    return Err(AppError::Asr(ERR_DOWNLOAD, "下载数据停滞超时".to_string()));
                }
            }
        }
    }
    if remaining > 0 {
        return Err(AppError::Asr(
            ERR_DOWNLOAD,
            format!(
                "{context}: 数据不完整，期望 {expected} 字节，实际 {} 字节",
                expected - remaining
            ),
        ));
    }
    file.flush()
        .await
        .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("{context}: {err}")))?;
    Ok(())
}

/// 单流下载（文件过小或探测不支持分段）：已有内容视为文件前缀，追加式续传。
///
/// 服务器忽略 Range 返回 200 时：已有前缀与整包拼接必然损坏（size/SHA 失败），
/// 故丢弃旧文件从头重下；进度计数器不做重置（重置会造成 0↔100% 横跳），
/// 重复计数量最多为旧前缀字节数，远小于重置整个共享计数器的影响。
async fn download_single(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    total: u64,
    downloaded: Arc<AtomicU64>,
) -> Result<(), AppError> {
    let mut resume_at = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    if resume_at > total {
        // 旧文件比探测大小还大：内容必然错位，丢弃重下。
        warn!(url, resume_at, total, "已有文件超出探测大小，丢弃重下");
        let _ = std::fs::remove_file(dest);
        resume_at = 0;
    }
    let mut request_range = None;
    let mut append = resume_at > 0;
    if append {
        request_range = Some(format!("bytes={resume_at}-"));
    }
    let response =
        send_with_redirects(client, reqwest::Method::GET, url, request_range.as_deref()).await?;
    let mut remaining = total - resume_at;
    match response.status() {
        reqwest::StatusCode::PARTIAL_CONTENT => {}
        reqwest::StatusCode::OK => {
            if resume_at > 0 {
                warn!(url, "下载源不支持断点续传，丢弃已下载字节从头开始");
            }
            append = false;
            remaining = total;
        }
        status => {
            return Err(AppError::Asr(
                ERR_DOWNLOAD,
                format!("下载源返回错误: {status}"),
            ));
        }
    }

    let mut file = if append {
        tokio::fs::OpenOptions::new().append(true).open(dest).await
    } else {
        tokio::fs::File::create(dest).await
    }
    .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("创建下载文件失败: {err}")))?;

    stream_to_file(
        response,
        &mut file,
        0,
        remaining,
        &downloaded,
        "写入下载文件失败",
    )
    .await
}

/// 多段并发下载：每段写独立 `.partN`，全部完成后按序拼装进 `dest` 并清理段文件。
async fn download_parts(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    parts: &[PartRange],
    downloaded: Arc<AtomicU64>,
) -> Result<(), AppError> {
    let mut set = tokio::task::JoinSet::new();
    for (index, part) in parts.iter().enumerate() {
        let client = client.clone();
        let url = url.to_string();
        let part = *part;
        let part_path = part_file_path(dest, index);
        let downloaded = downloaded.clone();
        set.spawn(async move { download_part(&client, &url, &part_path, part, downloaded).await });
    }
    // 任一段失败即整体失败（JoinSet drop 会 abort 未完成 task，段文件保留续传）。
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(err),
            Err(err) => return Err(AppError::Asr(ERR_DOWNLOAD, format!("下载任务失败: {err}"))),
        }
    }

    // 拼装前逐段校验落盘大小：续传偏移与段计划不一致（远端大小变化 / 旧版残留）
    // 时报错带段号与期望/实际值，绝不把错位数据拼进最终文件。
    for (index, part) in parts.iter().enumerate() {
        let part_path = part_file_path(dest, index);
        let actual = std::fs::metadata(&part_path)
            .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("读取分段文件失败: {err}")))?
            .len();
        if actual != part.len() {
            return Err(AppError::Asr(
                ERR_DOWNLOAD,
                format!(
                    "分段文件大小不匹配（第 {} 段: 期望 {} 字节，实际 {} 字节），续传偏移与已落盘字节不一致",
                    index + 1,
                    part.len(),
                    actual
                ),
            ));
        }
    }

    let mut out = tokio::fs::File::create(dest)
        .await
        .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("创建模型文件失败: {err}")))?;
    for index in 0..parts.len() {
        let part_path = part_file_path(dest, index);
        let mut segment = tokio::fs::File::open(&part_path)
            .await
            .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("读取分段文件失败: {err}")))?;
        tokio::io::copy(&mut segment, &mut out)
            .await
            .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("拼装模型文件失败: {err}")))?;
    }
    out.flush()
        .await
        .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("写入模型文件失败: {err}")))?;
    for index in 0..parts.len() {
        let _ = std::fs::remove_file(part_file_path(dest, index));
    }
    Ok(())
}

/// 段文件路径：`{dest}.part{index}`。
fn part_file_path(dest: &Path, index: usize) -> PathBuf {
    let name = format!("{}.part{index}", dest.display());
    PathBuf::from(name)
}

/// 速度计算：区间字节差 / 区间耗时（bytes-per-sec），并更新游标（T-017 实时速度显示）。
pub fn speed_of(cursor_bytes: &mut u64, current_bytes: u64, cursor_tick: &mut Instant) -> u64 {
    let elapsed = cursor_tick.elapsed();
    let speed = if elapsed.is_zero() {
        0
    } else {
        current_bytes.saturating_sub(*cursor_bytes) * 1000 / elapsed.as_millis().max(1) as u64
    };
    *cursor_bytes = current_bytes;
    *cursor_tick = Instant::now();
    speed
}

/// 单段下载：基于已落盘字节续传（Range 从 `part.start + existing` 起）。
///
/// - 206 Partial Content：seek 到 `existing` 追加续写；
/// - 200 OK（服务器忽略 Range）：旧段前缀是脏数据，删除后按整流跳过 `part.start`
///   字节、截取本段区间重写，绝不拼接脏数据（避免 size/SHA-256 校验失败）；
/// - 段文件比计划大（远端大小变化 / 旧计划残留）：内容必然错位，丢弃整段重下；
/// - 进度只计本段字节一次（`existing` 仅作为跨会话续传补偿），不超限、不回退。
async fn download_part(
    client: &reqwest::Client,
    url: &str,
    part_path: &Path,
    part: PartRange,
    downloaded: Arc<AtomicU64>,
) -> Result<(), AppError> {
    let mut existing = std::fs::metadata(part_path).map(|m| m.len()).unwrap_or(0);
    if existing == part.len() {
        // 本段已下满（上次会话落盘）：补偿进度即可，无需重下。
        downloaded.fetch_add(part.len(), Ordering::Relaxed);
        return Ok(());
    }
    if existing > part.len() {
        // 分段计划变化（如远端大小不同）导致旧段超出本段：内容必然错位，丢弃重下。
        warn!(
            url,
            start = part.start,
            existing,
            planned = part.len(),
            "分段文件超出计划大小，丢弃重下"
        );
        let _ = std::fs::remove_file(part_path);
        existing = 0;
    }

    let range_value = resume_range_value(part, existing);
    let response =
        send_with_redirects(client, reqwest::Method::GET, url, Some(&range_value)).await?;

    let (mut file, skip, remaining) = match response.status() {
        reqwest::StatusCode::PARTIAL_CONTENT => {
            downloaded.fetch_add(existing, Ordering::Relaxed);
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create(true)
                // 显式不截断：段文件按已有前缀续写，截断会破坏断点续传。
                .truncate(false)
                .open(part_path)
                .await
                .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("创建分段文件失败: {err}")))?;
            file.seek(std::io::SeekFrom::Start(existing))
                .await
                .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("写入分段文件失败: {err}")))?;
            (file, 0u64, part.len() - existing)
        }
        reqwest::StatusCode::OK => {
            // 服务器忽略 Range：整流从文件头 0 开始。已有前缀与整流拼接必然损坏，
            // 删除旧段后跳过 part.start 字节、截取本段长度重写。
            warn!(
                url,
                start = part.start,
                end = part.end,
                "下载源忽略 Range，分段按整流截取"
            );
            let _ = std::fs::remove_file(part_path);
            let file = tokio::fs::File::create(part_path)
                .await
                .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("创建分段文件失败: {err}")))?;
            (file, part.start, part.len())
        }
        status => {
            return Err(AppError::Asr(
                ERR_DOWNLOAD,
                format!("下载源返回错误: {status}"),
            ));
        }
    };

    stream_to_file(
        response,
        &mut file,
        skip,
        remaining,
        &downloaded,
        "写入分段文件失败",
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::super::asr::MODELS;
    use super::*;

    /// 极简临时目录（tech/02 未登记 tempfile crate，测试内自建）。
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vt-md-test-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("测试临时目录");
        dir
    }

    /// 极简 HTTP 服务器：顺序服务 `count` 次请求，返回 (url, 捕获的请求头列表 handle)。
    /// 响应由 `respond` 依据收到的请求头与序号生成（测试 200/206/302 等行为）。
    async fn mini_server(
        count: usize,
        respond: impl Fn(&str, usize) -> Vec<u8> + Send + 'static,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定测试端口");
        let addr = listener.local_addr().expect("测试端口地址");
        let handle = tokio::spawn(async move {
            let mut heads = Vec::new();
            for index in 0..count {
                let (mut socket, _) = listener.accept().await.expect("接受测试连接");
                let mut buf = Vec::new();
                let mut tmp = [0u8; 2048];
                // 测试请求均不带 body：读到头结束符即可。
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = socket.read(&mut tmp).await.expect("读取测试请求");
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                }
                let head = String::from_utf8_lossy(&buf).to_string();
                socket
                    .write_all(&respond(&head, index))
                    .await
                    .expect("写入测试响应");
                heads.push(head);
            }
            heads
        });
        (format!("http://{addr}/file"), handle)
    }

    /// 构造原始 HTTP 响应字节（content-length 按 body 实际字节数计算，允许非 UTF-8 数据）。
    fn http_response(status_line: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
        let mut response = format!("{status_line}\r\n").into_bytes();
        for (key, value) in headers {
            response.extend_from_slice(format!("{key}: {value}\r\n").as_bytes());
        }
        response.extend_from_slice(
            format!(
                "content-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        );
        response.extend_from_slice(body);
        response
    }

    #[test]
    fn plan_parts_single_for_small_file() {
        let parts = plan_parts(4 * 1024 * 1024, 4, MIN_PART_BYTES);
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0],
            PartRange {
                start: 0,
                end: 4 * 1024 * 1024 - 1
            }
        );
    }

    #[test]
    fn plan_parts_covers_full_range_without_overlap() {
        let total = 100_000_001u64;
        let parts = plan_parts(total, 4, MIN_PART_BYTES);
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0].start, 0);
        assert_eq!(parts.last().unwrap().end, total - 1);
        for window in parts.windows(2) {
            assert_eq!(window[0].end + 1, window[1].start);
        }
        let covered: u64 = parts.iter().map(PartRange::len).sum();
        assert_eq!(covered, total);
    }

    #[test]
    fn plan_parts_caps_threads_for_medium_file() {
        // 够两段但不够四段：线程数压到 2。
        let total = MIN_PART_BYTES * 2 + 100;
        let parts = plan_parts(total, 4, MIN_PART_BYTES);
        assert_eq!(parts.len(), 2);
    }

    #[test]
    fn plan_parts_zero_total_is_empty() {
        assert!(plan_parts(0, 4, MIN_PART_BYTES).is_empty());
    }

    #[test]
    fn mirror_files_resolves_direct_file_urls() {
        let meta = &MODELS[0];
        let files = mirror_files(meta);
        assert_eq!(files.len(), 2);
        assert!(files[0].url.ends_with("model.int8.onnx"));
        assert_eq!(files[0].dest_name, "model.onnx");
        assert!(files[1].url.ends_with("tokens.txt"));
        assert_eq!(files[1].dest_name, "tokens.txt");
    }

    #[test]
    fn speed_of_measures_delta_over_elapsed() {
        let mut cursor_bytes = 0u64;
        let mut cursor_tick = Instant::now();
        std::thread::sleep(Duration::from_millis(50));
        let speed = speed_of(&mut cursor_bytes, 10_000, &mut cursor_tick);
        assert!(speed > 0, "50ms 内新增 10000 字节应有正速度");
        assert_eq!(cursor_bytes, 10_000);
        // 无新增字节 → 速度 0。
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(speed_of(&mut cursor_bytes, 10_000, &mut cursor_tick), 0);
    }

    // ---- 续传 offset 计算 ----

    #[test]
    fn resume_range_starts_after_existing_bytes() {
        let part = PartRange {
            start: 1_000,
            end: 1_999,
        };
        // 无已落盘字节：从段起点起。
        assert_eq!(resume_range_value(part, 0), "bytes=1000-1999");
        // 已落盘 250 字节：Range 从段起点 + 250 起，与磁盘前缀严格对齐。
        assert_eq!(resume_range_value(part, 250), "bytes=1250-1999");
        // 差 1 字节下满：只取最后一个字节。
        assert_eq!(resume_range_value(part, 999), "bytes=1999-1999");
    }

    // ---- 200 vs 206 处理（本地极简 HTTP 集成） ----

    #[tokio::test]
    async fn part_206_appends_after_existing_prefix() {
        // 段 [100, 199]：预置 40 字节前缀，206 续传剩余 60 字节。
        let part = PartRange {
            start: 100,
            end: 199,
        };
        let prefix: Vec<u8> = (100..140).collect();
        let suffix: Vec<u8> = (140..200).collect();
        let dir = temp_dir("part-206");
        let part_path = dir.join("model.onnx.part0");
        std::fs::write(&part_path, &prefix).expect("预置段前缀");

        let (url, request) = mini_server(1, move |head, _index| {
            assert!(
                head.contains("range: bytes=140-199"),
                "续传请求必须基于已落盘字节: {head}"
            );
            http_response(
                "HTTP/1.1 206 Partial Content",
                &[("content-range", "bytes 140-199/200")],
                &suffix,
            )
        })
        .await;

        let client = build_client(None).expect("测试客户端");
        let downloaded = Arc::new(AtomicU64::new(0));
        download_part(&client, &url, &part_path, part, downloaded.clone())
            .await
            .expect("206 续传下载");

        let expected: Vec<u8> = (100..200).collect();
        assert_eq!(
            std::fs::read(&part_path).expect("读取段文件"),
            expected,
            "续传后段内容 = 前缀 + 206 数据"
        );
        // 进度 = 前缀补偿 40 + 新增 60，恰好一段且不超。
        assert_eq!(downloaded.load(Ordering::Relaxed), part.len());
        request.await.expect("测试服务器任务失败（响应断言 panic）");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn part_200_slicing_drops_stale_prefix() {
        // 服务器忽略 Range 返回整包 200：必须丢弃脏前缀，从整流截取本段区间。
        let part = PartRange {
            start: 100,
            end: 199,
        };
        let full: Vec<u8> = (0..200).collect();
        let dir = temp_dir("part-200");
        let part_path = dir.join("model.onnx.part0");
        std::fs::write(&part_path, vec![b'x'; 40]).expect("预置脏前缀");

        let (url, request) = mini_server(1, move |head, _index| {
            assert!(head.contains("range:"), "分段请求必须带 Range 头: {head}");
            http_response("HTTP/1.1 200 OK", &[], &full)
        })
        .await;

        let client = build_client(None).expect("测试客户端");
        let downloaded = Arc::new(AtomicU64::new(0));
        download_part(&client, &url, &part_path, part, downloaded.clone())
            .await
            .expect("200 整流截取下载");

        let expected: Vec<u8> = (100..200).collect();
        assert_eq!(
            std::fs::read(&part_path).expect("读取段文件"),
            expected,
            "200 回退后段内容 = 整流 [part.start, part.end] 截取，无脏前缀"
        );
        assert_eq!(downloaded.load(Ordering::Relaxed), part.len());
        request.await.expect("测试服务器任务失败（响应断言 panic）");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn part_truncated_stream_errors_with_expected_actual() {
        // 服务器 206 但数据短缺：报错须含期望/实际值。
        let part = PartRange { start: 0, end: 99 };
        let short: Vec<u8> = (0..30).collect();
        let dir = temp_dir("part-short");
        let part_path = dir.join("model.onnx.part0");

        let (url, request) = mini_server(1, move |_head, _index| {
            http_response(
                "HTTP/1.1 206 Partial Content",
                &[("content-range", "bytes 0-99/100")],
                &short,
            )
        })
        .await;

        let client = build_client(None).expect("测试客户端");
        let err = download_part(&client, &url, &part_path, part, Arc::new(AtomicU64::new(0)))
            .await
            .expect_err("短缺数据必须报错");
        let message = format!("{err:?}");
        assert!(
            message.contains("期望 100 字节"),
            "报错须含期望值: {message}"
        );
        assert!(
            message.contains("实际 30 字节"),
            "报错须含实际值: {message}"
        );
        request.await.expect("测试服务器任务失败（响应断言 panic）");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- 进度单调性 ----

    #[tokio::test]
    async fn single_200_restart_never_resets_progress() {
        // 回归：服务器忽略 Range 返回 200 时，进度计数器绝不可被重置
        // （旧实现 store(0) 造成 0↔100% 横跳）；已计数的前序字节保持单调。
        let full: Vec<u8> = (0..100).collect();
        let expected = full.clone();
        let dir = temp_dir("single-200");
        let dest = dir.join("tokens.txt");
        std::fs::write(&dest, vec![b'y'; 50]).expect("预置旧前缀");

        let (url, request) = mini_server(1, move |_head, _index| {
            http_response("HTTP/1.1 200 OK", &[], &full)
        })
        .await;

        let client = build_client(None).expect("测试客户端");
        let downloaded = Arc::new(AtomicU64::new(1234));
        download_single(&client, &url, &dest, 100, downloaded.clone())
            .await
            .expect("200 重下");

        // 计数器未被重置：1234（前序文件进度） + 100（本文件实际写入）。
        assert_eq!(
            downloaded.load(Ordering::Relaxed),
            1234 + 100,
            "进度必须单调不减，禁止重置计数器"
        );
        assert_eq!(
            std::fs::read(&dest).expect("读取下载文件"),
            expected,
            "200 重下后文件为整包内容，无脏前缀"
        );
        request.await.expect("测试服务器任务失败（响应断言 panic）");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn part_progress_never_exceeds_part_len() {
        // 服务器 206 但多发数据（Content-Range 区间外仍吐字节）：
        // 进度以段长度为限，绝不超 100%。
        let part = PartRange { start: 0, end: 9 };
        let mut body: Vec<u8> = (0..10).collect();
        body.extend_from_slice(&[b'!'; 50]); // 多发 50 字节
        let dir = temp_dir("part-overflow");
        let part_path = dir.join("model.onnx.part0");

        let (url, request) = mini_server(1, move |_head, _index| {
            http_response(
                "HTTP/1.1 206 Partial Content",
                &[("content-range", "bytes 0-9/10")],
                &body,
            )
        })
        .await;

        let client = build_client(None).expect("测试客户端");
        let downloaded = Arc::new(AtomicU64::new(0));
        download_part(&client, &url, &part_path, part, downloaded.clone())
            .await
            .expect("多发数据截断下载");
        assert_eq!(
            downloaded.load(Ordering::Relaxed),
            part.len(),
            "进度封顶段长度，多发数据不计入"
        );
        request.await.expect("测试服务器任务失败（响应断言 panic）");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- 302 重定向：Range 头必须随身携带 ----

    #[tokio::test]
    async fn redirect_preserves_range_header() {
        // 第一跳 302 到第二跳服务器；第二跳必须收到完整 Range 头（GitHub 302 场景）。
        let (url_b, request_b) = mini_server(1, move |head, _index| {
            assert!(
                head.contains("range: bytes=7-9"),
                "重定向后 Range 头必须保留: {head}"
            );
            http_response(
                "HTTP/1.1 206 Partial Content",
                &[("content-range", "bytes 7-9/100")],
                b"xyz",
            )
        })
        .await;
        let (url_a, request_a) = mini_server(1, move |_head, _index| {
            http_response("HTTP/1.1 302 Found", &[("location", &url_b)], b"")
        })
        .await;

        let client = build_client(None).expect("测试客户端");
        let response =
            send_with_redirects(&client, reqwest::Method::GET, &url_a, Some("bytes=7-9"))
                .await
                .expect("手动跟随重定向");
        assert_eq!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT);
        request_a
            .await
            .expect("测试服务器任务失败（响应断言 panic）");
        request_b
            .await
            .expect("测试服务器任务失败（响应断言 panic）");
    }

    #[tokio::test]
    async fn probe_size_follows_redirect_and_reads_content_range() {
        // HEAD 与 GET 探针都被 302：手动跟随后从 GET 探针的 Content-Range 读总大小。
        // 服务器 B 需服务两跳（HEAD + GET）；服务器 A 每次均回 302。
        let (url_b, request_b) = mini_server(2, move |head, _index| {
            if head.starts_with("HEAD") {
                http_response("HTTP/1.1 405 Method Not Allowed", &[], b"")
            } else {
                assert!(
                    head.contains("range: bytes=0-0"),
                    "探针请求必须带 Range 头: {head}"
                );
                http_response(
                    "HTTP/1.1 206 Partial Content",
                    &[("content-range", "bytes 0-0/12345")],
                    b"a",
                )
            }
        })
        .await;
        let (url_a, request_a) = mini_server(2, move |_head, _index| {
            http_response("HTTP/1.1 302 Found", &[("location", &url_b)], b"")
        })
        .await;

        let client = build_client(None).expect("测试客户端");
        let size = probe_size(&client, &url_a).await.expect("探测大小");
        assert_eq!(size, 12345);
        request_a
            .await
            .expect("测试服务器任务失败（响应断言 panic）");
        request_b
            .await
            .expect("测试服务器任务失败（响应断言 panic）");
    }
}
