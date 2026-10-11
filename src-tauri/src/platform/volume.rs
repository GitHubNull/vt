//! 系统输出音量控制（PRD 3.8.4「录音时静音」，默认开）。
//!
//! 录音会话期间静音系统默认输出设备（`audio.mute_speaker_while_recording`），
//! 会话结束（含异常 / Esc 取消）恢复。三平台实现以 `#[cfg(target_os)]` 条件编译，
//! 并附 mock 实现供服务层单测（AGENTS.md：`platform/mod.rs` 平台适配层约定）。
//!
//! 最小权限说明（AGENTS.md 3.5）：本能力仅切换默认输出设备的静音标志
//! （mute），不读取 / 不修改用户音量值，不做任何持久化；
//! 任何平台失败仅降级为 warn，绝不影响录音主路径。

use std::fmt;

/// 音量控制错误（仅用于日志与测试断言，不上屏）。
#[derive(Debug, Clone)]
pub struct VolumeError(String);

impl fmt::Display for VolumeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for VolumeError {}

impl From<String> for VolumeError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

/// 系统输出音量控制抽象：调用方持有恢复守卫（`MuteGuard`，见 audio.rs），
/// 平台实现只做一次原子操作，不缓存状态。
///
/// `Sync` 要求：`MuteGuard` 跨 await 持有 `Arc<dyn VolumeControl>`，
/// 需要 `Arc` 可在线程间安全传递（Send），故 trait 需为 `Send + Sync`；
/// 现有实现均为无内部可变状态（或经 `Mutex`）的结构，天然满足。
pub trait VolumeControl: Send + Sync {
    /// 设置默认输出设备静音标志；`muted = false` 表示恢复输出。
    fn set_mute(&self, muted: bool) -> Result<(), VolumeError>;
}

/// 平台工厂：返回当前操作系统的默认输出设备音量控制。
pub fn system_volume_control() -> std::sync::Arc<dyn VolumeControl> {
    #[cfg(target_os = "linux")]
    {
        std::sync::Arc::new(PulseVolumeControl)
    }
    #[cfg(target_os = "macos")]
    {
        std::sync::Arc::new(MacosVolumeControl)
    }
    #[cfg(target_os = "windows")]
    {
        std::sync::Arc::new(WindowsVolumeControl)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        std::sync::Arc::new(UnsupportedVolumeControl)
    }
}

/// 不支持的平台：静默 no-op（不 warn——该平台本就没有此能力，避免噪音日志）。
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
struct UnsupportedVolumeControl;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl VolumeControl for UnsupportedVolumeControl {
    fn set_mute(&self, _muted: bool) -> Result<(), VolumeError> {
        Ok(())
    }
}

/// Linux（PulseAudio / PipeWire）：经 `pactl` 控制默认 sink 静音。
///
/// `pactl` 缺失（纯 ALSA 环境等）时 warn 一次后降级为 no-op，不影响录音。
#[cfg(target_os = "linux")]
struct PulseVolumeControl;

