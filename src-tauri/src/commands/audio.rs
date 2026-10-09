//! 音频命令：设备枚举（T-005；T-013 录音环境页消费）。

use tauri::State;

use crate::error::AppError;
use crate::platform::audio::AudioDeviceInfo;
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
