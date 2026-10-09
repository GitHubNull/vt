//! Esc 取消录音（T-010，F-INPUT-07）：录音期间动态注册的全局取消快捷键。
//!
//! - 录音进入 Recording 时注册（配置 `hotkey.cancel_trigger`，默认 Escape），
//!   结束 / 取消后注销——避免常驻占用 Esc 影响其他应用（T-010 SPEC 步骤 2）；
//! - 复用 T-004 的 `HotkeyBackend` 注册通道，但使用**独立后端实例**：
//!   精准 `unregister` 只影响 Esc，不碰主录音键（trait 新增方法，平台实现经
//!   global-shortcut 的 `unregister`）；
//! - Esc 只发 `RecordCommand::Cancel`，由 AudioService 状态机消费，
//!   本服务不直接操作录音状态；
//! - 注册失败（被占用 / 平台限制）仅告警：取消能力降级，录音主链路不受影响。
//!
//! 最小权限说明（AGENTS.md 3.5）：仅在录音期间注册用户配置的取消键并接收其
//! 按下事件，不拦截、不记录其他按键。

use std::sync::Arc;

use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};

use crate::events::{AppEvent, RecordingState};
use crate::platform::hotkey::{HotkeyBackend, RawHotkeyEvent};
use crate::services::hotkey::RecordCommand;

/// Esc 取消服务：录音期间临时注册取消快捷键。
pub struct EscCancelService {
    backend: Arc<dyn HotkeyBackend>,
    /// 取消快捷键串（配置 `hotkey.cancel_trigger`，默认 `Escape`）。
    accelerator: String,
    record_tx: mpsc::Sender<RecordCommand>,
}

impl EscCancelService {
    pub fn new(
        backend: Arc<dyn HotkeyBackend>,
        accelerator: String,
        record_tx: mpsc::Sender<RecordCommand>,
    ) -> Self {
        Self {
            backend,
            accelerator,
            record_tx,
        }
    }

