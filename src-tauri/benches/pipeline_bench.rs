//! T-012 核心链路基准（tech/05 5.4）：固定音频样本驱动全链路
//! （AudioReady → AsrService → PostProcess → PipelineService → mock 上屏），
//! 输出 NFR-P-03 度量段（录音结束 → 上屏完成）的 P50 / P95。
//!
//! 运行：`cargo bench --features test-utils`（或 `cargo build --release --bench
//! pipeline_bench --features test-utils` 后直接运行二进制）。
//! 模型缓存：`VT_MODELS_CACHE` 或仓库 `tmp/models-cache/`（缺省时输出 SKIP 退出）。
//!
//! 说明：本脚本测量的是"识别 + 后处理 + 上屏编排"段（不含音频采集与设备打开，
//! NFR-P-01/02 需真实麦克风，基准机人工实测）；上屏用 mock 注入器隔离
//! GUI / 目标应用不确定性——NFR-P-03 预算中的"上屏 ≤150 ms"需真机补测。

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use vibetype::events::{AppEvent, RecordingResultEvent};
use vibetype::platform::inject::MockTextInjector;
use vibetype::services::asr::AsrService;
use vibetype::services::config_store::ConfigStore;
use vibetype::services::dict::DictService;
use vibetype::services::inject::{InjectService, Notifier};
use vibetype::services::pipeline::{PipelineService, TimingSample};
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

/// 把缓存中的 int8 模型文件硬链接（同盘零拷贝）到临时 models/<id>/ 目录。
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
        "vt-pipeline-bench-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 空通知器（基准不产生系统通知副作用）。
struct NullNotifier;

impl Notifier for NullNotifier {
    fn notify(&self, _title: &str, _body: &str) {}
}

