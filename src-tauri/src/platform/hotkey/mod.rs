//! 全局快捷键平台适配层（tech/04 4.5、tech/09 9.2.1）。
//!
//! 三平台统一基于 Tauri `global-shortcut` 插件（其底层：Windows RegisterHotKey /
//! macOS NSEvent 全局监听 / Linux X11 XRecord）。平台差异说明：
//! - macOS：全局监听需要"辅助功能"权限，缺失时注册失败 → 由服务层降级为
//!   错误事件（E-HOTKEY-001）并引导授权（tech/09 9.4），不崩溃；
//! - Windows：单独右 Alt 作为快捷键可能受系统保留影响，注册失败时引导换键
//!   （tech/09 9.2.1，v0.1 不做低级钩子）;
//! - Linux Wayland：协议限制无法全局监听，注册失败并提示（v1.0 提供替代方案）。
//!
//! 最小权限说明（AGENTS.md 3.5）：本能力仅注册用户配置的全局快捷键并接收其
//! 按下 / 松开事件，不拦截、不记录其他按键。

use std::sync::Arc;

use tauri_plugin_global_shortcut::GlobalShortcutExt;
use tokio::sync::mpsc;

/// 后端上报的原始按键事件（Pressed / Released）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawHotkeyEvent {
    Pressed,
    Released,
}

/// 平台后端错误（命令层边界统一转换为 AppError）。
#[derive(Debug, thiserror::Error)]
pub enum HotkeyError {
    /// 快捷键串非法（如未知键名）。
    #[error("快捷键格式非法: {0}")]
    InvalidAccelerator(String),
    /// 被其他应用占用 / 系统保留（含 macOS 辅助功能权限缺失）。
    #[error("快捷键注册失败（被占用或权限缺失）")]
    RegisterFailed,
    /// 其他平台错误。
    #[error("平台错误: {0}")]
    Platform(String),
}

/// 全局快捷键后端抽象：`register` 成功后事件经构造时给定的通道上报。
pub trait HotkeyBackend: Send + Sync {
    /// 注册快捷键；事件通过 `RawHotkeyEvent` 通道上报（Pressed / Released）。
    fn register(&self, accelerator: &str) -> Result<(), HotkeyError>;
    /// 注销全部已注册快捷键。
    fn unregister_all(&self) -> Result<(), HotkeyError>;
}

/// 三平台统一实现：Tauri `global-shortcut` 插件。
struct TauriHotkeyBackend {
    app: tauri::AppHandle,
    /// 原始按键事件出口（容量 8，控制类通道满时丢弃并记日志）。
    events: mpsc::Sender<RawHotkeyEvent>,
}

impl HotkeyBackend for TauriHotkeyBackend {
    fn register(&self, accelerator: &str) -> Result<(), HotkeyError> {
        use tauri_plugin_global_shortcut::ShortcutState;

        let events = self.events.clone();
        let shortcut = accelerator.to_string();
        // 注册全局快捷键：系统级能力，仅监听用户指定的组合键（最小权限见模块头注释）。
        self.app
            .global_shortcut()
            .on_shortcut(shortcut.as_str(), move |_app, _shortcut, event| {
                // 回调运行在全局热键线程；try_send 不阻塞，通道满则丢弃该事件。
                let raw = match event.state {
                    ShortcutState::Pressed => RawHotkeyEvent::Pressed,
                    ShortcutState::Released => RawHotkeyEvent::Released,
                };
                if events.try_send(raw).is_err() {
                    tracing::warn!("快捷键事件通道已满，丢弃事件");
                }
            })
            .map_err(|err| {
                if err.to_string().contains("Parse") || err.to_string().contains("parse") {
                    HotkeyError::InvalidAccelerator(accelerator.to_string())
                } else {
                    HotkeyError::RegisterFailed
                }
            })
    }

    fn unregister_all(&self) -> Result<(), HotkeyError> {
        self.app
            .global_shortcut()
            .unregister_all()
            .map_err(|err| HotkeyError::Platform(err.to_string()))
    }
}

/// 创建三平台默认后端（tech/04 4.5 工厂函数）。
pub fn default_backend(
    app: tauri::AppHandle,
    events: mpsc::Sender<RawHotkeyEvent>,
) -> Arc<dyn HotkeyBackend> {
    Arc::new(TauriHotkeyBackend { app, events })
}

#[cfg(test)]
pub use mock::MockHotkeyBackend;

/// 测试用 mock 后端（tech/04 4.5：服务层单测不依赖真实 OS 能力）。
#[cfg(test)]
pub mod mock {
    use super::*;
    use std::sync::Mutex;

    /// 记录注册历史的 mock；`fail_next` 置位时下一次注册失败（模拟占用冲突）。
    pub struct MockHotkeyBackend {
        pub registered: Mutex<Vec<String>>,
        pub fail_next: std::sync::atomic::AtomicBool,
    }

    impl MockHotkeyBackend {
        pub fn new() -> Self {
            Self {
                registered: Mutex::new(Vec::new()),
                fail_next: std::sync::atomic::AtomicBool::new(false),
            }
        }

        /// 当前已注册的唯一快捷键（v0.1 只注册一个录音触发键）。
        pub fn current(&self) -> Option<String> {
            self.registered.lock().unwrap().last().cloned()
        }
    }

    impl HotkeyBackend for MockHotkeyBackend {
        fn register(&self, accelerator: &str) -> Result<(), HotkeyError> {
            if self
                .fail_next
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(HotkeyError::RegisterFailed);
            }
            self.registered
                .lock()
                .unwrap()
                .push(accelerator.to_string());
            Ok(())
        }

        fn unregister_all(&self) -> Result<(), HotkeyError> {
            self.registered.lock().unwrap().clear();
            Ok(())
        }
    }
}
