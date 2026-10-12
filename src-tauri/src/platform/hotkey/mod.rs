//! 全局快捷键平台适配层（tech/04 4.5、tech/09 9.2.1）。
//!
//! 三平台统一基于 Tauri `global-shortcut` 插件（其底层：Windows RegisterHotKey /
//! macOS NSEvent 全局监听 / Linux X11 XRecord）。平台差异说明：
//! - macOS：全局监听需要"辅助功能"权限，缺失时注册失败 → 由服务层降级为
//!   错误事件（E-HOTKEY-001）并引导授权（tech/09 9.4），不崩溃；
//! - Windows：global-hotkey 的 RegisterHotKey 路径不映射修饰键 Code，单个左右
//!   修饰键作触发键时改走本模块的低级键盘钩子后端（`windows_llhook`，
//!   WH_KEYBOARD_LL，含 PRD 3.1.1 规则 3/4 的组合键抑制与 0.3 s 误触门）；
//! - Linux Wayland：协议限制无法全局监听，注册失败并提示（v1.0 提供替代方案）。
//!
//! 最小权限说明（AGENTS.md 3.5）：本能力仅注册用户配置的全局快捷键并接收其
//! 按下 / 松开事件，不拦截、不记录其他按键。

use std::str::FromStr;
use std::sync::Arc;

use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};
use tokio::sync::mpsc;

#[cfg(windows)]
mod windows_composite;
#[cfg(windows)]
mod windows_llhook;

mod modifier_key;
mod normalize;
mod os_presets;

pub use normalize::normalize_accelerator;
pub use os_presets::{os_preset_conflicts, presets, OsPresetHotkey};

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
    /// 鼠标侧键（MouseBack / MouseForward）：v0.1 全局快捷键后端仅覆盖键盘，
    /// 鼠标侧键需平台级钩子（Windows WH_MOUSE_LL / X11 XRecord / macOS CGEventTap），
    /// 显式报错而非静默丢弃，待产品决策后随对应平台任务实现。
    #[error("鼠标侧键暂不支持作为快捷键: {0}")]
    MouseKeyUnsupported(String),
    /// 被其他应用占用 / 系统保留（含 macOS 辅助功能权限缺失）。
    #[error("快捷键注册失败（被占用或权限缺失）")]
    RegisterFailed,
    /// 其他平台错误。
    #[error("平台错误: {0}")]
    Platform(String),
}

/// 组合串 → global-shortcut `Shortcut`（前端 HotkeyInput 契约，T-013 移交修复 2）。
///
/// 格式：段为 keyboard-types `Code` 名（如 `"AltRight"`、`"Space"`），以 `+` 连接；
/// **最后一段为主键，其余段为修饰键**——因此 `"AltRight"`（单修饰键）、
/// `"ControlLeft+ShiftLeft+Space"`（左右修饰键）均可表达。
///
/// 修饰键左右归属说明：Windows `RegisterHotKey` / 通用 Modifier 位无法区分左右，
/// 左右修饰键统一映射到同一 Modifier 位（平台限制，tech/09 9.2.1）；主键的左右
/// 归属（如右 Alt 本身作为触发键）由 `Code::AltRight` 精确保留。
pub fn parse_accelerator(spec: &str) -> Result<Shortcut, HotkeyError> {
    if matches!(spec, "MouseBack" | "MouseForward") {
        return Err(HotkeyError::MouseKeyUnsupported(spec.to_string()));
    }
    let tokens: Vec<&str> = spec.split('+').map(str::trim).collect();
    // Unidentified：部分键盘布局（右 Alt 被映射为 AltGr）下 webview 会产生该
    // code，keyboard-types 虽能解析但对应不到任何物理键，必须按非法串拒绝（E-HOTKEY-002）。
    if tokens
        .iter()
        .any(|token| token.is_empty() || *token == "Unidentified")
    {
        return Err(HotkeyError::InvalidAccelerator(spec.to_string()));
    }
    let (key_token, modifier_tokens) = tokens
        .split_last()
        .ok_or_else(|| HotkeyError::InvalidAccelerator(spec.to_string()))?;
    let mut modifiers = Modifiers::empty();
    for token in modifier_tokens {
        modifiers |= modifier_from_token(token)
            .ok_or_else(|| HotkeyError::InvalidAccelerator(spec.to_string()))?;
    }
    let key =
        Code::from_str(key_token).map_err(|_| HotkeyError::InvalidAccelerator(spec.to_string()))?;
    Ok(Shortcut::new(Some(modifiers), key))
}

