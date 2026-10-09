//! 系统通知平台实现（F-INPUT-09 无焦点兜底；经 Tauri notification 插件，
//! 三平台由插件内部收敛，Linux 需通知服务可用）。
//!
//! 最小权限说明（AGENTS.md 3.5）：仅发送应用自身的本地通知，不读取通知内容、
//! 不监听系统通知。

use std::sync::Arc;

use tauri_plugin_notification::NotificationExt;

use crate::services::inject::Notifier;

/// Tauri 通知插件实现：发送失败只记日志，不影响上屏主流程（尽力而为）。
pub struct TauriNotifier {
    app: tauri::AppHandle,
}

impl TauriNotifier {
    pub fn new(app: tauri::AppHandle) -> Arc<Self> {
        Arc::new(Self { app })
    }
}

impl Notifier for TauriNotifier {
    fn notify(&self, title: &str, body: &str) {
        if let Err(err) = self
            .app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
        {
            tracing::warn!(error = %err, "系统通知发送失败");
        }
    }
}
