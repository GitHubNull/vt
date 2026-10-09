//! VibeType 桌面端入口（tech/04 4.1）：初始化日志、Tauri Builder、插件与命令注册。
//!
//! 最小权限说明（AGENTS.md 3.5）：本进程仅访问麦克风（T-005）、全局快捷键（T-004）、
//! 剪贴板与文本注入（T-007）等系统能力；配置与日志只写入平台标准目录（tech/08 8.1）。

mod commands;
mod error;
mod events;
mod platform;
mod services;
mod state;

use state::AppState;
use tauri::{Emitter, Manager};
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            init_logging(app)?;
            let state = tauri::async_runtime::block_on(init_services(app))?;
            app.manage(state);
            tracing::info!("VibeType 后端初始化完成");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::system::ping,
            commands::config::get_config,
            commands::config::set_config,
            commands::hotkey::trigger_test_hotkey,
        ])
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

/// 初始化数据目录与核心服务（tech/08 8.1；T-003 / T-004）。
async fn init_services(app: &mut tauri::App) -> Result<AppState, Box<dyn std::error::Error>> {
    // 首次启动创建配置 / 数据 / 日志三个平台标准目录（tech/08 8.1）。
    let config_dir = app.path().app_config_dir()?;
    let data_dir = app.path().app_data_dir()?;
    let log_dir = app.path().app_log_dir()?;
    for dir in [&config_dir, &data_dir, &log_dir] {
        std::fs::create_dir_all(dir)?;
    }
    tracing::info!(
        config_dir = %config_dir.display(),
        data_dir = %data_dir.display(),
        log_dir = %log_dir.display(),
        "数据目录初始化完成"
    );

    // 事件广播通道（tech/04 4.2）：服务 → 前端事件桥。
    let (event_tx, _rx) = events::channel();
    spawn_event_bridge(app.handle().clone(), event_tx.clone());

    // ConfigStore 单例：加载配置（损坏自动回退 .bak / 默认）。
    let config = services::config_store::ConfigStore::load(&config_dir, event_tx.clone())?;

    // 录音控制通道（tech/04 4.2，容量 8）：快捷键 → AudioService（T-005 消费）。
    let (record_tx, record_rx) = tokio::sync::mpsc::channel(8);
    // T-004 联调：临时消费者记录日志验证事件流（T-005 起由 AudioService 接管）。
    spawn_record_command_logger(record_rx);

    // T-004 全局快捷键：注册失败（被占用 / 权限缺失）不致命，错误经事件通知前端。
    let (raw_tx, raw_rx) = tokio::sync::mpsc::channel(8);
    let hotkey_backend = platform::hotkey::default_backend(app.handle().clone(), raw_tx);
    let hotkey = std::sync::Arc::new(services::hotkey::HotkeyService::new(
        hotkey_backend,
        event_tx.clone(),
        record_tx.clone(),
        &config.get().await.hotkey,
    ));
    let hotkey_config = config.get().await.hotkey;
    if let Err(err) = hotkey.apply_config(&hotkey_config).await {
        tracing::warn!(code = %err.code(), "快捷键初始注册失败");
        let _ = event_tx.send(events::AppEvent::HotkeyError {
            code: err.code(),
            message: err.message().to_string(),
        });
    }
    hotkey.start(config.clone(), raw_rx, event_tx.subscribe());

    Ok(AppState::new(config, record_tx))
}

/// 临时联调消费者：把录音控制命令记日志（T-004；T-005 由 AudioService 接管）。
fn spawn_record_command_logger(
    mut rx: tokio::sync::mpsc::Receiver<services::hotkey::RecordCommand>,
) {
    tauri::async_runtime::spawn(async move {
        while let Some(cmd) = rx.recv().await {
            tracing::info!(?cmd, "录音控制命令");
        }
    });
}

/// 事件桥：把服务广播的 AppEvent 转换为 Tauri Event 推给全部窗口（tech/03 3.5）。
fn spawn_event_bridge(app: tauri::AppHandle, tx: tokio::sync::broadcast::Sender<events::AppEvent>) {
    let mut rx = tx.subscribe();
    tauri::async_runtime::spawn(async move {
        while let Ok(event) = rx.recv().await {
            let Some(name) = event.frontend_event_name() else {
                continue; // 内部事件（如 AudioReady）不转发前端
            };
            // 广播通道无订阅者时 emit 失败属正常（如无窗口打开），忽略错误。
            let _ = app.emit(name, &event);
        }
    });
}
