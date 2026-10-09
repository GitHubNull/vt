//! AppState：各服务句柄的聚合，注入 Tauri State（tech/04 4.1）。

use tokio::sync::mpsc;

use crate::services::config_store::ConfigStore;
use crate::services::hotkey::RecordCommand;

/// 应用全局状态（各服务句柄聚合）。
///
/// 句柄均为 `Clone`（内部 `Arc`），命令层经 `State<'_, AppState>` 共享。
pub struct AppState {
    /// 配置存储单例（T-003）：全部配置读写必经此句柄。
    pub config: ConfigStore,
    /// 录音控制命令发送端（tech/04 4.2，容量 8）：快捷键 / 命令层 / 小窗 → AudioService。
    pub record_tx: mpsc::Sender<RecordCommand>,
}

impl AppState {
    pub fn new(config: ConfigStore, record_tx: mpsc::Sender<RecordCommand>) -> Self {
        Self { config, record_tx }
    }
}
