//! AsrService（T-006，tech/04 4.4、tech/05 5.3.3）：本地离线 ASR 引擎接入与模型管理。
//!
//! - 引擎选型：sherpa-rs（绑定 sherpa-onnx，纯本地推理，断网可用，产品原则 1）；
//! - 模型注册表：内置推荐模型元数据（id / 名称 / 档位 / 大小 / SHA-256 / 下载地址），
//!   模型文件存模型目录 `models/<id>/`，不打进安装包（NFR-P-09）；
//!   模型目录默认在安装目录旁 `models/`（不可写时回退数据目录），可在设置中自定义（T-017）；
//! - 下载（T-006，体验增强 T-017）：默认镜像源直传散文件（hf-mirror.com），
//!   官方源整包归档兜底（tar.bz2），多线程分段并发 + 段级断点续传，
//!   进度（含实时速度）经 `model:progress` 事件节流广播；支持取消；
//!   完成后 SHA-256 校验，损坏即删除并拒绝加载，引导重新下载；
//! - 加载 / 切换 / 删除：`spawn_blocking` 中加载引擎（ParaformerRecognizer，Send + Sync），
//!   当前模型持久化在配置 `asr.current_model_id`（T-003）；
//! - 推理消费 `recording:audio_ready`（内部事件）→ `spawn_blocking` 推理 →
//!   内部事件 `Recognized` 交给后处理管线（T-011）；失败广播 `recording:result`
//!   Failed + 稳定错误编号（E-ASR-xxx）；
//! - 性能埋点：模型加载与推理耗时记日志（对照 NFR-P-03/04 预算；日志不含识别文本，NFR-S-02）。
//!
//! 隐私（NFR-S-02）：识别文本只经事件通道传递，绝不写入日志；
//! 推理在本地完成，音频与文本均不出本机。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use sha2::Digest;
use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::{info, warn};

use crate::error::AppError;
use crate::events::{AppEvent, DownloadStatus, RecordingResultEvent};
use crate::services::config_store::{ConfigSection, ConfigStore, ModelSource};
use crate::services::model_download;
use crate::services::punct::{PunctModelInfo, PunctService};
use crate::services::session::SessionTracker;

/// 错误编号：模型未下载 / 无可用模型（引导先下载模型）。
pub const ERR_MODEL_MISSING: u32 = 1;
/// 错误编号：模型加载失败。
pub const ERR_LOAD_FAILED: u32 = 2;
/// 错误编号：模型文件哈希校验失败（引导重新下载）。
pub const ERR_HASH_MISMATCH: u32 = 3;
/// 错误编号：推理过程失败。
pub const ERR_INFERENCE: u32 = 4;
/// 错误编号：删除使用中的模型被拒绝。
pub const ERR_MODEL_IN_USE: u32 = 5;
/// 错误编号：下载失败（网络错误等）。
pub const ERR_DOWNLOAD: u32 = 6;
/// 错误编号：未知模型 ID。
pub const ERR_UNKNOWN_MODEL: u32 = 7;

/// 引擎期望的输入采样率（sherpa-rs Paraformer 特征配置固定 16 kHz）。
pub const ENGINE_SAMPLE_RATE: u32 = 16_000;

/// 下载进度事件节流间隔（避免 1 GB 下载刷爆事件通道）。
const PROGRESS_THROTTLE: std::time::Duration = std::time::Duration::from_millis(250);

/// 模型档位（前端按档位映射说明文案，i18n）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    /// 准确率优先。
    Accuracy,
    /// 速度优先 / 低配置电脑推荐。
    Speed,
}

/// 内置推荐模型元数据（T-006 SPEC 步骤 2）。
#[derive(Debug, Clone, Copy)]
pub struct ModelMeta {
    /// 稳定模型 ID（配置 `asr.current_model_id` 引用）。
    pub id: &'static str,
    /// 模型显示名。
    pub name: &'static str,
    /// 适用语言说明。
    pub languages: &'static str,
    /// 档位。
    pub tier: ModelTier,
    /// 模型文件大小（字节，用于进度与磁盘预估）。
    pub size_bytes: u64,
    /// 模型文件 SHA-256（下载后校验；损坏拒绝加载）。
    pub sha256: &'static str,
    /// 词表文件 SHA-256（与模型文件同样校验；两个内置模型共用同一份 tokens.txt）。
    pub tokens_sha256: &'static str,
    /// 模型归档下载地址（官方 GitHub Release）。
    pub url: &'static str,
    /// 归档内模型文件成员路径。
    pub model_file: &'static str,
    /// 归档内词表文件成员路径。
    pub tokens_file: &'static str,
}

/// sherpa-onnx Paraformer 中文模型（2023-03-28 发布，FunASR 训练）。
const PARAFORMER_ZH_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-paraformer-zh-2023-03-28.tar.bz2";

/// 内置模型共用的词表文件 SHA-256（两个条目 tokens_file 指向同一份 tokens.txt，
/// 2026-10-11 从 hf-mirror 实文件计算）。
const PARAFORMER_ZH_TOKENS_SHA256: &str =
    "59aba8873a2ed1e122c25fee421e25f283b63290efbde85c1f01a853d83cb6e6";

/// 内置模型清单（同一份归档按量化档位拆分为两个条目，各自只解包所需文件）。
pub const MODELS: &[ModelMeta] = &[
    ModelMeta {
        id: "paraformer-zh-int8",
        name: "Paraformer 中文",
        languages: "中文（普通话）",
        tier: ModelTier::Speed,
        size_bytes: 223_385_835,
        sha256: "9ada9127ca5b82320385ac12340eb8b05dee64fd45cf8cf593ec693826ec2fd7",
        tokens_sha256: PARAFORMER_ZH_TOKENS_SHA256,
        url: PARAFORMER_ZH_URL,
        model_file: "sherpa-onnx-paraformer-zh-2023-03-28/model.int8.onnx",
        tokens_file: "sherpa-onnx-paraformer-zh-2023-03-28/tokens.txt",
    },
    ModelMeta {
        id: "paraformer-zh",
        name: "Paraformer 中文",
        languages: "中文（普通话）",
        tier: ModelTier::Accuracy,
        size_bytes: 863_821_140,
        sha256: "3883fdb54abe1ab077997c8e14cf0532fc2498f8fb5eb118cd53d4ba05e4d169",
        tokens_sha256: PARAFORMER_ZH_TOKENS_SHA256,
        url: PARAFORMER_ZH_URL,
        model_file: "sherpa-onnx-paraformer-zh-2023-03-28/model.onnx",
        tokens_file: "sherpa-onnx-paraformer-zh-2023-03-28/tokens.txt",
    },
];

/// 按 ID 查模型元数据。
pub fn model_meta(id: &str) -> Option<&'static ModelMeta> {
    MODELS.iter().find(|meta| meta.id == id)
}

/// 模型运行目录（数据目录 `models/` 下，NFR-P-09 不进安装包）。
pub fn model_dir(models_dir: &Path, id: &str) -> PathBuf {
    models_dir.join(id)
}