/// nearest-rank 百分位（p ∈ (0, 1]）。
fn percentile(sorted: &[u128], p: f64) -> u128 {
    assert!(!sorted.is_empty());
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn report(label: &str, samples: &[TimingSample], nfr: &str, budget_ms: u128) {
    if samples.is_empty() {
        println!("{label}: 无样本");
        return;
    }
    let mut totals: Vec<u128> = samples.iter().map(|s| s.total_ms).collect();
    let mut asr: Vec<u128> = samples.iter().map(|s| s.asr_ms).collect();
    let mut post: Vec<u128> = samples.iter().map(|s| s.post_ms).collect();
    let mut inject: Vec<u128> = samples.iter().map(|s| s.inject_ms).collect();
    for values in [&mut totals, &mut asr, &mut post, &mut inject] {
        values.sort_unstable();
    }
    println!(
        "{label}: n={} | 全链(录音结束→上屏完成) P50={}ms P95={}ms | 识别 P50={}ms P95={}ms | 后处理 P50={}ms | 上屏编排 P50={}ms",
        samples.len(),
        percentile(&totals, 0.50),
        percentile(&totals, 0.95),
        percentile(&asr, 0.50),
        percentile(&asr, 0.95),
        percentile(&post, 0.50),
        percentile(&inject, 0.50),
    );
    // NFR-P-03：≤10 s 语音 P95 上屏 ≤800 ms；NFR-P-04：60 s 语音 P95 ≤2 s
    // （tech/05 5.4 预算：识别 ≤600/1700 ms + 后处理 ≤50 ms + 上屏 ≤150 ms）。
    let p95 = percentile(&totals, 0.95);
    println!(
        "{label}: {nfr} 判定(P95 ≤{budget_ms}ms): {} —— 注意：本机非基准机，mock 上屏不含真实目标应用耗时，数据仅供参考，发布验收以基准机实测为准",
        if p95 <= budget_ms { "PASS" } else { "FAIL(需基准机复核)" }
    );
}

/// 驱动一次完整链路：begin → AudioReady → 等待统一结果事件（Ok / Degraded / Failed）。
async fn run_once(
    events_tx: &tokio::sync::broadcast::Sender<AppEvent>,
    events_rx: &mut tokio::sync::broadcast::Receiver<AppEvent>,
    tracker: &SessionTracker,
    session_id: u64,
    samples: Arc<Vec<f32>>,
    sample_rate: u32,
) -> RecordingResultEvent {
    tracker.begin(session_id);
    let _ = events_tx.send(AppEvent::AudioReady {
        session_id,
        sample_rate,
        samples,
    });
    let outcome = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if let AppEvent::RecordingResult(result) = events_rx.recv().await.unwrap() {
                return result;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("会话 {session_id} 等待结果超时"));
    // 等会话终态广播，保证样本完整入缓冲（SessionFinished 在结果事件之后发送）。
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let AppEvent::SessionFinished { .. } = events_rx.recv().await.unwrap() {
                return;
            }
        }
    })
    .await
    .expect("等待 SessionFinished 超时");
    outcome
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let Some(cache) = models_cache_dir() else {
        println!("SKIP: 未找到模型缓存（tmp/models-cache 或 VT_MODELS_CACHE）");
        return;
    };
    let wav_dir = cache.join("extracted/sherpa-onnx-paraformer-zh-2023-03-28/test_wavs");
    let dir = test_dir();
    let models_dir = dir.join("models");
    std::fs::create_dir_all(&models_dir).unwrap();
    stage_model(&models_dir, &cache);

    // ---- 组装与生产一致的全链路（mock 上屏隔离 GUI） ----
    let (events_tx, mut events_rx) = vibetype::events::channel();
    let config = ConfigStore::load(&dir, events_tx.clone()).unwrap();
    let tracker = Arc::new(SessionTracker::new());
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
    let inject = InjectService::new(injector.clone(), Arc::new(NullNotifier), config.clone());
    let pipeline = PipelineService::new(inject, events_tx.clone(), tracker.clone());
    pipeline.clone().start(events_tx.subscribe());
    let timings = pipeline.timings();

    // ---- 样本 1：≤10 s 语音（NFR-P-03） ----
    let wav_path = wav_dir.join("0.wav").to_string_lossy().into_owned();
    let (wav_samples, wav_rate) = tokio::task::spawn_blocking(move || {
        sherpa_rs::read_audio_file(&wav_path).expect("读取基准 WAV 失败")
    })
    .await
    .unwrap();
    let seconds = wav_samples.len() as f64 / wav_rate as f64;
    println!(
        "基准样本: test_wavs/0.wav ≈ {:.1}s @ {}Hz；迭代次数可用 VT_BENCH_ITERS 覆盖（默认 20）",
        seconds, wav_rate
    );
    let iters: u64 = std::env::var("VT_BENCH_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let mut oks = 0usize;
    for i in 0..iters {
        let result = run_once(
            &events_tx,
            &mut events_rx,
            &tracker,
            i + 1,
            Arc::new(wav_samples.clone()),
            wav_rate,
        )
        .await;
        match result {
            RecordingResultEvent::Ok { .. } => oks += 1,
            RecordingResultEvent::Degraded { reason } => {
                println!("  迭代 {}: 降级完成（{}）", i + 1, reason)
            }
            RecordingResultEvent::Failed { code, .. } => {
                println!("  迭代 {}: 失败（{}）", i + 1, code)
            }
        }
    }
    println!("  成功 {} / {} 次", oks, iters);
    report("NFR-P-03(≤10s 语音)", &timings.snapshot(), "NFR-P-03", 800);

    // ---- 样本 2：60 s 语音（NFR-P-04，3 次迭代的参考值） ----
    if std::env::var("VT_BENCH_LONG").is_ok() {
        let mut long = wav_samples.repeat(11); // ≈ 60 s
        long.truncate(60 * wav_rate as usize);
        timings.snapshot(); // 清空旧样本：snapshot 只读，直接用新分段
        let before = timings.snapshot().len();
        for i in 0..3 {
            run_once(
                &events_tx,
                &mut events_rx,
                &tracker,
                1000 + i + 1,
                Arc::new(long.clone()),
                wav_rate,
            )
            .await;
        }
        let all = timings.snapshot();
        report("NFR-P-04(60s 语音, 参考)", &all[before..], "NFR-P-04", 2000);
    }

    let _ = std::fs::remove_dir_all(&dir);
}
