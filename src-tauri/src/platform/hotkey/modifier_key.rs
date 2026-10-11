//! "单个左右修饰键作触发键"的判定（Windows 低级钩子路径的路由依据）。
//!
//! 独立成模块是为了让该纯函数在所有平台可单测，并支持 Windows 目标的
//! 独立编译验证（本机无 MSVC，主 crate 无法整包 check Windows 目标）。

/// "单个左右修饰键作触发键"的快捷键串 → 该键 Win32 VK（0xA0–0xA5）；
/// 其他串（组合键、非修饰键、Meta/Super）返回 None，继续走 RegisterHotKey 路径。
///
/// 背景（tech/09 9.2.1）：Windows 的 RegisterHotKey（global-hotkey 0.8）不映射
/// 修饰键 Code，裸修饰键必须交由低级钩子覆盖；Meta/Super（Win 键）单独作触发键
/// 会与系统开始菜单严重冲突，有意不覆盖。
#[cfg_attr(not(windows), allow(dead_code))] // 仅 Windows 组合后端消费；其余平台保留供单测
pub(crate) fn modifier_trigger_vk(spec: &str) -> Option<u32> {
    let mut tokens = spec.split('+');
    let token = tokens.next()?.trim();
    if token.is_empty() || tokens.next().is_some() {
        return None;
    }
    Some(match token {
        "AltLeft" => 0xA4,
        "AltRight" => 0xA5,
        "ControlLeft" => 0xA2,
        "ControlRight" => 0xA3,
        "ShiftLeft" => 0xA0,
        "ShiftRight" => 0xA1,
        _ => return None,
    })
}
