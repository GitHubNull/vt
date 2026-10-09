//! 快捷键联调命令（T-004 SPEC 步骤 7：临时联调，正式 UI 在后续任务）。

use tauri::State;

use crate::error::AppError;
use crate::services::hotkey::RecordCommand;
use crate::state::AppState;

/// 临时联调命令：直接往录音控制通道注入一条命令，验证事件流。
///
/// `command`：`start` / `stop` / `toggle` / `cancel`。
#[tauri::command]
pub async fn trigger_test_hotkey(
    state: State<'_, AppState>,
    command: String,
) -> Result<(), AppError> {
    let cmd = match command.as_str() {
        "start" => RecordCommand::Start,
        "stop" => RecordCommand::Stop,
        "toggle" => RecordCommand::Toggle,
        "cancel" => RecordCommand::Cancel,
        _ => {
            return Err(AppError::Hotkey(2, format!("未知录音控制命令: {command}")));
        }
    };
    state
        .record_tx
        .send(cmd)
        .await
        .map_err(|_| AppError::Hotkey(3, "录音控制通道不可用".to_string()))
}
