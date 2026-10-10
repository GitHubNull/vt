//! 模型下载器（T-017，PRD 3.8.7 v1.5）：多线程分段并发 + 断点续传。
//!
//! - 分段：按文件大小切 N 个 Range 段并发下载（服务端不支持 Range 时回退单流）；
//! - 续传：每段独立 `.partN` 文件保留已下载字节，应用退出 / 失败后重下按段继续；
//! - 进度：调用方持有共享 `AtomicU64`，自行节流折算速度并上报（事件层在 asr.rs）；
//! - 代理：`build_client` 按配置构造 reqwest Client，只作用于模型下载；
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
pub fn build_client(proxy: Option<&str>) -> Result<reqwest::Client, AppError> {
    let mut builder = reqwest::Client::builder()
        .read_timeout(READ_TIMEOUT)
        .connect_timeout(Duration::from_secs(15));
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

/// 探测远端文件大小：优先 HEAD，被拒不支持时退回 1 字节 Range 探针读 Content-Range。
async fn probe_size(client: &reqwest::Client, url: &str) -> Option<u64> {
    if let Ok(resp) = client.head(url).send().await {
        let len = resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        if let Some(len) = len.filter(|l| *l > 0) {
            return Some(len);
        }
    }
    if let Ok(resp) = client
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send()
        .await
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

/// 下载单文件到 `dest`（多段并发，段级断点续传），进度累计进 `downloaded`。
///
/// - `expected_total` 为期望大小（HEAD 失败时的进度兜底），实际以远端探测为准；
/// - 服务端不支持 Range 时退化为单流整文件下载（已有字节视为前缀续传）；
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
        download_single(client, url, dest, downloaded).await?;
    } else {
        download_parts(client, url, dest, &parts, downloaded).await?;
    }
    Ok(total)
}

/// 单流下载（文件过小或探测不支持分段）：已有内容视为文件前缀，追加式续传。
async fn download_single(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    downloaded: Arc<AtomicU64>,
) -> Result<(), AppError> {
    let resume_at = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    let mut request = client.get(url);
    let mut append = resume_at > 0;
    if append {
        request = request.header(reqwest::header::RANGE, format!("bytes={resume_at}-"));
    }
    let response = request
        .send()
        .await
        .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("连接下载源失败: {err}")))?;
    match response.status() {
        reqwest::StatusCode::PARTIAL_CONTENT => {}
        reqwest::StatusCode::OK => {
            if resume_at > 0 {
                warn!(url, "下载源不支持断点续传，从头开始");
            }
            append = false;
            downloaded.store(0, Ordering::Relaxed);
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

    let mut stream = response.bytes_stream();
    let mut last_activity = Instant::now();
    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        file.write_all(&bytes)
                            .await
                            .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("写入下载文件失败: {err}")))?;
                        downloaded.fetch_add(bytes.len() as u64, Ordering::Relaxed);
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
    file.flush()
        .await
        .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("写入下载文件失败: {err}")))?;
    Ok(())
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

/// 单段下载：seek 写已下载前缀之后的区间；段内断点续传。
async fn download_part(
    client: &reqwest::Client,
    url: &str,
    part_path: &Path,
    part: PartRange,
    downloaded: Arc<AtomicU64>,
) -> Result<(), AppError> {
    let existing = std::fs::metadata(part_path).map(|m| m.len()).unwrap_or(0);
    if existing >= part.len() {
        downloaded.fetch_add(part.len(), Ordering::Relaxed);
        return Ok(());
    }
    downloaded.fetch_add(existing, Ordering::Relaxed);
    let start = part.start + existing;
    let response = client
        .get(url)
        .header(
            reqwest::header::RANGE,
            format!("bytes={start}-{}", part.end),
        )
        .send()
        .await
        .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("连接下载源失败: {err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Asr(
            ERR_DOWNLOAD,
            format!("下载源返回错误: {}", response.status()),
        ));
    }

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

    let mut stream = response.bytes_stream();
    let mut last_activity = Instant::now();
    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        file.write_all(&bytes)
                            .await
                            .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("写入分段文件失败: {err}")))?;
                        downloaded.fetch_add(bytes.len() as u64, Ordering::Relaxed);
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
    file.flush()
        .await
        .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("写入分段文件失败: {err}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::asr::MODELS;
    use super::*;

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
}
