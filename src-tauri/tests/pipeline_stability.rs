//! T-012 稳定性测试（NFR-R-01 雏形）：100 次连续录音循环（假音频帧驱动全链路，
//! mock 上屏），断言无崩溃、状态机每次回到可录音状态、内存增长 ≤20 MB。
//!
//! 需要 feature `test-utils`：`cargo test --features test-utils --test pipeline_stability`。
//! 模型缓存：`VT_MODELS_CACHE` 或仓库 `tmp/models-cache/`（缺省时跳过）。
#![allow(clippy::unwrap_used)]
#![cfg(feature = "test-utils")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc};
use vibetype::events::{AppEvent, RecordingState};
use vibetype::platform::audio::MockAudioBackend;
use vibetype::platform::inject::MockTextInjector;
use vibetype::services::asr::AsrService;
use vibetype::services::audio::AudioService;
use vibetype::services::config_store::ConfigStore;
use vibetype::services::dict::DictService;
use vibetype::services::hotkey::RecordCommand;
use vibetype::services::inject::{InjectService, Notifier};
use vibetype::services::pipeline::PipelineService;
use vibetype::services::post_process::PostProcess;
use vibetype::services::session::SessionTracker;

/// 循环次数（SPEC 验收：100 次连续录音循环）。
const ROUNDS: usize = 100;
/// 内存增长上限（NFR-R-01 雏形 / SPEC 验收：≤20 MB）。
const MAX_RSS_GROWTH_BYTES: u64 = 20 * 1024 * 1024;

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

fn test_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vt-stability-it-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct NullNotifier;

impl Notifier for NullNotifier {
    fn notify(&self, _title: &str, _body: &str) {}
}

/// 进程常驻内存（RSS，字节；Linux /proc）。非 Linux 返回 None（跳过内存断言）。
#[cfg(target_os = "linux")]
fn rss_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let resident_pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(resident_pages * 4096)
}

#[cfg(not(target_os = "linux"))]
fn rss_bytes() -> Option<u64> {
    None
}

async fn wait_state(rx: &mut broadcast::Receiver<AppEvent>, state: RecordingState) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
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

/// 等待一轮会话终态（SessionFinished：上屏完成 / 失败 / 无效丢弃都会广播），
/// 并等状态机实际回退到 Warming（actor 消费 SessionFinished 后的广播）。
///
/// 必须等后者再开始下一轮：SessionFinished 由识别 / 上屏任务广播，actor 可能尚未
/// 消费；此时触发开始会被"Processing 中忽略"规则正确吞掉（真实用户只会看到
/// actor 的状态广播后才开始下一轮，测试对齐该时序）。
async fn wait_finished(rx: &mut broadcast::Receiver<AppEvent>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut finished = false;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("等待 SessionFinished 超时（状态机可能卡在 Processing）")
            .expect("事件通道关闭");
        match event {
            AppEvent::SessionFinished { .. } => finished = true,
            // actor 消费终态后的状态回退广播：此时才开始下一轮。
            AppEvent::RecordingState {
                state: RecordingState::Warming | RecordingState::Idle,
                ..
            } if finished => return,
            _ => {}
        }
    }
}

#[tokio::test]
async fn hundred_rounds_no_leak_state_machine_recovers() {
    let Some(cache) = models_cache_dir() else {
        eprintln!("跳过：未找到模型缓存（tmp/models-cache 或 VT_MODELS_CACHE）");
        return;
    };
    let dir = test_dir();
    let models_dir = dir.join("models");
    std::fs::create_dir_all(&models_dir).unwrap();
    stage_model(&models_dir, &cache);

    // ---- 与 main.rs 一致的全链路组装（mock 上屏 / mock 音频后端） ----
    let (events_tx, mut events_rx) = vibetype::events::channel();
    let config = ConfigStore::load(&dir, events_tx.clone()).unwrap();
    let tracker = Arc::new(SessionTracker::new());
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
    let inject = InjectService::new(injector.clone(), Arc::new(NullNotifier), config.clone());
    let pipeline = PipelineService::new(inject, events_tx.clone(), tracker.clone());
    pipeline.clone().start(events_tx.subscribe());

    wait_state(&mut events_rx, RecordingState::Warming).await;

    // 假音频帧：0.25 s 低幅噪声（16 kHz × 0.25 = 4000 样本），分 50 ms 块喂入。
    let fake_frame: Vec<f32> = (0..4000).map(|i| 0.02 * (i as f32 * 0.1).sin()).collect();

    // 预热 5 轮让引擎 / 分配器稳定后再取内存基线。
    for _ in 0..5 {
        record_tx.send(RecordCommand::Start).await.unwrap();
        wait_state(&mut events_rx, RecordingState::Recording).await;
        for chunk in fake_frame.chunks(800) {
            backend.push_frame(chunk.to_vec());
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        record_tx.send(RecordCommand::Stop).await.unwrap();
        wait_finished(&mut events_rx).await;
    }
    let rss_baseline = rss_bytes();
    if rss_baseline.is_none() {
        eprintln!("提示：非 Linux 环境，跳过内存增长断言（其余断言照常）");
    }

    // ---- 100 次连续录音循环 ----
    let started = std::time::Instant::now();
    for round in 0..ROUNDS {
        record_tx.send(RecordCommand::Start).await.unwrap();
        wait_state(&mut events_rx, RecordingState::Recording).await;
        for chunk in fake_frame.chunks(800) {
            backend.push_frame(chunk.to_vec());
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        record_tx.send(RecordCommand::Stop).await.unwrap();
        // 终态即状态机回退的触发器：每轮必须到达（卡死会在这里超时失败）。
        wait_finished(&mut events_rx).await;
        // 每轮至少触发一次终态后，状态机必须已离开 Processing：
        // 下一轮的 Start 能进入 Recording 即证明（Processing 中开始会被忽略）。
        if round % 25 == 0 {
            eprintln!("  稳定性进度: {round}/{ROUNDS} 轮");
        }
    }
    let elapsed = started.elapsed();
    eprintln!("  100 轮耗时: {elapsed:.1?}");

    // 探针：新的开始被接受 → 状态机确已离开 Processing 回到可录音状态
    // （wait_finished 已保证 actor 消费终态并广播 Warming）。
    record_tx.send(RecordCommand::Start).await.unwrap();
    wait_state(&mut events_rx, RecordingState::Recording).await;
    record_tx.send(RecordCommand::Cancel).await.unwrap();
    wait_state(&mut events_rx, RecordingState::Cancelled).await;

    // 内存增长断言（RSS 终点 - 基线 ≤ 20 MB）。
    if let (Some(baseline), Some(after)) = (rss_baseline, rss_bytes()) {
        let growth = after.saturating_sub(baseline);
        eprintln!(
            "  RSS: baseline={}MB after={}MB growth={}MB",
            baseline / 1024 / 1024,
            after / 1024 / 1024,
            growth / 1024 / 1024
        );
        assert!(
            growth <= MAX_RSS_GROWTH_BYTES,
            "内存增长超限: {growth} 字节 > {MAX_RSS_GROWTH_BYTES} 字节"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
