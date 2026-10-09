//! 模型管理命令（T-006 契约，F-SET-06 服务端部分；前端封装 `src/api/asr.ts` 逐字对应）。
//!
//! 命令名与参数名为前后端契约（tech/03 3.5），修改须同步前端。

use tauri::State;

use crate::error::AppError;
use crate::services::asr::ModelInfo;
use crate::state::AppState;

/// 模型清单（含下载 / 激活 / 损坏状态），T-014 模型管理页数据源。
#[tauri::command]
pub async fn list_models(state: State<'_, AppState>) -> Result<Vec<ModelInfo>, AppError> {
    Ok(state.asr.list_models().await)
}

/// 下载模型（幂等；已在下载中直接返回）。进度经 `model:progress` 事件推送。
#[tauri::command]
pub async fn download_model(state: State<'_, AppState>, model_id: String) -> Result<(), AppError> {
    state.asr.download_model(model_id).await
}

/// 取消进行中的下载。
#[tauri::command]
pub async fn cancel_download(state: State<'_, AppState>, model_id: String) -> Result<(), AppError> {
    state.asr.cancel_download(model_id).await
}

/// 删除模型；删除激活模型时回退到首个可用模型并广播 `asr:notice` 提示。
#[tauri::command]
pub async fn delete_model(state: State<'_, AppState>, model_id: String) -> Result<(), AppError> {
    state.asr.delete_model(model_id).await
}

/// 切换激活模型：已下载模型间切换，切换后下一次识别即时生效。
#[tauri::command]
pub async fn set_active_model(
    state: State<'_, AppState>,
    model_id: String,
) -> Result<(), AppError> {
    state.asr.set_active_model(model_id).await
}

/// 识别指定 16 kHz 单声道 WAV 文件（本地验证命令，返回识别文本）。
///
/// 供开发 / 验收驱动识别链路（T-011 验收亦用此命令）；识别文本不入日志（NFR-S-02）。
#[tauri::command]
pub async fn asr_recognize_wav(
    state: State<'_, AppState>,
    wav_path: String,
) -> Result<String, AppError> {
    state.asr.recognize_wav(wav_path).await
}
