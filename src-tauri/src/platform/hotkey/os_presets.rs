//! OS 常用快捷键预警表（T-543 SPEC 步骤 3，PRD 3.1.1 规则 7 / F-SYS-03）。
//!
//! 录入的组合若与 OS / 桌面环境的高频快捷键一致，仅**警告**（允许保存）：全局注册
//! 多数仍会成功，实际后果是"该场景下按键同时触发系统操作、录音可能被抢占"，因此
//! PRD 明确：与已知常用软件快捷键冲突时给出提示但允许保存。
//!
//! 归一化口径：与 [`super::normalize_accelerator`] 输出一致——修饰键无左右侧
//! （Meta / Super 同位映射为 `Super`）、按字母序排列（Alt < Control < Shift < Super）。
//! 表内条目直接写归一化形式；匹配时把用户录入串归一化后查表。
//!
//! 用途标识（`usage`）是 i18n 键后缀：`hotkey.conflict.osPresets.<usage>`，文案在
//! `src/i18n/locales/`（zh-CN / en-US），本模块不含用户可见文案。
//!
//! 三平台清单（WINDOWS / MACOS / LINUX）均为常量、无条件编译（单一平台构建也会
//! 校验全部表：条目形态错误在任何平台都能被测试发现）；`presets()` 按目标平台选择。

/// OS 常用快捷键预警条目。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OsPresetHotkey {
    /// 归一化组合（`normalize_accelerator` 输出格式，如 `"Control+KeyC"`）。
    pub normalized: &'static str,
    /// 用途标识（i18n 键后缀：`hotkey.conflict.osPresets.<usage>`）。
    pub usage: &'static str,
}

/// Windows 常用组合（系统级 + 通用应用级）。
// 非目标平台的表不参与本平台运行时选择，但随测试全量编译做形态校验（all_tables_are_wellformed）。
#[allow(dead_code)]
const WINDOWS_PRESETS: &[OsPresetHotkey] = &[
    OsPresetHotkey {
        normalized: "Control+KeyC",
        usage: "copy",
    },
    OsPresetHotkey {
        normalized: "Control+KeyV",
        usage: "paste",
    },
    OsPresetHotkey {
        normalized: "Control+KeyX",
        usage: "cut",
    },
    OsPresetHotkey {
        normalized: "Control+KeyZ",
        usage: "undo",
    },
    OsPresetHotkey {
        normalized: "Control+KeyY",
        usage: "redo",
    },
    OsPresetHotkey {
        normalized: "Control+KeyA",
        usage: "select_all",
    },
    OsPresetHotkey {
        normalized: "Control+KeyS",
        usage: "save",
    },
    OsPresetHotkey {
        normalized: "Control+KeyF",
        usage: "find",
    },
    OsPresetHotkey {
        normalized: "Control+KeyP",
        usage: "print",
    },
    OsPresetHotkey {
        normalized: "Control+KeyW",
        usage: "close_tab",
    },
    OsPresetHotkey {
        normalized: "Control+Shift+Escape",
        usage: "task_manager",
    },
    OsPresetHotkey {
        normalized: "Alt+F4",
        usage: "close_window",
    },
    OsPresetHotkey {
        normalized: "Alt+Tab",
        usage: "switch_app",
    },
    OsPresetHotkey {
        normalized: "Super+KeyD",
        usage: "show_desktop",
    },
    OsPresetHotkey {
        normalized: "Super+KeyE",
        usage: "file_explorer",
    },
    OsPresetHotkey {
        normalized: "Super+KeyI",
        usage: "system_settings",
    },
    OsPresetHotkey {
        normalized: "Super+KeyL",
        usage: "lock_screen",
    },
    OsPresetHotkey {
        normalized: "Super+KeyR",
        usage: "run_dialog",
    },
    OsPresetHotkey {
        normalized: "Super+KeyS",
        usage: "search",
    },
    OsPresetHotkey {
        normalized: "Super+KeyA",
        usage: "quick_settings",
    },
    OsPresetHotkey {
        normalized: "Super+Tab",
        usage: "task_view",
    },
    OsPresetHotkey {
        normalized: "PrintScreen",
        usage: "screenshot",
    },
    OsPresetHotkey {
        normalized: "F1",
        usage: "help",
    },
];

