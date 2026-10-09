//! 音频命令：设备枚举（T-005；T-013 录音环境页消费）、录音控制（T-008/T-009/T-010
//! 前端入口：悬浮球取消区、小窗麦克风按钮 / 取消按钮）。

use tauri::State;

use crate::error::AppError;
use crate::platform::audio::AudioDeviceInfo;
use crate::services::hotkey::RecordCommand;
use crate::state::AppState;

/// 列出系统输入设备（麦克风）。
///
/// 返回 `AudioDeviceInfo[]`：`id`（设备标识，用于 `audio.input_device_id`）、
/// `name`（展示名）、`is_default`（是否系统默认）。
#[tauri::command]
pub async fn list_audio_devices(
    state: State<'_, AppState>,
) -> Result<Vec<AudioDeviceInfo>, AppError> {
    state
        .audio_backend
        .list_devices()
        .map_err(|err| AppError::Audio(3, err.to_string()))
}

/// 录音控制命令（T-008 悬浮球取消区、T-009 小窗麦克风按钮、T-010 取消入口）。
///
/// `command`：`start` / `stop` / `toggle` / `cancel`；与快捷键共用同一录音状态机
/// （AudioService 单会话互斥：录音中重复 start 被忽略，T-009 验收 4）。
#[tauri::command]
pub async fn control_recording(
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
