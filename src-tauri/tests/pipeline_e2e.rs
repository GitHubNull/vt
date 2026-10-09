//! T-012 集成测试：假音频帧驱动全链路（真实识别引擎 + mock 上屏），
//! 断言最终上屏文本与统一结果事件。
//!
//! 链路：AudioService（mock 后端喂真实语音帧）→ AsrService → PostProcess
//! （替换词典）→ PipelineService → InjectService（mock 注入器）。
//! 覆盖 SPEC 验收：词典替换在链路中生效、Stage 关闭时原文直出、
//! 无焦点降级路径真实生效、取消无残留上屏。
//!
//! 模型缓存：`VT_MODELS_CACHE` 或仓库 `tmp/models-cache/`（缺省时跳过）。
//! 需要 feature `test-utils`（平台层 mock）：`cargo test --features test-utils`。
#![allow(clippy::unwrap_used)]
#![cfg(feature = "test-utils")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{broadcast, mpsc};
use vibetype::events::{AppEvent, RecordingResultEvent, RecordingState};
use vibetype::platform::audio::MockAudioBackend;
use vibetype::platform::inject::{FocusState, MockTextInjector};
use vibetype::services::asr::AsrService;
use vibetype::services::audio::AudioService;
use vibetype::services::config_store::{ConfigSection, ConfigStore};
use vibetype::services::dict::DictService;
use vibetype::services::hotkey::RecordCommand;
use vibetype::services::inject::{InjectService, Notifier};
use vibetype::services::pipeline::PipelineService;
use vibetype::services::post_process::PostProcess;
use vibetype::services::session::SessionTracker;

/// 预置模型缓存根目录（与 tests/asr_recognition.rs 同规则）。
fn models_cache_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("VT_MODELS_CACHE") {
        return Some(PathBuf::from(dir));
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cache = manifest.join("../tmp/models-cache");
    cache.is_dir().then_some(cache)
}

fn stage_model(models_dir: &std::path::Path, cache: &std::path::Path) {
    let extracted = cache.join("extracted/sherpa-onnx-paraformer-zh-2023-03-28");
    let target = models_dir.join("paraformer-zh-int8");
    std::fs::create_dir_all(&target).unwrap();
    for (from, to) in [
        ("model.int8.onnx", "model.onnx"),
        ("tokens.txt", "tokens.txt"),
    ] {
        std::fs::hard_link(extracted.join(from), target.join(to))
            .or_else(|_| std::fs::copy(extracted.join(from), target.join(to)).map(|_| ()))
            .unwrap();
    }
}

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vt-pipeline-it-{name}-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 记录通知的 mock（F-INPUT-09 断言）。
struct RecordingNotifier {
    notifications: Mutex<Vec<(String, String)>>,
}

impl Notifier for RecordingNotifier {
    fn notify(&self, title: &str, body: &str) {
        self.notifications
            .lock()
            .unwrap()
            .push((title.to_string(), body.to_string()));
    }
}

/// 全链路测试环境：与 main.rs init_services 的组装顺序一致。
struct Chain {
    backend: Arc<MockAudioBackend>,
    injector: Arc<MockTextInjector>,
    notifier: Arc<RecordingNotifier>,
    config: ConfigStore,
    dict: DictService,
    cmd_tx: mpsc::Sender<RecordCommand>,
    events_rx: broadcast::Receiver<AppEvent>,
    #[allow(dead_code)]
    asr: Arc<AsrService>,
    #[allow(dead_code)]
    pipeline: Arc<PipelineService>,
}

