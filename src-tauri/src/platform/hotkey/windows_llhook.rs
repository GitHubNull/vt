//! Windows 低级键盘钩子后端（WH_KEYBOARD_LL）：修饰键单独作为触发键的覆盖路径。
//!
//! 背景（tech/09 9.2.1）：global-hotkey 0.8 的 Windows 实现走 `RegisterHotKey`，
//! 其 `key_to_vk` 不映射任何修饰键 Code（AltLeft / AltRight / ControlLeft…），
//! 且 `RegisterHotKey` 对裸修饰键本身不可靠（右 Alt 同时是 AltGr），
//! 因此默认触发键"右 Alt"在 Windows 上注册即失败、按键毫无响应。
//! 本模块用低级键盘钩子覆盖"单个左右修饰键作触发键"的场景；
//! 组合键 / 功能键仍走 global-shortcut 的 RegisterHotKey 路径。
//!
//! 产品规则实现（PRD 3.1.1）：
//! - 规则 3：按下触发键后又按其他键（含其他修饰键，如 AltGr 合成的左 Ctrl）
//!   视为组合键，不触发录音；
//! - 规则 4：按住不足 0.3 秒视为误触，不触发；0.3 秒后仍按住才发出 Pressed。
//!
//! 最小权限说明（AGENTS.md 3.5）：钩子只比对目标修饰键的按下 / 松开并上报事件，
//! 不记录、不拦截其他按键（一律 `CallNextHookEx` 透传）。

use std::sync::{mpsc as std_mpsc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::mpsc;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, PM_NOREMOVE, WH_KEYBOARD_LL, WM_KEYDOWN,
    WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use super::{HotkeyError, RawHotkeyEvent};

// Win32 虚拟键码（固定值，自定义常量避免 windows-sys 版本类型差异）。
const VK_LSHIFT: u32 = 0xA0;
const VK_RSHIFT: u32 = 0xA1;
const VK_LCONTROL: u32 = 0xA2;
const VK_RCONTROL: u32 = 0xA3;
const VK_LMENU: u32 = 0xA4;
const VK_RMENU: u32 = 0xA5;
/// KBDLLHOOKSTRUCT.flags：事件由 SendInput 等注入（跳过，避免自我触发 / 回放误触发）。
const LLKHF_INJECTED: u32 = 0x0000_0010;
/// PRD 3.1.1 规则 4：按住不足该时长视为误触。
const MISFIRE_HOLD: Duration = Duration::from_millis(300);

/// 钩子进程内共享状态（SetWindowsHookEx 回调无用户指针，用进程级静态）。
struct LlState {
    /// 目标修饰键 VK（仅单个修饰键触发走本路径）。
    vk: u32,
    /// 原始按键事件出口（与 global-shortcut 路径同一个通道）。
    sender: mpsc::Sender<RawHotkeyEvent>,
    /// 误触门：Some(gen) 表示已按下、正在 300 ms 观察窗内；gen 用于使过期定时器失效。
    armed: Option<u64>,
    /// 单调代数：每次状态翻转自增，使上一个观察窗 / 定时器失效。
    gen: u64,
    /// 已发出 Pressed、未发 Released。
    triggered: bool,
}

static STATE: OnceLock<Mutex<Option<LlState>>> = OnceLock::new();

fn state() -> &'static Mutex<Option<LlState>> {
    STATE.get_or_init(|| Mutex::new(None))
}

