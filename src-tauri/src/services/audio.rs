//! AudioService（tech/04 4.4、tech/05 5.3.2）：actor 模式的音频采集服务。
//!
//! - 状态机 `Idle / Warming / Recording`，迁移经 `recording:state` 广播（tech/05 5.2）；
//! - 常驻预热（F-SET-03 可配，默认开）：启动时按配置打开设备流；**非录音状态采样
//!   直接丢弃**——代码路径上无缓冲、无写盘（NFR-PR-07，见 `handle_frame`）；
//! - 录音会话：Start 起缓冲帧（容量 = 时长上限 × 实际采样率，上限 5 分钟可配 1–10），
//!   Stop 打包经内部事件 `AudioReady` 交给 AsrService（T-006 消费），Cancel 丢弃；
//! - 音量电平：回调内 RMS 计算，50 ms 节流广播 `recording:level`（tech/05 5.3.2）；
//! - 设备异常（拔出 / 打开失败）：状态回退 Idle / Warming + `recording:result`
//!   失败事件带错误编号（NFR-R-03），指定设备断开自动回退系统默认并提示；
//! - 性能埋点：Start 到首帧耗时记日志（验证 NFR-P-01 ≤150 ms / NFR-P-02 ≤500 ms）。
//!
//! 隐私（NFR-PR-02 / NFR-S-02）：音频帧只在内存缓冲，绝不写盘；
//! 日志只出现状态、耗时与设备名，不出现任何音频内容。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};

use crate::error::AppError;
use crate::events::{AppEvent, RecordingResultEvent, RecordingState};
use crate::platform::audio::{AudioBackend, AudioChunk, AudioError, InputStream};
use crate::services::config_store::{ConfigSection, ConfigStore};
use crate::services::hotkey::RecordCommand;

/// 错误编号：打开输入设备 / 流失败（NFR-R-03 明确提示）。
pub const ERR_OPEN_STREAM: u32 = 1;
/// 错误编号：采集流运行中错误（设备拔出、独占失败等）。
pub const ERR_STREAM_FAILURE: u32 = 2;

/// 帧通道容量（tech/04 4.2：约 2 秒缓冲 @16 kHz / 50 ms 块）。
const FRAME_CHANNEL_CAPACITY: usize = 64;
/// 电平广播节流间隔（tech/05 5.3.2）。
const LEVEL_THROTTLE: Duration = Duration::from_millis(50);
/// 录音时长上限检查与热插拔重试 tick 间隔。
const TICK_INTERVAL: Duration = Duration::from_millis(100);
/// 默认热插拔重试间隔（main.rs 传入；测试注入更短间隔）。
pub const REOPEN_RETRY: Duration = Duration::from_secs(2);

/// RMS 音量电平（归一化 0.0~1.0）。
pub fn rms_level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum();
    (sum / samples.len() as f64).sqrt().min(1.0) as f32
}

/// 录音会话：内存缓冲（NFR-PR-02：识别后即弃，绝不写盘）。
struct RecordingSession {
    id: u64,
    samples: Vec<f32>,
    /// 缓冲硬上限（样本数）：达到后截断，时长上限由 actor 定时器保证。
    capacity: usize,
    /// 设备实际采样率（随 AudioReady 交付）。
    sample_rate: u32,
    started: Instant,
}

impl RecordingSession {
    fn new(id: u64, capacity: usize, sample_rate: u32) -> Self {
        Self {
            id,
            samples: Vec::with_capacity(capacity.min(16000 * 10)),
            capacity,
            sample_rate,
            started: Instant::now(),
        }
    }

    /// 追加帧；超出容量截断（防御性硬上限）。
    fn push(&mut self, chunk: &[f32]) {
        let room = self.capacity.saturating_sub(self.samples.len());
        self.samples
            .extend_from_slice(&chunk[..chunk.len().min(room)]);
    }

    /// 是否到达时长上限（PRD 3.1.1 规则 5：到达上限自动结束）。
    fn cap_reached(&self, max_duration: Duration) -> bool {
        self.started.elapsed() >= max_duration
    }
}

