//! 服务间事件广播与前端事件桥（tech/04 4.2、tech/03 3.5）。
//!
//! 服务不直接持有 Tauri AppHandle：状态变更经 `broadcast::Sender<AppEvent>` 广播，
//! 由 `main.rs` 的事件桥统一转换为 Tauri Event 推给前端（容量 256，tech/04 4.2）。
//! 标记"内部事件"的变体只供服务间消费，不转发前端。

use serde::Serialize;
use tokio::sync::broadcast;

use crate::services::config_store::{ConfigSection, HotkeyMode};

/// 事件广播通道容量（tech/04 4.2 关键通道表）。
pub const EVENT_CHANNEL_CAPACITY: usize = 256;

/// 创建事件广播通道。
pub fn channel() -> (broadcast::Sender<AppEvent>, broadcast::Receiver<AppEvent>) {
    broadcast::channel(EVENT_CHANNEL_CAPACITY)
}

/// 配置损坏回退来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoverySource {
    /// 已回退到上次良好副本 `config.json.bak`。
    Backup,
    /// `.bak` 同样损坏 / 不存在，已使用默认配置。
    Default,
}

/// 录音状态机状态（tech/05 5.2）。
// 允许 dead_code：由 T-005 AudioService 构造。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingState {
    Idle,
    Warming,
    Recording,
    Processing,
    Cancelled,
}

/// 模型下载状态。
// 允许 dead_code：由 T-014 模型管理构造。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Downloading,
    Completed,
    Failed,
    Cancelled,
}

/// 录音结果事件载荷（tech/05 5.5）。
// 允许 dead_code：由 T-005 / T-012 构造。
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RecordingResultEvent {
    /// 识别并上屏成功。
    Ok,
    /// 降级完成（如走剪贴板兜底），`reason` 说明降级路径。
    Degraded { reason: String },
    /// 失败，`code` 为稳定错误编号（如 `E-AUDIO-001`）。
    Failed { code: String, message: String },
}

/// 应用事件：服务间广播 + 前端推送的统一载荷。
// 允许 dead_code：全部变体由 T-004 ~ T-016 各服务构造，本任务（T-003）先建通道。
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppEvent {
    /// 配置某节写盘成功，携带变更节名（多窗口同步，tech/08 8.2）。
    ConfigChanged { section: ConfigSection },
    /// 配置文件损坏，已自动回退（.bak 或默认配置）。
    ConfigRecovered { source: RecoverySource },
    /// 录音状态机迁移。
    RecordingState {
        state: RecordingState,
        mode: Option<HotkeyMode>,
    },
    /// 音量电平（RMS 归一化 0.0~1.0），节流 50 ms。
    RecordingLevel { level: f32 },
    /// 录音结果反馈。
    RecordingResult(RecordingResultEvent),
    /// 录音音频就绪（内部事件，仅 AsrService 消费，不转发前端；T-006 接入）。
    AudioReady {
        session_id: u64,
        sample_rate: u32,
        samples: std::sync::Arc<Vec<f32>>,
    },
    /// 模型下载进度。
    ModelDownloadProgress {
        model_id: String,
        status: DownloadStatus,
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    /// 快捷键注册失败等错误提示（携带稳定错误编号）。
    HotkeyError { code: String, message: String },
}

impl AppEvent {
    /// 前端事件名（tech/03 3.5）；`None` 表示内部事件，不转发前端。
    pub fn frontend_event_name(&self) -> Option<&'static str> {
        match self {
            Self::ConfigChanged { .. } => Some("config:changed"),
            Self::ConfigRecovered { .. } => Some("config:recovered"),
            Self::RecordingState { .. } => Some("recording:state"),
            Self::RecordingLevel { .. } => Some("recording:level"),
            Self::RecordingResult(_) => Some("recording:result"),
            Self::AudioReady { .. } => None,
            Self::ModelDownloadProgress { .. } => Some("model:progress"),
            Self::HotkeyError { .. } => Some("hotkey:error"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_event_names_match_contract() {
        use crate::services::config_store::ConfigSection;
        assert_eq!(
            AppEvent::ConfigChanged {
                section: ConfigSection::Hotkey
            }
            .frontend_event_name(),
            Some("config:changed")
        );
        assert_eq!(
            AppEvent::RecordingLevel { level: 0.5 }.frontend_event_name(),
            Some("recording:level")
        );
        assert!(AppEvent::AudioReady {
            session_id: 1,
            sample_rate: 16_000,
            samples: std::sync::Arc::new(Vec::new())
        }
        .frontend_event_name()
        .is_none());
    }
}
