//! 核心链路编排（T-012，tech/05 5.1 / 5.5）：识别 → 后处理 → 上屏的端到端串联。
//!
//! 链路：`AudioReady`（AudioService）→ `Recognized`（AsrService）→
//! `PipelineOutput`（PostProcess）→ **PipelineService** → InjectService → 目标应用光标。
//!
//! - **结果事件统一**：上屏成功 → `recording:result { Ok }`；走剪贴板兜底 / 降级
//!   （`InjectReport.degraded`）→ `{ Degraded, reason }`；上屏失败 → `{ Failed, code }`。
//!   悬浮球 / 小窗消费同一事件（tech/05 5.5）；
//! - **上屏前最后检查**：消费 `PipelineOutput` 时按会话代次再次确认（T-010）——
//!   取消 / 新会话使在途结果失效，无效结果不上屏、无反馈泄漏，并广播
//!   `SessionFinished` 让状态机收尾；
//! - **竞态收口**：`SessionFinished` 是会话终态的唯一信号（上屏完成 / ASR 失败 /
//!   无效丢弃都会广播），AudioService 据此从 Processing 回退 Idle / Warming，
//!   Processing 期间的开始命令被忽略（tech/05 5.2）；
//! - **耗时埋点**（NFR-P-03）：录音结束（AudioReady）→ 上屏完成逐段计时
//!   （识别 / 后处理 / 上屏 + 全链总耗时），写日志并写入 [`PipelineTimings`]
//!   供基准脚本输出 P50 / P95（benches/pipeline_bench.rs）。
//!
//! 隐私（NFR-S-02）：识别文本只经事件通道传递与上屏，绝不写入日志；
//! 日志只出现会话号、耗时与降级原因。

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::events::{AppEvent, RecordingResultEvent};
use crate::services::inject::InjectService;
use crate::services::session::SessionTracker;

/// 单会话耗时样本（一段会话的链路分段耗时，毫秒）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimingSample {
    pub session_id: u64,
    /// 识别段：AudioReady → Recognized。
    pub asr_ms: u128,
    /// 后处理段：Recognized → PipelineOutput。
    pub post_ms: u128,
    /// 上屏段：PipelineOutput → 上屏完成。
    pub inject_ms: u128,
    /// 全链总耗时：AudioReady → 上屏完成（NFR-P-03 度量段）。
    pub total_ms: u128,
}

/// 耗时样本环形缓冲（容量封顶，供基准脚本 / 测试读取；隐私安全——只含耗时数字）。
#[derive(Debug, Clone, Default)]
pub struct PipelineTimings {
    samples: Arc<Mutex<VecDeque<TimingSample>>>,
}

/// 缓冲容量上限（远超基准迭代数，防无界增长）。
const TIMINGS_CAP: usize = 4096;

impl PipelineTimings {
    fn push(&self, sample: TimingSample) {
        if let Ok(mut samples) = self.samples.lock() {
            if samples.len() >= TIMINGS_CAP {
                samples.pop_front();
            }
            samples.push_back(sample);
        }
    }

    /// 全部样本快照（基准脚本 percentile 计算用）。
    pub fn snapshot(&self) -> Vec<TimingSample> {
        self.samples
            .lock()
            .map(|samples| samples.iter().copied().collect())
            .unwrap_or_default()
    }
}

/// PipelineService：消费管线出口 → 上屏 → 统一结果反馈 + 会话终态广播。
pub struct PipelineService {
    inject: InjectService,
    events: broadcast::Sender<AppEvent>,
    /// 会话代次追踪（T-010）：上屏前最后检查。
    tracker: Arc<SessionTracker>,
    /// 会话分段计时戳（session_id → 各段起点）。
    stamps: Mutex<HashMap<u64, StageStamps>>,
    /// 耗时样本（基准 / 测试读取）。
    timings: PipelineTimings,
}

/// 一个会话在链路各段起点的计时戳。
#[derive(Debug, Clone, Copy)]
struct StageStamps {
    /// 录音结束（AudioReady）。
    ready_at: Instant,
    /// 识别完成（Recognized）。
    recognized_at: Option<Instant>,
    /// 管线出口（PipelineOutput）。
    output_at: Option<Instant>,
}

impl PipelineService {
    pub fn new(
        inject: InjectService,
        events: broadcast::Sender<AppEvent>,
        tracker: Arc<SessionTracker>,
    ) -> Arc<Self> {
        Arc::new(Self {
            inject,
            events,
            tracker,
            stamps: Mutex::new(HashMap::new()),
            timings: PipelineTimings::default(),
        })
    }

    /// 耗时样本句柄（基准脚本 / 测试断言）。
    pub fn timings(&self) -> PipelineTimings {
        self.timings.clone()
    }

