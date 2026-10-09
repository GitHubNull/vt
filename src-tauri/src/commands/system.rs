//! 系统联调命令。

use crate::error::AppError;

/// 前端联调探活命令：返回 `"pong"`。
#[tauri::command]
pub async fn ping() -> Result<String, AppError> {
    Ok("pong".to_string())
}