/// macOS 常用组合（Cmd = Meta / Super 同位；系统级 + 通用应用级）。
// 非目标平台的表不参与本平台运行时选择，但随测试全量编译做形态校验（all_tables_are_wellformed）。
#[allow(dead_code)]
const MACOS_PRESETS: &[OsPresetHotkey] = &[
    OsPresetHotkey {
        normalized: "Super+KeyC",
        usage: "copy",
    },
    OsPresetHotkey {
        normalized: "Super+KeyV",
        usage: "paste",
    },
    OsPresetHotkey {
        normalized: "Super+KeyX",
        usage: "cut",
    },
    OsPresetHotkey {
        normalized: "Super+KeyZ",
        usage: "undo",
    },
    OsPresetHotkey {
        normalized: "Super+KeyA",
        usage: "select_all",
    },
    OsPresetHotkey {
        normalized: "Super+KeyS",
        usage: "save",
    },
    OsPresetHotkey {
        normalized: "Super+KeyF",
        usage: "find",
    },
    OsPresetHotkey {
        normalized: "Super+KeyP",
        usage: "print",
    },
    OsPresetHotkey {
        normalized: "Super+KeyW",
        usage: "close_window",
    },
    OsPresetHotkey {
        normalized: "Super+KeyQ",
        usage: "quit_app",
    },
    OsPresetHotkey {
        normalized: "Super+KeyM",
        usage: "minimize",
    },
    OsPresetHotkey {
        normalized: "Super+KeyH",
        usage: "hide_app",
    },
    OsPresetHotkey {
        normalized: "Super+Tab",
        usage: "switch_app",
    },
    OsPresetHotkey {
        normalized: "Super+Space",
        usage: "spotlight",
    },
    OsPresetHotkey {
        normalized: "Control+Space",
        usage: "input_source",
    },
    OsPresetHotkey {
        normalized: "Shift+Super+Key3",
        usage: "screenshot",
    },
    OsPresetHotkey {
        normalized: "Shift+Super+Key4",
        usage: "screenshot",
    },
];

/// Linux 常用组合（GNOME / KDE 默认，含通用应用级）。
// 非目标平台的表不参与本平台运行时选择，但随测试全量编译做形态校验（all_tables_are_wellformed）。
#[allow(dead_code)]
const LINUX_PRESETS: &[OsPresetHotkey] = &[
    OsPresetHotkey {
        normalized: "Control+KeyC",
        usage: "copy",
    },
    OsPresetHotkey {
        normalized: "Control+KeyV",
        usage: "paste",
    },
    OsPresetHotkey {
        normalized: "Control+KeyX",
        usage: "cut",
    },
    OsPresetHotkey {
        normalized: "Control+KeyZ",
        usage: "undo",
    },
    OsPresetHotkey {
        normalized: "Control+KeyY",
        usage: "redo",
    },
    OsPresetHotkey {
        normalized: "Control+KeyA",
        usage: "select_all",
    },
    OsPresetHotkey {
        normalized: "Control+KeyS",
        usage: "save",
    },
    OsPresetHotkey {
        normalized: "Control+KeyF",
        usage: "find",
    },
    OsPresetHotkey {
        normalized: "Control+KeyW",
        usage: "close_tab",
    },
    OsPresetHotkey {
        normalized: "Alt+Control+KeyT",
        usage: "terminal",
    },
    OsPresetHotkey {
        normalized: "Alt+Control+KeyL",
        usage: "lock_screen",
    },
    OsPresetHotkey {
        normalized: "Super+KeyL",
        usage: "lock_screen",
    },
    OsPresetHotkey {
        normalized: "Super+KeyA",
        usage: "app_grid",
    },
    OsPresetHotkey {
        normalized: "Super+KeyD",
        usage: "show_desktop",
    },
    OsPresetHotkey {
        normalized: "Super+Tab",
        usage: "switch_app",
    },
    OsPresetHotkey {
        normalized: "Alt+Tab",
        usage: "switch_app",
    },
    OsPresetHotkey {
        normalized: "Alt+F2",
        usage: "run_dialog",
    },
    OsPresetHotkey {
        normalized: "Alt+Control+ArrowLeft",
        usage: "workspace_switch",
    },
    OsPresetHotkey {
        normalized: "Alt+Control+ArrowRight",
        usage: "workspace_switch",
    },
    OsPresetHotkey {
        normalized: "PrintScreen",
        usage: "screenshot",
    },
    OsPresetHotkey {
        normalized: "F1",
        usage: "help",
    },
];

/// 其余平台：暂无维护的清单（三平台桌面端之外不出现）。
#[allow(dead_code)]
const EMPTY_PRESETS: &[OsPresetHotkey] = &[];

/// 当前平台的 OS 常用快捷键清单。
pub fn presets() -> &'static [OsPresetHotkey] {
    #[cfg(target_os = "windows")]
    {
        WINDOWS_PRESETS
    }
    #[cfg(target_os = "macos")]
    {
        MACOS_PRESETS
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        #[cfg(target_os = "linux")]
        {
            LINUX_PRESETS
        }
        #[cfg(not(target_os = "linux"))]
        {
            EMPTY_PRESETS
        }
    }
}

