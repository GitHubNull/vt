//! 快捷键命令：T-004 联调通道 + T-543 冲突校验。

use tauri::State;

use crate::error::AppError;
use crate::services::hotkey;
use crate::services::hotkey::{HotkeySlot, HotkeyValidationReport, RecordCommand};
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

/// 快捷键冲突校验（T-543，F-SYS-03）：设置页录入的即时反馈数据源。
///
/// 入参为待校验的快捷键槽位集合（录音触发键 / 取消键 / 将来的模板键），
/// 返回应用内重复组（保存将被拒绝）与 OS 常用组合预警（允许保存）。
/// 只读校验：不落盘、不注册；保存时的权威校验在 `commands/config.rs`
/// 的 `set_config` 中接入（E-HOTKEY-003）。
#[tauri::command]
pub async fn validate_hotkeys(slots: Vec<HotkeySlot>) -> Result<HotkeyValidationReport, AppError> {
    // 纯函数校验（归一化 + 预警表匹配），无状态访问；空集合返回空报告。
    Ok(hotkey::validate_hotkeys(&slots))
}