    /// 启动事件循环：录音状态驱动注册 / 注销配对；Esc 按下 → 录音取消命令。
    pub fn start(
        self: Arc<Self>,
        mut raw_rx: mpsc::Receiver<RawHotkeyEvent>,
        mut state_rx: broadcast::Receiver<AppEvent>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // armed = Esc 已注册；注册与注销严格配对（测试断言）。
            let mut armed = false;
            loop {
                tokio::select! {
                    Some(raw) = raw_rx.recv() => {
                        if armed && matches!(raw, RawHotkeyEvent::Pressed) {
                            info!("取消快捷键按下，发送取消录音命令");
                            if self.record_tx.send(RecordCommand::Cancel).await.is_err() {
                                warn!("录音控制通道接收端已关闭");
                                break;
                            }
                        }
                    }
                    event = state_rx.recv() => {
                        match event {
                            Ok(AppEvent::RecordingState {
                                state: RecordingState::Recording,
                                ..
                            }) => {
                                if !armed {
                                    match self.backend.register(&self.accelerator) {
                                        Ok(()) => {
                                            armed = true;
                                            info!("取消快捷键已注册");
                                        }
                                        Err(err) => {
                                            warn!(error = %err, "取消快捷键注册失败，取消能力降级");
                                        }
                                    }
                                }
                            }
                            Ok(AppEvent::RecordingState { .. }) => {
                                if armed {
                                    if let Err(err) = self.backend.unregister(&self.accelerator) {
                                        warn!(error = %err, "取消快捷键注销失败");
                                    }
                                    armed = false;
                                }
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::channel;

    /// 建立测试环境：mock 后端 + 事件通道 + 录音控制通道。
    struct TestEnv {
        backend: Arc<crate::platform::hotkey::MockHotkeyBackend>,
        events_tx: broadcast::Sender<AppEvent>,
        raw_tx: mpsc::Sender<RawHotkeyEvent>,
        record_rx: mpsc::Receiver<RecordCommand>,
    }

    fn setup() -> TestEnv {
        let backend = Arc::new(crate::platform::hotkey::MockHotkeyBackend::new());
        let (events_tx, _rx) = channel();
        let (raw_tx, raw_rx) = mpsc::channel(8);
        let (record_tx, record_rx) = mpsc::channel(8);
        let service = Arc::new(EscCancelService::new(
            backend.clone(),
            "Escape".to_string(),
            record_tx,
        ));
        service.start(raw_rx, events_tx.subscribe());
        TestEnv {
            backend,
            events_tx,
            raw_tx,
            record_rx,
        }
    }

    fn fire_state(env: &TestEnv, state: RecordingState) {
        let _ = env
            .events_tx
            .send(AppEvent::RecordingState { state, mode: None });
    }

    /// 等待谓词满足（注册 / 注销经 select! 调度，需给事件循环一个 tick）。
    async fn wait_for(cond: impl Fn() -> bool) {
        for _ in 0..50 {
            if cond() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("等待条件超时");
    }

    #[tokio::test]
    async fn registers_on_recording_and_unregisters_on_stop() {
        let env = setup();
        fire_state(&env, RecordingState::Recording);
        wait_for(|| env.backend.current().as_deref() == Some("Escape")).await;

        fire_state(&env, RecordingState::Warming);
        wait_for(|| env.backend.current().is_none()).await;

        // 配对断言：一次注册对应一次注销。
        assert_eq!(
            env.backend
                .register_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert_eq!(
            env.backend
                .unregister_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn esc_press_while_recording_sends_cancel() {
        let mut env = setup();
        fire_state(&env, RecordingState::Recording);
        wait_for(|| env.backend.current().is_some()).await;

        env.raw_tx.send(RawHotkeyEvent::Pressed).await.unwrap();
        assert_eq!(env.record_rx.recv().await, Some(RecordCommand::Cancel));
        // Released 不产生额外命令。
        env.raw_tx.send(RawHotkeyEvent::Released).await.unwrap();
        fire_state(&env, RecordingState::Warming);
        wait_for(|| env.backend.current().is_none()).await;
        assert!(env.record_rx.try_recv().is_err(), "松开不产生命令");
    }

    #[tokio::test]
    async fn esc_press_outside_recording_is_ignored() {
        let mut env = setup();
        // 未注册时按下 Esc：无命令。
        env.raw_tx.send(RawHotkeyEvent::Pressed).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(env.record_rx.try_recv().is_err());
        assert_eq!(
            env.backend
                .register_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn registration_failure_stays_unarmed_and_pairing_holds() {
        let mut env = setup();
        // 第一次注册失败：armed = false，结束后不应有注销。
        env.backend
            .fail_next
            .store(true, std::sync::atomic::Ordering::SeqCst);
        fire_state(&env, RecordingState::Recording);
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(env.backend.current().is_none());

        env.raw_tx.send(RawHotkeyEvent::Pressed).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(env.record_rx.try_recv().is_err(), "未注册时按下不生效");

        fire_state(&env, RecordingState::Warming);
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert_eq!(
            env.backend
                .unregister_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            0,
            "注册失败不得产生注销（配对）"
        );

        // 下一轮录音注册成功，仍保持配对。
        fire_state(&env, RecordingState::Recording);
        wait_for(|| env.backend.current().is_some()).await;
        fire_state(&env, RecordingState::Idle);
        wait_for(|| env.backend.current().is_none()).await;
        assert_eq!(
            env.backend
                .register_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            env.backend
                .unregister_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            "注册 / 注销必须配对"
        );
    }

    #[tokio::test]
    async fn repeated_recording_cycles_keep_register_unregister_balanced() {
        let env = setup();
        for _ in 0..3 {
            fire_state(&env, RecordingState::Recording);
            wait_for(|| env.backend.current().is_some()).await;
            fire_state(&env, RecordingState::Cancelled);
            wait_for(|| env.backend.current().is_none()).await;
        }
        assert_eq!(
            env.backend
                .register_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            3
        );
        assert_eq!(
            env.backend
                .unregister_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            3,
            "三轮录音后注册 / 注销仍配对"
        );
    }
}