/// 模型在运行目录内的规范文件名（下载解包时统一改名）。
pub(crate) const MODEL_ONNX_NAME: &str = "model.onnx";
pub(crate) const TOKENS_NAME: &str = "tokens.txt";

/// 解析默认模型目录：优先安装目录旁 `models/`（探测可写），不可写回退数据目录
/// `models/`（T-017，PRD 3.8.7 v1.5）。探测方式 = 建目录 + 写删探针文件。
pub fn resolve_default_models_dir(exe_dir: &Path, data_models_dir: &Path) -> PathBuf {
    let candidate = exe_dir.join("models");
    let probe = candidate.join(".write-probe");
    let writable = std::fs::create_dir_all(&candidate)
        .and_then(|()| std::fs::write(&probe, b"vt"))
        .and_then(|()| std::fs::remove_file(&probe))
        .is_ok();
    if writable {
        info!(dir = %candidate.display(), "模型目录：安装目录 models/");
        candidate
    } else {
        info!(
            dir = %data_models_dir.display(),
            "安装目录不可写，模型目录回退到数据目录"
        );
        data_models_dir.to_path_buf()
    }
}

/// 删除模型后的提示文案（T-014 验收 4.6.12）。
///
/// 只有删除的是激活模型才涉及回退 / 无可用模型；删除非激活模型不提及这些，
/// 避免"明明还有其他可用模型"却提示没有可用模型的逻辑错误。
fn delete_notice(was_active: bool, fallback: Option<&str>) -> String {
    if !was_active {
        return "已删除模型".to_string();
    }
    match fallback {
        Some(id) => format!(
            "已删除模型，识别将使用 {}",
            model_meta(id).map(|m| m.name).unwrap_or(id)
        ),
        None => "已删除模型，当前没有可用模型，请下载模型后再使用语音输入".to_string(),
    }
}

/// 模型完整性校验：model.onnx 与 tokens.txt 均计算 SHA-256 并与元数据比对
/// （验收 4.6.4；任一文件损坏 / 被篡改都返回 E-ASR-003，错误信息指明文件与
/// 期望 / 实际哈希）。
pub fn verify_model_hash(model_dir: &Path, meta: &ModelMeta) -> Result<(), AppError> {
    verify_file_hash(
        &model_dir.join(MODEL_ONNX_NAME),
        MODEL_ONNX_NAME,
        meta.sha256,
    )?;
    verify_file_hash(
        &model_dir.join(TOKENS_NAME),
        TOKENS_NAME,
        meta.tokens_sha256,
    )
}

/// 单文件 SHA-256 校验（读取失败视为加载失败，哈希不符视为损坏）。
fn verify_file_hash(path: &Path, label: &str, expected: &str) -> Result<(), AppError> {
    let bytes = std::fs::read(path)
        .map_err(|err| AppError::Asr(ERR_LOAD_FAILED, format!("读取{label}失败: {err}")))?;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&bytes);
    let actual = hex_lower(&hasher.finalize());
    if actual != expected {
        return Err(AppError::Asr(
            ERR_HASH_MISMATCH,
            format!("{label} 校验失败（期望 {expected}，实际 {actual}），请删除后重新下载"),
        ));
    }
    Ok(())
}

/// 字节切片转小写十六进制（避免额外依赖）。
pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// 线性插值重采样到引擎采样率（16 kHz）。
///
/// AudioService 交付的是设备实际采样率（如 48 kHz），sherpa-rs 的 Paraformer
/// 特征配置固定 16 kHz，推理前必须重采样。
pub fn resample_to_engine(samples: &[f32], from_rate: u32) -> Vec<f32> {
    if from_rate == ENGINE_SAMPLE_RATE || samples.is_empty() {
        return samples.to_vec();
    }
    let out_len =
        (samples.len() as u64 * u64::from(ENGINE_SAMPLE_RATE) / u64::from(from_rate)) as usize;
    let mut out = Vec::with_capacity(out_len.max(1));
    let step = f64::from(from_rate) / f64::from(ENGINE_SAMPLE_RATE);
    for i in 0..out_len.max(1) {
        let pos = i as f64 * step;
        let idx = pos as usize;
        let frac = (pos - idx as f64) as f32;
        let a = samples.get(idx).copied().unwrap_or(0.0);
        let b = samples.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

/// 前端模型卡片状态（T-014 消费）。
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub languages: String,
    pub tier: ModelTier,
    pub size_bytes: u64,
    /// 已下载且哈希校验通过。
    pub downloaded: bool,
    /// 当前激活（下一次识别使用）。
    pub active: bool,
    /// 模型文件存在但校验失败（损坏，引导重新下载）。
    pub invalid: bool,
    /// 下载中进度（非下载中为 None）。
    pub progress: Option<DownloadProgress>,
    /// 生效的模型存储目录（T-017；默认安装目录 models/ 或数据目录回退 / 自定义）。
    pub models_dir: String,
}

/// 下载进度快照（含实时速度，T-017）。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    /// 瞬时下载速度（bytes/sec；0 = 无新进展或已结束）。
    pub bytes_per_sec: u64,
}

/// 推理引擎句柄：sherpa-rs Paraformer（`transcribe` 为 &mut self，经 Mutex 串行化；
/// 同一时间只有一个录音会话，互斥不影响并发）。
type Engine = Arc<std::sync::Mutex<sherpa_rs::paraformer::ParaformerRecognizer>>;

/// 取引擎锁： poisoning 时恢复数据（识别引擎无内存态一致性要求）。
fn lock_engine(
    engine: &Engine,
) -> std::sync::MutexGuard<'_, sherpa_rs::paraformer::ParaformerRecognizer> {
    engine
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// AsrService：模型生命周期管理 + 推理消费 AudioReady。
pub struct AsrService {
    /// 默认模型目录（启动时按可写性解析；配置 `asr.models_dir` 优先，见
    /// `effective_models_dir`）。
    models_dir: PathBuf,
    config: ConfigStore,
    events: broadcast::Sender<AppEvent>,
    /// 已加载引擎（Mutex 串行化加载与推理；同一时间只有一个录音会话）。
    engine: Arc<Mutex<Option<(String, Engine)>>>,
    /// 已下载且校验通过的模型缓存（避免每次全量哈希）。
    verified: Arc<RwLock<HashSet<String>>>,
    /// 进行中的下载任务（model_id → AbortHandle）。
    downloads: Arc<Mutex<HashMap<String, tokio::task::AbortHandle>>>,
    /// 下载进度快照（model_id → 进度）。
    download_progress: Arc<RwLock<HashMap<String, DownloadProgress>>>,
    /// 引擎加载互斥：切换 / 删除与推理串行。
    lifecycle: Arc<Mutex<()>>,
    /// 会话代次追踪（T-010）：结果回调按代次 / 取消标记判过期丢弃。
    tracker: Arc<SessionTracker>,
    /// 识别标点恢复（T-545）：模型生命周期与推理执行，随识别输出句读。
    punct: Arc<PunctService>,
}

