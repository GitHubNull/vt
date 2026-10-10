//! 测试用文本注入 mock（tech/04 4.5：服务层单测不依赖真实输入设备 / 显示服务器）。
// test-utils feature 下编译进 lib（非 cfg(test)）：unwrap 仅出现在测试替身
// （锁中毒即测试逻辑错误，panic 可接受），与 cfg(test) 语义一致。
#![allow(clippy::unwrap_used)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use super::{ClipboardBackup, FocusState, InjectError};

/// 记录一次模拟粘贴快捷键（断言"只发 Ctrl/Cmd+V，不发回车"）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteShortcut {
    pub modifier: &'static str,
    pub key: char,
}

/// mock 注入器：全路径可控、全调用可断言。
///
/// 行为开关用内部可变性（`Arc<MockTextInjector>` 共享时也可调整）。
pub struct MockTextInjector {
    pub typed: Mutex<Vec<String>>,
    pub backups: Mutex<Vec<()>>,
    pub clipboard: Mutex<String>,
    pub written: Mutex<Vec<String>>,
    pub pastes: Mutex<Vec<PasteShortcut>>,
    pub restores: Mutex<Vec<ClipboardBackup>>,
    /// 置位时 type_text 返回错误（模拟目标应用不接受键入）。
    pub fail_type: AtomicBool,
    /// 置位时 type_text 返回权限错误（模拟 macOS 未授权辅助功能）。
    pub type_permission_denied: AtomicBool,
    /// 置位时 write_clipboard 失败。
    pub fail_clipboard_write: AtomicBool,
    /// 置位时 simulate_paste 失败。
    pub fail_paste: AtomicBool,
    pub focus: Mutex<FocusState>,
    pub focused_app: Mutex<Option<String>>,
    /// 调用顺序日志（断言"写入 → 粘贴 → 恢复"顺序）。
    pub ops: Mutex<Vec<&'static str>>,
    /// simulate_paste 发生时间（断言粘贴后经过驻留时延才恢复剪贴板）。
    pub paste_at: Mutex<Option<std::time::Instant>>,
    /// clipboard_restore 发生时间。
    pub restore_at: Mutex<Option<std::time::Instant>>,
}

impl Default for MockTextInjector {
    fn default() -> Self {
        Self {
            typed: Mutex::new(Vec::new()),
            backups: Mutex::new(Vec::new()),
            clipboard: Mutex::new("原剪贴板内容".to_string()),
            written: Mutex::new(Vec::new()),
            pastes: Mutex::new(Vec::new()),
            restores: Mutex::new(Vec::new()),
            fail_type: AtomicBool::new(false),
            type_permission_denied: AtomicBool::new(false),
            fail_clipboard_write: AtomicBool::new(false),
            fail_paste: AtomicBool::new(false),
            focus: Mutex::new(FocusState::Focused),
            focused_app: Mutex::new(None),
            ops: Mutex::new(Vec::new()),
            paste_at: Mutex::new(None),
            restore_at: Mutex::new(None),
        }
    }
}

impl MockTextInjector {
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置焦点状态（`Arc` 共享下可调用）。
    pub fn set_focus(&self, state: FocusState) {
        *self.focus.lock().unwrap() = state;
    }

    /// 设置前台应用名（`Arc` 共享下可调用）。
    pub fn set_focused_app(&self, app: Option<String>) {
        *self.focused_app.lock().unwrap() = app;
    }

    /// 当前 mock 剪贴板内容（验证恢复 / 保留行为）。
    pub fn clipboard_content(&self) -> String {
        self.clipboard.lock().unwrap().clone()
    }

    /// 断言从未发生模拟键入 / 粘贴。
    pub fn assert_no_input(&self) {
        assert!(self.typed.lock().unwrap().is_empty(), "不应发生模拟键入");
        assert!(self.pastes.lock().unwrap().is_empty(), "不应发生模拟粘贴");
    }
}

impl super::TextInjector for MockTextInjector {
    fn type_text(&self, text: &str) -> Result<(), InjectError> {
        if self.type_permission_denied.load(Ordering::SeqCst) {
            return Err(InjectError::Permission("辅助功能未授权".to_string()));
        }
        if self.fail_type.load(Ordering::SeqCst) {
            return Err(InjectError::Type("目标应用拒绝键入".to_string()));
        }
        // 模拟真实边界：键入前同样过清洗（防实现绕过）。
        self.typed.lock().unwrap().push(text.to_string());
        Ok(())
    }

    fn clipboard_backup(&self) -> Result<ClipboardBackup, InjectError> {
        self.ops.lock().unwrap().push("backup");
        self.backups.lock().unwrap().push(());
        Ok(ClipboardBackup::Text(
            self.clipboard.lock().unwrap().clone(),
        ))
    }

    fn write_clipboard(&self, text: &str) -> Result<(), InjectError> {
        self.ops.lock().unwrap().push("write");
        if self.fail_clipboard_write.load(Ordering::SeqCst) {
            return Err(InjectError::Clipboard("剪贴板写入失败".to_string()));
        }
        self.written.lock().unwrap().push(text.to_string());
        *self.clipboard.lock().unwrap() = text.to_string();
        Ok(())
    }

    fn simulate_paste(&self) -> Result<(), InjectError> {
        if self.fail_paste.load(Ordering::SeqCst) {
            return Err(InjectError::Paste("粘贴快捷键发送失败".to_string()));
        }
        // modifier 由 cfg 决定，与 enigo_impl 保持一致。
        let modifier = if cfg!(target_os = "macos") {
            "meta"
        } else {
            "ctrl"
        };
        self.ops.lock().unwrap().push("paste");
        self.pastes
            .lock()
            .unwrap()
            .push(PasteShortcut { modifier, key: 'v' });
        *self.paste_at.lock().unwrap() = Some(std::time::Instant::now());
        Ok(())
    }

    fn clipboard_restore(&self, backup: &ClipboardBackup) -> Result<(), InjectError> {
        self.ops.lock().unwrap().push("restore");
        *self.restore_at.lock().unwrap() = Some(std::time::Instant::now());
        self.restores.lock().unwrap().push(backup.clone());
        if let ClipboardBackup::Text(text) = backup {
            *self.clipboard.lock().unwrap() = text.clone();
        }
        Ok(())
    }

    fn input_focus(&self) -> FocusState {
        *self.focus.lock().unwrap()
    }

    fn focused_app(&self) -> Option<String> {
        self.focused_app.lock().unwrap().clone()
    }
}