/// 归一化组合命中的 OS 常用快捷键用途（同一用途可能有多组键，如 macOS 截图）。
pub fn os_preset_conflicts(normalized: &str) -> Vec<&'static str> {
    presets()
        .iter()
        .filter(|preset| preset.normalized == normalized)
        .map(|preset| preset.usage)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::hotkey::normalize_accelerator;

    /// 规范修饰键名（normalize_accelerator 输出侧）。
    const CANONICAL_MODIFIERS: [&str; 4] = ["Alt", "Control", "Shift", "Super"];

    /// 由归一化形式反推一个"用户录入形态"（左右侧修饰键 + 乱序）。
    fn user_form_of(normalized: &str) -> String {
        let tokens: Vec<&str> = normalized.split('+').collect();
        let (main, modifiers) = tokens.split_last().expect("条目至少含主键");
        let mut modifier_tokens: Vec<String> = modifiers
            .iter()
            .map(|name| {
                assert!(
                    CANONICAL_MODIFIERS.contains(name),
                    "条目 {normalized} 的修饰键 {name} 不是规范名"
                );
                format!("{name}Left")
            })
            .collect();
        modifier_tokens.reverse(); // 乱序：同时验证修饰键顺序无关
        modifier_tokens.push((*main).to_string());
        modifier_tokens.join("+")
    }

    /// 条目形态不变量对三张表全部生效（任何单一平台构建都能发现所有表的笔误）。
    fn assert_table_invariants(table: &[OsPresetHotkey], name: &str) {
        let mut normalized_keys: Vec<&str> = Vec::with_capacity(table.len());
        for preset in table {
            let input = user_form_of(preset.normalized);
            let normalized = normalize_accelerator(&input)
                .unwrap_or_else(|| panic!("{name}: 录入形态 {input} 应可归一化"));
            assert_eq!(
                normalized, preset.normalized,
                "{name}: 条目 {input} 归一化后应与表一致"
            );
            let main = preset.normalized.rsplit('+').next().expect("非空");
            assert!(
                !main.is_empty() && !CANONICAL_MODIFIERS.contains(&main),
                "{name}: 条目 {} 的主键 {} 不能是修饰键规范名",
                preset.normalized,
                main
            );
            normalized_keys.push(preset.normalized);
        }
        normalized_keys.sort_unstable();
        let count = normalized_keys.len();
        normalized_keys.dedup();
        assert_eq!(normalized_keys.len(), count, "{name}: 归一化条目不得重复");
    }

    #[test]
    fn all_tables_are_wellformed() {
        assert_table_invariants(WINDOWS_PRESETS, "windows");
        assert_table_invariants(MACOS_PRESETS, "macos");
        assert_table_invariants(LINUX_PRESETS, "linux");
        assert_table_invariants(EMPTY_PRESETS, "empty");
    }

    #[test]
    fn current_platform_lookup_matches_every_entry() {
        for preset in presets() {
            let input = user_form_of(preset.normalized);
            let normalized = normalize_accelerator(&input).expect("合法组合");
            let usages = os_preset_conflicts(&normalized);
            assert!(
                usages.contains(&preset.usage),
                "归一化 {normalized} 应命中用途 {}",
                preset.usage
            );
        }
    }

    #[test]
    fn non_preset_combo_misses() {
        // 默认录音键 Alt+R 与取消键 Esc 均不在任何平台的预警表内。
        assert!(os_preset_conflicts("Alt+KeyR").is_empty());
        assert!(os_preset_conflicts("Escape").is_empty());
        assert!(os_preset_conflicts("").is_empty());
    }

    #[test]
    fn windows_and_linux_copy_combo_is_preset() {
        // Ctrl+C 在 Windows / Linux 清单中均为"复制"。
        let normalized = normalize_accelerator("ControlLeft+KeyC").expect("合法组合");
        assert_eq!(normalized, "Control+KeyC");
        assert_eq!(os_preset_conflicts(&normalized), vec!["copy"]);
        // Alt+F4（关闭窗口）在 Windows 清单中。
        let alt_f4 = normalize_accelerator("AltLeft+F4").expect("合法组合");
        assert_eq!(alt_f4, "Alt+F4");
        assert_eq!(
            os_preset_conflicts(&alt_f4),
            if cfg!(target_os = "windows") {
                vec!["close_window"]
            } else {
                Vec::new()
            }
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_copy_combo_is_preset() {
        // macOS 复制为 Cmd+C；Meta 归一化为 Super（canonical_modifier_name）。
        let normalized = normalize_accelerator("MetaLeft+KeyC").expect("合法组合");
        assert_eq!(normalized, "Super+KeyC");
        assert_eq!(os_preset_conflicts(&normalized), vec!["copy"]);
    }
}
