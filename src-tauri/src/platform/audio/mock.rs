//! 测试用音频 mock 后端（tech/04 4.5：服务层单测不依赖真实麦克风）。
// test-utils feature 下编译进 lib（非 cfg(test)）：unwrap 仅出现在测试替身
// （锁中毒 / 通道满即测试逻辑错误，panic 可接受），与 cfg(test) 语义一致。
#![allow(clippy::unwrap_used)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use super::{AudioBackend, AudioChunk, AudioDeviceInfo, AudioError, InputStream};

/// mock 已打开的流，测试经其注入帧与错误。
pub struct MockStreamHandle {
    /// 打开时请求的设备（调试验试/后续任务消费；保留供断言扩展）。
    #[allow(dead_code)]
    pub device_id: Option<String>,
    /// 流采样率（保留供断言扩展）。
    #[allow(dead_code)]
    pub sample_rate: u32,
    pub frames: mpsc::Sender<AudioChunk>,
    pub errors: mpsc::Sender<AudioError>,
}

/// mock 输入流：持有即保活。
pub struct MockInputStream {
    pub sample_rate: u32,
    pub alive: Arc<AtomicBool>,
}

impl InputStream for MockInputStream {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

impl Drop for MockInputStream {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
    }
}

/// mock 音频后端：固定设备清单 + 可注入帧 / 流错误。
pub struct MockAudioBackend {
    pub devices: Vec<AudioDeviceInfo>,
    /// 置位时 open_input 返回 NoDevice（模拟无设备 / 打开失败）。
    pub fail_open: AtomicBool,
    /// 打开次数与请求设备（验证回退到默认设备）。
    pub open_requests: Mutex<Vec<Option<String>>>,
    /// 已打开的流（测试注入帧 / 错误）。
    pub streams: Mutex<Vec<MockStreamHandle>>,
    /// open_input 计数。
    pub open_count: Arc<AtomicU64>,
}

impl MockAudioBackend {
    pub fn with_default_device() -> Arc<Self> {
        Arc::new(Self {
            devices: vec![
                AudioDeviceInfo {
                    id: "内置麦克风".to_string(),
                    name: "内置麦克风".to_string(),
                    is_default: true,
                },
                AudioDeviceInfo {
                    id: "USB 麦克风".to_string(),
                    name: "USB 麦克风".to_string(),
                    is_default: false,
                },
            ],
            fail_open: AtomicBool::new(false),
            open_requests: Mutex::new(Vec::new()),
            streams: Mutex::new(Vec::new()),
            open_count: Arc::new(AtomicU64::new(0)),
        })
    }

    /// 向最近一次打开的流注入一帧（try_send：禁止在异步上下文阻塞）。
    pub fn push_frame(&self, samples: Vec<f32>) {
        let streams = self.streams.lock().unwrap();
        let Some(stream) = streams.last() else {
            panic!("mock 后端尚无打开的流");
        };
        let chunk = AudioChunk {
            samples: Arc::new(samples),
        };
        assert!(stream.frames.try_send(chunk).is_ok(), "帧通道已满");
    }

    /// 向最近一次打开的流注入一个流错误（模拟设备拔出）。
    pub fn push_stream_error(&self) {
        let streams = self.streams.lock().unwrap();
        let Some(stream) = streams.last() else {
            panic!("mock 后端尚无打开的流");
        };
        assert!(
            stream
                .errors
                .try_send(AudioError::Stream("设备已拔出".to_string()))
                .is_ok(),
            "错误通道已满"
        );
    }
}

impl AudioBackend for MockAudioBackend {
    fn list_devices(&self) -> Result<Vec<AudioDeviceInfo>, AudioError> {
        Ok(self.devices.clone())
    }

    fn open_input(
        &self,
        device_id: Option<&str>,
        frames: mpsc::Sender<AudioChunk>,
        errors: mpsc::Sender<AudioError>,
    ) -> Result<Box<dyn InputStream>, AudioError> {
        self.open_count.fetch_add(1, Ordering::SeqCst);
        self.open_requests
            .lock()
            .unwrap()
            .push(device_id.map(str::to_string));
        if self.fail_open.load(Ordering::SeqCst) {
            return Err(AudioError::NoDevice);
        }
        // 指定设备必须在设备清单中，否则视为已断开（驱动 NFR-R-03 回退路径）。
        if let Some(id) = device_id {
            if !self.devices.iter().any(|device| device.id == id) {
                return Err(AudioError::DeviceUnavailable(id.to_string()));
            }
        }
        let stream = MockStreamHandle {
            device_id: device_id.map(str::to_string),
            sample_rate: super::TARGET_SAMPLE_RATE,
            frames,
            errors,
        };
        self.streams.lock().unwrap().push(stream);
        Ok(Box::new(MockInputStream {
            sample_rate: super::TARGET_SAMPLE_RATE,
            alive: Arc::new(AtomicBool::new(true)),
        }))
    }
}