async fn setup_chain(name: &str, focus: FocusState) -> Chain {
    let cache = models_cache_dir().expect("需要模型缓存（tmp/models-cache 或 VT_MODELS_CACHE）");
    let dir = test_dir(name);
    let models_dir = dir.join("models");
    std::fs::create_dir_all(&models_dir).unwrap();
    stage_model(&models_dir, &cache);

    let (events_tx, mut events_rx) = vibetype::events::channel();
    let config = ConfigStore::load(&dir, events_tx.clone()).unwrap();
    let tracker = Arc::new(SessionTracker::new());

    // 常驻预热（与生产默认一致）：等待 Warming 后帧有落点。
    let (record_tx, record_rx) = mpsc::channel(8);
    let backend = MockAudioBackend::with_default_device();
    let audio = Arc::new(AudioService::new(
        backend.clone(),
        config.clone(),
        events_tx.clone(),
        tracker.clone(),
    ));
    audio.start(record_rx, events_tx.subscribe(), Duration::from_millis(20));

    let asr = Arc::new(AsrService::new(
        models_dir,
        config.clone(),
        events_tx.clone(),
        tracker.clone(),
    ));
    asr.reload_engine().await.expect("引擎加载失败");
    asr.clone().start(events_tx.subscribe());

    let dict = DictService::load(&dir, events_tx.clone()).unwrap();
    let post_process = PostProcess::new(
        dict.clone(),
        config.clone(),
        events_tx.clone(),
        tracker.clone(),
    );
    post_process.clone().start(events_tx.subscribe());

    let injector = Arc::new(MockTextInjector::new());
    injector.set_focus(focus);
    let notifier = Arc::new(RecordingNotifier {
        notifications: Mutex::new(Vec::new()),
    });
    let inject = InjectService::new(injector.clone(), notifier.clone(), config.clone());
    let pipeline = PipelineService::new(inject, events_tx.clone(), tracker.clone());
    pipeline.clone().start(events_tx.subscribe());

    // 等待预热完成（帧通道就绪）。
    wait_state(&mut events_rx, RecordingState::Warming).await;
    Chain {
        backend,
        injector,
        notifier,
        config,
        dict,
        cmd_tx: record_tx,
        events_rx,
        asr,
        pipeline,
    }
}

/// 读取官方测试 WAV（真实语音，16 kHz）作为"假音频帧"的数据源。
fn speech_samples(cache_wav: &str) -> (Vec<f32>, u32) {
    let cache = models_cache_dir().unwrap();
    let path = cache
        .join("extracted/sherpa-onnx-paraformer-zh-2023-03-28/test_wavs")
        .join(cache_wav);
    sherpa_rs::read_audio_file(path.to_string_lossy().as_ref()).expect("读取测试 WAV 失败")
}

async fn wait_state(rx: &mut broadcast::Receiver<AppEvent>, state: RecordingState) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("等待 recording:state 超时")
            .expect("事件通道关闭");
        if let AppEvent::RecordingState { state: s, .. } = event {
            if s == state {
                return;
            }
        }
    }
}

/// 等下一个统一结果事件（带超时）。
async fn next_result(rx: &mut broadcast::Receiver<AppEvent>) -> RecordingResultEvent {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("等待 recording:result 超时")
            .expect("事件通道关闭");
        if let AppEvent::RecordingResult(result) = event {
            return result;
        }
    }
}

