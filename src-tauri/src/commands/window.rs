//! 窗口命令：小窗唤起（T-009 后端契约；托盘菜单 T-015 共用）、悬浮球窗口管理
//! （T-008：`show_float` / `hide_float` / `move_float`）、设置窗唤起（T-009 小窗按钮）。

use tauri::State;

use crate::error::AppError;
use crate::state::AppState;

/// 唤起语音输入小窗：已存在则取消最小化、显示并聚焦（T-009 SPEC 步骤 4）。
#[tauri::command]
pub async fn show_mini_window(
    _state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), AppError> {
    crate::windows::show_mini_window(&app)
}

/// 唤起设置主窗口（小窗"设置"按钮消费，T-009）。
#[tauri::command]
pub async fn show_settings_window(
    _state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), AppError> {
    crate::windows::show_settings_window(&app)
}

/// 显示悬浮球（T-008）：主入口是录音状态联动的 `spawn_float_manager`，
/// 此命令供调试与前端兜底（恢复记忆位置、不抢焦点）。
#[tauri::command]
pub async fn show_float(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<(), AppError> {
    crate::windows::show_float(&app, &state.config).await
}

/// 隐藏悬浮球（T-008：结果反馈结束后由悬浮球页调用，等效销毁）。
#[tauri::command]
pub async fn hide_float(app: tauri::AppHandle) -> Result<(), AppError> {
    crate::windows::hide_float(&app)
}

/// 移动悬浮球并持久化位置（T-008：拖动结束时调用，位置记忆到配置）。
#[tauri::command]
pub async fn move_float(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    x: i32,
    y: i32,
) -> Result<(), AppError> {
    crate::windows::move_float(&app, &state.config, x, y).await
}
