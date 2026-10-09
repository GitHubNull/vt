//! 文本上屏平台适配层（tech/04 4.5、tech/05 5.3.5、tech/09 9.2.2）。
//!
//! 最小权限说明（AGENTS.md 3.5）：本能力仅向"当前焦点的输入位置"发送文本：
//! - `type_text` 逐字符模拟键盘输入；**不暴露任何单独的执行类按键**（无 Enter
//!   按键事件出口，NFR-S-05 的 v0.1 硬约束；换行作为文本内容由调用方决定）；
//! - 剪贴板仅在粘贴路径短时占用（备份 → 写入 → 粘贴 → 恢复），
//!   绝不读取剪贴板内容用于其他用途；
//! - 焦点检测只查询系统"当前焦点窗口"的存在性，不读取窗口内容。
//!
//! 平台差异（tech/09 9.2.2）：Windows SendInput / macOS CGEvent（需辅助功能权限，
//! 未授权时返回 [`InjectError::Permission`] 由服务层降级）/ Linux X11 XTest；
//! Wayland 受 compositor 限制，模拟键入可能不可用，剪贴板路径为主（标注受限）。

use std::sync::Arc;

use serde::Serialize;
use thiserror::Error;

mod enigo_impl;
#[cfg(test)]
mod mock;

pub use enigo_impl::EnigoTextInjector;
#[cfg(test)]
pub use mock::MockTextInjector;

/// 平台后端错误。
#[derive(Debug, Clone, Error)]
pub enum InjectError {
    /// 模拟键入失败（含 macOS 辅助功能未授权之外的失败）。
    #[error("模拟键入失败: {0}")]
    Type(String),
    /// 辅助功能 / 权限缺失（macOS AXIsProcessTrusted = false 等），应降级剪贴板路径并引导授权。
    // 允许 dead_code：仅 macOS 实现构造（cfg 平台分层）。
    #[allow(dead_code)]
    #[error("辅助功能权限未授权: {0}")]
    Permission(String),
    /// 剪贴板读写失败。
    #[error("剪贴板操作失败: {0}")]
    Clipboard(String),
    /// 模拟粘贴快捷键失败。
    #[error("模拟粘贴失败: {0}")]
    Paste(String),
    /// 其他平台错误。
    #[error("平台错误: {0}")]
    Platform(String),
}

/// 剪贴板备份（粘贴路径执行前读取，执行后恢复）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardBackup {
    /// 剪贴板中是一段文本。
    Text(String),
    /// 剪贴板为空或内容非文本（图片 / 文件等）。
    ///
    /// 受限说明：v0.1 文本路径无法把非文本内容写回剪贴板，恢复时记警告并保持现状。
    Unsupported,
}

/// 输入焦点状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusState {
    /// 检测到可接收输入的焦点。
    Focused,
    /// 明确检测不到输入焦点（桌面空白等）。
    Unfocused,
    /// 无法判定（Wayland 受限、X 连接失败等）：继续尝试主路径，失败再走兜底。
    Unknown,
}

/// 上屏方式（`inject_text` 命令返回，前端 / T-012 联调消费）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectMethod {
    /// 模拟键入成功。
    Typed,
    /// 剪贴板粘贴成功（含主选与降级）。
    Pasted,
    /// 无焦点 / 全部路径失败的兜底：文本已写入剪贴板并通知（F-INPUT-09）。
    ClipboardFallback,
}

/// 上屏结果报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InjectReport {
    /// 实际生效的上屏方式。
    pub method: InjectMethod,
    /// 降级原因（无焦点 / 权限引导 / 兼容清单命中 / 主路径失败等），主路径成功时为 `None`。
    pub degraded: Option<String>,
}

/// 文本注入器（tech/04 4.5：每个能力一个 trait，工厂函数在模块底部）。
///
/// 全部方法同步执行（enigo / 剪贴板插件均为同步 API），由服务层在
/// `spawn_blocking` 中调用，避免阻塞异步运行时。
pub trait TextInjector: Send + Sync {
    /// 逐字符模拟键入文本。
    ///
    /// 硬约束（NFR-S-05）：实现不得产生 Enter / Return 按键事件出口；
    /// 文本中的 `\n` 由 enigo 作为文本换行发送，是否生效取决于目标应用。
    fn type_text(&self, text: &str) -> Result<(), InjectError>;
    /// 读取当前剪贴板内容用于备份（仅文本内容可恢复，其余记 Unsupported）。
    fn clipboard_backup(&self) -> Result<ClipboardBackup, InjectError>;
    /// 把文本写入剪贴板。
    fn write_clipboard(&self, text: &str) -> Result<(), InjectError>;
    /// 模拟粘贴快捷键（Ctrl+V / Cmd+V）。
    fn simulate_paste(&self) -> Result<(), InjectError>;
    /// 恢复备份的剪贴板内容。
    fn clipboard_restore(&self, backup: &ClipboardBackup) -> Result<(), InjectError>;
    /// 查询当前是否存在输入焦点。
    fn input_focus(&self) -> FocusState;
    /// 当前前台应用名（兼容性清单匹配用）。
    ///
    /// v0.1 返回 `None`：前台应用识别属场景识别（v0.5 T-501，本任务范围外），
    /// 清单机制在服务层就绪，识别接入后自动生效。
    fn focused_app(&self) -> Option<String>;
}

/// 三平台默认注入器：enigo 统一封装（tech/04 4.5 工厂函数）。
pub fn default_injector(app: tauri::AppHandle) -> Result<Arc<dyn TextInjector>, InjectError> {
    Ok(Arc::new(EnigoTextInjector::new(app)?))
}

/// 注入前的文本清洗（安全边界，NFR-S-05）：
/// - 保留 `\n` / `\t`（换行 / 制表属于文本内容，PRD 3.1.3 允许）；
/// - 其余控制字符（`\r`、ESC 等）一律移除——它们可能触发目标应用的执行行为；
/// - 平台层 `type_text` 收到的是"纯文本"，任何路径都不会构造单独的回车按键事件。
pub fn sanitize_inject_text(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_control() || *ch == '\n' || *ch == '\t')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_newline_tab_removes_other_controls() {
        assert_eq!(sanitize_inject_text("a\rb\nc\td"), "ab\nc\td");
        // ESC 被移除后残留的普通字符是无害文本（不成转义序列）。
        assert_eq!(sanitize_inject_text("x\u{1b}[31my"), "x[31my");
        assert_eq!(sanitize_inject_text("中文标点，。"), "中文标点，。");
        assert_eq!(sanitize_inject_text("\u{0}\u{7}ok"), "ok");
    }

    #[test]
    fn sanitize_empty_when_all_controls() {
        assert_eq!(sanitize_inject_text("\r\n\u{1b}"), "\n");
        assert_eq!(sanitize_inject_text("\u{0}"), "");
    }
}
