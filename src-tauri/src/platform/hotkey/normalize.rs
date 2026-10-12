//! 快捷键串归一化（T-543）：应用内冲突校验与 OS 预警表匹配的等价比较基础。
//!
//! 纯逻辑模块（无平台 API），与 [`super::parse_accelerator`] 保持同一套合法性格式：
//! 段为 keyboard-types `Code` 名（如 `"AltRight"`、`"Space"`），以 `+` 连接，
//! **最后一段为主键，其余段为修饰键**。
//!
//! 等价规则（tech/09 9.2.1：平台不区分左右修饰位）：
//! - 左右修饰键等价（`ControlLeft+KeyC` ≡ `ControlRight+KeyC`）；
//! - 修饰键顺序无关、去重（`ShiftLeft+ControlLeft+KeyC` ≡ `Control+Shift+KeyC`）；
//! - Meta 与 Super 同一位（`MetaLeft+KeyD` ≡ `SuperRight+KeyD`）；
//! - 单修饰键作触发键时主键原文保留（`"AltRight"`，无修饰段）。

/// 修饰键 token → 无左右侧规范名（等价比较用，T-543）。
///
/// 与 `super::modifier_from_token` 保持同一映射：平台不区分左右修饰位
/// （tech/09 9.2.1），因此 `ControlLeft+KeyC` 与 `ControlRight+KeyC` 视为同一快捷键。
fn canonical_modifier_name(token: &str) -> Option<&'static str> {
    match token {
        "ControlLeft" | "ControlRight" | "Control" => Some("Control"),
        "AltLeft" | "AltRight" | "Alt" => Some("Alt"),
        "ShiftLeft" | "ShiftRight" | "Shift" => Some("Shift"),
        "MetaLeft" | "MetaRight" | "SuperLeft" | "SuperRight" | "Meta" | "Super" => Some("Super"),
        _ => None,
    }
}

/// 归一化快捷键串，用于等价比较（T-543 应用内冲突校验与 OS 常用组合预警表匹配）。
///
/// 规则：按 `+` 分段去空白；**最后一段为主键**（Code 原文保留）；其余段须为修饰键
/// （映射为无左右侧规范名、去重、按字母序排序）；输出 `修饰键...+主键`。
/// - 与 `super::parse_accelerator` 同一套合法性格式，空串 / 空段 / `Unidentified` /
///   非修饰键出现在修饰键位置 → `None`（不参与比较）；
/// - 无修饰键时输出即主键（如 `"F2"`、`"AltRight"` 单修饰键作触发键）。
pub fn normalize_accelerator(spec: &str) -> Option<String> {
    let tokens: Vec<&str> = spec.split('+').map(str::trim).collect();
    if tokens
        .iter()
        .any(|token| token.is_empty() || *token == "Unidentified")
    {
        return None;
    }
    // 鼠标侧键不是可注册的组合键（parse_accelerator 显式报 MouseKeyUnsupported，
    // v0.1 后端仅覆盖键盘），不参与等价比较。
    if tokens
        .iter()
        .any(|token| matches!(*token, "MouseBack" | "MouseForward"))
    {
        return None;
    }
    let (main, modifier_tokens) = tokens.split_last()?;
    let mut modifiers: Vec<&'static str> = Vec::with_capacity(modifier_tokens.len());
    for token in modifier_tokens {
        let name = canonical_modifier_name(token)?;
        if !modifiers.contains(&name) {
            modifiers.push(name);
        }
    }
    modifiers.sort_unstable();
    let mut normalized = modifiers.join("+");
    if !normalized.is_empty() {
        normalized.push('+');
    }
    normalized.push_str(main);
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use super::normalize_accelerator;

    #[test]
    fn normalize_single_key_and_single_modifier() {
        assert_eq!(normalize_accelerator("F2").as_deref(), Some("F2"));
        // 单修饰键作触发键：主键原文保留，无修饰段。
        assert_eq!(
            normalize_accelerator("AltRight").as_deref(),
            Some("AltRight")
        );
    }

    #[test]
    fn normalize_trims_whitespace_around_tokens() {
        assert_eq!(
            normalize_accelerator("AltLeft + KeyR").as_deref(),
            Some("Alt+KeyR")
        );
    }

    #[test]
    fn normalize_modifiers_sorted_and_deduped() {
        assert_eq!(
            normalize_accelerator("ShiftLeft+ControlLeft+KeyC").as_deref(),
            Some("Control+Shift+KeyC")
        );
        // 重复修饰键（左 + 右映射同一规范名）去重。
        assert_eq!(
            normalize_accelerator("ControlLeft+ControlRight+KeyC").as_deref(),
            Some("Control+KeyC")
        );
        // Meta / Super 同一位。
        assert_eq!(
            normalize_accelerator("SuperLeft+KeyD").as_deref(),
            Some("Super+KeyD")
        );
        assert_eq!(
            normalize_accelerator("MetaRight+KeyD").as_deref(),
            Some("Super+KeyD")
        );
    }

    #[test]
    fn normalize_equates_left_and_right_modifiers() {
        // 平台不区分左右修饰位（tech/09 9.2.1）：等价。
        assert_eq!(
            normalize_accelerator("ControlLeft+KeyC"),
            normalize_accelerator("ControlRight+KeyC")
        );
        // 修饰键集合不同：不等价。
        assert_ne!(
            normalize_accelerator("ControlLeft+KeyC"),
            normalize_accelerator("AltLeft+KeyC")
        );
    }

    #[test]
    fn normalize_rejects_blank_and_invalid() {
        assert_eq!(normalize_accelerator(""), None);
        assert_eq!(normalize_accelerator("AltLeft+"), None);
        assert_eq!(normalize_accelerator("AltLeft++KeyC"), None);
        assert_eq!(normalize_accelerator("AltLeft+Unidentified"), None);
        // 非修饰键出现在修饰键位置。
        assert_eq!(normalize_accelerator("KeyC+KeyD"), None);
        // 鼠标侧键不是可注册组合键（parse_accelerator 显式不支持），不参与比较。
        assert_eq!(normalize_accelerator("MouseBack"), None);
        assert_eq!(normalize_accelerator("MouseForward"), None);
        assert_eq!(normalize_accelerator("AltLeft+MouseBack"), None);
    }
}