#[cfg(target_os = "linux")]
impl VolumeControl for PulseVolumeControl {
    fn set_mute(&self, muted: bool) -> Result<(), VolumeError> {
        use std::process::Command;
        use std::sync::atomic::{AtomicBool, Ordering};

        /// pactl 缺失只提示一次（每次录音都刷一条 warn 没有意义）。
        static PACTL_MISSING: AtomicBool = AtomicBool::new(false);
        let value = if muted { "1" } else { "0" };
        let result = Command::new("pactl")
            .args(["set-sink-mute", "@DEFAULT_SINK@", value])
            .output();
        match result {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if !PACTL_MISSING.swap(true, Ordering::Relaxed) {
                    tracing::warn!("未找到 pactl，录音静音功能降级为 no-op（不影响录音）");
                }
                Ok(())
            }
            Err(err) => Err(VolumeError(format!("无法执行 pactl: {err}"))),
            Ok(output) if !output.status.success() => Err(VolumeError(format!(
                "pactl set-sink-mute 失败: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))),
            Ok(_) => Ok(()),
        }
    }
}

/// macOS：经 AppleScript 切换系统静音（`set volume with/without muted`）。
/// 失败仅返回错误，由调用方 warn，不影响录音。
#[cfg(target_os = "macos")]
struct MacosVolumeControl;

#[cfg(target_os = "macos")]
impl VolumeControl for MacosVolumeControl {
    fn set_mute(&self, muted: bool) -> Result<(), VolumeError> {
        use std::process::Command;
        let script = if muted {
            "set volume with muted"
        } else {
            "set volume without muted"
        };
        let output = Command::new("osascript")
            .args(["-e", script])
            .output()
            .map_err(|err| VolumeError(format!("无法执行 osascript: {err}")))?;
        if !output.status.success() {
            return Err(VolumeError(format!(
                "osascript 设置静音失败: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(())
    }
}

/// Windows：WASAPI 默认渲染设备 `IAudioEndpointVolume::SetMute`。
///
/// windows-sys 0.59 仅提供函数绑定与 GUID 常量，不含 MMDevice COM 接口定义，
/// 此处按 endpointvolume.h / mmdeviceapi.h 的 vtable 布局手写最小接口
/// （IUnknown 前缀 + 目标方法之前的全部槽位，字段顺序即虚表顺序，不得调整）。
///
/// 最小权限说明：仅 `Activate` 默认渲染端点的音量接口并调用 `SetMute`，
/// 不触碰音量电平与其它端点属性；每次调用独立 `CoInitializeEx` /
/// `CoUninitialize` 配对（`RPC_E_CHANGED_MODE` 表示线程已被初始化，直接复用）。
#[cfg(target_os = "windows")]
mod windows_impl {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    use windows_sys::core::{GUID, HRESULT};
    use windows_sys::Win32::Foundation::{BOOL, RPC_E_CHANGED_MODE, S_FALSE, S_OK};
    use windows_sys::Win32::Media::Audio::{
        eMultimedia, eRender, EDataFlow, ERole, MMDeviceEnumerator,
    };
    use windows_sys::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };

    use super::{VolumeControl, VolumeError};

    /// IMMDeviceEnumerator 接口 IID（mmdeviceapi.h）。
    const IID_IMM_DEVICE_ENUMERATOR: GUID =
        GUID::from_u128(0xa956_64d2_9614_4f35_a746_de8d_b636_17e6);
    /// IAudioEndpointVolume 接口 IID（endpointvolume.h）。
    const IID_I_AUDIO_ENDPOINT_VOLUME: GUID =
        GUID::from_u128(0x5cdf_2c82_841e_4546_9722_0cf7_4078_229a);

    type Object = *mut c_void;
    type QI =
        unsafe extern "system" fn(this: Object, riid: *const GUID, ppv: *mut Object) -> HRESULT;

    /// IUnknown 公共前缀：任意 COM 接口指针都可安全强转后调用 `release`。
    #[repr(C)]
    struct IUnknownVtbl {
        query_interface: QI,
        add_ref: unsafe extern "system" fn(this: Object) -> u32,
        release: unsafe extern "system" fn(this: Object) -> u32,
    }

    #[repr(C)]
    struct IUnknown {
        vtbl: *const IUnknownVtbl,
    }

    /// IMMDeviceEnumerator 虚表（EnumAudioEndpoints / GetDefaultAudioEndpoint /
    /// 两个通知回调槽位；我们只调用 GetDefaultAudioEndpoint）。
    #[repr(C)]
    struct IMMDeviceEnumeratorVtbl {
        query_interface: QI,
        add_ref: unsafe extern "system" fn(this: Object) -> u32,
        release: unsafe extern "system" fn(this: Object) -> u32,
        enum_audio_endpoints: unsafe extern "system" fn(
            this: Object,
            data_flow: EDataFlow,
            device_state_mask: u32,
            devices: *mut Object,
        ) -> HRESULT,
        get_default_audio_endpoint: unsafe extern "system" fn(
            this: Object,
            data_flow: EDataFlow,
            role: ERole,
            device: *mut Object,
        ) -> HRESULT,
        register_endpoint_notification_callback:
            unsafe extern "system" fn(this: Object, client: Object) -> HRESULT,
        unregister_endpoint_notification_callback:
            unsafe extern "system" fn(this: Object, client: Object) -> HRESULT,
    }

    #[repr(C)]
    struct IMMDeviceEnumerator {
        vtbl: *const IMMDeviceEnumeratorVtbl,
    }

    /// IMMDevice 虚表（Activate 之前的槽位：Activate / OpenPropertyStore /
    /// GetId / GetState；我们只调用 Activate）。
    #[repr(C)]
    struct IMMDeviceVtbl {
        query_interface: QI,
        add_ref: unsafe extern "system" fn(this: Object) -> u32,
        release: unsafe extern "system" fn(this: Object) -> u32,
        activate: unsafe extern "system" fn(
            this: Object,
            iid: *const GUID,
            cls_ctx: u32,
            activation_params: *mut c_void,
            interface: *mut Object,
        ) -> HRESULT,
        open_property_store: unsafe extern "system" fn(
            this: Object,
            access: u32,
            properties: *mut Object,
        ) -> HRESULT,
        get_id: unsafe extern "system" fn(this: Object, id: *mut *mut u16) -> HRESULT,
        get_state: unsafe extern "system" fn(this: Object, state: *mut u32) -> HRESULT,
    }

    #[repr(C)]
    struct IMMDevice {
        vtbl: *const IMMDeviceVtbl,
    }

    /// IAudioEndpointVolume 虚表（完整 21 槽位，SetMute 位于第 15 槽；
    /// 仅 RegisterControlChangeNotify 与 SetMute 有真实调用，其余保持签名占位）。
    #[repr(C)]
    struct IAudioEndpointVolumeVtbl {
        query_interface: QI,
        add_ref: unsafe extern "system" fn(this: Object) -> u32,
        release: unsafe extern "system" fn(this: Object) -> u32,
        register_control_change_notify:
            unsafe extern "system" fn(this: Object, notify: Object) -> HRESULT,
        unregister_control_change_notify:
            unsafe extern "system" fn(this: Object, notify: Object) -> HRESULT,
        get_channel_count: unsafe extern "system" fn(this: Object, count: *mut u32) -> HRESULT,
        set_master_volume_level: unsafe extern "system" fn(
            this: Object,
            level_db: f32,
            event_context: *const GUID,
        ) -> HRESULT,
        get_master_volume_level:
            unsafe extern "system" fn(this: Object, level_db: *mut f32) -> HRESULT,
        set_master_volume_level_scalar: unsafe extern "system" fn(
            this: Object,
            level: f32,
            event_context: *const GUID,
        ) -> HRESULT,
        get_master_volume_level_scalar:
            unsafe extern "system" fn(this: Object, level: *mut f32) -> HRESULT,
        set_channel_volume_level: unsafe extern "system" fn(
            this: Object,
            channel: u32,
            level_db: f32,
            event_context: *const GUID,
        ) -> HRESULT,
        get_channel_volume_level:
            unsafe extern "system" fn(this: Object, channel: u32, level_db: *mut f32) -> HRESULT,
        set_channel_volume_level_scalar: unsafe extern "system" fn(
            this: Object,
            channel: u32,
            level: f32,
            event_context: *const GUID,
        ) -> HRESULT,
        get_channel_volume_level_scalar:
            unsafe extern "system" fn(this: Object, channel: u32, level: *mut f32) -> HRESULT,
        set_mute: unsafe extern "system" fn(
            this: Object,
            mute: BOOL,
            event_context: *const GUID,
        ) -> HRESULT,
        get_mute: unsafe extern "system" fn(this: Object, mute: *mut BOOL) -> HRESULT,
        get_volume_step_info: unsafe extern "system" fn(
            this: Object,
            step: *mut u32,
            step_count: *mut u32,
        ) -> HRESULT,
        volume_step_up:
            unsafe extern "system" fn(this: Object, event_context: *const GUID) -> HRESULT,
        volume_step_down:
            unsafe extern "system" fn(this: Object, event_context: *const GUID) -> HRESULT,
        query_hardware_support_mask:
            unsafe extern "system" fn(this: Object, mask: *mut u32) -> HRESULT,
        get_volume_range: unsafe extern "system" fn(
            this: Object,
            min_db: *mut f32,
            max_db: *mut f32,
            step_db: *mut f32,
        ) -> HRESULT,
    }

    #[repr(C)]
    struct IAudioEndpointVolume {
        vtbl: *const IAudioEndpointVolumeVtbl,
    }

    /// 释放 COM 接口指针（IUnknown 前缀布局共享，任意接口可安全强转）。
    unsafe fn release(ptr: Object) {
        if !ptr.is_null() {
            let unknown = &*(ptr as *mut IUnknown);
            ((*(unknown.vtbl)).release)(ptr);
        }
    }

    /// 设置默认渲染端点静音（COM 初始化 / 反初始化配对在最外层完成）。
    unsafe fn set_default_render_mute(muted: bool) -> Result<(), VolumeError> {
        let mut enumerator: Object = null_mut();
        // SAFETY：CoCreateInstance 按 CLSID 创建进程内 COM 对象；CLSCTX_ALL 仅
        // 进程内 / 本地服务，不触发网络激活。输出指针在成功后非空。
        let hr = CoCreateInstance(
            &MMDeviceEnumerator,
            null_mut(),
            CLSCTX_ALL,
            &IID_IMM_DEVICE_ENUMERATOR,
            &mut enumerator,
        );
        if hr != S_OK || enumerator.is_null() {
            return Err(VolumeError(format!(
                "创建 MMDeviceEnumerator 失败: 0x{hr:08X}"
            )));
        }

        let mut device: Object = null_mut();
        // SAFETY：enumerator 是刚创建的 IMMDeviceEnumerator 有效指针；device 为
        // 输出指针，成功后由本函数负责 release。
        let hr = ((*((*enumerator.cast::<IMMDeviceEnumerator>()).vtbl)).get_default_audio_endpoint)(
            enumerator,
            eRender,
            eMultimedia,
            &mut device,
        );
        release(enumerator);
        if hr != S_OK || device.is_null() {
            return Err(VolumeError(format!("获取默认输出设备失败: 0x{hr:08X}")));
        }

        let mut endpoint: Object = null_mut();
        // SAFETY：device 为 IMMDevice 有效指针；Activate 返回的 endpoint 由本
        // 函数负责 release。
        let hr = ((*((*device.cast::<IMMDevice>()).vtbl)).activate)(
            device,
            &IID_I_AUDIO_ENDPOINT_VOLUME,
            CLSCTX_ALL,
            null_mut(),
            &mut endpoint,
        );
        release(device);
        if hr != S_OK || endpoint.is_null() {
            return Err(VolumeError(format!(
                "激活 IAudioEndpointVolume 失败: 0x{hr:08X}"
            )));
        }

        // SAFETY：endpoint 为 IAudioEndpointVolume 有效指针；event_context 传空
        // 表示不标记事件来源。
        let hr = ((*((*endpoint.cast::<IAudioEndpointVolume>()).vtbl)).set_mute)(
            endpoint,
            i32::from(muted),
            null(),
        );
        release(endpoint);
        if hr != S_OK {
            return Err(VolumeError(format!("SetMute 失败: 0x{hr:08X}")));
        }
        Ok(())
    }

    pub struct WindowsVolumeControl;

    impl VolumeControl for WindowsVolumeControl {
        fn set_mute(&self, muted: bool) -> Result<(), VolumeError> {
            // SAFETY：CoInitializeEx / CoUninitialize 配对使用；线程已被其他
            // 组件以不同并发模型初始化（RPC_E_CHANGED_MODE）时计数未增加，
            // 直接复用现有套间。
            unsafe {
                let init_hr = CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
                let initialized = init_hr == S_OK || init_hr == S_FALSE;
                let result = set_default_render_mute(muted);
                if initialized {
                    CoUninitialize();
                }
                if init_hr != S_OK && init_hr != S_FALSE && init_hr != RPC_E_CHANGED_MODE {
                    return Err(VolumeError(format!("COM 初始化失败: 0x{init_hr:08X}")));
                }
                result
            }
        }
    }
}

#[cfg(target_os = "windows")]
use windows_impl::WindowsVolumeControl;

/// 测试用音量控制 mock：记录 `set_mute` 调用序列（与 MockAudioBackend 同一约定，
/// test-utils feature 下编译进 lib 供集成测试注入）。
#[cfg(any(test, feature = "test-utils"))]
pub struct MockVolumeControl {
    calls: std::sync::Mutex<Vec<bool>>,
}

#[cfg(any(test, feature = "test-utils"))]
impl MockVolumeControl {
    pub fn new() -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// 已记录的调用序列（`true` = 静音，`false` = 恢复）。
    pub fn calls(&self) -> Vec<bool> {
        self.calls
            .lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl Default for MockVolumeControl {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl VolumeControl for MockVolumeControl {
    fn set_mute(&self, muted: bool) -> Result<(), VolumeError> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(muted);
        }
        Ok(())
    }
}
