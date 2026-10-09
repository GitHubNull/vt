//! cpal 后端：设备枚举与输入流打开（tech/09 9.2.6）。
//!
//! 最小权限说明（AGENTS.md 3.5）：仅枚举输入设备并打开用户选择的麦克风输入流，
//! 不访问输出设备（录音时静音扬声器由 T-013 之后的独立任务处理）。

use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Sample;
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::{
    AudioBackend, AudioChunk, AudioDeviceInfo, AudioError, InputStream, TARGET_CHANNELS,
    TARGET_SAMPLE_RATE,
};

/// cpal 统一后端（Windows / macOS / Linux 三平台共用，差异在 cpal 内部收敛）。
pub struct CpalAudioBackend;

impl AudioBackend for CpalAudioBackend {
    fn list_devices(&self) -> Result<Vec<AudioDeviceInfo>, AudioError> {
        let host = cpal::default_host();
        let default_name = host
            .default_input_device()
            .and_then(|device| device.name().ok());
        let devices = host
            .input_devices()
            .map_err(|err| AudioError::OpenStream(err.to_string()))?
            .filter_map(|device| {
                // 设备名不可读（罕见，如设备恰在枚举中拔出）时跳过该设备。
                let name = device.name().ok()?;
                Some(AudioDeviceInfo {
                    id: name.clone(),
                    name,
                    is_default: default_name.as_deref() == Some(device.name().ok()?.as_str()),
                })
            })
            .collect();
        Ok(devices)
    }

    fn open_input(
        &self,
        device_id: Option<&str>,
        frames: mpsc::Sender<AudioChunk>,
        errors: mpsc::Sender<AudioError>,
    ) -> Result<Box<dyn InputStream>, AudioError> {
        let host = cpal::default_host();
        let device = match device_id {
            Some(id) => {
                let found = host.input_devices().ok().and_then(|devices| {
                    devices
                        .filter_map(|device| device.name().ok().map(|name| (name, device)))
                        .find(|(name, _device)| name == id)
                        .map(|(_name, device)| device)
                });
                match found {
                    Some(device) => device,
                    None => return Err(AudioError::DeviceUnavailable(id.to_string())),
                }
            }
            None => host.default_input_device().ok_or(AudioError::NoDevice)?,
        };

        // 优先请求 16 kHz / 单声道 / f32（tech/05 5.3.2）；设备不支持时退设备默认配置。
        let config = device
            .default_input_config()
            .map_err(|err| AudioError::OpenStream(err.to_string()))?;
        let channels = if config.channels() > TARGET_CHANNELS {
            // 多声道设备请求单声道；不支持则保持原声道数，回调内混单声道。
            match device.supported_input_configs() {
                Ok(mut ranges) => {
                    if ranges.any(|range| range.channels() == TARGET_CHANNELS) {
                        TARGET_CHANNELS
                    } else {
                        config.channels()
                    }
                }
                Err(_) => config.channels(),
            }
        } else {
            TARGET_CHANNELS
        };
        let sample_format = config.sample_format();

        // 优先 16 kHz（tech/05 5.3.2），设备不支持时回退其默认采样率。
        let sample_rates = [TARGET_SAMPLE_RATE, config.sample_rate().0];
        let mut last_err = None;
        for rate in sample_rates {
            let stream_config = cpal::StreamConfig {
                channels,
                sample_rate: cpal::SampleRate(rate),
                buffer_size: cpal::BufferSize::Default,
            };
            // 采样格式回退：优先 f32，其次 i16 / u16（cpal Sample 转换到 f32）。
            let result = match sample_format {
                cpal::SampleFormat::F32 => {
                    build_stream::<f32>(&device, &stream_config, frames.clone(), errors.clone())
                }
                cpal::SampleFormat::I16 => {
                    build_stream::<i16>(&device, &stream_config, frames.clone(), errors.clone())
                }
                cpal::SampleFormat::U16 => {
                    build_stream::<u16>(&device, &stream_config, frames.clone(), errors.clone())
                }
                other => {
                    last_err = Some(AudioError::OpenStream(format!(
                        "不支持的采样格式: {other:?}"
                    )));
                    break;
                }
            };
            match result {
                Ok(stream) => {
                    info!(device = %device_desc(&device), sample_rate = rate, "麦克风输入流已打开");
                    return Ok(Box::new(CpalInputStream {
                        _stream: stream,
                        sample_rate: rate,
                    }));
                }
                Err(err) => last_err = Some(err),
            }
        }
        Err(last_err.unwrap_or(AudioError::OpenStream("未知错误".to_string())))
    }
}

fn device_desc(device: &cpal::Device) -> String {
    device.name().unwrap_or_else(|_| "<未知设备>".to_string())
}

struct CpalInputStream {
    /// 持有即保活；cpal Stream 在 drop 时关闭。
    _stream: cpal::Stream,
    sample_rate: u32,
}

// SAFETY：cpal Stream 因平台内部含原始指针（如 ALSA 句柄）被标记 !Send；
// 本服务只持有句柄（不动其内部状态），数据经 cpal 自有音频线程回调交付，
// 句柄跨线程移动不会引入数据竞争。
unsafe impl Send for CpalInputStream {}

impl InputStream for CpalInputStream {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// 构建输入流并启动采集。
///
/// 帧回调运行在 cpal 音频线程：try_send 不阻塞，通道满则丢帧并记日志
/// （背压原则见 tech/04 4.2）；多声道数据在此混单声道。
fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    frames: mpsc::Sender<AudioChunk>,
    errors: mpsc::Sender<AudioError>,
) -> Result<cpal::Stream, AudioError>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let channels = config.channels.max(1) as usize;
    let stream = device
        .build_input_stream(
            config,
            move |data: &[T], _info| {
                // 混单声道：多声道取均值，避免通道相位问题（tech/05 5.3.2 单声道要求）。
                let mono: Vec<f32> = if channels == 1 {
                    data.iter().map(|s| f32::from_sample(*s)).collect()
                } else {
                    data.chunks(channels)
                        .map(|frame| {
                            frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>()
                                / channels as f32
                        })
                        .collect()
                };
                // 音频帧只进内存缓冲，绝不写盘（NFR-PR-02）。
                if frames
                    .try_send(AudioChunk {
                        samples: Arc::new(mono),
                    })
                    .is_err()
                {
                    // 帧通道满：丢帧不阻塞采集线程（tech/04 4.2），记日志限频由服务层节流。
                    warn!("音频帧通道已满，丢帧");
                }
            },
            move |err| {
                // 设备拔出 / 独占失败：上报给 AudioService 做状态回退与恢复（NFR-R-03）。
                let _ = errors.try_send(AudioError::Stream(err.to_string()));
            },
            None,
        )
        .map_err(|err| AudioError::OpenStream(err.to_string()))?;
    stream
        .play()
        .map_err(|err| AudioError::OpenStream(err.to_string()))?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// cpal 后端冒烟：无显示 / 音频服务器的 CI 环境允许失败，但错误必须是
    /// 受控 AudioError 而非 panic（tech/04 4.8 平台层冒烟）。
    #[test]
    fn cpal_backend_smoke() {
        let backend = CpalAudioBackend;
        let _ = backend.list_devices();
        let (frames_tx, _frames_rx) = mpsc::channel(4);
        let (errors_tx, _errors_rx) = mpsc::channel(4);
        let result = backend.open_input(Some("__vt_no_such_device__"), frames_tx, errors_tx);
        assert!(matches!(
            result.err(),
            Some(AudioError::DeviceUnavailable(_))
        ));
    }
}
