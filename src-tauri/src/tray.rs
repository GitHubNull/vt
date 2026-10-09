//! 系统托盘（F-SYS-01、PRD 3.15.1、tech/09 9.2.5）：常驻后台入口。
//!
//! - 双态图标：空闲（常规）/ 录音中（红点），经 `recording:state` 事件切换；
//! - 菜单：打开语音输入小窗、打开设置、退出（"连接手机"为 v1.0 预留，本阶段不显示）；
//! - 左键唤起小窗（PRD 默认行为），右键弹出菜单；
//! - 退出菜单：进程退出（配置已即时落盘，actor 任务随进程退出回收）。
//!
//! Linux 说明（tech/09 9.2.5）：托盘依赖 AppIndicator 实现（KDE / Ubuntu 自带；
//! GNOME 需 AppIndicator 扩展，未安装时图标不可见属环境限制，行为已在此记录）。
//!
//! 最小权限说明（AGENTS.md 3.5）：仅注册托盘图标与菜单，不监听全局输入。

use std::time::Instant;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tokio::sync::broadcast;
use tracing::info;

use crate::events::{AppEvent, RecordingState};
use crate::windows;

/// 托盘菜单项 ID。
const MENU_OPEN_MINI: &str = "open-mini";
const MENU_OPEN_SETTINGS: &str = "open-settings";
const MENU_QUIT: &str = "quit";

/// 托盘图标（编译期嵌入，保证开发 / 打包环境一致可用）。
const ICON_DEFAULT: &[u8] = include_bytes!("../icons/tray-default.png");
const ICON_RECORDING: &[u8] = include_bytes!("../icons/tray-recording.png");

/// 注册托盘图标、菜单与状态联动；`process_start` 为进程启动时刻（NFR-P-08 埋点）。
pub fn setup_tray(
    app: &tauri::App,
    mut events_rx: broadcast::Receiver<AppEvent>,
    process_start: Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    let default_icon = Image::from_bytes(ICON_DEFAULT)?;
    let recording_icon = Image::from_bytes(ICON_RECORDING)?;

    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, MENU_OPEN_MINI, "打开语音输入小窗", true, None::<&str>)?,
            &MenuItem::with_id(app, MENU_OPEN_SETTINGS, "打开设置", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, MENU_QUIT, "退出", true, None::<&str>)?,
        ],
    )?;

    let tray = TrayIconBuilder::with_id("main-tray")
        .tooltip("VibeType 码语")
        .icon(default_icon.clone())
        .menu(&menu)
        // 左键按 PRD 唤起小窗，不出菜单；菜单走右键。
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_OPEN_MINI => {
                if let Err(err) = windows::show_mini_window(app) {
                    tracing::warn!(code = %err.code(), "托盘唤起小窗失败");
                }
            }
            MENU_OPEN_SETTINGS => {
                if let Err(err) = windows::show_settings_window(app) {
                    tracing::warn!(code = %err.code(), "托盘唤起设置失败");
                }
            }
            MENU_QUIT => {
                info!("托盘退出：进程关停（配置已即时落盘）");
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle().clone();
                if let Err(err) = windows::show_mini_window(&app) {
                    tracing::warn!(code = %err.code(), "托盘左键唤起小窗失败");
                }
            }
        })
        .build(app)?;

    // 录音状态联动：Recording 切红点图标，其余状态恢复常规图标（PRD 3.15.1）。
    let state_icon_task = tray.clone();
    tauri::async_runtime::spawn(async move {
        while let Ok(event) = events_rx.recv().await {
            if let AppEvent::RecordingState { state, .. } = event {
                let icon = if state == RecordingState::Recording {
                    recording_icon.clone()
                } else {
                    default_icon.clone()
                };
                if let Err(err) = state_icon_task.set_icon(Some(icon)) {
                    tracing::warn!(error = %err, "托盘图标切换失败");
                }
            }
        }
    });

    // NFR-P-08 冷启动埋点：进程启动到托盘可见的耗时（目标 ≤3 s）。
    info!(
        elapsed_ms = %process_start.elapsed().as_millis(),
        "系统托盘已注册"
    );
    Ok(())
}
