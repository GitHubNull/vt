//! HotkeyService（tech/04 4.4、tech/05 5.3.1）：全局快捷键注册与录音控制事件出口。
//!
//! - 消费 `platform/hotkey/` 的原始按键事件（Pressed / Released）；
//! - 按配置模式映射为录音控制命令（F-INPUT-02：按住 / 切换），经 mpsc 通道发给
//!   AudioService（tech/04 4.2 录音控制通道，容量 8）；
//! - 监听 `config:changed` 的 hotkey 节，平滑重注册：先注销再注册，失败回滚旧
//!   快捷键并广播 `hotkey:error` 事件（错误编号 E-HOTKEY-001，T-004 SPEC 步骤 4/5）；
//! - 快捷键事件只驱动录音状态机，不做任何 UI 操作（tech/05 5.3.1）。
//!
//! 已知平台限制（tech/09 9.2.1）：
//! - Windows：单个修饰键作触发键由低级键盘钩子覆盖（platform/hotkey/windows_llhook），
//!   PRD 3.1.1 规则 3（组合键抑制）与规则 4（0.3 s 误触门）仅在钩子路径生效；
//!   组合键 / 功能键仍走 RegisterHotKey，无该门控；
//! - X11：修饰键事件依赖 global-hotkey XRecord 路径，"按了其他键则不触发"
//!   （PRD 3.1.1 规则 3）未实现，组合键场景可能误触发；
//! - Wayland：协议限制无法全局监听（v1.0 提供替代方案）。

use std::sync::Arc;

use tokio::sync::{broadcast, mpsc};
use tracing::info;
use tracing::warn;

use crate::error::AppError;
use crate::events::AppEvent;
use crate::platform::hotkey::{HotkeyBackend, HotkeyError, RawHotkeyEvent};
use crate::services::config_store::{ConfigSection, ConfigStore, HotkeyConfig, HotkeyMode};

/// 快捷键注册失败错误编号（T-004 SPEC 步骤 5）。
pub const ERR_REGISTER_FAILED: u32 = 1;
/// 快捷键串非法（组合格式无法解析，E-HOTKEY-002）。
pub const ERR_INVALID_SPEC: u32 = 2;

/// 录音控制命令（T-004 SPEC 步骤 1）：快捷键 / 命令层 / 小窗统一出口。
///
/// 由 AudioService 消费（tech/05 5.2 状态机）；`Cancel` 由 T-010 消费路径接入。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordCommand {
    Start,
    Stop,
    Toggle,
    Cancel,
}

/// 触发模式映射器（按住 / 切换的状态推导，纯逻辑可单测）。
///
/// - 按住（Hold）：Pressed → Start，Released → Stop；
/// - 切换（Toggle）：Pressed 翻转内部状态，首次 → Start、再次 → Stop；
///   Released 忽略。
#[derive(Debug, Clone)]
pub struct ModeMapper {
    mode: HotkeyMode,
    /// 切换模式下当前是否处于"已按下开始"状态。
    toggle_armed: bool,
}

impl ModeMapper {
    pub fn new(mode: HotkeyMode) -> Self {
        Self {
            mode,
            toggle_armed: false,
        }
    }

    /// 模式热切换时调用：重置内部状态，避免残留 armed 造成首按即 Stop。
    pub fn set_mode(&mut self, mode: HotkeyMode) {
        if self.mode != mode {
            self.mode = mode;
            self.toggle_armed = false;
        }
    }

    /// 映射原始按键事件为录音控制命令。
    pub fn map(&mut self, event: RawHotkeyEvent) -> Option<RecordCommand> {
        match self.mode {
            HotkeyMode::Hold => match event {
                RawHotkeyEvent::Pressed => Some(RecordCommand::Start),
                RawHotkeyEvent::Released => Some(RecordCommand::Stop),
            },
            HotkeyMode::Toggle => match event {
                RawHotkeyEvent::Pressed => {
                    self.toggle_armed = !self.toggle_armed;
                    Some(if self.toggle_armed {
                        RecordCommand::Start
                    } else {
                        RecordCommand::Stop
                    })
                }
                RawHotkeyEvent::Released => None,
            },
        }
    }
}

/// HotkeyService：持有后端与注册状态，事件循环在 `start()` 中运行。
pub struct HotkeyService {
    backend: Arc<dyn HotkeyBackend>,
    events: broadcast::Sender<AppEvent>,
    record_tx: mpsc::Sender<RecordCommand>,
    /// 当前已注册快捷键与模式（重注册 / 回滚的依据）。
    inner: Arc<tokio::sync::Mutex<Inner>>,
}

