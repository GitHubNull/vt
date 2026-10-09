//! 配置命令：分节读写（tech/04 4.3；T-003 SPEC 1.2）。
//!
//! 命令层只做参数校验（节名枚举、值 Schema 由服务层二次校验），不写日志正文。

use tauri::State;

use crate::error::AppError;
use crate::services::config_store::{AppConfig, ConfigSection};
use crate::state::AppState;

/// 获取完整配置镜像（前端 `api.getConfig()`）。
#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<AppConfig, AppError> {
    Ok(state.config.get().await)
}

/// 更新指定配置节并原子落盘；成功后广播 `config:changed`。
///
/// - `section`：`general` / `hotkey` / `audio` / `output` / `dict` / `about`；
/// - `value`：该节的完整 JSON 对象（Schema 见 `services/config_store.rs`，与前端 types.ts 同步）。
#[tauri::command]
pub async fn set_config(
    state: State<'_, AppState>,
    section: String,
    value: serde_json::Value,
) -> Result<(), AppError> {
    let section = ConfigSection::parse(&section)
        .ok_or_else(|| AppError::Config(2, format!("未知配置节: {section}")))?;
    state.config.set_section(section, value).await
}