/// 左右修饰键 → Modifier 位（右侧修饰键与左侧映射到同一位，平台限制见上）。
fn modifier_from_token(token: &str) -> Option<Modifiers> {
    match token {
        "ControlLeft" | "ControlRight" => Some(Modifiers::CONTROL),
        "AltLeft" | "AltRight" => Some(Modifiers::ALT),
        "ShiftLeft" | "ShiftRight" => Some(Modifiers::SHIFT),
        "MetaLeft" | "MetaRight" | "SuperLeft" | "SuperRight" => Some(Modifiers::SUPER),
        _ => None,
    }
}

/// 注册失败归因（T-543 SPEC 步骤 2）：区分"被占用 / 系统保留"与平台级失败。
///
/// global-hotkey 的错误经插件以字符串透出（`Error::GlobalHotKey(String)`）：
/// - `"HotKey already registered"`：`RegisterHotKey` 的 `ERROR_HOTKEY_ALREADY_REGISTERED`
///   （Windows）、X11 `GrabKey` Access 冲突（Linux）、Carbon `EventHotKey` 冲突（macOS）
///   —— 三个平台的"被占用"路径统一归 `RegisterFailed`；
/// - 其余（macOS 系统拒绝、X11 连接错误等）：归 `Platform` 并保留原始信息供诊断。
fn classify_register_error(plugin_message: &str) -> HotkeyError {
    if plugin_message.to_lowercase().contains("already registered") {
        HotkeyError::RegisterFailed
    } else {
        HotkeyError::Platform(plugin_message.to_string())
    }
}

/// 全局快捷键后端抽象：`register` 成功后事件经构造时给定的通道上报。
pub trait HotkeyBackend: Send + Sync {
    /// 注册快捷键；事件通过 `RawHotkeyEvent` 通道上报（Pressed / Released）。
    fn register(&self, accelerator: &str) -> Result<(), HotkeyError>;
    /// 精准注销指定快捷键（T-010：Esc 临时注册注销用，不影响其他注册）。
    fn unregister(&self, accelerator: &str) -> Result<(), HotkeyError>;
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
        // 组合串翻译为 Shortcut（keyboard-types Code 手动构造，支持左右修饰键与
        // 单修饰键格式；解析失败在此返回 InvalidAccelerator）。
        let shortcut = parse_accelerator(accelerator)?;
        // 注册全局快捷键：系统级能力，仅监听用户指定的组合键（最小权限见模块头注释）。
        self.app
            .global_shortcut()
            .on_shortcut(shortcut, move |_app, _shortcut, event| {
                // 回调运行在全局热键线程；try_send 不阻塞，通道满则丢弃该事件。
                let raw = match event.state {
                    ShortcutState::Pressed => RawHotkeyEvent::Pressed,
                    ShortcutState::Released => RawHotkeyEvent::Released,
                };
                if events.try_send(raw).is_err() {
                    tracing::warn!("快捷键事件通道已满，丢弃事件");
                }
            })
            // 解析已在上方完成；此处错误按占用 / 平台问题归因（T-543 classify_register_error）。
            .map_err(|err| classify_register_error(&err.to_string()))
    }

    fn unregister(&self, accelerator: &str) -> Result<(), HotkeyError> {
        // 精准注销（unregister_all 会连带注销主录音键等其他注册）。
        let shortcut = parse_accelerator(accelerator)?;
        self.app
            .global_shortcut()
            .unregister(shortcut)
            .map_err(|err| HotkeyError::Platform(err.to_string()))
    }

    fn unregister_all(&self) -> Result<(), HotkeyError> {
        self.app
            .global_shortcut()
            .unregister_all()
            .map_err(|err| HotkeyError::Platform(err.to_string()))
    }
}

