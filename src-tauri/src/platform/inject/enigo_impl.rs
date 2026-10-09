//! enigo 统一注入器：Windows SendInput / macOS CGEvent / Linux X11 XTest
//! （tech/09 9.2.2；Wayland 下 enigo 不可用时构造或调用报错，由服务层降级到
//! 剪贴板路径——剪贴板路径为主，符合 tech/09 对 Wayland 的受限标注）。
//!
//! 最小权限说明（AGENTS.md 3.5）：仅发送文本键入与 Ctrl/Cmd+V 粘贴快捷键，
//! 不暴露回车 / 执行类按键（NFR-S-05）；剪贴板经 Tauri 内置插件读写
//! （tech/02 2.3），仅用于粘贴路径的短时备份 / 写入 / 恢复。

use std::sync::Mutex;

use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tracing::warn;

use super::{ClipboardBackup, FocusState, InjectError, TextInjector};

/// enigo + Tauri 剪贴板插件的默认注入器。
pub struct EnigoTextInjector {
    /// enigo 非线程安全（内部连接状态），Mutex 串行化调用。
    enigo: Mutex<Enigo>,
    app: tauri::AppHandle,
}

impl EnigoTextInjector {
    pub fn new(app: tauri::AppHandle) -> Result<Self, InjectError> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|err| InjectError::Platform(format!("无法初始化输入模拟: {err}")))?;
        Ok(Self {
            enigo: Mutex::new(enigo),
            app,
        })
    }

    fn enigo(&self) -> std::sync::MutexGuard<'_, Enigo> {
        self.enigo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 模拟 Ctrl+V（macOS 为 Cmd+V）。
    fn press_paste_shortcut(&self) -> Result<(), InjectError> {
        let mut enigo = self.enigo();
        let modifier = if cfg!(target_os = "macos") {
            Key::Meta
        } else {
            Key::Control
        };
        enigo
            .key(modifier, Direction::Press)
            .map_err(|err| InjectError::Paste(err.to_string()))?;
        let result = enigo.key(Key::Unicode('v'), Direction::Click);
        let release = enigo.key(modifier, Direction::Release);
        result.map_err(|err| InjectError::Paste(err.to_string()))?;
        release.map_err(|err| InjectError::Paste(err.to_string()))
    }
}

impl TextInjector for EnigoTextInjector {
    fn type_text(&self, text: &str) -> Result<(), InjectError> {
        check_typing_permission()?;
        self.enigo()
            .text(text)
            .map_err(|err| InjectError::Type(err.to_string()))
    }

    fn clipboard_backup(&self) -> Result<ClipboardBackup, InjectError> {
        // 空剪贴板 / 非文本内容（图片、文件）读取文本会失败：视为 Unsupported，
        // 粘贴路径仍可执行，恢复阶段记警告（见模块文档"受限说明"）。
        match self.app.clipboard().read_text() {
            Ok(text) => Ok(ClipboardBackup::Text(text)),
            Err(_) => Ok(ClipboardBackup::Unsupported),
        }
    }

    fn write_clipboard(&self, text: &str) -> Result<(), InjectError> {
        self.app
            .clipboard()
            .write_text(text.to_string())
            .map_err(|err| InjectError::Clipboard(err.to_string()))
    }

    fn simulate_paste(&self) -> Result<(), InjectError> {
        self.press_paste_shortcut()
    }

    fn clipboard_restore(&self, backup: &ClipboardBackup) -> Result<(), InjectError> {
        match backup {
            ClipboardBackup::Text(text) => self
                .app
                .clipboard()
                .write_text(text.clone())
                .map_err(|err| InjectError::Clipboard(err.to_string())),
            ClipboardBackup::Unsupported => {
                // v0.1 受限：非文本剪贴板内容无法经文本路径恢复，保持现状并提示。
                warn!("剪贴板原内容非文本，无法自动恢复");
                Ok(())
            }
        }
    }

    fn input_focus(&self) -> FocusState {
        detect_input_focus()
    }

    fn focused_app(&self) -> Option<String> {
        // 场景识别属 v0.5 T-501（本任务范围外），v0.1 一律返回 None。
        None
    }
}

/// 模拟键入前的权限检查：macOS 需辅助功能授权，未授权时返回 Permission
/// 错误，由服务层降级剪贴板路径并引导授权（SPEC T-007 步骤 4）。
#[cfg(target_os = "macos")]
fn check_typing_permission() -> Result<(), InjectError> {
    // SAFETY：AXIsProcessTrusted 无参数、无副作用，只查询本进程授权状态。
    if unsafe { accessibility_sys::AXIsProcessTrusted() } {
        Ok(())
    } else {
        Err(InjectError::Permission(
            "请在 系统设置 › 隐私与安全性 › 辅助功能 中授权 VibeType".to_string(),
        ))
    }
}

#[cfg(not(target_os = "macos"))]
fn check_typing_permission() -> Result<(), InjectError> {
    Ok(())
}

/// 检测当前是否存在输入焦点（各平台启发式，无法判定时返回 Unknown）。
#[cfg(target_os = "windows")]
fn detect_input_focus() -> FocusState {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    // SAFETY：GetForegroundWindow 只读查询前台窗口句柄，无副作用。
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        FocusState::Unfocused
    } else {
        FocusState::Focused
    }
}

#[cfg(target_os = "macos")]
fn detect_input_focus() -> FocusState {
    use accessibility_sys::{
        kAXErrorSuccess, AXUIElementCopyAttributeValue, AXUIElementCreateSystemWide,
    };
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;
    // SAFETY：创建系统级 AXUIElement 并查询"焦点 UI 元素"，均为只读操作。
    let system_wide = unsafe { AXUIElementCreateSystemWide() };
    let attribute = CFString::new("AXFocusedUIElement");
    let mut value: core_foundation::base::CFTypeRef = std::ptr::null();
    // SAFETY：system_wide 由系统函数创建；value 接收输出引用。
    let error = unsafe {
        AXUIElementCopyAttributeValue(system_wide, attribute.as_concrete_TypeRef(), &mut value)
    };
    // SAFETY：value 非空时为由系统函数创建的新引用，需释放避免泄漏。
    if error == kAXErrorSuccess && !value.is_null() {
        unsafe { core_foundation::base::CFRelease(value) };
        FocusState::Focused
    } else {
        FocusState::Unfocused
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn detect_input_focus() -> FocusState {
    // Wayland：compositor 不提供全局焦点查询（tech/09 9.2.2 受限标注）。
    if std::env::var_os("DISPLAY").is_none() {
        return FocusState::Unknown;
    }
    let Ok((connection, _screen)) = x11rb::connect(None) else {
        return FocusState::Unknown;
    };
    use x11rb::protocol::xproto::ConnectionExt as _;
    let Ok(cookie) = connection.get_input_focus() else {
        return FocusState::Unknown;
    };
    let Ok(reply) = cookie.reply() else {
        return FocusState::Unknown;
    };
    // focus = 0（None）表示无焦点窗口；1（PointerRoot）表示焦点跟随指针，视为有焦点。
    match reply.focus {
        0 => FocusState::Unfocused,
        _ => FocusState::Focused,
    }
}
