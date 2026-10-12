//! 前台窗口信息读取（T-501，SPEC: doc/specs/v0.5/T-501-scene-detection.md）。
//!
//! 三平台实现随任务补充；调用 OS API 处必须附最小权限注释（AGENTS.md 3.5）。
//! 读取到的窗口标题等识别信息不得写入日志（NFR-S-02）。

use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(any(test, feature = "test-utils"))]
mod mock;
#[cfg(any(test, feature = "test-utils"))]
pub use mock::MockWindowInfoProvider;

/// 前台应用信息（`active_app()` 返回值，三平台统一模型）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    /// 应用显示名（如 "Visual Studio Code"）。
    pub name: String,
    /// 进程名（如 "code"；场景规则库主要按此匹配）。
    ///
    /// 平台语义：Windows / Linux 为进程可执行名（不含扩展名）；macOS 为
    /// bundle identifier（`frontmostApplication` 不额外查进程信息，bundle id
    /// 对场景规则匹配更稳定），无 bundle id 时为空串。
    pub process: String,
    /// 窗口标题（AI 对话页等按标题模式细分时使用；不得入日志）。
    ///
    /// 平台语义：macOS 读取窗口标题需辅助功能权限，未授权时为空串（降级，
    /// 场景匹配退回 bundle id / 应用名，SPEC T-501 步骤 5）。
    pub title: String,
}

/// 前台窗口信息提供者（tech/04 4.5 平台适配层 trait）。
///
/// 最小权限说明（AGENTS.md 3.5）：各平台实现仅读取前台窗口句柄 / 应用元数据
/// （窗口标题、进程名或 bundle id），不注入消息、不读写其他进程内存、不做持久化；
/// 返回的识别信息（尤其窗口标题）不得写入日志（NFR-S-02），由调用方保证。
pub trait WindowInfoProvider: Send + Sync {
    /// 读取当前前台应用信息；识别受限（Wayland / 权限缺失 / 无前台窗口）时
    /// 返回空 `AppInfo`（name / process / title 全空），服务层映射为 `Unknown` 场景。
    fn active_app(&self) -> AppInfo;
}

/// 平台工厂：返回当前操作系统的默认前台窗口信息提供者（tech/04 4.5）。
pub fn default_provider() -> Arc<dyn WindowInfoProvider> {
    #[cfg(target_os = "linux")]
    {
        Arc::new(linux::X11WindowInfo)
    }
    #[cfg(target_os = "macos")]
    {
        Arc::new(macos::MacosWindowInfo)
    }
    #[cfg(target_os = "windows")]
    {
        Arc::new(windows::WindowsWindowInfo)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Arc::new(UnsupportedWindowInfo)
    }
}

/// 不支持的平台：静默返回空 `AppInfo`（映射为 Unknown 场景，不 warn——
/// 该平台本就没有此能力，避免噪音日志）。
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
struct UnsupportedWindowInfo;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl WindowInfoProvider for UnsupportedWindowInfo {
    fn active_app(&self) -> AppInfo {
        AppInfo::default()
    }
}
