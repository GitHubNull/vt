//! 前台窗口信息读取（T-501，SPEC: doc/specs/v0.5/T-501-scene-detection.md）。
//!
//! 三平台实现随任务补充；调用 OS API 处必须附最小权限注释（AGENTS.md 3.5）。
//! 读取到的窗口标题等识别信息不得写入日志（NFR-S-02）。

use serde::{Deserialize, Serialize};

/// 前台应用信息（`active_app()` 返回值，三平台统一模型）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    /// 应用显示名（如 "Visual Studio Code"）。
    pub name: String,
    /// 进程名（如 "code"；场景规则库主要按此匹配）。
    pub process: String,
    /// 窗口标题（AI 对话页等按标题模式细分时使用；不得入日志）。
    pub title: String,
}
