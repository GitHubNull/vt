//! 音频采集平台适配层（tech/04 4.5、tech/09 9.2.6）。
//!
//! 三平台统一基于 `cpal`（Windows WASAPI / macOS CoreAudio / Linux ALSA、
//! PulseAudio、PipeWire 均经 cpal 后端，tech/09 9.2.6）。
//!
//! 最小权限说明（AGENTS.md 3.5）：本能力仅访问用户选择的麦克风输入设备；
//! 非录音状态下采样在 AudioService 内直接丢弃（NFR-PR-07），不做任何持久化。
//! 设备热插拔三平台行为不同，统一由 AudioService 做枚举刷新与错误恢复（NFR-R-03）。

use std::sync::Arc;

use serde::Serialize;
use tokio::sync::mpsc;

/// 目标采样参数（tech/05 5.3.2）：16 kHz / 单声道 / f32。
///
/// 设备不支持 16 kHz 时按设备默认配置打开，实际采样率随帧上报，
/// 重采样 / 格式转换在引擎输入侧完成（tech/05 5.3.3）。
pub const TARGET_SAMPLE_RATE: u32 = 16_000;
/// 目标声道数。
pub const TARGET_CHANNELS: u16 = 1;

/// 输入设备信息（`list_audio_devices` 命令返回元素，与前端 types.ts 同步）。
///
/// cpal 0.15 无稳定设备 ID，`id` 使用设备名（重名时系统内通常仍唯一；
/// 跨重启可能变化，配置回退逻辑见 AudioService）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AudioDeviceInfo {
    /// 设备标识（当前实现为设备名）。
    pub id: String,
    /// 展示名。
    pub name: String,
    /// 是否为系统默认输入设备。
    pub is_default: bool,
}

/// 平台后端错误。
#[derive(Debug, Clone, thiserror::Error)]
pub enum AudioError {
    /// 无可用输入设备。
    #[error("无可用输入设备")]
    NoDevice,
    /// 指定设备不可用（已断开 / 不存在）。
    #[error("指定设备不可用: {0}")]
    DeviceUnavailable(String),
    /// 打开音频流失败。
    #[error("打开音频流失败: {0}")]
    OpenStream(String),
    /// 采集流运行中错误（拔出、独占失败等）。
    #[error("采集流错误: {0}")]
    Stream(String),
}

/// 帧块：单次回调的数据（f32、已混单声道）。设备实际采样率见 `InputStream::sample_rate`。
#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub samples: Arc<Vec<f32>>,
}

/// 输入流句柄：持有即保活，drop 即关闭流（关闭为同步操作，尽快释放设备）。
pub trait InputStream: Send {
    /// 设备实际采样率（可能非 16 kHz，视设备支持而定）。
    fn sample_rate(&self) -> u32;
}

/// 音频后端抽象（tech/04 4.5）。
pub trait AudioBackend: Send + Sync {
    /// 枚举系统输入设备。
    fn list_devices(&self) -> Result<Vec<AudioDeviceInfo>, AudioError>;
    /// 打开输入流。
    ///
    /// - `device_id`：`None` 表示系统默认设备；
    /// - 帧数据经 `frames` 通道上报（容量 64，满时丢弃最旧帧由服务层决定，
    ///   实际上游回调不阻塞，满则丢帧并记日志，tech/04 4.2 背压原则）；
    /// - 流运行中错误经 `errors` 通道上报（设备拔出等，NFR-R-03）。
    fn open_input(
        &self,
        device_id: Option<&str>,
        frames: mpsc::Sender<AudioChunk>,
        errors: mpsc::Sender<AudioError>,
    ) -> Result<Box<dyn InputStream>, AudioError>;
}

mod cpal_backend;
// test-utils：集成测试（tests/）与基准（benches/）共享 mock（T-012）。
#[cfg(any(test, feature = "test-utils"))]
mod mock;

pub use cpal_backend::CpalAudioBackend;
#[cfg(any(test, feature = "test-utils"))]
pub use mock::MockAudioBackend;

/// 三平台默认后端：cpal（tech/04 4.5 工厂函数）。
pub fn default_backend() -> Arc<dyn AudioBackend> {
    Arc::new(CpalAudioBackend)
}
