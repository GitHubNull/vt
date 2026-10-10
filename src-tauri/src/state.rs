//! AppState：各服务句柄的聚合，注入 Tauri State（tech/04 4.1）。

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::platform::audio::AudioBackend;
use crate::services::asr::AsrService;
use crate::services::config_store::ConfigStore;
use crate::services::dict::DictService;
use crate::services::hotkey::RecordCommand;
use crate::services::inject::InjectService;
use crate::services::log::LogStore;
use crate::services::pipeline::PipelineService;
use crate::services::post_process::PostProcess;

/// 应用全局状态（各服务句柄聚合）。
///
/// 句柄均为 `Clone`（内部 `Arc`），命令层经 `State<'_, AppState>` 共享。
pub struct AppState {
    /// 配置存储单例（T-003）：全部配置读写必经此句柄。
    pub config: ConfigStore,
    /// 录音控制命令发送端（tech/04 4.2，容量 8）：快捷键 / 命令层 / 小窗 → AudioService。
    pub record_tx: mpsc::Sender<RecordCommand>,
    /// 音频后端句柄（T-005）：设备枚举命令直连后端（与 AudioService 同一实例族）。
    pub audio_backend: Arc<dyn AudioBackend>,
    /// 文本上屏服务（T-007）：三级降级链（模拟键入 → 剪贴板粘贴 → 无焦点兜底）。
    pub inject_service: InjectService,
    /// 识别引擎与模型管理（T-006）：加载 / 切换 / 下载 / 删除 / 推理。
    pub asr: Arc<AsrService>,
    /// 替换词典服务（T-011）：规则 CRUD / 启停 / 持久化。
    pub dict: DictService,
    /// 后处理管线（T-011）：识别文本 → 管线 → 出口事件。
    pub post_process: Arc<PostProcess>,
    /// 链路编排（T-012）：管线出口 → 上屏 → 统一结果反馈 + 耗时埋点。
    pub pipeline: Arc<PipelineService>,
    /// 日志服务句柄（T-018）：查询 / 搜索导出 / 记录目标热更新。
    pub log: LogStore,
}

impl AppState {
    /// 句柄聚合构造：参数即服务清单（tech/04 4.1），随服务增长自然扩展。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: ConfigStore,
        record_tx: mpsc::Sender<RecordCommand>,
        audio_backend: Arc<dyn AudioBackend>,
        inject_service: InjectService,
        asr: Arc<AsrService>,
        dict: DictService,
        post_process: Arc<PostProcess>,
        pipeline: Arc<PipelineService>,
        log: LogStore,
    ) -> Self {
        Self {
            config,
            record_tx,
            audio_backend,
            inject_service,
            asr,
            dict,
            post_process,
            pipeline,
            log,
        }
    }
}
