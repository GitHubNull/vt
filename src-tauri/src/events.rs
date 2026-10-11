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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Downloading,
    Completed,
    Failed,
    Cancelled,
}

/// 录音结果事件载荷（tech/05 5.5）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RecordingResultEvent {
    /// 上屏成功（T-012：`text` 为后处理后的最终文本，已由 InjectService 输入目标应用）。
    Ok { text: String },
    /// 降级完成（如走剪贴板兜底），`reason` 说明降级路径。
    Degraded { reason: String },
    /// 失败，`code` 为稳定错误编号（如 `E-AUDIO-001`）。
    Failed { code: String, message: String },
}

/// 应用事件：服务间广播 + 前端推送的统一载荷。
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
    /// 识别完成（内部事件，仅后处理管线消费，不转发前端；T-011 接入）。
    /// 隐私（NFR-S-02）：识别文本只经事件通道传递，绝不写入日志。
    Recognized { session_id: u64, text: String },
    /// 后处理管线出口（内部事件，T-012 上屏编排消费；T-011 起由管线广播）。
    PipelineOutput { session_id: u64, text: String },
    /// 录音会话终态（内部事件，T-012）：识别 / 后处理 / 上屏编排任一环节走到
    /// 终态（上屏完成、失败反馈、无效丢弃）时广播；AudioService 消费后从
    /// Processing 回退到 Idle / Warming（状态机收尾的唯一出口）。
    SessionFinished { session_id: u64 },
    /// 模型下载进度（`bytes_per_sec` 为瞬时速度，T-017）。
    ModelDownloadProgress {
        model_id: String,
        status: DownloadStatus,
        downloaded_bytes: u64,
        total_bytes: u64,
        bytes_per_sec: u64,
    },
    /// 模型管理提示（如删除激活模型后已回退到首个可用模型）。
    AsrNotice { code: String, message: String },
    /// 替换词典变更（多窗口同步，T-011）。
    DictChanged,
    /// 快捷键注册失败等错误提示（携带稳定错误编号）。
    HotkeyError { code: String, message: String },
    /// 当前场景变更（T-501，前端场景规则联动；`scene` 为场景标识字符串）。
    SceneChanged { scene: String },
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
            Self::Recognized { .. } => None,
            Self::PipelineOutput { .. } => None,
            Self::SessionFinished { .. } => None,
            Self::ModelDownloadProgress { .. } => Some("model:progress"),
            Self::AsrNotice { .. } => Some("asr:notice"),
            Self::DictChanged => Some("dict:changed"),
            Self::HotkeyError { .. } => Some("hotkey:error"),
            Self::SceneChanged { .. } => Some("scene:changed"),
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
