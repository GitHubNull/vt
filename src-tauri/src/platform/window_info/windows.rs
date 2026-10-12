//! Windows 前台窗口识别（T-501，tech/09 9.2.3）。
//!
//! 调用链：`GetForegroundWindow` → `GetWindowThreadProcessId` 取前台窗口归属 PID →
//! `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` + `QueryFullProcessImageNameW`
//! 取进程可执行名 → `GetWindowTextW` 取窗口标题。
//!
//! [`AppInfo`] 字段语义（与 mod.rs 的平台语义注释一致）：
//! - `process`：进程可执行名去扩展名（如 "code"、"WindowsTerminal"），规则库主匹配键；
//! - `name`：与 `process` 相同（Windows 侧获取友好显示名需版本资源 API 的
//!   `Win32_System_Version` feature，为保持最小权限与既有依赖不变暂不引入；
//!   规则匹配按 process / name 双字段等值匹配，此实现不影响匹配正确性）；
//! - `title`：窗口标题（不入日志，NFR-S-02）。
//!
//! 最小权限说明（AGENTS.md 3.5）：仅以 `PROCESS_QUERY_LIMITED_INFORMATION`
//! 查询前台窗口归属进程的镜像路径与窗口标题（user32 只读 API）；不打开其他
//! 进程的读写句柄、不注入消息、不枚举窗口历史；识别信息不写日志（NFR-S-02）。

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
};

use super::AppInfo;

/// 前台窗口信息提供者（Windows 实现）。
pub struct WindowsWindowInfo;

impl super::WindowInfoProvider for WindowsWindowInfo {
    fn active_app(&self) -> AppInfo {
        // SAFETY：以下均为 user32 / kernel32 只读查询调用，参数为句柄与输出缓冲区，
        // 无可导致进程级状态破坏的调用序列；前台窗口为 0（锁屏 / 无交互会话）时
        // 各查询自然返回空值，走空 `AppInfo` 降级路径。
        unsafe {
            let window = GetForegroundWindow();
            if window.is_null() {
                return AppInfo::default();
            }
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(window, &mut pid);
            if pid == 0 {
                return AppInfo {
                    name: String::new(),
                    process: String::new(),
                    title: window_title(window),
                };
            }
            let process = process_name(pid).unwrap_or_default();
            let name = process.clone();
            AppInfo {
                name,
                process,
                title: window_title(window),
            }
        }
    }
}

/// 进程可执行名（去目录与 `.exe` 扩展名；查询失败返回 `None`）。
fn process_name(pid: u32) -> Option<String> {
    // SAFETY：OpenProcess 仅申请受限查询权限（PROCESS_QUERY_LIMITED_INFORMATION，
    // 无 UAC 提权需求）；句柄为 0（权限拒绝 / 进程退出）时返回 None 降级。
    let handle: HANDLE = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let mut buffer = [0u16; 1024];
    let mut len = buffer.len() as u32;
    // SAFETY：buffer / len 为输出缓冲与容量指针，符合 QueryFullProcessImageNameW 契约。
    let ok = unsafe {
        QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut len)
    };
    // SAFETY：本函数创建的句柄必须配对关闭。
    unsafe { CloseHandle(handle) };
    if ok == 0 || len == 0 {
        return None;
    }
    let path = String::from_utf16_lossy(&buffer[..len as usize]);
    // 末段为文件名（镜像路径分隔符统一为 `\`，容错处理 `/`）。
    let file_name = path.rsplit(['\\', '/']).next().unwrap_or(&path);
    Some(
        file_name
            .strip_suffix(".exe")
            .unwrap_or(file_name)
            .to_string(),
    )
}

/// 窗口标题（失败 / 无标题返回空串；标题不写日志，NFR-S-02）。
fn window_title(window: windows_sys::Win32::Foundation::HWND) -> String {
    // SAFETY：GetWindowTextLengthW / GetWindowTextW 为成对的只读文本查询；
    // 缓冲区按返回长度 + 1（结尾 NUL）分配，调用后按实际拷贝数截断。
    unsafe {
        let len = GetWindowTextLengthW(window);
        if len <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; (len + 1) as usize];
        let copied = GetWindowTextW(window, buffer.as_mut_ptr(), len + 1);
        if copied <= 0 {
            return String::new();
        }
        buffer.truncate(copied as usize);
        String::from_utf16_lossy(&buffer)
    }
}
