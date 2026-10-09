//! VibeType 桌面端入口（tech/04 4.1）：初始化日志、Tauri Builder、插件与命令注册。
//!
//! 最小权限说明（AGENTS.md 3.5）：本进程仅访问麦克风（后续任务）、全局快捷键、
//! 剪贴板与文本注入等系统能力；T-001 尚未注册任何系统能力，仅使用窗口与日志目录。

mod commands;
mod error;
mod platform;
mod services;
mod state;

use state::AppState;
use tauri::Manager;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .setup(|app| {
            init_logging(app)?;
            tracing::info!("VibeType 后端初始化完成");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::system::ping])
        .run(tauri::generate_context!())
        .expect("error while running vibetype application");
}

/// 初始化 tracing：按天滚动到平台标准日志目录，保留策略见 tech/08 8.1 / tech/04 4.7。
fn init_logging(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // 平台标准日志目录（tech/08 8.1），如 Linux 的 ~/.local/state/vibetype/logs
    let log_dir = app.path().app_log_dir()?;
    std::fs::create_dir_all(&log_dir)?;

    let (writer, guard) = services::log::LogService::daily_writer(&log_dir);
    // guard 必须存活至进程退出；进程退出时由 OS 回收，未刷新缓冲随进程结束丢弃属可接受行为。
    std::mem::forget(guard);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(writer).with_ansi(false))
        .with(filter)
        .init();

    tracing::info!(log_dir = %log_dir.display(), "日志系统初始化完成");
    Ok(())
}