/// 已打开的输入流与其帧 / 错误通道（通道从结构体拆出便于 select! 分支分别借用）。
struct ActiveStream {
    stream: Box<dyn InputStream>,
    frames: mpsc::Receiver<AudioChunk>,
    errors: mpsc::Receiver<AudioError>,
}

impl ActiveStream {
    fn split(
        self,
    ) -> (
        Box<dyn InputStream>,
        mpsc::Receiver<AudioChunk>,
        mpsc::Receiver<AudioError>,
    ) {
        (self.stream, self.frames, self.errors)
    }
}

/// 采集统计（可观测性 + NFR-PR-07 丢弃路径断言）。
#[derive(Default)]
pub struct AudioStats {
    /// 预热等非录音状态丢弃的样本数（NFR-PR-07）。
    pub dropped_samples: AtomicUsize,
}

/// AudioService actor。事件循环在 `start()` 中运行，状态私有、消息驱动。
pub struct AudioService {
    backend: Arc<dyn AudioBackend>,
    config: ConfigStore,
    events: broadcast::Sender<AppEvent>,
    stats: Arc<AudioStats>,
}

impl AudioService {
    pub fn new(
        backend: Arc<dyn AudioBackend>,
        config: ConfigStore,
        events: broadcast::Sender<AppEvent>,
    ) -> Self {
        Self {
            backend,
            config,
            events,
            stats: Arc::new(AudioStats::default()),
        }
    }

    /// 采集统计句柄（测试断言 NFR-PR-07 丢弃路径）。
    #[cfg(test)]
    pub fn stats(&self) -> Arc<AudioStats> {
        self.stats.clone()
    }

