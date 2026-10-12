//! 识别标点恢复服务（T-545，F-SET-02、PRD 3.1.5）。
//!
//! - 模型：sherpa-onnx OfflinePunctuation（ct-transformer 中英标点，int8 量化），
//!   归档仅官方 Release 提供（镜像源无对应仓库，2026-10-12 实测 404），
//!   词表内嵌 ONNX metadata，引擎只需 `model.onnx` 单文件；
//! - 生命周期：走 model_download 下载体系（多线程分段 + 断点续传 + `model:progress`
//!   事件 + SHA-256 校验 + 删除），文件存模型目录 `<models_dir>/punct-ct-transformer-zh-en-int8/`；
//! - 执行：AsrService 在识别推理的 `spawn_blocking` 内、`Recognized` 事件前调用
//!   [`PunctService::punctuate_sync`]；引擎懒加载（哈希把关 + 阻塞加载），加载后常驻；
//! - 降级：开关关闭 / 模型未下载 / 校验失败 / 引擎加载失败一律原文直出，
//!   不报错、不阻塞识别主链路（产品原则 1 本地优先，NFR-P-03 零开销降级）；
//! - 隐私（NFR-S-02）：识别文本只经内存与事件通道传递，绝不写入日志；
//!   标点推理纯本地，模型文件不出本机。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use sha2::Digest;
use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::{info, warn};

use crate::error::AppError;
use crate::events::{AppEvent, DownloadStatus};
use crate::services::asr::{hex_lower, DownloadProgress};
use crate::services::config_store::ConfigStore;
use crate::services::model_download;

/// 错误编号（与 asr.rs 同域同义：未下载 1 / 加载失败 2 / 哈希不符 3 / 下载失败 6）。
const ERR_MODEL_MISSING: u32 = 1;
const ERR_LOAD_FAILED: u32 = 2;
const ERR_HASH_MISMATCH: u32 = 3;
const ERR_DOWNLOAD: u32 = 6;

/// 标点模型稳定 ID（进度事件 model_id、模型目录名共用）。
pub const PUNCT_MODEL_ID: &str = "punct-ct-transformer-zh-en-int8";

/// 标点模型显示名。
const PUNCT_MODEL_NAME: &str = "标点恢复";

/// 官方归档下载地址（sherpa-onnx Release `punctuation-models` tag；
/// 2026-10-12 curl -IL 实测 302 → 200。镜像源无对应仓库，见模块注释）。
const ARCHIVE_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/punctuation-models/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8.tar.bz2";

/// 归档大小（字节，2026-10-12 实测；下载进度兜底值）。
const ARCHIVE_SIZE_BYTES: u64 = 64_717_756;

/// 归档内模型文件成员路径（词表 tokens.json 内嵌 ONNX，不参与加载，不解包）。
const MODEL_MEMBER: &str =
    "sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8/model.int8.onnx";

/// 模型文件大小（字节，2026-10-12 实测；完整性校验 + 前端展示用）。
pub const MODEL_SIZE_BYTES: u64 = 75_519_198;

/// 模型文件 SHA-256（2026-10-12 从官方归档成员实测计算；损坏拒绝加载）。
const MODEL_SHA256: &str = "65a3fb9f5ad7bfb96bf69e0dc4481df97f6ee60513c1d94ce981ba6effd524b1";

/// 解包后统一文件名（与 asr.rs 识别模型命名约定一致）。
const MODEL_ONNX_NAME: &str = "model.onnx";

/// 下载进度事件节流间隔（与 asr.rs 一致，避免刷爆事件通道）。
const PROGRESS_THROTTLE: std::time::Duration = std::time::Duration::from_millis(250);

/// 模型文件哈希状态机（原子存储，单模型无需表结构）。
const HASH_UNKNOWN: u8 = 0;
const HASH_VERIFIED: u8 = 1;
const HASH_INVALID: u8 = 2;

/// 标点推理引擎句柄（`add_punctuation` 为 &mut self，经 Mutex 串行化；
/// 同一时间只有一个录音会话，互斥不影响并发）。
type PunctEngine = Arc<std::sync::Mutex<sherpa_rs::punctuate::Punctuation>>;