struct Inner {
    accelerator: String,
    mapper: ModeMapper,
}

impl HotkeyService {
    pub fn new(
        backend: Arc<dyn HotkeyBackend>,
        events: broadcast::Sender<AppEvent>,
        record_tx: mpsc::Sender<RecordCommand>,
        initial: &HotkeyConfig,
    ) -> Self {
        Self {
            backend,
            events,
            record_tx,
            inner: Arc::new(tokio::sync::Mutex::new(Inner {
                accelerator: initial.trigger.clone(),
                mapper: ModeMapper::new(initial.mode),
            })),
        }
    }

    /// 按配置注册快捷键；失败时回滚旧快捷键并返回错误（调用方广播错误事件）。
    pub async fn apply_config(&self, config: &HotkeyConfig) -> Result<(), AppError> {
        // 先注销再注册（T-004 SPEC 步骤 4）；注册失败回滚旧配置。
        self.backend
            .unregister_all()
            .map_err(|err| AppError::Hotkey(ERR_REGISTER_FAILED, err.to_string()))?;
        if let Err(err) = self.backend.register(&config.trigger) {
            // 回滚：重新注册旧快捷键。
            let old = self.inner.lock().await.accelerator.clone();
            warn!(error = %err, "快捷键注册失败，回滚旧快捷键");
            if let Err(rollback_err) = self.backend.register(&old) {
                warn!(error = %rollback_err, "回滚旧快捷键也失败");
            }
            return Err(match err {
                // 串非法 / 鼠标侧键等输入侧问题：错误编号区分，前端给引导提示。
                HotkeyError::InvalidAccelerator(_) | HotkeyError::MouseKeyUnsupported(_) => {
                    AppError::Hotkey(ERR_INVALID_SPEC, err.to_string())
                }
                _ => AppError::Hotkey(
                    ERR_REGISTER_FAILED,
                    format!("快捷键 {} 注册失败（可能被占用）", config.trigger),
                ),
            });
        }
        let mut inner = self.inner.lock().await;
        inner.accelerator = config.trigger.clone();
        inner.mapper.set_mode(config.mode);
        info!("快捷键已注册");
        Ok(())
    }