/// 创建三平台默认后端（tech/04 4.5 工厂函数）。
///
/// Windows 为组合后端（修饰键触发 → 低级钩子，其余 → global-shortcut 插件）；
/// 其他平台直接用 global-shortcut 插件。
pub fn default_backend(
    app: tauri::AppHandle,
    events: mpsc::Sender<RawHotkeyEvent>,
) -> Arc<dyn HotkeyBackend> {
    #[cfg(windows)]
    {
        let tauri_backend = TauriHotkeyBackend {
            app,
            events: events.clone(),
        };
        Arc::new(windows_composite::WindowsCompositeBackend::new(
            tauri_backend,
            events,
        ))
    }
    #[cfg(not(windows))]
    {
        Arc::new(TauriHotkeyBackend { app, events })
    }
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
        /// 置位时下一次注册返回该错误并清空（T-543：模拟平台级注册失败等归因）。
        pub next_error: Mutex<Option<HotkeyError>>,
        /// 成功注册次数（T-010 Esc 注册 / 注销配对断言）。
        pub register_calls: std::sync::atomic::AtomicUsize,
        /// 注销调用次数（unregister + unregister_all 合计）。
        pub unregister_calls: std::sync::atomic::AtomicUsize,
    }

    impl Default for MockHotkeyBackend {
        fn default() -> Self {
            Self {
                registered: Mutex::new(Vec::new()),
                fail_next: std::sync::atomic::AtomicBool::new(false),
                next_error: Mutex::new(None),
                register_calls: std::sync::atomic::AtomicUsize::new(0),
                unregister_calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    impl MockHotkeyBackend {
        pub fn new() -> Self {
            Self::default()
        }

        /// 当前已注册的唯一快捷键（v0.1 只注册一个录音触发键）。
        pub fn current(&self) -> Option<String> {
            self.registered.lock().unwrap().last().cloned()
        }
    }

    impl HotkeyBackend for MockHotkeyBackend {
        fn register(&self, accelerator: &str) -> Result<(), HotkeyError> {
            // 指定错误优先（T-543 归因测试用），一次性。
            if let Some(err) = self.next_error.lock().unwrap().take() {
                return Err(err);
            }
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
            self.register_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        fn unregister(&self, accelerator: &str) -> Result<(), HotkeyError> {
            self.registered
                .lock()
                .unwrap()
                .retain(|item| item != accelerator);
            self.unregister_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        fn unregister_all(&self) -> Result<(), HotkeyError> {
            self.registered.lock().unwrap().clear();
            self.unregister_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_modifier_key() {
        // 单修饰键格式：右 Alt 作为主键（无修饰位）。
        let shortcut = parse_accelerator("AltRight").unwrap();
        assert_eq!(shortcut.key, Code::AltRight);
        assert!(shortcut.mods.is_empty());
    }

    #[test]
    fn parse_combo_with_left_right_modifiers() {
        let shortcut = parse_accelerator("ControlLeft+ShiftLeft+Space").unwrap();
        assert_eq!(shortcut.key, Code::Space);
        assert!(shortcut.mods.contains(Modifiers::CONTROL));
        assert!(shortcut.mods.contains(Modifiers::SHIFT));
    }

    #[test]
    fn parse_right_modifiers_map_to_same_bits() {
        let right = parse_accelerator("ControlRight+AltRight+KeyQ").unwrap();
        assert_eq!(right.key, Code::KeyQ);
        assert!(right.mods.contains(Modifiers::CONTROL));
        assert!(right.mods.contains(Modifiers::ALT));
        // 左/右修饰键映射到同一 Modifier 位（平台限制，见 parse_accelerator 文档）。
        let left = parse_accelerator("ControlLeft+AltLeft+KeyQ").unwrap();
        assert_eq!(right.mods, left.mods);
    }

    #[test]
    fn parse_modifier_as_main_key() {
        // 组合中右 Alt 作为主键、左 Ctrl 作为修饰键。
        let shortcut = parse_accelerator("ControlLeft+AltRight").unwrap();
        assert_eq!(shortcut.key, Code::AltRight);
        assert!(shortcut.mods.contains(Modifiers::CONTROL));
    }

    #[test]
    fn parse_unknown_key_is_invalid() {
        assert!(matches!(
            parse_accelerator("NotAKey"),
            Err(HotkeyError::InvalidAccelerator(_))
        ));
        assert!(matches!(
            parse_accelerator("ControlLeft++Space"),
            Err(HotkeyError::InvalidAccelerator(_))
        ));
    }

    #[test]
    fn parse_unidentified_is_invalid() {
        // 右 Alt 被映射为 AltGr 的键盘布局会录出 Unidentified，按非法串拒绝（E-HOTKEY-002）。
        assert!(matches!(
            parse_accelerator("AltLeft+Unidentified"),
            Err(HotkeyError::InvalidAccelerator(_))
        ));
    }

    #[test]
    fn parse_mouse_side_keys_explicitly_unsupported() {
        // 鼠标侧键：显式报不支持（不静默丢弃），错误供服务层转为引导提示。
        assert!(matches!(
            parse_accelerator("MouseBack"),
            Err(HotkeyError::MouseKeyUnsupported(_))
        ));
        assert!(matches!(
            parse_accelerator("MouseForward"),
            Err(HotkeyError::MouseKeyUnsupported(_))
        ));
    }

    #[test]
    fn classify_register_error_attributes_occupied() {
        // 三平台"被占用"路径的统一错误串（global-hotkey AlreadyRegistered）。
        assert!(matches!(
            classify_register_error("GlobalHotKey error: HotKey already registered: HotKey { mods: CONTROL, key: KeyC }"),
            HotkeyError::RegisterFailed
        ));
        // macOS RegisterEventHotKey 失败等：平台级失败，保留原文。
        assert!(matches!(
            classify_register_error(
                "Unable to register hotkey: RegisterEventHotKey failed for KeyC"
            ),
            HotkeyError::Platform(_)
        ));
    }

    use super::modifier_key::modifier_trigger_vk;

    #[test]
    fn modifier_trigger_vk_matches_single_modifiers() {
        // 六个左右修饰键单独作触发键 → 低级钩子路径（Win32 VK 0xA0–0xA5）。
        assert_eq!(modifier_trigger_vk("AltRight"), Some(0xA5));
        assert_eq!(modifier_trigger_vk("AltLeft"), Some(0xA4));
        assert_eq!(modifier_trigger_vk("ControlRight"), Some(0xA3));
        assert_eq!(modifier_trigger_vk("ControlLeft"), Some(0xA2));
        assert_eq!(modifier_trigger_vk("ShiftRight"), Some(0xA1));
        assert_eq!(modifier_trigger_vk("ShiftLeft"), Some(0xA0));
    }

    #[test]
    fn modifier_trigger_vk_rejects_combos_and_other_keys() {
        // 组合键（即使含修饰键）走 RegisterHotKey 路径。
        assert_eq!(modifier_trigger_vk("ControlLeft+AltRight"), None);
        assert_eq!(modifier_trigger_vk("ControlLeft+ShiftLeft+Space"), None);
        // 非修饰键、Win 键（有意不覆盖）、空段、鼠标侧键。
        assert_eq!(modifier_trigger_vk("F2"), None);
        assert_eq!(modifier_trigger_vk("Space"), None);
        assert_eq!(modifier_trigger_vk("MetaLeft"), None);
        assert_eq!(modifier_trigger_vk("SuperRight"), None);
        assert_eq!(modifier_trigger_vk(""), None);
        assert_eq!(modifier_trigger_vk("AltRight+"), None);
        assert_eq!(modifier_trigger_vk("MouseBack"), None);
    }
}
