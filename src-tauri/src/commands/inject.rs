//! 文本上屏命令（T-007；T-012 识别管线联调共用）。

use tauri::State;

use crate::error::AppError;
use crate::platform::inject::InjectReport;
use crate::state::AppState;

/// 把文本输入到当前光标处（三级降级链：模拟键入 → 剪贴板粘贴 → 无焦点兜底）。
///
/// 返回 `InjectReport { method, degraded }`：`method` 为 `typed` / `pasted` /
/// `clipboard_fallback`；`degraded` 为降级原因（无焦点 / 权限引导等），主路径
/// 成功时为 `null`。任何路径都不产生回车 / 执行类按键（NFR-S-05）。
#[tauri::command]
pub async fn inject_text(
    state: State<'_, AppState>,
    text: String,
) -> Result<InjectReport, AppError> {
    state.inject_service.inject_text(&text).await
}
