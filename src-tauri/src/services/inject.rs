//! InjectService（tech/05 5.3.5）：文本上屏三级降级链编排。
//!
//! 降级链（优先级递减）：
//! 1. **模拟键入**（enigo / 平台 API）：最通用；`paste_input = false` 时为主路径；
//! 2. **剪贴板粘贴**：`paste_input = true`（默认）时为主路径；目标应用键入异常
//!    （兼容清单命中，T-007 机制）或键入失败 / 权限缺失（macOS 辅助功能）时降级到此；
//!    执行序列：备份原剪贴板 → 写入文本 → 模拟 Ctrl/Cmd+V → 恢复原剪贴板
//!    （仅当 `keep_text_in_clipboard = true` 时保留识别文本）；
//! 3. **无焦点兜底**（F-INPUT-09）：检测不到输入焦点或全部路径失败时，文本写入
//!    剪贴板 + 系统通知"已复制，可手动粘贴"。
//!
//! 硬约束（NFR-S-05，AGENTS.md 3.2 原则 4）：本服务不输出任何回车 / 执行类按键。
//! 注入文本先经 [`sanitize_inject_text`] 清洗（保留 `\n` / `\t`，移除其余控制字符），
//! 平台层只暴露文本键入与粘贴快捷键两个出口。

use std::sync::Arc;

use tracing::{info, warn};

use crate::error::AppError;
use crate::platform::inject::{
    sanitize_inject_text, FocusState, InjectError, InjectMethod, InjectReport, TextInjector,
};
use crate::services::config_store::ConfigStore;

/// 错误编号：注入文本为空（清洗后）。
pub const ERR_EMPTY_TEXT: u32 = 1;
/// 错误编号：全部上屏路径失败且兜底写剪贴板也失败。
pub const ERR_ALL_PATHS_FAILED: u32 = 2;

/// 粘贴快捷键发出后、恢复原剪贴板前的等待时延。
///
/// `simulate_paste`（enigo SendInput）只把 Ctrl+V 事件送入系统输入队列即返回，
/// 目标应用异步处理按键并读取剪贴板；若立即恢复剪贴板，目标应用读到的将是
/// 旧内容（Windows 上必现，表现为"粘贴出历史剪贴板内容"）。等待目标应用
/// 完成粘贴后再恢复。
const PASTE_SETTLE_DURATION: std::time::Duration = std::time::Duration::from_millis(200);

/// 系统通知抽象（F-INPUT-09 无焦点兜底提示；测试以 mock 断言）。
pub trait Notifier: Send + Sync {
    /// 发送系统通知（尽力而为：失败只记日志，不影响主流程）。
    fn notify(&self, title: &str, body: &str);
}

/// InjectService：降级链编排。平台调用为同步阻塞操作，统一进 `spawn_blocking`。
#[derive(Clone)]
pub struct InjectService {
    injector: Arc<dyn TextInjector>,
    notifier: Arc<dyn Notifier>,
    config: ConfigStore,
}

impl InjectService {
    pub fn new(
        injector: Arc<dyn TextInjector>,
        notifier: Arc<dyn Notifier>,
        config: ConfigStore,
    ) -> Self {
        Self {
            injector,
            notifier,
            config,
        }
    }

    /// 把文本输入到当前光标处（`inject_text` 命令与 T-012 管线共用入口）。
    pub async fn inject_text(&self, text: &str) -> Result<InjectReport, AppError> {
        let text = sanitize_inject_text(text);
        if text.is_empty() {
            return Err(AppError::Inject(ERR_EMPTY_TEXT, "注入文本为空".to_string()));
        }
        let output = self.config.get().await.output;

        // 第 3 级前置判定：明确无输入焦点时直接走兜底（F-INPUT-09）。
        if self.injector.input_focus() == FocusState::Unfocused {
            info!("未检测到输入焦点，进入无焦点兜底");
            return self
                .fallback_to_clipboard(&text, "未检测到输入焦点，已复制到剪贴板")
                .await;
        }

        // 兼容清单命中（已知模拟键入异常应用）：强制剪贴板路径（T-007 机制）。
        let compat_hit = self
            .injector
            .focused_app()
            .map(|app| {
                output
                    .compat_clipboard_apps
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(&app))
            })
            .unwrap_or(false);

        // 主路径顺序由"使用复制粘贴输入"配置决定（PRD 3.8.3，默认粘贴优先）。
        let mut methods: std::collections::VecDeque<InjectMethod> =
            if output.paste_input || compat_hit {
                [InjectMethod::Pasted, InjectMethod::Typed].into()
            } else {
                [InjectMethod::Typed, InjectMethod::Pasted].into()
            };

