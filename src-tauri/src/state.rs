//! AppState：各服务句柄的聚合，注入 Tauri State（tech/04 4.1）。

use crate::services::config_store::ConfigStore;

/// 应用全局状态（各服务句柄聚合）。
///
/// 句柄均为 `Clone`（内部 `Arc`），命令层经 `State<'_, AppState>` 共享。
pub struct AppState {
    /// 配置存储单例（T-003）：全部配置读写必经此句柄。
    pub config: ConfigStore,
}

impl AppState {
    pub fn new(config: ConfigStore) -> Self {
        Self { config }
    }
}