impl AsrService {
    pub fn new(
        models_dir: PathBuf,
        config: ConfigStore,
        events: broadcast::Sender<AppEvent>,
        tracker: Arc<SessionTracker>,
    ) -> Self {
        let punct = Arc::new(PunctService::new(
            models_dir.clone(),
            config.clone(),
            events.clone(),
        ));
        Self {
            models_dir,
            config,
            events,
            engine: Arc::new(Mutex::new(None)),
            verified: Arc::new(RwLock::new(HashSet::new())),
            downloads: Arc::new(Mutex::new(HashMap::new())),
            download_progress: Arc::new(RwLock::new(HashMap::new())),
            lifecycle: Arc::new(Mutex::new(())),
            tracker,
            punct,
        }
    }

    /// 启动推理循环：消费 `AudioReady`，产出 `Recognized`（T-011 管线消费）。
    pub fn start(self: Arc<Self>, mut events_rx: broadcast::Receiver<AppEvent>) {
        tokio::spawn(async move {
            loop {
                match events_rx.recv().await {
                    Ok(AppEvent::AudioReady {
                        session_id,
                        sample_rate,
                        samples,
                    }) => {
                        self.handle_audio(session_id, sample_rate, samples).await;
                    }
                    Ok(AppEvent::ConfigChanged {
                        section: ConfigSection::Asr,
                    }) => {
                        // 激活模型变更：卸载引擎，下一次录音时按需重载（即时生效）。
                        // 存储目录可能同时变更：清掉校验缓存，后续按新目录懒校验。
                        let mut engine = self.engine.lock().await;
                        if engine.is_some() {
                            info!("激活模型已变更，卸载引擎待重载");
                            *engine = None;
                        }
                        drop(engine);
                        self.verified.write().await.clear();
                        // T-545：标点引擎与模型认证缓存一并重置（存储目录可能变更）。
                        self.punct.reset().await;
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });
    }

    /// 生效的模型目录：配置 `asr.models_dir` 非空优先，否则用启动时解析的默认目录。
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

    /// 模型文件是否齐全（不保证完好：完好性以哈希校验为准）。
    fn files_present_at(models_dir: &Path, meta: &ModelMeta) -> bool {
        let dir = model_dir(models_dir, meta.id);
        dir.join(MODEL_ONNX_NAME).exists() && dir.join(TOKENS_NAME).exists()
    }

    /// 懒校验：文件齐全但未入 `verified` 缓存的模型，逐个哈希校验并更新缓存。
    ///
    /// 应用重启后 `verified` 为空，此处把磁盘上的已下载模型重新认证为可用，
    /// 损坏文件保持未认证（`list_models` 标记 invalid，加载路径拒绝使用）。
    async fn verify_present_models(&self) {
        let models_dir = self.effective_models_dir().await;
        let pending: Vec<ModelMeta> = {
            let verified = self.verified.read().await;
            MODELS
                .iter()
                .filter(|meta| !verified.contains(meta.id))
                .filter(|meta| Self::files_present_at(&models_dir, meta))
                .copied()
                .collect()
        };
        for meta in pending {
            let dir = model_dir(&models_dir, meta.id);
            let ok = tokio::task::spawn_blocking(move || verify_model_hash(&dir, &meta).is_ok())
                .await
                .unwrap_or(false);
            let mut verified = self.verified.write().await;
            if ok {
                verified.insert(meta.id.to_string());
            } else {
                verified.remove(meta.id);
                warn!(model = %meta.id, "模型文件校验失败，标记为损坏");
            }
        }
    }

    /// 模型清单（含下载 / 激活 / 损坏状态），T-014 模型管理页数据源。
    pub async fn list_models(&self) -> Vec<ModelInfo> {
        self.verify_present_models().await;
        let models_dir = self.effective_models_dir().await;
        let verified = self.verified.read().await;
        let progress = self.download_progress.read().await;
        // active 只认哈希校验通过的模型（损坏模型显示 invalid，不显示为使用中）。
        let active = self.config.get().await.asr.current_model_id;
        let active = active.filter(|id| verified.contains(id)).or_else(|| {
            MODELS
                .iter()
                .map(|meta| meta.id)
                .find(|id| verified.contains(*id))
                .map(str::to_string)
        });
        MODELS
            .iter()
            .map(|meta| {
                let files_present = Self::files_present_at(&models_dir, meta);
                let is_verified = verified.contains(meta.id);
                let invalid = files_present && !is_verified;
                ModelInfo {
                    id: meta.id.to_string(),
                    name: meta.name.to_string(),
                    languages: meta.languages.to_string(),
                    tier: meta.tier,
                    size_bytes: meta.size_bytes,
                    downloaded: is_verified,
                    active: active.as_deref() == Some(meta.id),
                    invalid,
                    progress: progress.get(meta.id).copied(),
                    models_dir: models_dir.display().to_string(),
                }
            })
            .collect()
    }

    /// 已验证或文件齐全（候选模型；加载路径仍会做一次哈希校验把关）。
    async fn is_usable_candidate(&self, id: &str) -> bool {
        if self.verified.read().await.contains(id) {
            return true;
        }
        let models_dir = self.effective_models_dir().await;
        model_meta(id)
            .map(|meta| Self::files_present_at(&models_dir, meta))
            .unwrap_or(false)
    }

    /// 解析当前应使用的模型：配置指定优先，否则首个已验证 / 文件齐全模型。
    async fn resolve_active_model(&self) -> Option<String> {
        let config_id = self.config.get().await.asr.current_model_id;
        if let Some(id) = config_id {
            if self.is_usable_candidate(&id).await {
                return Some(id);
            }
        }
        for meta in MODELS {
            if self.is_usable_candidate(meta.id).await {
                return Some(meta.id.to_string());
            }
        }
        None
    }

    /// 下载模型：流式下载归档并解包所需成员，完成校验后自动激活（当前无可用模型时）。
    pub async fn download_model(&self, model_id: String) -> Result<(), AppError> {
        let meta = *model_meta(&model_id)
            .ok_or_else(|| AppError::Asr(ERR_UNKNOWN_MODEL, format!("未知模型: {model_id}")))?;
        {
            let mut downloads = self.downloads.lock().await;
            if downloads.contains_key(&model_id) {
                return Ok(()); // 已在下载中，幂等
            }
            let handle = tokio::spawn({
                let service = self.clone_handle();
                async move {
                    if let Err(err) = service.run_download(meta).await {
                        warn!(code = %err.code(), "模型下载失败");
                    }
                }
            });
            downloads.insert(model_id, handle.abort_handle());
        }
        Ok(())
    }

    fn clone_handle(&self) -> Arc<Self> {
        Arc::new(Self {
            models_dir: self.models_dir.clone(),
            config: self.config.clone(),
            events: self.events.clone(),
            engine: self.engine.clone(),
            verified: self.verified.clone(),
            downloads: self.downloads.clone(),
            download_progress: self.download_progress.clone(),
            lifecycle: self.lifecycle.clone(),
            tracker: self.tracker.clone(),
            punct: self.punct.clone(),
        })
    }

    /// 取消进行中的下载。
    pub async fn cancel_download(&self, model_id: String) -> Result<(), AppError> {
        let handle = self.downloads.lock().await.remove(&model_id);
        if let Some(handle) = handle {
            handle.abort();
            self.download_progress.write().await.remove(&model_id);
            let _ = self.events.send(AppEvent::ModelDownloadProgress {
                model_id,
                status: DownloadStatus::Cancelled,
                downloaded_bytes: 0,
                total_bytes: 0,
                bytes_per_sec: 0,
            });
        }
        Ok(())
    }

    /// 删除模型：使用中（激活）模型允许删除但回退到首个可用模型并提示（T-014）；
    /// 删除后识别走回退模型，无可用模型时下次录音给出引导（E-ASR-001）。
    pub async fn delete_model(&self, model_id: String) -> Result<(), AppError> {
        let meta = model_meta(&model_id)
            .ok_or_else(|| AppError::Asr(ERR_UNKNOWN_MODEL, format!("未知模型: {model_id}")))?;
        self.cancel_download(model_id.clone()).await.ok();
        let _guard = self.lifecycle.lock().await;

        let was_active = self.resolve_active_model().await.as_deref() == Some(meta.id);
        let dir = model_dir(&self.effective_models_dir().await, &model_id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .map_err(|err| AppError::Asr(ERR_LOAD_FAILED, format!("删除模型失败: {err}")))?;
        }
        self.verified.write().await.remove(&model_id);

        // 删除激活模型：卸载引擎并回退到首个可用模型（有则立即重载生效）。
        let mut fallback: Option<String> = None;
        if was_active {
            let mut engine = self.engine.lock().await;
            *engine = None;
            drop(engine);
            for other in MODELS {
                if other.id != meta.id && self.is_usable_candidate(other.id).await {
                    fallback = Some(other.id.to_string());
                    break;
                }
            }
            if let Some(fallback_id) = &fallback {
                info!(fallback = %fallback_id, "已删除激活模型，回退到首个可用模型");
            }
        }
        // 配置中的 current_model_id 若指向已删模型，清掉走自动回退。
        let mut asr_config = self.config.get().await.asr;
        if asr_config.current_model_id.as_deref() == Some(meta.id) {
            asr_config.current_model_id = None;
            let value = serde_json::to_value(&asr_config)
                .map_err(|err| AppError::Config(4, format!("配置序列化失败: {err}")))?;
            self.config.set_section(ConfigSection::Asr, value).await?;
        }
        let _ = self.events.send(AppEvent::AsrNotice {
            code: "E-ASR-000".to_string(),
            message: delete_notice(was_active, fallback.as_deref()),
        });
        Ok(())
    }

    /// 切换激活模型：校验已下载后立即重载引擎（下一次录音用新模型，T-014 验收）。
    pub async fn set_active_model(&self, model_id: String) -> Result<(), AppError> {
        let meta = model_meta(&model_id)
            .ok_or_else(|| AppError::Asr(ERR_UNKNOWN_MODEL, format!("未知模型: {model_id}")))?;
        if !self.is_usable_candidate(meta.id).await {
            return Err(AppError::Asr(
                ERR_MODEL_MISSING,
                "模型尚未下载，请先下载后再切换".to_string(),
            ));
        }
        let mut asr_config = self.config.get().await.asr;
        asr_config.current_model_id = Some(model_id);
        let value = serde_json::to_value(&asr_config)
            .map_err(|err| AppError::Config(4, format!("配置序列化失败: {err}")))?;
        self.config.set_section(ConfigSection::Asr, value).await?;
        // ConfigChanged(Asr) 事件会卸载引擎；此处主动重载使切换即时生效。
        self.reload_engine().await?;
        Ok(())
    }

    /// 重载引擎（按当前激活模型）。失败保留旧引擎不动作（由调用方决定提示）。
    pub async fn reload_engine(&self) -> Result<(), AppError> {
        let _guard = self.lifecycle.lock().await;
        let model_id = self.resolve_active_model().await.ok_or_else(|| {
            AppError::Asr(
                ERR_MODEL_MISSING,
                "没有可用的识别模型，请先下载模型".to_string(),
            )
        })?;
        let meta = *model_meta(&model_id)
            .ok_or_else(|| AppError::Asr(ERR_UNKNOWN_MODEL, format!("未知模型: {model_id}")))?;
        let engine = self.load_engine(&meta).await?;
        *self.engine.lock().await = Some((meta.id.to_string(), engine));
        Ok(())
    }

    /// 加载引擎（spawn_blocking：模型加载为耗时阻塞操作，tech/04 4.4）。
    async fn load_engine(&self, meta: &ModelMeta) -> Result<Engine, AppError> {
        let meta = *meta;
        let dir = model_dir(&self.effective_models_dir().await, meta.id);
        let engine = tokio::task::spawn_blocking(move || load_engine_blocking(&meta, &dir))
            .await
            .map_err(|err| AppError::Asr(ERR_LOAD_FAILED, format!("模型加载任务失败: {err}")))??;
        self.verified.write().await.insert(meta.id.to_string());
        Ok(engine)
    }

    /// 录音音频就绪：确保引擎已加载（懒加载），随后推理。
    async fn handle_audio(&self, session_id: u64, sample_rate: u32, samples: Arc<Vec<f32>>) {
        // 懒加载：引擎缺失时按当前激活模型加载（首次使用 / 切换后）。
        {
            let engine = self.engine.lock().await;
            if engine.is_none() {
                drop(engine);
                if let Err(err) = self.reload_engine().await {
                    warn!(code = %err.code(), "识别引擎不可用");
                    let _ =
                        self.events
                            .send(AppEvent::RecordingResult(RecordingResultEvent::Failed {
                                code: err.code(),
                                message: err.message().to_string(),
                            }));
                    // 会话终态：无 Recognized 产出，由本服务广播（T-012）。
                    let _ = self.events.send(AppEvent::SessionFinished { session_id });
                    return;
                }
            }
        }

        let engine_guard = self.engine.lock().await;
        let Some((model_id, engine)) = engine_guard.as_ref() else {
            return; // 上面已报错，防御性返回
        };
        let engine = engine.clone();
        let model_id = model_id.clone();
        drop(engine_guard);

        let events = self.events.clone();
        let tracker = self.tracker.clone();
        // T-545：标点恢复在识别同一 `spawn_blocking` 语境执行（不阻塞事件循环）；
        // 开关关闭零开销（不读模型、不加载引擎），模型未就绪原文直出（降级不报错）。
        let models_dir = self.effective_models_dir().await;
        let punct = self.punct.clone();
        let punct_enabled = self.config.get().await.asr.punct.enabled;
        tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            let prepared = resample_to_engine(&samples, sample_rate);
            let result = lock_engine(&engine).transcribe(ENGINE_SAMPLE_RATE, &prepared);
            // NFR-P-03/04 埋点：日志只出现耗时与模型 ID，不出现识别文本（NFR-S-02）。
            info!(
                session_id,
                model = %model_id,
                samples = prepared.len(),
                elapsed_ms = %started.elapsed().as_millis(),
                "识别推理完成"
            );
            // T-010 会话代次检查：识别期间会话被取消或已过期（新会话开始），
            // 结果丢弃不上屏（音频即弃语义的识别侧兜底）。
            if !tracker.is_valid(session_id) {
                info!(session_id, "识别结果已过期或已取消，丢弃");
                return;
            }
            if result.text.trim().is_empty() {
                let _ = events.send(AppEvent::RecordingResult(RecordingResultEvent::Failed {
                    code: AppError::Asr(ERR_INFERENCE, String::new()).code(),
                    message: "未能识别出有效内容，请靠近麦克风重试".to_string(),
                }));
                // 会话终态：无 Recognized 产出，由本服务广播（T-012）。
                let _ = events.send(AppEvent::SessionFinished { session_id });
                return;
            }
            // T-545 标点恢复（PRD 3.1.5：位于后处理管线之前）：开关开启且模型可用时
            // 恢复句读，供"删除结尾标点"等输出选项消费；不可用时原文直出。
            let mut text = result.text;
            if punct_enabled {
                text = punct.punctuate_sync(&models_dir, &text);
            }
            // 识别文本 → 后处理管线（T-011 消费；上屏在 T-012 串联）。
            let _ = events.send(AppEvent::Recognized { session_id, text });
        });
    }

    /// 下载主体：取源下载（镜像直传 / 官方归档）→ 校验 → 落盘 → 自动激活。
    async fn run_download(&self, meta: ModelMeta) -> Result<(), AppError> {
        let model_id = meta.id.to_string();
        self.download_progress.write().await.insert(
            model_id.clone(),
            DownloadProgress {
                downloaded_bytes: 0,
                total_bytes: 0,
                bytes_per_sec: 0,
            },
        );
        let result = self.download_inner(&meta).await;
        self.downloads.lock().await.remove(&model_id);
        match result {
            Ok(()) => {
                self.download_progress.write().await.remove(&model_id);
                // 自动激活：当前没有任何可用模型时（含首启下载引导场景）。
                let should_activate = {
                    let verified = self.verified.read().await;
                    MODELS
                        .iter()
                        .filter(|m| m.id != meta.id)
                        .all(|m| !verified.contains(m.id))
                };
                if should_activate {
                    info!(model = %meta.id, "首个模型下载完成，自动激活");
                    let _ = self.reload_engine().await;
                }
                Ok(())
            }
            Err(err) => {
                self.download_progress.write().await.remove(&model_id);
                let _ = self.events.send(AppEvent::ModelDownloadProgress {
                    model_id,
                    status: DownloadStatus::Failed,
                    downloaded_bytes: 0,
                    total_bytes: meta.size_bytes,
                    bytes_per_sec: 0,
                });
                Err(err)
            }
        }
    }

    async fn download_inner(&self, meta: &ModelMeta) -> Result<(), AppError> {
        let model_id = meta.id;
        let _ = self.events.send(AppEvent::ModelDownloadProgress {
            model_id: model_id.to_string(),
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
        // 旧版整包临时文件已无续传价值，顺手清理。
        let tmp_dir = models_dir.join(format!(".{model_id}.tmp"));
        std::fs::create_dir_all(&tmp_dir)
            .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("创建下载目录失败: {err}")))?;
        let _ = std::fs::remove_file(models_dir.join(format!(".{model_id}.tar.bz2")));

        let downloaded = Arc::new(AtomicU64::new(0));
        let total_known = Arc::new(AtomicU64::new(0));

        // 进度节流广播（含实时速度）。
        // 取消下载是对整个任务 handle.abort()：正常路径的 ticker.abort()（download_inner
        // 返回前）不会执行，ticker 成为孤儿任务继续广播假进度。故每拍自查下载是否仍在
        // downloads 注册表（cancel_download 会先摘除注册），不在即退出。
        let ticker = {
            let events = self.events.clone();
            let progress_map = self.download_progress.clone();
            let downloads = self.downloads.clone();
            let model_id = model_id.to_string();
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
                    // 到第二拍注册必然完成（见 download_model）。
                    if first_tick {
                        first_tick = false;
                    } else if !downloads.lock().await.contains_key(&model_id) {
                        break;
                    }
                    let done = downloaded.load(Ordering::Relaxed);
                    let total = total_known.load(Ordering::Relaxed);
                    let speed = model_download::speed_of(&mut last_bytes, done, &mut last_tick);
                    progress_map.write().await.insert(
                        model_id.clone(),
                        DownloadProgress {
                            downloaded_bytes: done,
                            total_bytes: total,
                            bytes_per_sec: speed,
                        },
                    );
                    let _ = events.send(AppEvent::ModelDownloadProgress {
                        model_id: model_id.clone(),
                        status: DownloadStatus::Downloading,
                        downloaded_bytes: done,
                        total_bytes: total,
                        bytes_per_sec: speed,
                    });
                }
            })
        };

        // 取源下载 → 完整性检查 + 哈希校验；镜像源失败或校验不过回退官方源（T-017）。
        let mut result: Result<u64, AppError> = match asr_config.model_source {
            ModelSource::Mirror => {
                self.fetch_mirror(meta, &client, &tmp_dir, &downloaded, &total_known)
                    .await
            }
            ModelSource::Official => {
                self.fetch_official(meta, &client, &tmp_dir, &downloaded, &total_known)
                    .await
            }
        };
        // 镜像源失败或产物校验不过：清理后回退官方源重下。
        if result.is_ok() && asr_config.model_source == ModelSource::Mirror {
            result = Self::check_download(meta, &tmp_dir).map(|()| 0);
        }
        if result.is_err() && asr_config.model_source == ModelSource::Mirror {
            warn!(model = %meta.id, "镜像源不可用，回退官方源下载");
            let _ = std::fs::remove_dir_all(&tmp_dir);
            let _ = std::fs::create_dir_all(&tmp_dir);
            downloaded.store(0, Ordering::Relaxed);
            total_known.store(0, Ordering::Relaxed);
            result = self
                .fetch_official(meta, &client, &tmp_dir, &downloaded, &total_known)
                .await;
        }
        if result.is_ok() {
            result = Self::check_download(meta, &tmp_dir).map(|()| 0);
        }
        ticker.abort();
        if let Err(err) = result {
            // 哈希不匹配：续传无意义（已下载字节即错误数据），清掉强制全量重下。
            if matches!(&err, AppError::Asr(code, _) if *code == ERR_HASH_MISMATCH) {
                let _ = std::fs::remove_dir_all(&tmp_dir);
            }
            return Err(err);
        }

        let final_dir = model_dir(&models_dir, model_id);
        if final_dir.exists() {
            let _ = std::fs::remove_dir_all(&final_dir);
        }
        std::fs::rename(&tmp_dir, &final_dir)
            .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("安装模型失败: {err}")))?;
        self.verified.write().await.insert(model_id.to_string());
        info!(model = %model_id, "模型下载并校验完成");
        let _ = self.events.send(AppEvent::ModelDownloadProgress {
            model_id: model_id.to_string(),
            status: DownloadStatus::Completed,
            downloaded_bytes: meta.size_bytes,
            total_bytes: meta.size_bytes,
            bytes_per_sec: 0,
        });
        Ok(())
    }

    /// 下载产物完整性检查：必备文件齐全 + 模型 / 词表文件 SHA-256 校验（T-006/T-017）。
    fn check_download(meta: &ModelMeta, tmp_dir: &Path) -> Result<(), AppError> {
        if !tmp_dir.join(MODEL_ONNX_NAME).exists() || !tmp_dir.join(TOKENS_NAME).exists() {
            return Err(AppError::Asr(
                ERR_DOWNLOAD,
                "模型文件不完整，下载已中断".to_string(),
            ));
        }
        verify_model_hash(tmp_dir, meta)
    }

    /// 镜像源：逐文件直传下载（免解包），进度累计进共享计数器。
    ///
    /// 下载前先预探测各文件大小并写入 total_known——否则大文件下载期间
    /// total 为 0，前端百分比与剩余时间无法计算（进度条卡在 0%）。
    async fn fetch_mirror(
        &self,
        meta: &ModelMeta,
        client: &reqwest::Client,
        tmp_dir: &Path,
        downloaded: &Arc<AtomicU64>,
        total_known: &Arc<AtomicU64>,
    ) -> Result<u64, AppError> {
        let files = model_download::mirror_files(meta);
        let mut expected_sum = 0u64;
        for file in &files {
            let expected = if file.dest_name == MODEL_ONNX_NAME {
                meta.size_bytes
            } else {
                0
            };
            expected_sum += model_download::probe_size(client, &file.url)
                .await
                .unwrap_or(expected)
                .max(expected);
        }
        total_known.store(expected_sum, Ordering::Relaxed);

        let mut sum = 0u64;
        for file in files {
            let expected = if file.dest_name == MODEL_ONNX_NAME {
                meta.size_bytes
            } else {
                0
            };
            let total = model_download::download_file(
                client,
                &file.url,
                &tmp_dir.join(&file.dest_name),
                expected,
                model_download::DEFAULT_THREADS,
                downloaded.clone(),
            )
            .await?;
            sum += total;
            total_known.store(sum, Ordering::Relaxed);
        }
        Ok(sum)
    }

    /// 官方源：整包归档下载（同样多线程 + 断点续传）后解包所需成员。
    async fn fetch_official(
        &self,
        meta: &ModelMeta,
        client: &reqwest::Client,
        tmp_dir: &Path,
        downloaded: &Arc<AtomicU64>,
        total_known: &Arc<AtomicU64>,
    ) -> Result<u64, AppError> {
        let archive_path = tmp_dir.join("archive.tar.bz2");
        // 下载前预探测归档大小写入 total_known（原因同 fetch_mirror）。
        let probed = model_download::probe_size(client, meta.url).await;
        total_known.store(
            probed.unwrap_or(meta.size_bytes).max(meta.size_bytes),
            Ordering::Relaxed,
        );
        let total = model_download::download_file(
            client,
            meta.url,
            &archive_path,
            meta.size_bytes,
            model_download::DEFAULT_THREADS,
            downloaded.clone(),
        )
        .await?;
        total_known.store(total, Ordering::Relaxed);

        let archive_file = std::fs::File::open(&archive_path)
            .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("读取下载文件失败: {err}")))?;
        let decoder = bzip2::read::BzDecoder::new(std::io::BufReader::new(archive_file));
        let mut archive = tar::Archive::new(decoder);
        let entries = archive
            .entries()
            .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("读取模型归档失败: {err}")))?;
        for entry in entries {
            let mut entry =
                entry.map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("解包模型失败: {err}")))?;
            let path = entry
                .path()
                .map_err(|err| AppError::Asr(ERR_DOWNLOAD, format!("解包模型失败: {err}")))?;
            let name = path.to_string_lossy();
            let target = if name == meta.model_file {
                Some(MODEL_ONNX_NAME)
            } else if name == meta.tokens_file {
                Some(TOKENS_NAME)
            } else {
                None
            };
            if let Some(target) = target {
                entry.unpack(tmp_dir.join(target)).map_err(|err| {
                    AppError::Asr(ERR_DOWNLOAD, format!("写入模型文件失败: {err}"))
                })?;
            }
        }
        let _ = std::fs::remove_file(&archive_path);
        Ok(total)
    }

    /// 识别指定 16 kHz WAV 文件（T-006 本地验证命令；也供 T-011 验收驱动）。
    ///
    /// 识别文本经返回值交付调用方，绝不写入日志（NFR-S-02）。
    pub async fn recognize_wav(&self, wav_path: String) -> Result<String, AppError> {
        let (samples, sample_rate) = tokio::task::spawn_blocking(move || {
            sherpa_rs::read_audio_file(&wav_path)
                .map_err(|err| AppError::Asr(ERR_INFERENCE, format!("读取 WAV 失败: {err}")))
        })
        .await
        .map_err(|err| AppError::Asr(ERR_INFERENCE, format!("读取任务失败: {err}")))??;

        {
            let engine = self.engine.lock().await;
            if engine.is_none() {
                drop(engine);
                self.reload_engine().await?;
            }
        }
        let engine_guard = self.engine.lock().await;
        let Some((_, engine)) = engine_guard.as_ref() else {
            return Err(AppError::Asr(ERR_LOAD_FAILED, "识别引擎不可用".to_string()));
        };
        let engine = engine.clone();
        drop(engine_guard);
        let prepared = resample_to_engine(&samples, sample_rate);
        let result = tokio::task::spawn_blocking(move || {
            lock_engine(&engine).transcribe(ENGINE_SAMPLE_RATE, &prepared)
        })
        .await
        .map_err(|err| AppError::Asr(ERR_INFERENCE, format!("推理任务失败: {err}")))?;
        Ok(result.text)
    }

    /// 标点模型卡片状态（T-545，命令 `punct_model_status` 数据源）。
    pub async fn punct_model_status(&self) -> PunctModelInfo {
        self.punct.status().await
    }

    /// 下载标点模型（幂等；进度经 `model:progress` 事件推送，T-545）。
    pub async fn download_punct_model(&self) -> Result<(), AppError> {
        self.punct.download().await
    }

    /// 取消进行中的标点模型下载（T-545）。
    pub async fn cancel_punct_model(&self) -> Result<(), AppError> {
        self.punct.cancel().await
    }

    /// 删除标点模型：状态回退为未下载，标点恢复降级为原文直出（T-545）。
    pub async fn delete_punct_model(&self) -> Result<(), AppError> {
        self.punct.delete().await
    }
}