    /// 启动 actor：消费 RecordCommand；`reopen_retry` 为热插拔重试间隔。
    pub fn start(
        self: Arc<Self>,
        mut cmd_rx: mpsc::Receiver<RecordCommand>,
        mut config_rx: broadcast::Receiver<AppEvent>,
        reopen_retry: Duration,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut state = RecordingState::Idle;
            // 流句柄与帧 / 错误通道拆分为独立变量：select! 各分支分别借用，互不冲突。
            let mut stream_handle: Option<Box<dyn InputStream>> = None;
            let mut frames_rx: Option<mpsc::Receiver<AudioChunk>> = None;
            let mut errors_rx: Option<mpsc::Receiver<AudioError>> = None;
            let has_stream = |handle: &Option<Box<dyn InputStream>>| handle.is_some();
            let mut session: Option<RecordingSession> = None;
            let mut next_session_id = 1u64;
            // 首次触发到首帧的埋点（NFR-P-01 / NFR-P-02）。
            let mut start_instant: Option<Instant> = None;
            let mut last_level_emit = Instant::now() - LEVEL_THROTTLE;
            let mut reopen_at: Option<Instant> = None;
            let mut pending_device_update = false;

            let app_config = self.config.get().await;
            let mut keep_warm = app_config.audio.keep_warm;
            // 常驻预热：启动即打开设备流（NFR-P-01：按下即录，首帧即采）。
            if keep_warm {
                match self.open_stream().await {
                    Ok(active) => {
                        let (stream, frames, errors) = active.split();
                        state = RecordingState::Warming;
                        let _ = self
                            .events
                            .send(AppEvent::RecordingState { state, mode: None });
                        stream_handle = Some(stream);
                        frames_rx = Some(frames);
                        errors_rx = Some(errors);
                        info!("麦克风常驻预热已开启（非录音状态采样即时丢弃，NFR-PR-07）");
                    }
                    Err(err) => {
                        warn!(code = %err.code(), "预热打开麦克风失败");
                        let _ = self.events.send(AppEvent::RecordingResult(
                            RecordingResultEvent::Failed {
                                code: err.code(),
                                message: err.message().to_string(),
                            },
                        ));
                        reopen_at = Some(Instant::now() + reopen_retry);
                    }
                }
            }

            let mut tick = tokio::time::interval(TICK_INTERVAL);
            loop {
                tokio::select! {
                    command = cmd_rx.recv() => {
                        let Some(command) = command else { break };
                        let want_start = matches!(command, RecordCommand::Start)
                            || (matches!(command, RecordCommand::Toggle)
                                && state != RecordingState::Recording);
                        let want_stop = matches!(command, RecordCommand::Stop)
                            || (matches!(command, RecordCommand::Toggle)
                                && state == RecordingState::Recording);
                        if want_start {
                            // 同一时刻只有一个录音会话（tech/05 5.2）。
                            if state == RecordingState::Recording {
                                continue;
                            }
                            if !has_stream(&stream_handle) {
                                match self.open_stream().await {
                                    Ok(active) => {
                                        let (stream, frames, errors) = active.split();
                                        stream_handle = Some(stream);
                                        frames_rx = Some(frames);
                                        errors_rx = Some(errors);
                                    }
                                    Err(err) => {
                                        warn!(code = %err.code(), "打开麦克风失败");
                                        let _ = self.events.send(AppEvent::RecordingResult(
                                            RecordingResultEvent::Failed {
                                                code: err.code(),
                                                message: err.message().to_string(),
                                            },
                                        ));
                                        continue;
                                    }
                                }
                            }
                            let sample_rate = stream_handle
                                .as_ref()
                                .map(|stream| stream.sample_rate())
                                .unwrap_or(crate::platform::audio::TARGET_SAMPLE_RATE);
                            let max_minutes =
                                self.config.get().await.hotkey.max_recording_minutes;
                            let capacity = sample_rate as usize * 60 * max_minutes as usize;
                            session = Some(RecordingSession::new(
                                next_session_id,
                                capacity,
                                sample_rate,
                            ));
                            next_session_id += 1;
                            start_instant = Some(Instant::now());
                            state = RecordingState::Recording;
                            let mode = Some(self.config.get().await.hotkey.mode);
                            let _ = self.events.send(AppEvent::RecordingState { state, mode });
                            info!("录音会话开始");
                        } else if want_stop {
                            if state != RecordingState::Recording {
                                continue;
                            }
                            // 帧经独立通道到达：结束前排空已到达的帧，避免命令与帧
                            // 在 select! 中乱序导致尾帧丢失。
                            Self::drain_pending_frames(
                                frames_rx.as_mut(),
                                &self.events,
                                &self.stats,
                                &mut session,
                                &mut last_level_emit,
                            );
                            Self::finish_session(&self.events, &mut session, &mut state, keep_warm);
                            if !keep_warm {
                                // 非预热：结束即关流，释放设备。
                                stream_handle = None;
                                frames_rx = None;
                                errors_rx = None;
                            }
                            start_instant = None;
                        } else if matches!(command, RecordCommand::Cancel) {
                            if state != RecordingState::Recording {
                                continue;
                            }
                            session = None;
                            start_instant = None;
                            let _ = self.events.send(AppEvent::RecordingState {
                                state: RecordingState::Cancelled,
                                mode: None,
                            });
                            state = if keep_warm {
                                RecordingState::Warming
                            } else {
                                stream_handle = None;
                                frames_rx = None;
                                errors_rx = None;
                                RecordingState::Idle
                            };
                            let _ = self.events.send(AppEvent::RecordingState { state, mode: None });
                            info!("录音会话已取消，音频已丢弃");
                        }
                    }
                    chunk = async {
                        match frames_rx.as_mut() {
                            Some(rx) => rx.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        let Some(chunk) = chunk else { continue };
                        Self::handle_frame(
                            &self.events,
                            &self.stats,
                            &mut session,
                            &mut last_level_emit,
                            &chunk,
                        );
                        if let Some(t0) = start_instant {
                            // NFR-P-01 ≤150 ms（预热）/ NFR-P-02 ≤500 ms（非预热）埋点。
                            info!(elapsed_ms = %t0.elapsed().as_millis(), "录音首帧到达");
                            start_instant = None;
                        }
                    }
                    stream_error = async {
                        match errors_rx.as_mut() {
                            Some(rx) => rx.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        let Some(err) = stream_error else { continue };
                        warn!(error = %err, "麦克风采集流错误");
                        stream_handle = None;
                        reopen_at = Some(Instant::now() + reopen_retry);
                        // 错误前已到达帧通道的帧仍属本会话，照常入缓冲后随部分结果交付。
                        Self::drain_pending_frames(
                            frames_rx.as_mut(),
                            &self.events,
                            &self.stats,
                            &mut session,
                            &mut last_level_emit,
                        );
                        frames_rx = None;
                        errors_rx = None;
                        if let Some(active_session) = session.take() {
                            // 录音中拔出：已采集部分照常交付识别（tech/05 5.6）。
                            Self::deliver_audio(&self.events, active_session);
                        }
                        state = RecordingState::Idle;
                        let _ = self.events.send(AppEvent::RecordingState { state, mode: None });
                        let _ = self.events.send(AppEvent::RecordingResult(
                            RecordingResultEvent::Failed {
                                code: AppError::Audio(ERR_STREAM_FAILURE, String::new()).code(),
                                message: "麦克风异常断开，请检查设备".to_string(),
                            },
                        ));
                    }
                    config_event = config_rx.recv() => {
                        match config_event {
                            Ok(AppEvent::ConfigChanged { section: ConfigSection::Audio }) => {
                                let audio_config = self.config.get().await.audio;
                                let new_keep_warm = audio_config.keep_warm;
                                if state == RecordingState::Recording {
                                    // 录音中不切流：会话结束后应用新设备。
                                    pending_device_update = true;
                                } else {
                                    // 预热开关或设备选择变化：重开流应用新配置。
                                    stream_handle = None;
                                    frames_rx = None;
                                    errors_rx = None;
                                    if new_keep_warm {
                                        match self.open_stream().await {
                                            Ok(active) => {
                                                let (stream, frames, errors) = active.split();
                                                state = RecordingState::Warming;
                                                let _ = self.events.send(AppEvent::RecordingState { state, mode: None });
                                                stream_handle = Some(stream);
                                                frames_rx = Some(frames);
                                                errors_rx = Some(errors);
                                            }
                                            Err(err) => {
                                                state = RecordingState::Idle;
                                                let _ = self.events.send(AppEvent::RecordingResult(
                                                    RecordingResultEvent::Failed {
                                                        code: err.code(),
                                                        message: err.message().to_string(),
                                                    },
                                                ));
                                                reopen_at = Some(Instant::now() + reopen_retry);
                                            }
                                        }
                                    } else {
                                        state = RecordingState::Idle;
                                        let _ = self.events.send(AppEvent::RecordingState { state, mode: None });
                                    }
                                }
                                keep_warm = new_keep_warm;
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                    _ = tick.tick() => {
                        // 时长上限：到达自动结束并交付已采集音频（PRD 3.1.1 规则 5）。
                        if let Some(active_session) = &session {
                            let max_duration = {
                                let minutes =
                                    self.config.get().await.hotkey.max_recording_minutes;
                                Duration::from_secs(u64::from(minutes) * 60)
                            };
                            if active_session.cap_reached(max_duration) {
                                info!("录音到达时长上限，自动结束");
                                // 与 Stop 同理：上限触发前排空已到达帧，尾帧不丢。
                                Self::drain_pending_frames(
                                    frames_rx.as_mut(),
                                    &self.events,
                                    &self.stats,
                                    &mut session,
                                    &mut last_level_emit,
                                );
                                Self::finish_session(
                                    &self.events,
                                    &mut session,
                                    &mut state,
                                    keep_warm,
                                );
                                if !keep_warm {
                                    stream_handle = None;
                                    frames_rx = None;
                                    errors_rx = None;
                                }
                                start_instant = None;
                            }
                        }
                        // 预热 / 热插拔恢复：流缺失且到了重试时刻。
                        if !has_stream(&stream_handle) && state != RecordingState::Recording {
                            let due = reopen_at.is_none_or(|at| Instant::now() >= at)
                                && (keep_warm || pending_device_update);
                            if due {
                                match self.open_stream().await {
                                    Ok(active) => {
                                        let (stream, frames, errors) = active.split();
                                        state = RecordingState::Warming;
                                        let _ = self.events.send(AppEvent::RecordingState { state, mode: None });
                                        stream_handle = Some(stream);
                                        frames_rx = Some(frames);
                                        errors_rx = Some(errors);
                                        pending_device_update = false;
                                        reopen_at = None;
                                    }
                                    Err(_) => {
                                        reopen_at = Some(Instant::now() + reopen_retry);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        })
    }

    /// 打开输入流；指定设备不可用时回退系统默认并广播降级提示（NFR-R-03）。
    async fn open_stream(&self) -> Result<ActiveStream, AppError> {
        let requested = self.config.get().await.audio.input_device_id;
        let (frames_tx, frames_rx) = mpsc::channel(FRAME_CHANNEL_CAPACITY);
        let (errors_tx, errors_rx) = mpsc::channel(4);
        let stream = match self.backend.open_input(
            requested.as_deref(),
            frames_tx.clone(),
            errors_tx.clone(),
        ) {
            Ok(stream) => stream,
            Err(AudioError::DeviceUnavailable(_)) if requested.is_some() => {
                info!("指定麦克风不可用，回退系统默认设备");
                let _ =
                    self.events
                        .send(AppEvent::RecordingResult(RecordingResultEvent::Degraded {
                            reason: "指定麦克风不可用，已回退系统默认设备".to_string(),
                        }));
                self.backend
                    .open_input(None, frames_tx, errors_tx)
                    .map_err(map_open_err)?
            }
            Err(err) => return Err(map_open_err(err)),
        };
        Ok(ActiveStream {
            stream,
            frames: frames_rx,
            errors: errors_rx,
        })
    }

    /// 结束会话前（Stop / 错误 / 时长上限）排空帧通道中已到达的帧：
    /// 命令与帧走不同通道，select! 不保证先后，直接结束会丢失尾帧。
    fn drain_pending_frames(
        frames_rx: Option<&mut mpsc::Receiver<AudioChunk>>,
        events: &broadcast::Sender<AppEvent>,
        stats: &AudioStats,
        session: &mut Option<RecordingSession>,
        last_level_emit: &mut Instant,
    ) {
        let Some(frames_rx) = frames_rx else { return };
        while let Ok(chunk) = frames_rx.try_recv() {
            Self::handle_frame(events, stats, session, last_level_emit, &chunk);
        }
    }

    /// 处理一帧：录音中入缓冲；非录音状态直接丢弃（NFR-PR-07）。
    fn handle_frame(
        events: &broadcast::Sender<AppEvent>,
        stats: &AudioStats,
        session: &mut Option<RecordingSession>,
        last_level_emit: &mut Instant,
        chunk: &AudioChunk,
    ) {
        match session {
            Some(active) => active.push(&chunk.samples),
            None => {
                // NFR-PR-07：预热状态下采样在此丢弃——不缓冲、不写盘、不交付，
                // 代码路径上数据到本函数为止。
                stats
                    .dropped_samples
                    .fetch_add(chunk.samples.len(), Ordering::Relaxed);
            }
        }
        // 音量电平：RMS + 50 ms 节流广播（tech/05 5.3.2）。
        if last_level_emit.elapsed() >= LEVEL_THROTTLE {
            *last_level_emit = Instant::now();
            let _ = events.send(AppEvent::RecordingLevel {
                level: rms_level(&chunk.samples),
            });
        }
    }

    /// 结束会话：缓冲打包经内部事件交给 AsrService（T-006 消费），状态回退。
    fn finish_session(
        events: &broadcast::Sender<AppEvent>,
        session: &mut Option<RecordingSession>,
        state: &mut RecordingState,
        keep_warm: bool,
    ) {
        if let Some(active_session) = session.take() {
            Self::deliver_audio(events, active_session);
        }
        *state = if keep_warm {
            RecordingState::Warming
        } else {
            RecordingState::Idle
        };
        let _ = events.send(AppEvent::RecordingState {
            state: *state,
            mode: None,
        });
        info!("录音会话结束");
    }

    /// 交付已采集音频（内部事件 AudioReady；样本共享 Arc，零拷贝）。
    fn deliver_audio(events: &broadcast::Sender<AppEvent>, session: RecordingSession) {
        // T-006 起由 AsrService 订阅消费；v0.1 本事件不转发前端（事件桥过滤）。
        let _ = events.send(AppEvent::AudioReady {
            session_id: session.id,
            sample_rate: session.sample_rate,
            samples: Arc::new(session.samples),
        });
        info!(session_id = session.id, "录音音频就绪，等待识别");
    }
}

/// 打开流错误的统一映射（E-AUDIO-001）。
fn map_open_err(err: AudioError) -> AppError {
    match err {
        AudioError::NoDevice => AppError::Audio(ERR_OPEN_STREAM, "无可用输入设备".to_string()),
        other => AppError::Audio(ERR_OPEN_STREAM, other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::audio::MockAudioBackend;
    use crate::services::config_store::AudioConfig;

    /// 建立测试环境：mock 后端 + 临时目录配置（指定 keep_warm）+ 事件订阅 + 命令通道。
    struct TestEnv {
        backend: Arc<MockAudioBackend>,
        config: ConfigStore,
        events_rx: broadcast::Receiver<AppEvent>,
        cmd_tx: mpsc::Sender<RecordCommand>,
        stats: Arc<AudioStats>,
    }

    async fn setup(keep_warm: bool) -> TestEnv {
        let dir = std::env::temp_dir().join(format!(
            "vt-audio-test-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (events_tx, events_rx) = crate::events::channel();
        let config = ConfigStore::load(&dir, events_tx.clone()).unwrap();
        let mut audio = serde_json::to_value(AudioConfig::default()).unwrap();
        audio["keep_warm"] = serde_json::json!(keep_warm);
        config
            .set_section(ConfigSection::Audio, audio)
            .await
            .unwrap();

        let backend = MockAudioBackend::with_default_device();
        let service = Arc::new(AudioService::new(
            backend.clone(),
            config.clone(),
            events_tx.clone(),
        ));
        let stats = service.stats();
        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        service.start(cmd_rx, events_tx.subscribe(), Duration::from_millis(20));
        TestEnv {
            backend,
            config,
            events_rx,
            cmd_tx,
            stats,
        }
    }

    /// 收集事件流中匹配谓词的下一个事件（带超时，避免测试挂死）。
    async fn next_event(
        rx: &mut broadcast::Receiver<AppEvent>,
        timeout: Duration,
        pred: impl Fn(&AppEvent) -> bool,
    ) -> AppEvent {
        next_event_labelled(rx, timeout, pred, "?").await
    }

    async fn next_event_labelled(
        rx: &mut broadcast::Receiver<AppEvent>,
        timeout: Duration,
        pred: impl Fn(&AppEvent) -> bool,
        label: &str,
    ) -> AppEvent {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let event = tokio::time::timeout(remaining, rx.recv())
                .await
                .unwrap_or_else(|_| panic!("等待事件超时: {label}"));
            let event = event.expect("事件通道关闭");
            if pred(&event) {
                return event;
            }
        }
    }

    fn is_state(state: RecordingState) -> impl Fn(&AppEvent) -> bool {
        move |event| matches!(event, AppEvent::RecordingState { state: s, .. } if *s == state)
    }

    #[tokio::test]
    async fn preheat_discards_frames_without_buffering() {
        let mut env = setup(true).await;
        // 预热：流已打开（Warming）。
        next_event(
            &mut env.events_rx,
            Duration::from_secs(2),
            is_state(RecordingState::Warming),
        )
        .await;
        // 非录音状态喂入帧：全部丢弃，无缓冲累积（NFR-PR-07）。
        for _ in 0..10 {
            env.backend.push_frame(vec![0.5; 160]);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(env.stats.dropped_samples.load(Ordering::Relaxed) >= 160 * 10);
        // 未产生 AudioReady（缓冲为空、无音频交付）。
        assert!(!matches!(
            env.events_rx.try_recv(),
            Ok(AppEvent::AudioReady { .. })
        ));
    }

    #[tokio::test]
    async fn record_start_buffers_and_stop_delivers_audio() {
        let mut env = setup(false).await;
        // 非预热：Start 现场开流。
        env.cmd_tx.send(RecordCommand::Start).await.unwrap();
        next_event(
            &mut env.events_rx,
            Duration::from_secs(2),
            is_state(RecordingState::Recording),
        )
        .await;
        env.backend.push_frame(vec![0.25; 320]);
        env.backend.push_frame(vec![0.25; 320]);
        env.cmd_tx.send(RecordCommand::Stop).await.unwrap();
        let ready = next_event(&mut env.events_rx, Duration::from_secs(2), |event| {
            matches!(event, AppEvent::AudioReady { .. })
        })
        .await;
        match ready {
            AppEvent::AudioReady {
                samples,
                sample_rate,
                ..
            } => {
                assert_eq!(samples.len(), 640);
                assert_eq!(sample_rate, 16_000);
            }
            other => panic!("期望 AudioReady，实际 {other:?}"),
        }
        // 非预热：结束回 Idle 并关流。
        next_event(
            &mut env.events_rx,
            Duration::from_secs(2),
            is_state(RecordingState::Idle),
        )
        .await;
    }

    #[tokio::test]
    async fn toggle_starts_and_stops_session() {
        let mut env = setup(true).await;
        next_event(
            &mut env.events_rx,
            Duration::from_secs(2),
            is_state(RecordingState::Warming),
        )
        .await;
        env.cmd_tx.send(RecordCommand::Toggle).await.unwrap();
        next_event(
            &mut env.events_rx,
            Duration::from_secs(2),
            is_state(RecordingState::Recording),
        )
        .await;
        env.backend.push_frame(vec![0.1; 160]);
        env.cmd_tx.send(RecordCommand::Toggle).await.unwrap();
        next_event(&mut env.events_rx, Duration::from_secs(2), |event| {
            matches!(event, AppEvent::AudioReady { .. })
        })
        .await;
        // 预热模式结束回 Warming。
        next_event(
            &mut env.events_rx,
            Duration::from_secs(2),
            is_state(RecordingState::Warming),
        )
        .await;
    }

    #[tokio::test]
    async fn cancel_drops_buffered_audio() {
        let mut env = setup(true).await;
        next_event(
            &mut env.events_rx,
            Duration::from_secs(2),
            is_state(RecordingState::Warming),
        )
        .await;
        env.cmd_tx.send(RecordCommand::Start).await.unwrap();
        next_event(
            &mut env.events_rx,
            Duration::from_secs(2),
            is_state(RecordingState::Recording),
        )
        .await;
        env.backend.push_frame(vec![0.1; 320]);
        env.cmd_tx.send(RecordCommand::Cancel).await.unwrap();
        next_event(
            &mut env.events_rx,
            Duration::from_secs(2),
            is_state(RecordingState::Cancelled),
        )
        .await;
        // 取消后无 AudioReady（缓冲已丢弃）。
        assert!(!matches!(
            env.events_rx.try_recv(),
            Ok(AppEvent::AudioReady { .. })
        ));
    }

    #[tokio::test]
    async fn stream_error_finalizes_partial_and_recovers() {
        let mut env = setup(true).await;
        next_event(
            &mut env.events_rx,
            Duration::from_secs(2),
            is_state(RecordingState::Warming),
        )
        .await;
        env.cmd_tx.send(RecordCommand::Start).await.unwrap();
        next_event(
            &mut env.events_rx,
            Duration::from_secs(2),
            is_state(RecordingState::Recording),
        )
        .await;
        env.backend.push_frame(vec![0.2; 480]);
        // 模拟录音中拔出麦克风。
        env.backend.push_stream_error();
        // 已采集部分照常交付（tech/05 5.6）。
        next_event_labelled(
            &mut env.events_rx,
            Duration::from_secs(2),
            |event| matches!(event, AppEvent::AudioReady { .. }),
            "AudioReady(部分交付)",
        )
        .await;
        // 状态回退 Idle（错误分支先广播 Idle 再广播 Failed，按序等待）。
        next_event_labelled(
            &mut env.events_rx,
            Duration::from_secs(2),
            is_state(RecordingState::Idle),
            "Idle(回退)",
        )
        .await;
        // 双端提示：Failed 事件带错误编号 E-AUDIO-002（NFR-R-03）。
        let failed = next_event_labelled(
            &mut env.events_rx,
            Duration::from_secs(2),
            |event| {
                matches!(
                    event,
                    AppEvent::RecordingResult(RecordingResultEvent::Failed { .. })
                )
            },
            "Failed(E-AUDIO-002)",
        )
        .await;
        match failed {
            AppEvent::RecordingResult(RecordingResultEvent::Failed { code, .. }) => {
                assert_eq!(code, "E-AUDIO-002");
            }
            other => panic!("期望 Failed 事件，实际 {other:?}"),
        }
        // 重试后热插拔恢复（reopen_retry = 20 ms）。
        next_event_labelled(
            &mut env.events_rx,
            Duration::from_secs(2),
            is_state(RecordingState::Warming),
            "Warming(热插拔恢复)",
        )
        .await;
    }

    #[tokio::test]
    async fn specified_device_missing_falls_back_to_default() {
        let mut env = setup(true).await;
        // 等待启动预热完成（默认设备），确保后续 open 请求顺序确定。
        next_event_labelled(
            &mut env.events_rx,
            Duration::from_secs(2),
            is_state(RecordingState::Warming),
            "Warming(启动预热)",
        )
        .await;
        // 配置一个不存在的外置设备。
        let mut audio = serde_json::to_value(env.config.get().await.audio).unwrap();
        audio["input_device_id"] = serde_json::json!("已断开的 USB 麦克风");
        env.config
            .set_section(ConfigSection::Audio, audio)
            .await
            .unwrap();
        // 回退提示：Degraded 事件。
        next_event_labelled(
            &mut env.events_rx,
            Duration::from_secs(2),
            |event| {
                matches!(
                    event,
                    AppEvent::RecordingResult(RecordingResultEvent::Degraded { .. })
                )
            },
            "Degraded(设备回退)",
        )
        .await;
        // 最终仍成功打开（回退默认设备）。
        next_event_labelled(
            &mut env.events_rx,
            Duration::from_secs(2),
            is_state(RecordingState::Warming),
            "Warming(回退默认设备)",
        )
        .await;
        let requests = env.backend.open_requests.lock().unwrap().clone();
        // 第 1 次是启动预热（默认设备）；第 2 次请求指定设备；第 3 次回退默认设备。
        assert_eq!(
            requests.get(1).and_then(|r| r.as_deref()),
            Some("已断开的 USB 麦克风")
        );
        assert_eq!(
            requests.last().and_then(|r| r.as_deref()),
            None,
            "应回退默认设备"
        );
    }

    #[tokio::test]
    async fn open_failure_reports_error_code() {
        let mut env = setup(false).await;
        env.backend.fail_open.store(true, Ordering::SeqCst);
        env.cmd_tx.send(RecordCommand::Start).await.unwrap();
        let failed = next_event(&mut env.events_rx, Duration::from_secs(2), |event| {
            matches!(
                event,
                AppEvent::RecordingResult(RecordingResultEvent::Failed { .. })
            )
        })
        .await;
        match failed {
            AppEvent::RecordingResult(RecordingResultEvent::Failed { code, .. }) => {
                assert_eq!(code, "E-AUDIO-001");
            }
            other => panic!("期望 Failed 事件，实际 {other:?}"),
        }
    }

    #[test]
    fn rms_level_basic() {
        assert_eq!(rms_level(&[]), 0.0);
        // 满幅：RMS 限幅 1.0。
        assert_eq!(rms_level(&[1.0, -1.0, 1.0, -1.0]), 1.0);
        assert!((rms_level(&[0.5, -0.5]) - 0.5).abs() < 1e-6);
        assert_eq!(rms_level(&[0.0, 0.0]), 0.0);
    }

    #[test]
    fn session_capacity_truncates_and_cap_checks() {
        let mut session = RecordingSession::new(1, 100, 16_000);
        session.push(&[0.1; 150]);
        assert_eq!(session.samples.len(), 100, "超出容量应截断");
        assert!(!session.cap_reached(Duration::from_secs(300)));
        let mut expired = RecordingSession::new(2, 100, 16_000);
        expired.started = Instant::now() - Duration::from_secs(301);
        assert!(expired.cap_reached(Duration::from_secs(300)));
    }
}