/// 取引擎外层锁：poisoning 时恢复数据（标点引擎无内存态一致性要求）。
fn lock_engine_slot(
    slot: &std::sync::Mutex<Option<PunctEngine>>,
) -> std::sync::MutexGuard<'_, Option<PunctEngine>> {
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 取引擎内层锁（推理互斥，poisoning 同上）。
fn lock_engine(
    engine: &PunctEngine,
) -> std::sync::MutexGuard<'_, sherpa_rs::punctuate::Punctuation> {
    engine
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 前端标点模型卡片状态（ModelsView 消费；`src/api/asr.ts` 同步类型）。
#[derive(Debug, Clone, Serialize)]
pub struct PunctModelInfo {
    pub id: String,
    pub name: String,
    pub size_bytes: u64,
    /// 已下载且哈希校验通过。
    pub downloaded: bool,
    /// 模型文件存在但校验失败（损坏，引导重新下载）。
    pub invalid: bool,
    /// 下载中进度（非下载中为 None）。
    pub progress: Option<DownloadProgress>,
    /// 生效的模型存储目录（与识别模型同目录体系，T-017）。
    pub models_dir: String,
}

/// 标点模型运行目录：`<models_dir>/<PUNCT_MODEL_ID>/`。
fn punct_model_dir(models_dir: &Path) -> PathBuf {
    models_dir.join(PUNCT_MODEL_ID)
}

/// 单文件完整性校验：存在性 + 大小 + SHA-256（任一不符即 E-ASR-003 / 001，
/// 错误信息指明期望 / 实际值，引导重新下载；不误用损坏文件）。
fn verify_file_hash(
    path: &Path,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), AppError> {
    let bytes = std::fs::read(path)
        .map_err(|err| AppError::Asr(ERR_MODEL_MISSING, format!("读取标点模型文件失败: {err}")))?;
    let actual_size = bytes.len() as u64;
    if actual_size != expected_size {
        return Err(AppError::Asr(
            ERR_HASH_MISMATCH,
            format!(
                "标点模型文件大小不符（期望 {expected_size} 字节，实际 {actual_size} 字节），请删除后重新下载"
            ),
        ));
    }
    let mut hasher = sha2::Sha256::new();
    hasher.update(&bytes);
    let actual = hex_lower(&hasher.finalize());
    if actual != expected_sha256 {
        return Err(AppError::Asr(
            ERR_HASH_MISMATCH,
            format!(
                "标点模型文件校验失败（期望 {expected_sha256}，实际 {actual}），请删除后重新下载"
            ),
        ));
    }
    Ok(())
}

/// 从官方归档解包模型成员到 `dest`（找到第一个匹配成员即返回；
/// 归档损坏或缺少成员时报 E-ASR-006）。
fn unpack_model_member(archive_path: &Path, member: &str, dest: &Path) -> Result<(), AppError> {
    let file = std::fs::File::open(archive_path)
        .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("读取下载文件失败: {err}")))?;
    let decoder = bzip2::read::BzDecoder::new(std::io::BufReader::new(file));
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("读取标点模型归档失败: {err}")))?;
    for entry in entries {
        let mut entry =
            entry.map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("解包标点模型失败: {err}")))?;
        let path = entry
            .path()
            .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("解包标点模型失败: {err}")))?;
        if path.to_string_lossy() == member {
            entry.unpack(dest).map_err(|err| {
                AppError::Asr(ERR_DOWNLOAD, format!("写入标点模型文件失败: {err}"))
            })?;
            return Ok(());
        }
    }
    Err(AppError::Asr(
        ERR_DOWNLOAD,
        format!("标点模型归档中未找到 {member}"),
    ))
}

/// PunctService：标点模型生命周期管理 + 标点恢复执行。
pub struct PunctService {
    /// 默认模型目录（与 AsrService 同一目录体系；`asr.models_dir` 配置优先，
    /// 见 `effective_models_dir`）。
    models_dir: PathBuf,
    config: ConfigStore,
    events: broadcast::Sender<AppEvent>,
    /// 已加载引擎（懒加载后常驻；ConfigChanged(Asr) / 删除 / 下载完成时重置）。
    engine: Arc<std::sync::Mutex<Option<PunctEngine>>>,
    /// 模型文件哈希状态机（避免每次推理 / 状态查询重复全量哈希 75 MB 文件）。
    hash_state: Arc<AtomicU8>,
    /// 进行中的下载任务（None = 空闲；取消即摘除 + abort，与 asr.rs 同款防孤儿 ticker）。
    download: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
    /// 下载进度快照（供 `status` 查询）。
    download_progress: Arc<RwLock<Option<DownloadProgress>>>,
}