/// 录音一轮：开始 → 分块喂入真实语音帧 → 结束，等待统一结果事件。
async fn record_one_round(chain: &mut Chain, samples: &[f32]) -> RecordingResultEvent {
    chain.cmd_tx.send(RecordCommand::Start).await.unwrap();
    wait_state(&mut chain.events_rx, RecordingState::Recording).await;
    // 50 ms 块（16 kHz × 0.05 = 800 样本）分块喂入，模拟真实帧流。
    for chunk in samples.chunks(800) {
        chain.backend.push_frame(chunk.to_vec());
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    chain.cmd_tx.send(RecordCommand::Stop).await.unwrap();
    next_result(&mut chain.events_rx).await
}

#[tokio::test]
async fn dict_replace_applies_in_full_chain() {
    // SPEC 验收：词典替换在链路中生效——识别"感兴趣"被规则改写后上屏。
    let mut chain = setup_chain("dict-on", FocusState::Focused).await;
    // 规则来自 int8 模型对 0.wav 的实际输出（tests/asr_recognition.rs 已验证包含该词）。
    chain
        .dict
        .add_rule("感兴趣".to_string(), "E2E_DICT_MARKER".to_string(), true)
        .await
        .unwrap();
    // 等词典缓存刷新（dict:changed 驱动的异步 refresh）。
    tokio::time::sleep(Duration::from_millis(300)).await;

    let (samples, _) = speech_samples("0.wav");
    let result = record_one_round(&mut chain, &samples).await;
    let text = match result {
        RecordingResultEvent::Ok { text } => text,
        other => panic!("期望 Ok，实际 {other:?}"),
    };
    assert!(
        text.contains("E2E_DICT_MARKER"),
        "词典替换必须在链路中生效，实际: {text}"
    );
    assert!(!text.contains("感兴趣"), "错误词不应残留: {text}");
    // 上屏文本 = 结果文本（默认粘贴主路径：写入剪贴板 + 模拟粘贴）。
    let written = chain.injector.written.lock().unwrap();
    assert_eq!(written.last().map(String::as_str), Some(text.as_str()));
}

#[tokio::test]
async fn dict_disabled_passthrough_original_text() {
    // SPEC 验收：Stage（替换词典）关闭时原文直出（产品原则 3：自动改写可关闭）。
    let mut chain = setup_chain("dict-off", FocusState::Focused).await;
    chain
        .dict
        .add_rule("感兴趣".to_string(), "E2E_DICT_MARKER".to_string(), true)
        .await
        .unwrap();
    chain
        .config
        .set_section(ConfigSection::Dict, serde_json::json!({ "enabled": false }))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let (samples, _) = speech_samples("0.wav");
    let result = record_one_round(&mut chain, &samples).await;
    let text = match result {
        RecordingResultEvent::Ok { text } => text,
        other => panic!("期望 Ok，实际 {other:?}"),
    };
    assert!(
        text.contains("感兴趣"),
        "词典关闭时必须原文直出，实际: {text}"
    );
    assert!(!text.contains("E2E_DICT_MARKER"));
}

#[tokio::test]
async fn unfocused_degraded_path_copies_and_notifies() {
    // SPEC 验收：无焦点时复制 + 通知兜底在链路中真实生效（F-INPUT-09）。
    let mut chain = setup_chain("degraded", FocusState::Unfocused).await;
    let (samples, _) = speech_samples("0.wav");
    let result = record_one_round(&mut chain, &samples).await;
    let (text, reason) = match result {
        RecordingResultEvent::Degraded { reason } => {
            // 降级也带不出文本字段：从剪贴板写入断言最终文本。
            let written = chain.injector.written.lock().unwrap();
            (written.last().cloned().unwrap_or_default(), reason)
        }
        other => panic!("期望 Degraded，实际 {other:?}"),
    };
    assert!(
        reason.contains("剪贴板"),
        "降级原因应说明剪贴板兜底: {reason}"
    );
    assert!(!text.is_empty(), "降级路径仍须交付最终文本到剪贴板");
    // 兜底真实生效：剪贴板 = 最终文本、弹出通知、无键入 / 粘贴。
    assert_eq!(chain.injector.clipboard_content(), text);
    let notifications = chain.notifier.notifications.lock().unwrap();
    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].1.contains("剪贴板"));
    chain.injector.assert_no_input();
}

#[tokio::test]
async fn cancel_during_processing_leaves_no_residue() {
    // SPEC 验收：取消在任何阶段生效，无残留上屏（识别在途竞态，T-010/T-012）。
    let mut chain = setup_chain("cancel", FocusState::Focused).await;
    chain.cmd_tx.send(RecordCommand::Start).await.unwrap();
    wait_state(&mut chain.events_rx, RecordingState::Recording).await;
    let (samples, _) = speech_samples("1.wav");
    for chunk in samples.chunks(800) {
        chain.backend.push_frame(chunk.to_vec());
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    chain.cmd_tx.send(RecordCommand::Stop).await.unwrap();
    // 音频已交付、识别在途（Processing）时取消（T-010 Stop/Cancel 竞态窗口）。
    wait_state(&mut chain.events_rx, RecordingState::Processing).await;
    chain.cmd_tx.send(RecordCommand::Cancel).await.unwrap();
    wait_state(&mut chain.events_rx, RecordingState::Cancelled).await;
    // 无残留上屏：等一段超过推理耗时的窗口，断言无任何注入动作。
    tokio::time::sleep(Duration::from_secs(3)).await;
    chain.injector.assert_no_input();
    assert!(chain.injector.written.lock().unwrap().is_empty());
    // 状态机已回退（Warming：常驻预热），下一次录音可正常开始。
    wait_state(&mut chain.events_rx, RecordingState::Warming).await;
}