/// 物理按住判定（GetAsyncKeyState 高位）。
unsafe fn held(vk: u32) -> bool {
    (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0
}

/// 除目标键外的其他修饰键是否处于按住状态（AltGr 会合成按住左 Ctrl）。
unsafe fn other_modifier_held(target: u32) -> bool {
    held(VK_LSHIFT)
        || held(VK_RSHIFT)
        || held(VK_LCONTROL)
        || held(VK_RCONTROL)
        || (target != VK_LMENU && held(VK_LMENU))
        || (target != VK_RMENU && held(VK_RMENU))
}

/// 300 ms 误触门到期检查：观察窗未被取消且键仍按住才发出 Pressed。
fn misfire_timer(gen: u64) {
    std::thread::sleep(MISFIRE_HOLD);
    if let Ok(mut guard) = state().lock() {
        let Some(st) = guard.as_mut() else { return };
        if st.armed == Some(gen) && !st.triggered && unsafe { held(st.vk) } {
            st.armed = None;
            st.triggered = true;
            if st.sender.try_send(RawHotkeyEvent::Pressed).is_err() {
                tracing::warn!("快捷键事件通道已满，丢弃事件");
            }
        }
    }
}

unsafe extern "system" fn hook_proc(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if ncode >= 0 && lparam != 0 {
        let msg = wparam as u32;
        let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
        if is_down || is_up {
            // 右 Alt 产生 WM_SYSKEY*，必须一并处理。
            let info = &*(lparam as *const KBDLLHOOKSTRUCT);
            if info.flags & LLKHF_INJECTED == 0 {
                if let Ok(mut guard) = state().lock() {
                    if let Some(st) = guard.as_mut() {
                        if info.vkCode == st.vk {
                            if is_down {
                                if !st.triggered && st.armed.is_none() {
                                    if other_modifier_held(st.vk) {
                                        // 组合键形态（如 AltGr 合成 Ctrl+Alt）：本次按住不触发。
                                        st.gen += 1;
                                    } else {
                                        st.gen += 1;
                                        let gen = st.gen;
                                        st.armed = Some(gen);
                                        std::thread::spawn(move || misfire_timer(gen));
                                    }
                                }
                            } else if st.triggered {
                                st.triggered = false;
                                st.gen += 1;
                                if st.sender.try_send(RawHotkeyEvent::Released).is_err() {
                                    tracing::warn!("快捷键事件通道已满，丢弃事件");
                                }
                            } else {
                                // 300 ms 内松开：误触（PRD 规则 4），取消观察窗。
                                st.armed = None;
                                st.gen += 1;
                            }
                        } else if is_down && st.armed.is_some() {
                            // 观察窗内按了其他键：组合键（PRD 规则 3），取消触发。
                            st.armed = None;
                            st.gen += 1;
                        }
                    }
                }
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam)
}

/// 钩子线程：安装钩子 → 就绪信号 → 消息循环（LL 钩子必须持续泵消息，
/// 否则系统按 LowLevelHooksTimeout 静默摘除）。
unsafe fn hook_thread(
    vk: u32,
    events: mpsc::Sender<RawHotkeyEvent>,
    ready: std_mpsc::Sender<Result<u32, HotkeyError>>,
) {
    let hook: HHOOK = SetWindowsHookExW(
        WH_KEYBOARD_LL,
        Some(hook_proc),
        GetModuleHandleW(std::ptr::null()),
        0,
    );
    if hook.is_null() {
        let err = std::io::Error::last_os_error();
        let _ = ready.send(Err(HotkeyError::Platform(format!(
            "安装低级键盘钩子失败: {err}"
        ))));
        return;
    }
    // 先建立消息队列再发就绪，避免 stop() 的 PostThreadMessageW 因线程无队列而失败。
    let mut msg: MSG = std::mem::zeroed();
    PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
    *state().lock().unwrap() = Some(LlState {
        vk,
        sender: events,
        armed: None,
        gen: 0,
        triggered: false,
    });
    let _ = ready.send(Ok(GetCurrentThreadId()));
    // GetMessageW 遇 WM_QUIT 返回 0、出错返回非正值，均退出循环。
    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
    UnhookWindowsHookEx(hook);
}

struct LlThread {
    id: u32,
    join: std::thread::JoinHandle<()>,
}

/// 低级钩子后端：管理钩子线程生命周期；`HotkeyBackend` 的组合实现按需调用。
pub struct LlHookBackend {
    inner: Mutex<Option<LlThread>>,
}

impl Default for LlHookBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LlHookBackend {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }
    /// 以目标修饰键 VK 启动钩子（重复调用会先停旧钩子）。
    pub fn start(&self, vk: u32, events: mpsc::Sender<RawHotkeyEvent>) -> Result<(), HotkeyError> {
        self.stop();
        let (ready_tx, ready_rx) = std_mpsc::channel();
        let join = std::thread::Builder::new()
            .name("vt-llhotkey".into())
            .spawn(move || unsafe { hook_thread(vk, events, ready_tx) })
            .map_err(|err| HotkeyError::Platform(format!("钩子线程启动失败: {err}")))?;
        match ready_rx.recv() {
            Ok(Ok(id)) => {
                *self.inner.lock().unwrap() = Some(LlThread { id, join });
                Ok(())
            }
            Ok(Err(err)) => {
                let _ = join.join();
                Err(err)
            }
            Err(_) => {
                let _ = join.join();
                Err(HotkeyError::Platform("钩子线程未就绪即退出".into()))
            }
        }
    }

    /// 停止钩子并清空全局状态（幂等）。
    pub fn stop(&self) {
        if let Some(thread) = self.inner.lock().unwrap().take() {
            unsafe {
                let _ = PostThreadMessageW(thread.id, WM_QUIT, 0, 0);
            }
            let _ = thread.join.join();
        }
        if let Ok(mut guard) = state().lock() {
            *guard = None;
        }
    }
}

impl Drop for LlHookBackend {
    fn drop(&mut self) {
        self.stop();
    }
}
