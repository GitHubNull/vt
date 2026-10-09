//! 窗口命令：小窗唤起（T-009 后端契约；托盘菜单 T-015 共用）。

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