/// 阻塞加载引擎：哈希校验（未缓存时）+ 构造 ParaformerRecognizer。
fn load_engine_blocking(meta: &ModelMeta, dir: &Path) -> Result<Engine, AppError> {
    let model_path = dir.join(MODEL_ONNX_NAME);
    let tokens_path = dir.join(TOKENS_NAME);
    if !model_path.exists() || !tokens_path.exists() {
        return Err(AppError::Asr(
            ERR_MODEL_MISSING,
            "模型文件缺失，请先下载模型".to_string(),
        ));
    }
    // 每次加载都校验哈希（model.onnx + tokens.txt）：文件可能在下载后被外部破坏
    // 或词表被篡改（T-006 验收、4.6.4）。
    verify_model_hash(dir, meta)?;
    let started = Instant::now();
    let engine =
        sherpa_rs::paraformer::ParaformerRecognizer::new(sherpa_rs::paraformer::ParaformerConfig {
            model: model_path.to_string_lossy().into_owned(),
            tokens: tokens_path.to_string_lossy().into_owned(),
            provider: None,
            num_threads: Some(2),
            debug: false,
        })
        .map_err(|err| AppError::Asr(ERR_LOAD_FAILED, format!("模型加载失败: {err}")))?;
    info!(
        model = %meta.id,
        elapsed_ms = %started.elapsed().as_millis(),
        "识别引擎加载完成"
    );
    Ok(Arc::new(std::sync::Mutex::new(engine)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_registry_is_consistent() {
        // id 唯一。
        let mut ids = MODELS.iter().map(|m| m.id).collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), MODELS.len());
        for meta in MODELS {
            assert!(meta.size_bytes > 0);
            assert_eq!(meta.sha256.len(), 64);
            assert_eq!(meta.tokens_sha256.len(), 64);
            assert!(meta.url.starts_with("https://"));
            assert!(meta.model_file.ends_with(".onnx"));
            assert!(meta.tokens_file.ends_with("tokens.txt"));
        }
        // 同档位不重名冲突：两个条目共享展示名但档位不同。
        assert_ne!(MODELS[0].tier, MODELS[1].tier);
    }

    #[test]
    fn model_meta_lookup_by_id() {
        assert!(model_meta("paraformer-zh-int8").is_some());
        assert!(model_meta("paraformer-zh").is_some());
        assert!(model_meta("nope").is_none());
    }

    #[test]
    fn default_models_dir_prefers_exe_dir_and_falls_back() {
        let base = std::env::temp_dir().join(format!("vt-models-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let exe_dir = base.join("exe");
        let data_dir = base.join("data-models");
        std::fs::create_dir_all(&exe_dir).unwrap();

        // 可写：选 exe_dir/models（探针文件已清理）。
        let picked = resolve_default_models_dir(&exe_dir, &data_dir);
        assert_eq!(picked, exe_dir.join("models"));
        assert!(!picked.join(".write-probe").exists());

        // 不可写（父路径是一个已存在的文件）：回退数据目录。
        let blocked_file = base.join("blocked");
        std::fs::write(&blocked_file, b"x").unwrap();
        let picked = resolve_default_models_dir(&blocked_file, &data_dir);
        assert_eq!(picked, data_dir);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn hex_lower_formats_bytes() {
        assert_eq!(hex_lower(&[0x00, 0xff, 0x1a]), "00ff1a");
    }

    #[test]
    fn resample_identity_and_ratio() {
        // 已是 16 kHz：原样返回。
        let samples = vec![0.1f32; 160];
        assert_eq!(resample_to_engine(&samples, 16_000), samples);
        // 48 kHz → 16 kHz：长度约 1/3。
        let long = vec![0.2f32; 4800];
        let out = resample_to_engine(&long, 48_000);
        assert!((out.len() as i64 - 1600).abs() <= 1);
        // 空输入不 panic。
        assert!(resample_to_engine(&[], 48_000).is_empty());
    }

    #[test]
    fn resample_preserves_signal_shape() {
        // 常量信号重采样后仍近似常量（线性插值保直流分量）。
        let samples = vec![0.5f32; 3200];
        let out = resample_to_engine(&samples, 32_000);
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-6));
    }

    #[tokio::test]
    async fn hash_mismatch_rejected() {
        // 构造假模型文件：model.onnx 哈希必须失败（E-ASR-003）。
        let dir = std::env::temp_dir().join(format!("vt-asr-hash-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.onnx"), b"not a real model").unwrap();
        std::fs::write(dir.join("tokens.txt"), "a 1\n").unwrap();
        let meta = model_meta("paraformer-zh-int8").unwrap();
        let err = verify_model_hash(&dir, meta).unwrap_err();
        assert_eq!(err.code(), "E-ASR-003");
        // 错误信息指明文件与期望 / 实际哈希。
        let msg = err.to_string();
        assert!(msg.contains("model.onnx"), "错误信息应指明文件名: {msg}");
        assert!(msg.contains("期望"), "错误信息应含期望哈希: {msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tokens_hash_mismatch_rejected() {
        // 验收 4.6.4：tokens.txt 被篡改同样报错（E-ASR-003），不再只校验 model.onnx。
        // 用自建元数据构造"模型文件正确、词表文件被篡改"的场景。
        let dir = std::env::temp_dir().join(format!(
            "vt-asr-tokens-hash-test-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let model_bytes = b"fake onnx";
        let tokens_bytes = b"tampered tokens";
        std::fs::write(dir.join("model.onnx"), model_bytes).unwrap();
        std::fs::write(dir.join("tokens.txt"), tokens_bytes).unwrap();
        let meta = ModelMeta {
            id: "test",
            name: "测试",
            languages: "中文",
            tier: ModelTier::Accuracy,
            size_bytes: model_bytes.len() as u64,
            sha256: Box::leak(hex_lower(&sha2::Sha256::digest(model_bytes)).into_boxed_str()),
            tokens_sha256: "0".repeat(64).leak(),
            url: "https://example.com/model.tar.bz2",
            model_file: "model.onnx",
            tokens_file: "tokens.txt",
        };
        let err = verify_model_hash(&dir, &meta).unwrap_err();
        assert_eq!(err.code(), "E-ASR-003");
        let msg = err.to_string();
        assert!(msg.contains("tokens.txt"), "错误信息应指明词表文件: {msg}");
        assert!(msg.contains("期望"), "错误信息应含期望哈希: {msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn both_files_match_hash_ok() {
        // 双文件哈希均匹配时校验通过（自建元数据，真实文件内容）。
        let dir = std::env::temp_dir().join(format!(
            "vt-asr-hash-ok-test-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.onnx"), b"fake onnx").unwrap();
        std::fs::write(dir.join("tokens.txt"), b"real tokens").unwrap();
        let meta = ModelMeta {
            id: "test",
            name: "测试",
            languages: "中文",
            tier: ModelTier::Accuracy,
            size_bytes: 9,
            sha256: Box::leak(hex_lower(&sha2::Sha256::digest(b"fake onnx")).into_boxed_str()),
            tokens_sha256: Box::leak(
                hex_lower(&sha2::Sha256::digest(b"real tokens")).into_boxed_str(),
            ),
            url: "https://example.com/model.tar.bz2",
            model_file: "model.onnx",
            tokens_file: "tokens.txt",
        };
        assert!(verify_model_hash(&dir, &meta).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn list_models_marks_missing_and_active() {
        let dir = std::env::temp_dir().join(format!("vt-asr-list-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (events_tx, _rx) = crate::events::channel();
        let config = ConfigStore::load(&dir, events_tx).unwrap();
        let models_dir = dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let service = AsrService::new(
            models_dir,
            config,
            crate::events::channel().0,
            std::sync::Arc::new(crate::services::session::SessionTracker::new()),
        );
        let models = service.list_models().await;
        assert_eq!(models.len(), MODELS.len());
        // 未下载：全部 not downloaded，无激活。
        assert!(models
            .iter()
            .all(|m| !m.downloaded && !m.active && !m.invalid));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn present_but_corrupt_model_marked_invalid() {
        // 文件齐全但哈希不符（如重启后缓存清空 / 文件被外部破坏）：
        // list_models 懒校验后标记 invalid，且不列为可用候选。
        let dir = std::env::temp_dir().join(format!("vt-asr-corrupt-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (events_tx, _rx) = crate::events::channel();
        let config = ConfigStore::load(&dir, events_tx).unwrap();
        let models_dir = dir.join("models");
        let model_dir = model_dir(&models_dir, "paraformer-zh-int8");
        std::fs::create_dir_all(&model_dir).unwrap();
        std::fs::write(model_dir.join("model.onnx"), b"corrupted").unwrap();
        std::fs::write(model_dir.join("tokens.txt"), "a 1\n").unwrap();
        let service = AsrService::new(
            models_dir,
            config,
            crate::events::channel().0,
            std::sync::Arc::new(crate::services::session::SessionTracker::new()),
        );
        let models = service.list_models().await;
        let int8 = models
            .iter()
            .find(|m| m.id == "paraformer-zh-int8")
            .unwrap();
        assert!(int8.invalid, "哈希不符应标记 invalid");
        assert!(!int8.downloaded);
        assert!(!int8.active);
        // 加载路径把关：损坏模型拒绝加载（E-ASR-003），不静默使用。
        let err = service.reload_engine().await.unwrap_err();
        assert_eq!(err.code(), "E-ASR-003");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn delete_unknown_model_rejected() {
        let dir = std::env::temp_dir().join(format!("vt-asr-del-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let service = AsrService::new(
            dir.join("models"),
            ConfigStore::load(&dir, crate::events::channel().0).unwrap(),
            crate::events::channel().0,
            std::sync::Arc::new(crate::services::session::SessionTracker::new()),
        );
        let err = service.delete_model("nope".to_string()).await.unwrap_err();
        assert_eq!(err.code(), "E-ASR-007");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_notice_three_branches() {
        // 验收 4.6.12：删除激活模型有回退 / 无回退、删除非激活模型三种文案。
        assert_eq!(
            delete_notice(true, Some("paraformer-zh")),
            "已删除模型，识别将使用 Paraformer 中文"
        );
        assert_eq!(
            delete_notice(true, None),
            "已删除模型，当前没有可用模型，请下载模型后再使用语音输入"
        );
        // 非激活模型：不提及回退 / 无可用模型（原逻辑错误的修复点）。
        assert_eq!(delete_notice(false, None), "已删除模型");
        assert_eq!(delete_notice(false, Some("paraformer-zh")), "已删除模型");
    }

    /// 造两个"文件齐全"（未哈希认证）的模型目录，供删除场景测试。
    fn seed_models(models_dir: &Path) {
        for meta in MODELS {
            let dir = model_dir(models_dir, meta.id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(MODEL_ONNX_NAME), b"fake").unwrap();
            std::fs::write(dir.join(TOKENS_NAME), "a 1\n").unwrap();
        }
    }

    #[tokio::test]
    async fn delete_inactive_model_notice_is_plain() {
        // 原缺陷：删除非激活模型时 fallback 恒为 None，错误提示"没有可用模型"。
        // 修复后提示应只是"已删除模型"（激活模型 paraformer-zh-int8 仍在）。
        let dir = std::env::temp_dir().join(format!(
            "vt-asr-del-inactive-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let models_dir = dir.join("models");
        seed_models(&models_dir);
        let (tx, mut rx) = crate::events::channel();
        let service = AsrService::new(
            models_dir,
            ConfigStore::load(&dir, crate::events::channel().0).unwrap(),
            tx,
            std::sync::Arc::new(crate::services::session::SessionTracker::new()),
        );
        // 激活模型为首个可用（paraformer-zh-int8），删除另一个（非激活）。
        service
            .delete_model("paraformer-zh".to_string())
            .await
            .unwrap();
        let notice = loop {
            match rx.recv().await.unwrap() {
                AppEvent::AsrNotice { message, .. } => break message,
                _ => continue,
            }
        };
        assert_eq!(notice, "已删除模型");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn delete_active_model_falls_back_with_notice() {
        // 删除激活模型且有可用回退：提示回退模型名，配置清空走自动回退。
        let dir = std::env::temp_dir().join(format!(
            "vt-asr-del-active-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let models_dir = dir.join("models");
        seed_models(&models_dir);
        let (tx, mut rx) = crate::events::channel();
        let config = ConfigStore::load(&dir, crate::events::channel().0).unwrap();
        let service = AsrService::new(
            models_dir,
            config.clone(),
            tx,
            std::sync::Arc::new(crate::services::session::SessionTracker::new()),
        );
        service
            .delete_model("paraformer-zh-int8".to_string())
            .await
            .unwrap();
        let notice = loop {
            match rx.recv().await.unwrap() {
                AppEvent::AsrNotice { message, .. } => break message,
                _ => continue,
            }
        };
        assert_eq!(notice, "已删除模型，识别将使用 Paraformer 中文");
        // 配置中的 current_model_id 已清掉（走自动回退）。
        assert!(config.get().await.asr.current_model_id.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn delete_active_model_without_fallback_notice() {
        // 删除激活模型且无其他可用模型：提示下载引导。
        let dir = std::env::temp_dir().join(format!(
            "vt-asr-del-no-fallback-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let models_dir = dir.join("models");
        let only = model_dir(&models_dir, "paraformer-zh-int8");
        std::fs::create_dir_all(&only).unwrap();
        std::fs::write(only.join(MODEL_ONNX_NAME), b"fake").unwrap();
        std::fs::write(only.join(TOKENS_NAME), "a 1\n").unwrap();
        let (tx, mut rx) = crate::events::channel();
        let service = AsrService::new(
            models_dir,
            ConfigStore::load(&dir, crate::events::channel().0).unwrap(),
            tx,
            std::sync::Arc::new(crate::services::session::SessionTracker::new()),
        );
        service
            .delete_model("paraformer-zh-int8".to_string())
            .await
            .unwrap();
        let notice = loop {
            match rx.recv().await.unwrap() {
                AppEvent::AsrNotice { message, .. } => break message,
                _ => continue,
            }
        };
        assert_eq!(
            notice,
            "已删除模型，当前没有可用模型，请下载模型后再使用语音输入"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
