//! 日志命令：应用内查询 / 搜索导出（T-018，F-LOG-01 / F-LOG-03）。
//!
//! 数据库为同步 SQLite：查询与导出统一进 `spawn_blocking`，避免阻塞异步运行时；
//! 数据库记录目标未启用时服务层返回 E-LOG-001（前端据此禁用搜索区）。

use std::path::PathBuf;

use tauri::State;

use crate::error::AppError;
use crate::services::log::{ExportLogsResult, LogPage, LogQuery};
use crate::state::AppState;

/// 按条件查询日志（关键词 / 正则 / 大小写 / 级别 / 时间范围 / 分页）。
#[tauri::command]
pub async fn log_query(state: State<'_, AppState>, query: LogQuery) -> Result<LogPage, AppError> {
    let store = state.log.clone();
    tokio::task::spawn_blocking(move || store.query(&query))
        .await
        .map_err(|err| {
            AppError::Log(
                crate::services::log::LOG_ERR_TASK,
                format!("任务执行失败: {err}"),
            )
        })?
}

/// 按当前查询条件导出匹配日志到用户选择的路径（plugin-dialog 选定）。
#[tauri::command]
pub async fn log_export(
    state: State<'_, AppState>,
    query: LogQuery,
    target_path: String,
) -> Result<ExportLogsResult, AppError> {
    let store = state.log.clone();
    let target = PathBuf::from(target_path);
    tokio::task::spawn_blocking(move || store.export(&query, &target))
        .await
        .map_err(|err| {
            AppError::Log(
                crate::services::log::LOG_ERR_TASK,
                format!("任务执行失败: {err}"),
            )
        })?
}
