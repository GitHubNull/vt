//! 窗口生命周期控制：小窗 / 设置窗的唤起与"关闭即隐藏"（PRD 3.15、T-009）。
//!
//! 最小权限说明（AGENTS.md 3.5）：仅对本进程窗口执行 show / hide / focus，
//! 不操作其他应用窗口。

use tauri::Manager;

use crate::error::AppError;

/// 错误编号：目标窗口不存在。
pub const ERR_WINDOW_MISSING: u32 = 1;

/// 唤起语音输入小窗（T-009 命令与托盘左键 / 菜单共用）。
///
/// 已存在则取消最小化、显示并聚焦；不存在返回错误（窗口由 tauri.conf.json 声明）。
pub fn show_mini_window(app: &tauri::AppHandle) -> Result<(), AppError> {
    let window = app
        .get_webview_window("mini")
        .ok_or_else(|| AppError::Platform(ERR_WINDOW_MISSING, "小窗不存在".to_string()))?;
    let _ = window.unminimize();
    window
        .show()
        .map_err(|err| AppError::Platform(ERR_WINDOW_MISSING, format!("小窗显示失败: {err}")))?;
    window
        .set_focus()
        .map_err(|err| AppError::Platform(ERR_WINDOW_MISSING, format!("小窗聚焦失败: {err}")))?;
    Ok(())
}

/// 唤起设置主窗口（托盘菜单"打开设置"）。
pub fn show_settings_window(app: &tauri::AppHandle) -> Result<(), AppError> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| AppError::Platform(ERR_WINDOW_MISSING, "设置窗口不存在".to_string()))?;
    let _ = window.unminimize();
    window.show().map_err(|err| {
        AppError::Platform(ERR_WINDOW_MISSING, format!("设置窗口显示失败: {err}"))
    })?;
    window.set_focus().map_err(|err| {
        AppError::Platform(ERR_WINDOW_MISSING, format!("设置窗口聚焦失败: {err}"))
    })?;
    Ok(())
}