    /// 启动事件循环：原始按键事件 → 模式映射 → 录音控制通道；
    /// 同时订阅 `config:changed` 做热重注册。
    pub fn start(
        self: Arc<Self>,
        config: ConfigStore,
        mut raw_rx: mpsc::Receiver<RawHotkeyEvent>,
        mut config_rx: broadcast::Receiver<AppEvent>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(raw) = raw_rx.recv() => {
                        let cmd = {
                            let mut inner = self.inner.lock().await;
                            inner.mapper.map(raw)
                        };
                        if let Some(cmd) = cmd {
                            // 控制类通道满时返回错误而非丢弃（tech/04 4.2 背压原则）。
                            if self.record_tx.send(cmd).await.is_err() {
                                warn!("录音控制通道接收端已关闭");
                                break;
                            }
                        }
                    }
                    changed = config_rx.recv() => {
                        match changed {
                            Ok(AppEvent::ConfigChanged { section: ConfigSection::Hotkey }) => {
                                let hotkey = config.get().await.hotkey;
                                if let Err(err) = self.apply_config(&hotkey).await {
                                    // 注册失败：错误编号经事件通知前端（T-004 SPEC 验收 4）。
                                    let _ = self.events.send(AppEvent::HotkeyError {
                                        code: err.code(),
                                        message: err.message().to_string(),
                                    });
                                }
                            }
                            Ok(_) => {} // 其他节的配置变更与本服务无关
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                warn!("配置变更事件滞后，热更新可能延迟");
                            }
                            Err(_) => break, // 发送端全部关闭
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
    use crate::platform::hotkey::MockHotkeyBackend;

    #[test]
    fn hold_mode_maps_press_and_release() {
        let mut mapper = ModeMapper::new(HotkeyMode::Hold);
        assert_eq!(
            mapper.map(RawHotkeyEvent::Pressed),
            Some(RecordCommand::Start)
        );
        assert_eq!(
            mapper.map(RawHotkeyEvent::Released),
            Some(RecordCommand::Stop)
        );
    }

    #[test]
    fn toggle_mode_flips_on_each_press() {
        let mut mapper = ModeMapper::new(HotkeyMode::Toggle);
        assert_eq!(
            mapper.map(RawHotkeyEvent::Pressed),
            Some(RecordCommand::Start)
        );
        // 切换模式下 Released 不产生命令。
        assert_eq!(mapper.map(RawHotkeyEvent::Released), None);
        assert_eq!(
            mapper.map(RawHotkeyEvent::Pressed),
            Some(RecordCommand::Stop)
        );
        assert_eq!(
            mapper.map(RawHotkeyEvent::Pressed),
            Some(RecordCommand::Start)
        );
    }

    #[test]
    fn mode_switch_resets_toggle_state() {
        let mut mapper = ModeMapper::new(HotkeyMode::Toggle);
        assert_eq!(
            mapper.map(RawHotkeyEvent::Pressed),
            Some(RecordCommand::Start)
        );
        mapper.set_mode(HotkeyMode::Hold);
        mapper.set_mode(HotkeyMode::Toggle);
        // 重置后首按应为 Start 而非 Stop。
        assert_eq!(
            mapper.map(RawHotkeyEvent::Pressed),
            Some(RecordCommand::Start)
        );
    }

    fn test_service(
        backend: Arc<MockHotkeyBackend>,
    ) -> (Arc<HotkeyService>, mpsc::Receiver<RecordCommand>) {
        let (record_tx, record_rx) = mpsc::channel(8);
        let (events, _rx) = broadcast::channel(16);
        let service = Arc::new(HotkeyService::new(
            backend,
            events,
            record_tx,
            &HotkeyConfig::default(),
        ));
        (service, record_rx)
    }

    #[tokio::test]
    async fn apply_config_registers_new_accelerator() {
        let backend = Arc::new(MockHotkeyBackend::new());
        let (service, _rx) = test_service(backend.clone());
        let config = HotkeyConfig {
            trigger: "ControlLeft+ShiftLeft+Space".to_string(),
            mode: HotkeyMode::Toggle,
            ..Default::default()
        };
        service.apply_config(&config).await.unwrap();
        assert_eq!(
            backend.current().as_deref(),
            Some("ControlLeft+ShiftLeft+Space")
        );
        // 内部模式已同步：切换模式映射生效。
        let cmd = {
            let mut inner = service.inner.lock().await;
            inner.mapper.map(RawHotkeyEvent::Pressed)
        };
        assert_eq!(cmd, Some(RecordCommand::Start));
    }

    #[tokio::test]
    async fn register_failure_rolls_back_old_accelerator() {
        let backend = Arc::new(MockHotkeyBackend::new());
        let (service, _rx) = test_service(backend.clone());
        // 初始注册右 Alt。
        service
            .apply_config(&HotkeyConfig::default())
            .await
            .unwrap();
        // 新键被占用：注册失败，回滚旧键。
        backend
            .fail_next
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let config = HotkeyConfig {
            trigger: "F13".to_string(),
            ..Default::default()
        };
        let err = service.apply_config(&config).await.unwrap_err();
        assert_eq!(err.code(), "E-HOTKEY-001");
        // 注销后被清空，再回滚重新注册旧键 AltRight。
        assert_eq!(backend.current().as_deref(), Some("AltRight"));
    }

    #[tokio::test]
    async fn config_change_event_triggers_reregistration() {
        let backend = Arc::new(MockHotkeyBackend::new());
        let (service, mut record_rx) = test_service(backend.clone());
        let (config_dir, _keep) = {
            let dir = std::env::temp_dir().join(format!("vt-hotkey-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            (dir, ())
        };
        let (tx, _rx) = crate::events::channel();
        let config = ConfigStore::load(&config_dir, tx.clone()).unwrap();
        let (raw_tx, raw_rx) = mpsc::channel(8);
        let handle = service
            .clone()
            .start(config.clone(), raw_rx, tx.subscribe());

        // 初始注册。
        service
            .apply_config(&HotkeyConfig::default())
            .await
            .unwrap();
        // 模拟按键 → 录音控制命令出口。
        raw_tx.send(RawHotkeyEvent::Pressed).await.unwrap();
        assert_eq!(record_rx.recv().await, Some(RecordCommand::Start));

        // 修改配置（热更新）→ 服务自动重注册 F2。
        let mut value = serde_json::to_value(config.get().await.hotkey).unwrap();
        value["trigger"] = serde_json::json!("F2");
        config
            .set_section(ConfigSection::Hotkey, value)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(backend.current().as_deref(), Some("F2"));

        handle.abort();
        let _ = std::fs::remove_dir_all(&config_dir);
    }
}