impl PunctService {
    pub fn new(
        models_dir: PathBuf,
        config: ConfigStore,
        events: broadcast::Sender<AppEvent>,
    ) -> Self {
        Self {
            models_dir,
            config,
            events,
            engine: Arc::new(std::sync::Mutex::new(None)),
            hash_state: Arc::new(AtomicU8::new(HASH_UNKNOWN)),
            download: Arc::new(Mutex::new(None)),
            download_progress: Arc::new(RwLock::new(None)),
        }
    }

    /// 克隆句柄（下载任务持 Arc 自引用，与 asr.rs `clone_handle` 同款）。
    fn clone_handle(&self) -> Arc<Self> {
        Arc::new(Self {
            models_dir: self.models_dir.clone(),
            config: self.config.clone(),
            events: self.events.clone(),
            engine: self.engine.clone(),
            hash_state: self.hash_state.clone(),
            download: self.download.clone(),
            download_progress: self.download_progress.clone(),
        })
    }

    /// 生效的模型目录：配置 `asr.models_dir` 非空优先，否则用启动时解析的默认目录
    /// （与 AsrService::effective_models_dir 同一规则，保持两套模型同址）。
    async fn effective_models_dir(&self) -> PathBuf {
        let configured = self
            .config
            .get()
            .await
            .asr
            .models_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        configured.unwrap_or_else(|| self.models_dir.clone())
    }

    /// 模型卡片状态：未下载 / 已下载（哈希认证）/ 损坏（引导重下）/ 下载中。
    ///
    /// 懒校验：文件齐全但未认证的模型做一次 SHA-256（spawn_blocking），结果入状态机。
    pub async fn status(&self) -> PunctModelInfo {
        let models_dir = self.effective_models_dir().await;
        let model_file = punct_model_dir(&models_dir).join(MODEL_ONNX_NAME);
        let present = model_file.exists();
        if !present {
            // 文件被外部删除：状态回退未知，避免永远卡在已认证 / 损坏。
            self.hash_state.store(HASH_UNKNOWN, Ordering::Relaxed);
        } else if self.hash_state.load(Ordering::Relaxed) == HASH_UNKNOWN {
            let path = model_file.clone();
            let ok = tokio::task::spawn_blocking(move || {
                verify_file_hash(&path, MODEL_SIZE_BYTES, MODEL_SHA256).is_ok()
            })
            .await
            .unwrap_or(false);
            self.hash_state.store(
                if ok { HASH_VERIFIED } else { HASH_INVALID },
                Ordering::Relaxed,
            );
            if !ok {
                warn!("标点模型文件校验失败，标记为损坏");
            }
        }
        let state = self.hash_state.load(Ordering::Relaxed);
        PunctModelInfo {
            id: PUNCT_MODEL_ID.to_string(),
            name: PUNCT_MODEL_NAME.to_string(),
            size_bytes: MODEL_SIZE_BYTES,
            downloaded: present && state == HASH_VERIFIED,
            invalid: present && state == HASH_INVALID,
            progress: *self.download_progress.read().await,
            models_dir: models_dir.display().to_string(),
        }
    }

    /// 开始下载（幂等；已在下载中直接返回）。进度经 `model:progress` 事件推送。
    pub async fn download(&self) -> Result<(), AppError> {
        let mut download = self.download.lock().await;
        if download.is_some() {
            return Ok(());
        }
        let handle = tokio::spawn({
            let service = self.clone_handle();
            async move {
                if let Err(err) = service.run_download().await {
                    warn!(code = %err.code(), "标点模型下载失败");
                }
            }
        });
        *download = Some(handle.abort_handle());
        Ok(())
    }

    /// 取消进行中的下载。
    pub async fn cancel(&self) -> Result<(), AppError> {
        let handle = self.download.lock().await.take();
        if let Some(handle) = handle {
            handle.abort();
            self.download_progress.write().await.take();
            let _ = self.events.send(AppEvent::ModelDownloadProgress {
                model_id: PUNCT_MODEL_ID.to_string(),
                status: DownloadStatus::Cancelled,
                downloaded_bytes: 0,
                total_bytes: MODEL_SIZE_BYTES,
                bytes_per_sec: 0,
            });
        }
        Ok(())
    }

