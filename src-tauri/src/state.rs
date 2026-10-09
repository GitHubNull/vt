//! AppState：各服务句柄的聚合，注入 Tauri State（tech/04 4.1）。

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::platform::audio::AudioBackend;
use crate::services::config_store::ConfigStore;
use crate::services::hotkey::RecordCommand;
use crate::services::inject::InjectService;

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
}

impl AppState {
    pub fn new(
        config: ConfigStore,
        record_tx: mpsc::Sender<RecordCommand>,
        audio_backend: Arc<dyn AudioBackend>,
        inject_service: InjectService,
    ) -> Self {
        Self {
            config,
            record_tx,
            audio_backend,
            inject_service,
        }
    }
}
