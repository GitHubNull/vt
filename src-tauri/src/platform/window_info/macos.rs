//! macOS 前台窗口识别（T-501，tech/09 9.2.3：NSWorkspace / Accessibility）。
//!
//! - 应用名 / bundle id / pid：`NSWorkspace.sharedWorkspace.frontmostApplication`
//!   （[`AppInfo::process`] 按 mod.rs 平台语义存 bundle identifier，规则库主匹配键）；
//! - 窗口标题：辅助功能 API（Accessibility，`accessibility-sys`）读前台应用聚焦窗口的
//!   `kAXTitleAttribute`；**未授权时降级为空标题**（匹配退回 bundle id / 应用名，
//!   SPEC T-501 步骤 5：不崩溃、不主动弹权限对话框——零打扰，授权引导由前端负责）；
//! - `frontmostApplication` 返回 nil（无前台应用 / 登录窗等）→ 空 `AppInfo`（Unknown）。
//!
//! 最小权限说明（AGENTS.md 3.5）：NSWorkspace 仅读取前台应用元数据（名称 / bundle id /
//! pid）；Accessibility 仅在已获用户授权（`AXIsProcessTrusted`）时读取前台窗口标题，
//! 不注入 AX 事件、不遍历其他窗口、不修改任何 UI 状态；识别信息不写日志（NFR-S-02）。

use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use objc2::rc::{autoreleasepool, Retained};
use objc2_app_kit::{NSRunningApplication, NSWorkspace};

use super::AppInfo;

/// 前台窗口信息提供者（macOS 实现）。
pub struct MacosWindowInfo;

impl super::WindowInfoProvider for MacosWindowInfo {
    fn active_app(&self) -> AppInfo {
        // AppKit 对象须在 autoreleasepool 内使用；NSWorkspace 查询按 Apple 文档
        // 建议在主线程执行，本实现自后台 tokio 线程做只读查询（业界菜单栏应用
        // 同类用法），如遇问题可在平台工厂侧切换主线程派发（见 tech/09 9.2.3）。
        autoreleasepool(|_| {
            let Some(app) = frontmost_application() else {
                return AppInfo::default();
            };
            let name = app
                .localizedName()
                .map(|name| name.to_string())
                .unwrap_or_default();
            let process = app
                .bundleIdentifier()
                .map(|id| id.to_string())
                .unwrap_or_default();
            let title = window_title(app.processIdentifier());
            AppInfo {
                name,
                process,
                title,
            }
        })
    }
}

/// 前台应用（`frontmostApplication` 为 nil 时返回 `None`）。
fn frontmost_application() -> Option<Retained<NSRunningApplication>> {
    // NSWorkspace::sharedWorkspace / frontmostApplication 为 AppKit 只读查询；
    // 调用位于 autoreleasepool 内（objc2 生成方法本身安全）。
    NSWorkspace::sharedWorkspace().frontmostApplication()
}

/// 读取前台应用聚焦窗口标题；辅助功能未授权 / 无窗口 / 属性不可读时返回空串
/// （降级，不崩溃，SPEC T-501 步骤 5）。
///
/// 先取 `kAXFocusedWindowAttribute`（聚焦窗口，含 IDE 面板等），不可得再取
/// `kAXMainWindowAttribute`；窗口元素的 `kAXTitleAttribute` 即标题。
fn window_title(pid: i32) -> String {
    // SAFETY：AXIsProcessTrusted 只读查询当前进程的辅助功能授权状态。
    if !unsafe { accessibility_sys::AXIsProcessTrusted() } {
        // 未授权：降级为空标题，不崩溃也不弹窗（授权引导由设置页负责）。
        return String::new();
    }
    // SAFETY：以下 AX 调用均为只读 Copy 语义；返回的 CF 对象（窗口元素 / 标题
    // CFString）由本函数负责 CFRelease 或经 wrap_under_create_rule 转移所有权。
    unsafe {
        let app_element = accessibility_sys::AXUIElementCreateApplication(pid);
        if app_element.is_null() {
            return String::new();
        }
        let mut title = focused_window_title(app_element);
        if title.is_none() {
            title = main_window_title(app_element);
        }
        CFRelease(app_element as CFTypeRef);
        title.unwrap_or_default()
    }
}

/// 聚焦窗口（`kAXFocusedWindowAttribute`）的标题。
unsafe fn focused_window_title(app_element: accessibility_sys::AXUIElementRef) -> Option<String> {
    let window = copy_window_element(app_element, accessibility_sys::kAXFocusedWindowAttribute)?;
    element_title(window)
}

/// 主窗口（`kAXMainWindowAttribute`）的标题。
unsafe fn main_window_title(app_element: accessibility_sys::AXUIElementRef) -> Option<String> {
    let window = copy_window_element(app_element, accessibility_sys::kAXMainWindowAttribute)?;
    element_title(window)
}

/// 读取应用元素的窗口子元素（聚焦 / 主窗口）；返回的 CF 对象由调用方释放。
///
/// # Safety
/// `app_element` 须为有效的 AXUIElementRef；`attribute` 须为 AX 全局属性常量名。
unsafe fn copy_window_element(
    app_element: accessibility_sys::AXUIElementRef,
    attribute: &str,
) -> Option<accessibility_sys::AXUIElementRef> {
    let attr = CFString::new(attribute);
    let mut value: CFTypeRef = std::ptr::null();
    let status = accessibility_sys::AXUIElementCopyAttributeValue(
        app_element,
        attr.as_concrete_TypeRef(),
        &mut value,
    );
    if status != accessibility_sys::kAXErrorSuccess || value.is_null() {
        return None;
    }
    let window = value as accessibility_sys::AXUIElementRef;
    // 属性值可能不是窗口元素（异常应用），读不出标题即视为缺失，释放并返回 None。
    if !element_has_title(window) {
        CFRelease(window as CFTypeRef);
        return None;
    }
    Some(window)
}

/// 读取窗口元素的 `kAXTitleAttribute` 标题字符串，读完释放窗口元素（所有权转移）。
///
/// # Safety
/// `window` 须为 `copy_window_element` 返回的有效 AXUIElementRef。
unsafe fn element_title(window: accessibility_sys::AXUIElementRef) -> Option<String> {
    let attr = CFString::new(accessibility_sys::kAXTitleAttribute);
    let mut title: CFTypeRef = std::ptr::null();
    let status = accessibility_sys::AXUIElementCopyAttributeValue(
        window,
        attr.as_concrete_TypeRef(),
        &mut title,
    );
    // 标题 CFString 所有权转移给包装对象，drop 时统一 CFRelease。
    let result = if status == accessibility_sys::kAXErrorSuccess && !title.is_null() {
        Some(CFString::wrap_under_create_rule(title as CFStringRef).to_string())
    } else {
        None
    };
    CFRelease(window as CFTypeRef);
    result
}

/// 宽松校验元素可读出标题（避免把非窗口 AX 元素当窗口处理）；不转移所有权。
///
/// # Safety
/// `element` 须为有效的 AXUIElementRef。
unsafe fn element_has_title(element: accessibility_sys::AXUIElementRef) -> bool {
    let attr = CFString::new(accessibility_sys::kAXTitleAttribute);
    let mut title: CFTypeRef = std::ptr::null();
    let status = accessibility_sys::AXUIElementCopyAttributeValue(
        element,
        attr.as_concrete_TypeRef(),
        &mut title,
    );
    if status != accessibility_sys::kAXErrorSuccess {
        return false;
    }
    // 标题对象立即释放，此处只做存在性判定。
    if title.is_null() {
        false
    } else {
        CFRelease(title);
        true
    }
}
