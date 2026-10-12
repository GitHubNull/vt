//! 测试用前台窗口信息 mock（tech/04 4.5：服务层单测不依赖真实窗口系统）。
//!
//! 与 `MockAudioBackend` 同一约定：`any(test, feature = "test-utils")` 门控编译，
//! 生产构建不包含本文件。集成测试（tests/、benches/）经 test-utils feature 注入。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use super::{AppInfo, WindowInfoProvider};

/// mock 前台窗口信息提供者：返回预设的 [`AppInfo`]，并统计 `active_app` 调用次数
/// （供 SceneService 缓存行为的单测断言）。
pub struct MockWindowInfoProvider {
    app: Mutex<AppInfo>,
    calls: AtomicU64,
}

impl MockWindowInfoProvider {
    pub fn new() -> Self {
        Self::with_app(AppInfo::default())
    }

    /// 以预设的前台应用信息构造。
    pub fn with_app(app: AppInfo) -> Self {
        Self {
            app: Mutex::new(app),
            calls: AtomicU64::new(0),
        }
    }

    /// 更新前台应用信息（模拟用户切换前台窗口）。
    pub fn set_app(&self, app: AppInfo) {
        if let Ok(mut app_slot) = self.app.lock() {
            *app_slot = app;
        }
    }

    /// `active_app` 已被调用的次数（缓存命中不应增加计数）。
    pub fn call_count(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Default for MockWindowInfoProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowInfoProvider for MockWindowInfoProvider {
    fn active_app(&self) -> AppInfo {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.app.lock().map(|app| app.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_app_and_counts_calls() {
        let mock = MockWindowInfoProvider::with_app(AppInfo {
            name: "Visual Studio Code".to_string(),
            process: "code".to_string(),
            title: String::new(),
        });
        assert_eq!(mock.call_count(), 0);
        let first = mock.active_app();
        assert_eq!(first.process, "code");
        assert_eq!(mock.call_count(), 1);
        // 切换前台应用后返回新值。
        mock.set_app(AppInfo {
            name: "GNOME Terminal".to_string(),
            process: "gnome-terminal-server".to_string(),
            title: String::new(),
        });
        assert_eq!(mock.active_app().process, "gnome-terminal-server");
        assert_eq!(mock.call_count(), 2);
    }

    #[test]
    fn default_is_unknown_empty_app() {
        let mock = MockWindowInfoProvider::new();
        assert_eq!(mock.active_app(), AppInfo::default());
    }
}
