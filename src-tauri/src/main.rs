//! VibeType 桌面端入口（tech/04 4.1）：初始化日志、Tauri Builder、插件与命令注册。
//!
//! 模块实现在库目标（lib.rs），集成测试经库目标复用服务层。
//!
//! 最小权限说明（AGENTS.md 3.5）：本进程仅访问麦克风（T-005）、全局快捷键（T-004）、
//! 剪贴板与文本注入（T-007）等系统能力；配置与日志只写入平台标准目录（tech/08 8.1）。

use tauri::{Emitter, Manager};
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;
use vibetype::services;
use vibetype::state::AppState;
use vibetype::{commands, events, platform, tray};

fn main() {
    // NFR-P-08 冷启动埋点起点：进程启动到托盘可见的耗时。
    let process_start = std::time::Instant::now();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // T-007：剪贴板（粘贴路径备份 / 写入 / 恢复）与系统通知（无焦点兜底）。
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .on_window_event(|window, event| {
            // 关闭设置窗 / 小窗 = 隐藏到托盘，不退出应用（PRD 3.15、T-009）。
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let label = window.label();
                if label == "main" || label == "mini" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(move |app| {
            init_logging(app)?;
            let state = tauri::async_runtime::block_on(init_services(app, process_start))?;
            app.manage(state);
            tracing::info!("VibeType 后端初始化完成");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::system::ping,
            commands::config::get_config,
            commands::config::set_config,
            commands::hotkey::trigger_test_hotkey,
            commands::audio::list_audio_devices,
            commands::inject::inject_text,
            commands::audio::control_recording,
            commands::window::show_mini_window,
            commands::window::show_settings_window,
            // T-008 悬浮球窗口管理。
            commands::window::show_float,
            commands::window::hide_float,
            commands::window::move_float,
            commands::about::get_app_info,
            commands::about::check_update,
            commands::about::apply_update,
            commands::about::export_logs,
            commands::about::get_legal_text,
            // T-006 模型管理。
            commands::asr::list_models,
            commands::asr::download_model,
            commands::asr::cancel_download,
            commands::asr::delete_model,
            commands::asr::set_active_model,
            commands::asr::asr_recognize_wav,
            // T-011 替换词典。
            commands::dict::dict_list_rules,
            commands::dict::dict_add_rule,
            commands::dict::dict_update_rule,
            commands::dict::dict_delete_rule,
            commands::dict::dict_toggle_rule,
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
async fn init_services(
    app: &mut tauri::App,
    process_start: std::time::Instant,
) -> Result<AppState, Box<dyn std::error::Error>> {
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

    // 录音控制通道（tech/04 4.2，容量 8）：快捷键 → AudioService。
    let (record_tx, record_rx) = tokio::sync::mpsc::channel(8);

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

    // T-005 音频采集：actor 消费录音控制命令；预热开启失败自动重试（NFR-R-03）。
    let audio_backend = platform::audio::default_backend();
    let audio = std::sync::Arc::new(services::audio::AudioService::new(
        audio_backend.clone(),
        config.clone(),
        event_tx.clone(),
    ));
    audio.start(
        record_rx,
        event_tx.subscribe(),
        services::audio::REOPEN_RETRY,
    );

    // T-007 文本上屏：enigo 注入器 + Tauri 通知；三级降级链（服务层编排）。
    let injector = platform::inject::default_injector(app.handle().clone())?;
    let notifier = platform::notify::TauriNotifier::new(app.handle().clone());
    let inject_service = services::inject::InjectService::new(injector, notifier, config.clone());

    // T-006 本地 ASR：模型目录在数据目录 models/（NFR-P-09 不进安装包）。
    let models_dir = data_dir.join("models");
    std::fs::create_dir_all(&models_dir)?;
    let asr = std::sync::Arc::new(services::asr::AsrService::new(
        models_dir,
        config.clone(),
        event_tx.clone(),
    ));
    asr.clone().start(event_tx.subscribe());

    // T-011 替换词典 + 后处理管线：词典文件 dict/replace.json（数据目录）。
    let dict = services::dict::DictService::load(&data_dir, event_tx.clone())?;
    if dict.take_recovered().await {
        // 词典损坏回退空词典：提示但不阻塞启动（NFR-S-07 雏形）。
        let _ = event_tx.send(events::AppEvent::AsrNotice {
            code: "E-CONFIG-002".to_string(),
            message: "替换词典文件损坏，已回退为空词典，请重新添加规则".to_string(),
        });
    }
    let post_process =
        services::post_process::PostProcess::new(dict.clone(), config.clone(), event_tx.clone());
    post_process.clone().start(event_tx.subscribe());

    // T-015 系统托盘：双态图标 + 菜单 + 左键唤起小窗 + 录音状态联动。
    tray::setup_tray(app, event_tx.subscribe(), process_start)?;

    // T-008 悬浮球：录音进入 Recording 时显示（配置开启前提下），不抢焦点。
    vibetype::windows::spawn_float_manager(
        app.handle().clone(),
        config.clone(),
        event_tx.subscribe(),
    );

    Ok(AppState::new(
        config,
        record_tx,
        audio_backend,
        inject_service,
        asr,
        dict,
        post_process,
    ))
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