    /// 启动链路编排循环：消费 AudioReady / Recognized（计时戳）与
    /// PipelineOutput（上屏编排）；同时清理历史会话的残留计时戳。
    pub fn start(self: Arc<Self>, mut events_rx: broadcast::Receiver<AppEvent>) {
        tokio::spawn(async move {
            loop {
                match events_rx.recv().await {
                    Ok(AppEvent::AudioReady { session_id, .. }) => {
                        // 新会话音频到达：历史上一个会话必已终态，清理其残留计时戳
                        //（会话严格串行：Processing 期间的开始命令被 AudioService 忽略）。
                        if let Ok(mut stamps) = self.stamps.lock() {
                            stamps.retain(|&sid, _| sid >= session_id);
                            stamps.insert(
                                session_id,
                                StageStamps {
                                    ready_at: Instant::now(),
                                    recognized_at: None,
                                    output_at: None,
                                },
                            );
                        }
                    }
                    Ok(AppEvent::Recognized { session_id, .. }) => {
                        if let Ok(mut stamps) = self.stamps.lock() {
                            if let Some(entry) = stamps.get_mut(&session_id) {
                                entry.recognized_at = Some(Instant::now());
                            }
                        }
                    }
                    Ok(AppEvent::PipelineOutput { session_id, text }) => {
                        let output_at = Instant::now();
                        if let Ok(mut stamps) = self.stamps.lock() {
                            if let Some(entry) = stamps.get_mut(&session_id) {
                                entry.output_at = Some(output_at);
                            }
                        }
                        self.deliver(session_id, text).await;
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });
    }

    /// 管线出口 → 上屏 → 统一结果反馈 + 会话终态广播。
    async fn deliver(&self, session_id: u64, text: String) {
        // T-010 上屏前最后检查：取消 / 过期会话的结果在此截断——
        // 宁可漏转不可错转（AGENTS.md 3.2 原则 4）。
        if !self.tracker.is_valid(session_id) {
            info!(session_id, "会话已取消或过期，上屏前最后检查丢弃");
            self.finish_session(session_id);
            return;
        }
        let inject_started = Instant::now();
        let outcome = self.inject.inject_text(&text).await;
        let inject_ms = inject_started.elapsed().as_millis();
        match outcome {
            Ok(report) => {
                if let Some(reason) = report.degraded {
                    // 降级完成（无焦点兜底等）：文本已入剪贴板 + 已通知（F-INPUT-09）。
                    info!(session_id, %reason, "上屏降级完成");
                    let _ = self.events.send(AppEvent::RecordingResult(
                        RecordingResultEvent::Degraded { reason },
                    ));
                } else {
                    info!(session_id, "上屏成功");
                    let _ = self
                        .events
                        .send(AppEvent::RecordingResult(RecordingResultEvent::Ok { text }));
                }
            }
            Err(err) => {
                warn!(session_id, code = %err.code(), "上屏失败");
                let _ = self
                    .events
                    .send(AppEvent::RecordingResult(RecordingResultEvent::Failed {
                        code: err.code(),
                        message: err.message().to_string(),
                    }));
            }
        }
        self.log_breakdown(session_id, inject_ms);
        self.finish_session(session_id);
    }

    /// 广播会话终态（AudioService 消费后状态机回 Idle / Warming）。
    fn finish_session(&self, session_id: u64) {
        let _ = self.events.send(AppEvent::SessionFinished { session_id });
        if let Ok(mut stamps) = self.stamps.lock() {
            stamps.remove(&session_id);
        }
    }

    /// NFR-P-03 耗时分解：录音结束 → 上屏完成各段耗时写日志 + 样本入缓冲。
    fn log_breakdown(&self, session_id: u64, inject_ms: u128) {
        let Ok(stamps) = self.stamps.lock() else {
            return;
        };
        let Some(entry) = stamps.get(&session_id) else {
            return;
        };
        let asr_ms = entry
            .recognized_at
            .map(|at| at.duration_since(entry.ready_at).as_millis())
            .unwrap_or(0);
        let post_ms = match (entry.recognized_at, entry.output_at) {
            (Some(from), Some(to)) => to.duration_since(from).as_millis(),
            _ => 0,
        };
        let total_ms = entry.ready_at.elapsed().as_millis();
        // 只出现耗时数字，不出现任何文本内容（NFR-S-02）。
        info!(
            session_id,
            asr_ms, post_ms, inject_ms, total_ms, "上屏链路耗时分解（录音结束→上屏完成）"
        );
        self.timings.push(TimingSample {
            session_id,
            asr_ms,
            post_ms,
            inject_ms,
            total_ms,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::RecordingResultEvent;
    use crate::platform::inject::{FocusState, MockTextInjector};
    use crate::services::config_store::ConfigStore;

    /// 链路测试环境：mock 注入器 + 临时配置 + 事件订阅。
    struct Env {
        service: Arc<PipelineService>,
        injector: Arc<MockTextInjector>,
        events_rx: broadcast::Receiver<AppEvent>,
        #[allow(dead_code)]
        tracker: Arc<SessionTracker>,
    }

    async fn setup(focus: FocusState) -> Env {
        let dir = std::env::temp_dir().join(format!(
            "vt-pipeline-test-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (events_tx, events_rx) = crate::events::channel();
        let config = ConfigStore::load(&dir, events_tx.clone()).unwrap();
        let injector = Arc::new(MockTextInjector::new());
        injector.set_focus(focus);
        struct NullNotifier;
        impl crate::services::inject::Notifier for NullNotifier {
            fn notify(&self, _title: &str, _body: &str) {}
        }
        let inject = InjectService::new(injector.clone(), Arc::new(NullNotifier), config);
        let tracker = Arc::new(SessionTracker::new());
        let service = PipelineService::new(inject, events_tx, tracker.clone());
        service.clone().start(service.events.subscribe());
        Env {
            service,
            injector,
            events_rx,
            tracker,
        }
    }

    async fn next_result(rx: &mut broadcast::Receiver<AppEvent>) -> RecordingResultEvent {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let event = tokio::time::timeout(remaining, rx.recv())
                .await
                .expect("等待 recording:result 超时")
                .expect("事件通道关闭");
            if let AppEvent::RecordingResult(result) = event {
                return result;
            }
        }
    }

    #[tokio::test]
    async fn ok_path_injects_and_reports_ok() {
        let mut env = setup(FocusState::Focused).await;
        env.service.tracker.begin(1);
        // 真实链路顺序：AudioReady（计时起点）→ PipelineOutput（上屏）。
        let _ = env.service.events.send(AppEvent::AudioReady {
            session_id: 1,
            sample_rate: 16_000,
            samples: Arc::new(Vec::new()),
        });
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let _ = env.service.events.send(AppEvent::PipelineOutput {
            session_id: 1,
            text: "你好世界".to_string(),
        });
        match next_result(&mut env.events_rx).await {
            RecordingResultEvent::Ok { text } => assert_eq!(text, "你好世界"),
            other => panic!("期望 Ok，实际 {other:?}"),
        }
        // 默认粘贴主路径：文本经剪贴板写入 + 模拟粘贴。
        assert_eq!(env.injector.written.lock().unwrap().len(), 1);
        assert_eq!(env.injector.pastes.lock().unwrap().len(), 1);
        // 会话终态已广播（AudioService 据此回 Idle）。
        loop {
            if let AppEvent::SessionFinished { session_id } = env.events_rx.recv().await.unwrap() {
                assert_eq!(session_id, 1);
                break;
            }
        }
        // 耗时样本已记录（分段分解）。
        let samples = env.service.timings().snapshot();
        assert_eq!(samples.len(), 1);
        assert!(samples[0].inject_ms > 0 || samples[0].total_ms > 0);
    }

    #[tokio::test]
    async fn degraded_path_reports_degraded_reason() {
        let mut env = setup(FocusState::Unfocused).await;
        env.service.tracker.begin(2);
        let _ = env.service.events.send(AppEvent::PipelineOutput {
            session_id: 2,
            text: "无焦点文本".to_string(),
        });
        match next_result(&mut env.events_rx).await {
            RecordingResultEvent::Degraded { reason } => {
                assert!(
                    reason.contains("剪贴板"),
                    "降级原因应说明剪贴板兜底: {reason}"
                );
            }
            other => panic!("期望 Degraded，实际 {other:?}"),
        }
        // 兜底真实生效：文本入剪贴板、无键入 / 粘贴。
        assert_eq!(env.injector.clipboard_content(), "无焦点文本");
        env.injector.assert_no_input();
    }

    #[tokio::test]
    async fn invalid_session_dropped_before_inject() {
        let mut env = setup(FocusState::Focused).await;
        // 会话 9 未 begin（等价于已取消 / 已过期）：上屏前最后检查丢弃。
        assert!(!env.service.tracker.is_valid(9));
        let _ = env.service.events.send(AppEvent::PipelineOutput {
            session_id: 9,
            text: "不应上屏".to_string(),
        });
        // 无任何注入动作、无结果反馈（无泄漏）；只有终态广播。
        loop {
            match env.events_rx.recv().await.unwrap() {
                AppEvent::SessionFinished { session_id } => {
                    assert_eq!(session_id, 9);
                    break;
                }
                AppEvent::RecordingResult(_) => {
                    panic!("无效会话不得产生结果反馈")
                }
                _ => {}
            }
        }
        env.injector.assert_no_input();
        assert!(env.injector.written.lock().unwrap().is_empty());
    }
}
