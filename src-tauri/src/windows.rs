//! 窗口生命周期控制：小窗 / 设置窗的唤起与"关闭即隐藏"（PRD 3.15、T-009）；
//! 录音悬浮球的显隐、拖动与位置记忆（F-INPUT-05、T-008）。
//!
//! 最小权限说明（AGENTS.md 3.5）：仅对本进程窗口执行 show / hide / focus /
//! set_position，不操作其他应用窗口。

use tauri::{Manager, PhysicalPosition};
use tokio::sync::broadcast;
use tracing::warn;

use crate::error::AppError;
use crate::events::{AppEvent, RecordingState};
use crate::services::config_store::{ConfigSection, ConfigStore};

/// 错误编号：目标窗口不存在。
pub const ERR_WINDOW_MISSING: u32 = 1;

/// 悬浮球窗口声明尺寸（main.rs create_app_windows，胶囊两侧留 4 px 透明边距给动效呼吸）。
const FLOAT_WINDOW_WIDTH: i32 = 288;
const FLOAT_WINDOW_HEIGHT: i32 = 52;
/// 默认落点距屏幕底边距离（未拖动过且策略为底部居中时）。
const FLOAT_DEFAULT_BOTTOM_MARGIN: i32 = 96;

/// 唤起语音输入小窗（T-009 命令与托盘左键 / 菜单共用）。
///
/// 已存在则取消最小化、显示并聚焦；不存在返回错误（窗口在 setup 中创建）。
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

/// 唤起设置主窗口（托盘菜单"打开设置"、小窗"设置"按钮 T-009 共用）。
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

/// 显示悬浮球窗口（T-008）：恢复记忆位置（无记忆则屏幕底部居中），随后 show。
///
/// 不抢焦点：窗口创建时声明 `focusable: false`（tech/09 9.2.4），此处不调用 set_focus。
pub async fn show_float(app: &tauri::AppHandle, config: &ConfigStore) -> Result<(), AppError> {
    let window = app
        .get_webview_window("float")
        .ok_or_else(|| AppError::Platform(ERR_WINDOW_MISSING, "悬浮球窗口不存在".to_string()))?;
    if let Some((x, y)) = resolve_float_position(app, config).await {
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
    window
        .show()
        .map_err(|err| AppError::Platform(ERR_WINDOW_MISSING, format!("悬浮球显示失败: {err}")))?;
    Ok(())
}

/// 隐藏悬浮球窗口（"销毁"语义：窗口常驻、录音结束 / 取消 /
/// 结果反馈完成后隐藏，T-008）。
pub fn hide_float(app: &tauri::AppHandle) -> Result<(), AppError> {
    let window = app
        .get_webview_window("float")
        .ok_or_else(|| AppError::Platform(ERR_WINDOW_MISSING, "悬浮球窗口不存在".to_string()))?;
    window
        .hide()
        .map_err(|err| AppError::Platform(ERR_WINDOW_MISSING, format!("悬浮球隐藏失败: {err}")))?;
    Ok(())
}

/// 移动悬浮球并持久化位置（T-008 验收：拖动后位置下次保持）。
///
/// 位置存于 `general.float_ball_xy`（界面状态，写 general 节不触发快捷键重注册）。
pub async fn move_float(
    app: &tauri::AppHandle,
    config: &ConfigStore,
    x: i32,
    y: i32,
) -> Result<(), AppError> {
    let window = app
        .get_webview_window("float")
        .ok_or_else(|| AppError::Platform(ERR_WINDOW_MISSING, "悬浮球窗口不存在".to_string()))?;
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|err| AppError::Platform(ERR_WINDOW_MISSING, format!("悬浮球移动失败: {err}")))?;
    let mut general = config.get().await.general;
    general.float_ball_xy = Some([x, y]);
    let value = serde_json::to_value(&general)
        .map_err(|err| AppError::Platform(ERR_WINDOW_MISSING, format!("配置序列化失败: {err}")))?;
    config
        .set_section(ConfigSection::General, value)
        .await
        .map_err(|err| {
            AppError::Platform(ERR_WINDOW_MISSING, format!("悬浮球位置保存失败: {err}"))
        })?;
    Ok(())
}

/// 悬浮球落点：记忆位置优先；无记忆按主显示器底部居中。
///
/// "跟随光标"策略依赖平台光标 API，v0.1 按 tech/09 9.2.4 精神降级为底部居中
/// （拖动一次后始终使用记忆位置）。
async fn resolve_float_position(
    app: &tauri::AppHandle,
    config: &ConfigStore,
) -> Option<(i32, i32)> {
    if let Some([x, y]) = config.get().await.general.float_ball_xy {
        return Some((x, y));
    }
    let monitor = app.primary_monitor().ok().flatten()?;
    let size = monitor.size();
    let origin = monitor.position();
    Some((
        origin.x + (size.width as i32 - FLOAT_WINDOW_WIDTH) / 2,
        origin.y + size.height as i32 - FLOAT_WINDOW_HEIGHT - FLOAT_DEFAULT_BOTTOM_MARGIN,
    ))
}

/// 悬浮球窗口管理（T-008）：录音进入 Recording 且配置开启时显示；
/// 结束 / 取消 / 结果反馈后的隐藏由悬浮球页在完成反馈后调用 `hide_float` 完成
/// （PRD 4.3：完成 0.8 s、已取消 0.5 s、降级 2 s、失败的短暂反馈）。
pub fn spawn_float_manager(
    app: tauri::AppHandle,
    config: ConfigStore,
    mut events_rx: broadcast::Receiver<AppEvent>,
) {
    tauri::async_runtime::spawn(async move {
        while let Ok(event) = events_rx.recv().await {
            let AppEvent::RecordingState { state, .. } = event else {
                continue;
            };
            if state != RecordingState::Recording {
                continue;
            }
            // 录音状态可能晚于配置读取：每次现读（配置句柄为内存镜像，开销可忽略）。
            if !config.get().await.hotkey.show_float_ball {
                continue;
            }
            if let Err(err) = show_float(&app, &config).await {
                warn!(code = %err.code(), "悬浮球显示失败");
            }
        }
    });
}