        let mut last_error: Option<InjectError> = None;
        while let Some(method) = methods.pop_front() {
            // 首个尝试即用户选择的主路径，其后为降级路径。
            let result = match method {
                InjectMethod::Typed => self.try_typing(&text).await,
                InjectMethod::Pasted => self.try_paste(&text, output.keep_text_in_clipboard).await,
                InjectMethod::ClipboardFallback => break,
            };
            match result {
                Ok(report) => return Ok(report),
                Err(err) => {
                    warn!(method = ?method, error = %err, "上屏路径失败，尝试下一级降级");
                    last_error = Some(err);
                }
            }
        }

        // 全部路径失败 → 无焦点兜底（兜底成功时也返回降级报告）。
        warn!(error = ?last_error, "上屏路径全部失败，进入无焦点兜底");
        self.fallback_to_clipboard(&text, "上屏失败，已复制到剪贴板")
            .await
    }

    /// 路径 1：模拟键入。权限缺失（macOS 辅助功能）与普失败均返回降级原因。
    async fn try_typing(&self, text: &str) -> Result<InjectReport, InjectError> {
        let injector = self.injector.clone();
        let text = text.to_string();
        tokio::task::spawn_blocking(move || injector.type_text(&text))
            .await
            .map_err(|err| InjectError::Type(format!("任务执行失败: {err}")))??;
        info!("文本已模拟键入");
        Ok(InjectReport {
            method: InjectMethod::Typed,
            degraded: None,
        })
    }

    /// 路径 2：剪贴板粘贴（备份 → 写入 → 粘贴 → 按配置恢复）。
    async fn try_paste(
        &self,
        text: &str,
        keep_text_in_clipboard: bool,
    ) -> Result<InjectReport, InjectError> {
        let injector = self.injector.clone();
        let text_owned = text.to_string();
        tokio::task::spawn_blocking(move || {
            let backup = injector.clipboard_backup()?;
            let paste_result = (|| -> Result<(), InjectError> {
                injector.write_clipboard(&text_owned)?;
                injector.simulate_paste()
            })();
            // 无论粘贴成败，按配置恢复原剪贴板（PRD 3.1.3 规则 1）。
            // 粘贴成功时须等待目标应用完成粘贴（PASTE_SETTLE_DURATION），
            // 否则目标应用会读到恢复后的旧内容。
            if !keep_text_in_clipboard {
                if paste_result.is_ok() {
                    std::thread::sleep(PASTE_SETTLE_DURATION);
                }
                if let Err(err) = injector.clipboard_restore(&backup) {
                    warn!(error = %err, "剪贴板恢复失败");
                }
            }
            paste_result
        })
        .await
        .map_err(|err| InjectError::Clipboard(format!("任务执行失败: {err}")))??;

        info!("文本已剪贴板粘贴");
        Ok(InjectReport {
            method: InjectMethod::Pasted,
            degraded: None,
        })
    }

    /// 路径 3：无焦点兜底——文本写入剪贴板 + 系统通知（F-INPUT-09）。
    async fn fallback_to_clipboard(
        &self,
        text: &str,
        reason: &str,
    ) -> Result<InjectReport, AppError> {
        let injector = self.injector.clone();
        let text = text.to_string();
        tokio::task::spawn_blocking(move || injector.write_clipboard(&text))
            .await
            .map_err(|err| AppError::Inject(ERR_ALL_PATHS_FAILED, format!("任务执行失败: {err}")))?
            .map_err(|err| AppError::Inject(ERR_ALL_PATHS_FAILED, err.to_string()))?;

        self.notifier
            .notify("VibeType", "已复制到剪贴板，可手动粘贴");
        info!("无焦点兜底：文本已写入剪贴板并通知");
        Ok(InjectReport {
            method: InjectMethod::ClipboardFallback,
            degraded: Some(reason.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::inject::{ClipboardBackup, FocusState, MockTextInjector};

    /// 记录通知的 mock。
    struct MockNotifier {
        notifications: std::sync::Mutex<Vec<(String, String)>>,
    }

    impl MockNotifier {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                notifications: std::sync::Mutex::new(Vec::new()),
            })
        }

        fn notifications(&self) -> Vec<(String, String)> {
            self.notifications.lock().unwrap().clone()
        }
    }

    impl Notifier for MockNotifier {
        fn notify(&self, title: &str, body: &str) {
            self.notifications
                .lock()
                .unwrap()
                .push((title.to_string(), body.to_string()));
        }
    }

    /// 测试环境：mock 注入器 + 通知 + 临时目录配置（可覆盖输出节）。
    struct Env {
        injector: Arc<MockTextInjector>,
        notifier: Arc<MockNotifier>,
        service: InjectService,
    }

    async fn setup(output_patch: Option<serde_json::Value>) -> Env {
        let dir = std::env::temp_dir().join(format!(
            "vt-inject-test-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (events_tx, _events_rx) = crate::events::channel();
        let config = ConfigStore::load(&dir, events_tx.clone()).unwrap();
        if let Some(patch) = output_patch {
            config
                .set_section(crate::services::config_store::ConfigSection::Output, patch)
                .await
                .unwrap();
        }
        let injector = Arc::new(MockTextInjector::new());
        let notifier = MockNotifier::new();
        let service = InjectService::new(injector.clone(), notifier.clone(), config.clone());
        Env {
            injector,
            notifier,
            service,
        }
    }

    /// 基于默认输出节打补丁（serde(default) 兼容：未patch字段保持默认）。
    fn patched_output(patch: serde_json::Value) -> serde_json::Value {
        let mut output =
            serde_json::to_value(crate::services::config_store::OutputConfig::default()).unwrap();
        for (key, value) in patch.as_object().cloned().unwrap_or_default() {
            output[key] = value;
        }
        output
    }

    #[tokio::test]
    async fn paste_is_default_primary_and_restores_clipboard() {
        let env = setup(None).await;
        let report = env.service.inject_text("识别结果").await.unwrap();
        assert_eq!(report.method, InjectMethod::Pasted);
        // 粘贴路径：备份 → 写入 → 粘贴 → 恢复；键入未发生。
        assert_eq!(env.injector.backups.lock().unwrap().len(), 1);
        assert_eq!(env.injector.pastes.lock().unwrap().len(), 1);
        assert_eq!(env.injector.typed.lock().unwrap().len(), 0);
        assert_eq!(
            env.injector.restores.lock().unwrap().as_slice(),
            &[ClipboardBackup::Text("原剪贴板内容".to_string())]
        );
        // 原剪贴板内容已恢复（默认不保留识别文本）。
        assert_eq!(env.injector.clipboard_content(), "原剪贴板内容");
        // 兜底未触发。
        assert!(env.notifier.notifications().is_empty());
    }

    #[tokio::test]
    async fn paste_settles_before_clipboard_restore() {
        // 回归：粘贴快捷键发出后须驻留 PASTE_SETTLE_DURATION 再恢复剪贴板，
        // 否则目标应用（Windows 必现）读到的是恢复后的旧剪贴板内容。
        let env = setup(None).await;
        env.service.inject_text("驻留时延").await.unwrap();
        assert_eq!(
            env.injector.ops.lock().unwrap().as_slice(),
            &["backup", "write", "paste", "restore"]
        );
        let paste_at = env.injector.paste_at.lock().unwrap().unwrap();
        let restore_at = env.injector.restore_at.lock().unwrap().unwrap();
        assert!(
            restore_at.duration_since(paste_at) >= PASTE_SETTLE_DURATION,
            "恢复剪贴板前须等待目标应用完成粘贴"
        );
    }

    #[tokio::test]
    async fn keep_text_in_clipboard_skips_restore() {
        let env = setup(Some(patched_output(serde_json::json!({
            "keep_text_in_clipboard": true,
        }))))
        .await;
        let report = env.service.inject_text("保留我").await.unwrap();
        assert_eq!(report.method, InjectMethod::Pasted);
        assert!(env.injector.restores.lock().unwrap().is_empty());
        assert_eq!(env.injector.clipboard_content(), "保留我");
    }

    #[tokio::test]
    async fn type_first_when_paste_input_disabled() {
        let env = setup(Some(patched_output(serde_json::json!({
            "paste_input": false,
        }))))
        .await;
        let report = env.service.inject_text("逐字输入").await.unwrap();
        assert_eq!(report.method, InjectMethod::Typed);
        assert_eq!(env.injector.typed.lock().unwrap().as_slice(), &["逐字输入"]);
        // 未走剪贴板路径。
        assert_eq!(env.injector.pastes.lock().unwrap().len(), 0);
        assert!(env.injector.restores.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn typing_failure_falls_back_to_paste() {
        let env = setup(Some(patched_output(serde_json::json!({
            "paste_input": false,
        }))))
        .await;
        env.injector
            .fail_type
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let report = env.service.inject_text("降级粘贴").await.unwrap();
        assert_eq!(report.method, InjectMethod::Pasted);
        assert_eq!(env.injector.pastes.lock().unwrap().len(), 1);
        // 剪贴板恢复原内容。
        assert_eq!(env.injector.clipboard_content(), "原剪贴板内容");
    }

    #[tokio::test]
    async fn permission_denied_degrades_to_paste() {
        let env = setup(Some(patched_output(serde_json::json!({
            "paste_input": false,
        }))))
        .await;
        env.injector
            .type_permission_denied
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let report = env.service.inject_text("macOS 未授权场景").await.unwrap();
        assert_eq!(report.method, InjectMethod::Pasted);
    }

    #[tokio::test]
    async fn compat_list_forces_paste_path() {
        let env = setup(Some(patched_output(serde_json::json!({
            "paste_input": false,
            "compat_clipboard_apps": ["weird-app"],
        }))))
        .await;
        env.injector.set_focused_app(Some("Weird-App".to_string()));
        let report = env.service.inject_text("兼容清单命中").await.unwrap();
        assert_eq!(report.method, InjectMethod::Pasted);
        // 键入被跳过（清单应用响应异常，T-007 机制）。
        assert_eq!(env.injector.typed.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn no_focus_copies_to_clipboard_and_notifies() {
        let env = setup(None).await;
        env.injector.set_focus(FocusState::Unfocused);
        let report = env.service.inject_text("无焦点文本").await.unwrap();
        assert_eq!(report.method, InjectMethod::ClipboardFallback);
        assert!(report.degraded.is_some());
        // 文本进入剪贴板、弹出通知；未发生键入 / 粘贴。
        assert_eq!(env.injector.clipboard_content(), "无焦点文本");
        let notifications = env.notifier.notifications();
        assert_eq!(notifications.len(), 1);
        assert!(notifications[0].1.contains("剪贴板"));
        env.injector.assert_no_input();
    }

    #[tokio::test]
    async fn all_paths_failed_still_falls_back() {
        let env = setup(None).await;
        env.injector
            .fail_paste
            .store(true, std::sync::atomic::Ordering::SeqCst);
        // paste 主路径失败 → 键入兜底（mock 键入成功）仍是 Pasted→Typed 降级链；
        // 此处让键入也失败，验证最终兜底。
        env.injector
            .fail_type
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let report = env.service.inject_text("全失败").await.unwrap();
        assert_eq!(report.method, InjectMethod::ClipboardFallback);
        assert_eq!(env.injector.clipboard_content(), "全失败");
        assert_eq!(env.notifier.notifications().len(), 1);
    }

    #[tokio::test]
    async fn clipboard_write_failure_reports_error() {
        let env = setup(None).await;
        // 剪贴板写入失败：粘贴路径失败 → 键入路径失败 → 兜底写剪贴板也失败 → 报错。
        env.injector
            .fail_clipboard_write
            .store(true, std::sync::atomic::Ordering::SeqCst);
        env.injector
            .fail_type
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let err = env.service.inject_text("写不进剪贴板").await.unwrap_err();
        assert_eq!(err.code(), "E-INJECT-002");
    }

    #[tokio::test]
    async fn empty_text_after_sanitize_is_rejected() {
        let env = setup(None).await;
        let err = env.service.inject_text("\u{0}\u{1b}").await.unwrap_err();
        assert_eq!(err.code(), "E-INJECT-001");
    }

    #[tokio::test]
    async fn control_chars_never_reach_injector() {
        let env = setup(Some(patched_output(serde_json::json!({
            "paste_input": false,
        }))))
        .await;
        let report = env.service.inject_text("a\rb\u{1b}c\nd").await.unwrap();
        assert_eq!(report.method, InjectMethod::Typed);
        let typed = env.injector.typed.lock().unwrap();
        assert_eq!(typed.as_slice(), &["abc\nd"]);
    }

    #[tokio::test]
    async fn paste_shortcut_is_ctrl_v_never_enter() {
        let env = setup(None).await;
        env.service.inject_text("粘贴快捷键断言").await.unwrap();
        let pastes = env.injector.pastes.lock().unwrap();
        assert_eq!(pastes.len(), 1);
        // 只允许修饰键 + V：不产生任何回车 / 执行类按键。
        assert_eq!(pastes[0].key, 'v');
        assert!(matches!(pastes[0].modifier, "ctrl" | "meta"));
    }
}