    /// 删除模型：取消下载、卸载引擎、清理模型目录与下载残留临时目录；
    /// 删除后状态回退为未下载（开关保持用户设置，下次识别原文直出直至重新下载）。
    pub async fn delete(&self) -> Result<(), AppError> {
        self.cancel().await.ok();
        *lock_engine_slot(&self.engine) = None;
        self.hash_state.store(HASH_UNKNOWN, Ordering::Relaxed);
        let models_dir = self.effective_models_dir().await;
        let dir = punct_model_dir(&models_dir);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|err| {
                AppError::Asr(ERR_LOAD_FAILED, format!("删除标点模型失败: {err}"))
            })?;
        }
        let tmp_dir = models_dir.join(format!(".{PUNCT_MODEL_ID}.tmp"));
        if tmp_dir.exists() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
        }
        info!("标点模型已删除");
        Ok(())
    }

    /// 识别配置变更（ConfigChanged(Asr)）：卸载引擎并清空哈希缓存
    /// （存储目录可能变更，引擎与认证状态一并失效，下次使用按新目录懒加载）。
    pub async fn reset(&self) {
        let mut slot = lock_engine_slot(&self.engine);
        if slot.take().is_some() {
            info!("识别配置已变更，卸载标点引擎待重载");
        }
        drop(slot);
        self.hash_state.store(HASH_UNKNOWN, Ordering::Relaxed);
    }

    /// 标点恢复执行（识别推理的 `spawn_blocking` 语境调用，禁止在异步上下文调用）。
    ///
    /// 模型未下载 / 校验失败 / 引擎加载失败一律原文直出（降级不报错）；
    /// 推理耗时记日志，不含任何文本（NFR-S-02）。
    pub fn punctuate_sync(&self, models_dir: &Path, text: &str) -> String {
        let Some(engine) = self.ensure_engine(models_dir) else {
            return text.to_string();
        };
        let started = Instant::now();
        let out = lock_engine(&engine).add_punctuation(text);
        info!(
            elapsed_ms = %started.elapsed().as_millis(),
            "标点模型推理完成"
        );
        out
    }

    /// 懒加载引擎（阻塞语境）：存在性 → 哈希把关（未认证时）→ 构造会话并常驻。
    ///
    /// 不可用返回 None；认证失败 / 加载失败置 `HASH_INVALID`，后续调用直接降级，
    /// 避免每次识别重复哈希 75 MB / 重建会话（重新下载或配置变更后重置重试）。
    fn ensure_engine(&self, models_dir: &Path) -> Option<PunctEngine> {
        if let Some(engine) = lock_engine_slot(&self.engine).as_ref() {
            return Some(engine.clone());
        }
        let model_path = punct_model_dir(models_dir).join(MODEL_ONNX_NAME);
        if !model_path.exists() {
            return None;
        }
        match self.hash_state.load(Ordering::Relaxed) {
            HASH_INVALID => return None,
            HASH_UNKNOWN => match verify_file_hash(&model_path, MODEL_SIZE_BYTES, MODEL_SHA256) {
                Ok(()) => self.hash_state.store(HASH_VERIFIED, Ordering::Relaxed),
                Err(err) => {
                    warn!(code = %err.code(), "标点模型文件校验失败，标点恢复降级为原文直出");
                    self.hash_state.store(HASH_INVALID, Ordering::Relaxed);
                    return None;
                }
            },
            _ => {}
        }
        let started = Instant::now();
        let engine =
            sherpa_rs::punctuate::Punctuation::new(sherpa_rs::punctuate::PunctuationConfig {
                model: model_path.to_string_lossy().into_owned(),
                debug: false,
                num_threads: Some(2),
                provider: None,
            });
        match engine {
            Ok(engine) => {
                info!(
                    elapsed_ms = %started.elapsed().as_millis(),
                    "标点引擎加载完成"
                );
                let engine = Arc::new(std::sync::Mutex::new(engine));
                *lock_engine_slot(&self.engine) = Some(engine.clone());
                Some(engine)
            }
            Err(err) => {
                warn!(error = %err, "标点引擎加载失败，标点恢复降级为原文直出");
                self.hash_state.store(HASH_INVALID, Ordering::Relaxed);
                None
            }
        }
    }

    /// 下载主体：官方归档下载（多线程分段 + 断点续传）→ 解包 → 校验 → 落盘。
    ///
    /// 不走 `model_source` 镜像优先策略：镜像仓库不存在（见模块注释），
    /// 镜像根地址为 Paraformer 专用仓库，套用会拼出 404 地址。
    async fn run_download(self: Arc<Self>) -> Result<(), AppError> {
        *self.download_progress.write().await = Some(DownloadProgress {
            downloaded_bytes: 0,
            total_bytes: 0,
            bytes_per_sec: 0,
        });
        let result = self.download_inner().await;
        self.download.lock().await.take();
        match result {
            Ok(()) => {
                self.download_progress.write().await.take();
                Ok(())
            }
            Err(err) => {
                self.download_progress.write().await.take();
                let _ = self.events.send(AppEvent::ModelDownloadProgress {
                    model_id: PUNCT_MODEL_ID.to_string(),
                    status: DownloadStatus::Failed,
                    downloaded_bytes: 0,
                    total_bytes: MODEL_SIZE_BYTES,
                    bytes_per_sec: 0,
                });
                Err(err)
            }
        }
    }

    async fn download_inner(&self) -> Result<(), AppError> {
        let _ = self.events.send(AppEvent::ModelDownloadProgress {
            model_id: PUNCT_MODEL_ID.to_string(),
            status: DownloadStatus::Downloading,
            downloaded_bytes: 0,
            total_bytes: 0,
            bytes_per_sec: 0,
        });

        let asr_config = self.config.get().await.asr;
        let client = model_download::build_client(asr_config.download_proxy.as_deref())?;
        let models_dir = self.effective_models_dir().await;
        std::fs::create_dir_all(&models_dir)
            .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("创建模型目录失败: {err}")))?;

        // 下载落临时目录 `.tmp/`：段文件（`.partN`）跨次运行保留 → 断点续传。
        let tmp_dir = models_dir.join(format!(".{PUNCT_MODEL_ID}.tmp"));
        std::fs::create_dir_all(&tmp_dir)
            .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("创建下载目录失败: {err}")))?;

        let downloaded = Arc::new(AtomicU64::new(0));
        let total_known = Arc::new(AtomicU64::new(0));

        // 进度节流广播（含实时速度）。取消 = 任务 handle.abort()：正常路径的
        // ticker.abort() 不会执行，ticker 会成为孤儿任务继续广播假进度，故每拍
        // 自查下载是否仍在注册表（cancel 会先摘除），不在即退出（与 asr.rs 同款）。
        let ticker = {
            let events = self.events.clone();
            let progress_slot = self.download_progress.clone();
            let download_slot = self.download.clone();
            let downloaded = downloaded.clone();
            let total_known = total_known.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(PROGRESS_THROTTLE);
                let mut last_bytes = 0u64;
                let mut last_tick = Instant::now();
                let mut first_tick = true;
                loop {
                    interval.tick().await;
                    // 首拍跳过自查：spawn 与注册之间存在理论上的并发窗口，
                    // 到第二拍注册必然完成（见 download）。
                    if first_tick {
                        first_tick = false;
                    } else if download_slot.lock().await.is_none() {
                        break;
                    }
                    let done = downloaded.load(Ordering::Relaxed);
                    let total = total_known.load(Ordering::Relaxed);
                    let speed = model_download::speed_of(&mut last_bytes, done, &mut last_tick);
                    *progress_slot.write().await = Some(DownloadProgress {
                        downloaded_bytes: done,
                        total_bytes: total,
                        bytes_per_sec: speed,
                    });
                    let _ = events.send(AppEvent::ModelDownloadProgress {
                        model_id: PUNCT_MODEL_ID.to_string(),
                        status: DownloadStatus::Downloading,
                        downloaded_bytes: done,
                        total_bytes: total,
                        bytes_per_sec: speed,
                    });
                }
            })
        };

        // 归档大小预探测写入 total_known（否则下载期间 total 为 0，进度条卡 0%）。
        let probed = model_download::probe_size(&client, ARCHIVE_URL).await;
        let total = probed.unwrap_or(ARCHIVE_SIZE_BYTES).max(ARCHIVE_SIZE_BYTES);
        total_known.store(total, Ordering::Relaxed);
        let archive_path = tmp_dir.join("archive.tar.bz2");
        let result = model_download::download_file(
            &client,
            ARCHIVE_URL,
            &archive_path,
            ARCHIVE_SIZE_BYTES,
            model_download::DEFAULT_THREADS,
            downloaded.clone(),
        )
        .await;
        total_known.store(
            result
                .as_ref()
                .map_or(total, |actual| (*actual).max(ARCHIVE_SIZE_BYTES)),
            Ordering::Relaxed,
        );
        ticker.abort();

        let unpack_result = result.and_then(|_total| {
            unpack_model_member(&archive_path, MODEL_MEMBER, &tmp_dir.join(MODEL_ONNX_NAME))
        });
        let _ = std::fs::remove_file(&archive_path);
        unpack_result?;
        // 下载产物完整性把关：大小 + SHA-256（哈希不符说明产物损坏）。
        verify_file_hash(
            &tmp_dir.join(MODEL_ONNX_NAME),
            MODEL_SIZE_BYTES,
            MODEL_SHA256,
        )?;

        let final_dir = punct_model_dir(&models_dir);
        if final_dir.exists() {
            let _ = std::fs::remove_dir_all(&final_dir);
        }
        std::fs::rename(&tmp_dir, &final_dir)
            .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("安装标点模型失败: {err}")))?;
        self.hash_state.store(HASH_VERIFIED, Ordering::Relaxed);
        info!("标点模型下载并校验完成");
        let _ = self.events.send(AppEvent::ModelDownloadProgress {
            model_id: PUNCT_MODEL_ID.to_string(),
            status: DownloadStatus::Completed,
            downloaded_bytes: MODEL_SIZE_BYTES,
            total_bytes: MODEL_SIZE_BYTES,
            bytes_per_sec: 0,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用模型目录（按进程 + 用例名隔离，tech/02 未登记 tempfile crate）。
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vt-punct-test-{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn service_for(dir: &Path) -> PunctService {
        PunctService::new(
            dir.to_path_buf(),
            ConfigStore::load(&temp_dir("config"), crate::events::channel().0).unwrap(),
            crate::events::channel().0,
        )
    }

    /// 构造含单个成员的微型 tar.bz2（归档解包逻辑的测试替身）。
    fn write_tar_bz2(archive_path: &Path, members: &[(&str, &[u8])]) {
        let file = std::fs::File::create(archive_path).unwrap();
        let encoder = bzip2::write::BzEncoder::new(file, bzip2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, content) in members {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, *content).unwrap();
        }
        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap();
    }

    #[test]
    fn model_meta_is_consistent() {
        assert!(!PUNCT_MODEL_ID.is_empty());
        assert!(ARCHIVE_URL.starts_with("https://"));
        assert!(ARCHIVE_URL.ends_with(".tar.bz2"));
        assert_eq!(MODEL_SHA256.len(), 64);
        const {
            assert!(ARCHIVE_SIZE_BYTES > 0 && MODEL_SIZE_BYTES > 0);
            assert!(
                MODEL_SIZE_BYTES > ARCHIVE_SIZE_BYTES,
                "int8 模型应大于压缩归档（压缩率合理性的弱断言）"
            );
        }
        // 成员路径与归档目录名一致，且为模型主文件。
        assert!(MODEL_MEMBER.starts_with("sherpa-onnx-punct-ct-transformer"));
        assert!(MODEL_MEMBER.ends_with("model.int8.onnx"));
    }

    #[test]
    fn unpack_model_member_extracts_named_member() {
        let dir = temp_dir("unpack");
        let archive = dir.join("archive.tar.bz2");
        let member_dir = "sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8";
        write_tar_bz2(
            &archive,
            &[
                (
                    format!("{member_dir}/README.md").as_str(),
                    b"readme" as &[u8],
                ),
                (
                    format!("{member_dir}/model.int8.onnx").as_str(),
                    b"onnx-bytes" as &[u8],
                ),
            ],
        );
        let dest = dir.join("model.onnx");
        unpack_model_member(&archive, MODEL_MEMBER, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"onnx-bytes");

        // 缺少成员：报 E-ASR-006。
        let err = unpack_model_member(&archive, "nonexistent/model.onnx", &dest).unwrap_err();
        assert_eq!(err.code(), "E-ASR-006");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_file_hash_pass_and_reject() {
        let dir = temp_dir("hash");
        let path = dir.join("model.onnx");
        let content = b"fake punct model bytes";
        std::fs::write(&path, content).unwrap();
        let mut hasher = sha2::Sha256::new();
        hasher.update(content);
        let expected = hex_lower(&hasher.finalize());
        // 大小 + 哈希均符：通过。
        assert!(verify_file_hash(&path, content.len() as u64, &expected).is_ok());
        // 大小不符：E-ASR-003。
        let err = verify_file_hash(&path, content.len() as u64 + 1, &expected).unwrap_err();
        assert_eq!(err.code(), "E-ASR-003");
        assert!(err.to_string().contains("大小不符"));
        // 内容被篡改（大小与上报值一致，哈希不符）：E-ASR-003，错误信息含期望 / 实际哈希。
        let tampered = b"tampered bytes";
        std::fs::write(&path, tampered).unwrap();
        let err = verify_file_hash(&path, tampered.len() as u64, &expected).unwrap_err();
        assert_eq!(err.code(), "E-ASR-003");
        let msg = err.to_string();
        assert!(msg.contains("校验失败") && msg.contains("期望"), "{msg}");
        // 文件缺失：E-ASR-001。
        let missing = dir.join("missing.onnx");
        let err = verify_file_hash(&missing, 1, &expected).unwrap_err();
        assert_eq!(err.code(), "E-ASR-001");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn status_missing_then_invalid_then_verified() {
        let base = temp_dir("status");
        let service = service_for(&base);
        // 生效目录 = 构造参数（配置未设 models_dir），直接在构造参数目录下落文件。
        let model_dir = punct_model_dir(&base);
        std::fs::create_dir_all(&model_dir).unwrap();

        // 未下载：downloaded / invalid 均为 false。
        let info = service.status().await;
        assert!(!info.downloaded && !info.invalid);
        assert_eq!(info.id, PUNCT_MODEL_ID);
        assert_eq!(info.size_bytes, MODEL_SIZE_BYTES);
        assert_eq!(info.models_dir, base.display().to_string());

        // 文件损坏（哈希不符）：invalid = true，downloaded = false（拒绝误用）。
        std::fs::write(model_dir.join(MODEL_ONNX_NAME), b"corrupted").unwrap();
        let info = service.status().await;
        assert!(info.invalid && !info.downloaded);

        // 测试种子已认证状态（真实 75 MB 模型无法在单测中落地）：
        // 认证后 downloaded = true，且不再重复校验。
        std::fs::write(model_dir.join(MODEL_ONNX_NAME), b"whatever").unwrap();
        service.hash_state.store(HASH_VERIFIED, Ordering::Relaxed);
        let info = service.status().await;
        assert!(info.downloaded && !info.invalid);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn punctuate_degrades_to_original_when_model_missing_or_invalid() {
        let base = temp_dir("degrade");
        let service = service_for(&base);
        // 模型未下载：原文直出，不报错。
        assert_eq!(service.punctuate_sync(&base, "你好世界"), "你好世界");
        // 文件损坏：哈希把关后置 INVALID，同样原文直出；重复调用不重试（缓存降级）。
        let model_dir = punct_model_dir(&base);
        std::fs::create_dir_all(&model_dir).unwrap();
        std::fs::write(model_dir.join(MODEL_ONNX_NAME), b"junk").unwrap();
        assert_eq!(service.punctuate_sync(&base, "你好世界"), "你好世界");
        assert_eq!(service.punctuate_sync(&base, "你好世界"), "你好世界");
        assert_eq!(service.hash_state.load(Ordering::Relaxed), HASH_INVALID);
        // 重置（ConfigChanged(Asr) 语义）：状态回未知，重新把关。
        service.reset().await;
        assert_eq!(service.hash_state.load(Ordering::Relaxed), HASH_UNKNOWN);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn delete_clears_state_and_files() {
        let base = temp_dir("delete");
        let service = service_for(&base);
        let model_dir = punct_model_dir(&base);
        std::fs::create_dir_all(&model_dir).unwrap();
        std::fs::write(model_dir.join(MODEL_ONNX_NAME), b"junk").unwrap();
        std::fs::create_dir_all(base.join(format!(".{PUNCT_MODEL_ID}.tmp"))).unwrap();
        service.hash_state.store(HASH_VERIFIED, Ordering::Relaxed);

        service.delete().await.unwrap();
        assert!(!model_dir.exists(), "模型目录应被删除");
        assert!(
            !base.join(format!(".{PUNCT_MODEL_ID}.tmp")).exists(),
            "下载临时目录应被清理"
        );
        assert_eq!(service.hash_state.load(Ordering::Relaxed), HASH_UNKNOWN);
        let info = service.status().await;
        assert!(!info.downloaded && !info.invalid, "删除后状态回退为未下载");
        let _ = std::fs::remove_dir_all(&base);
    }
}
