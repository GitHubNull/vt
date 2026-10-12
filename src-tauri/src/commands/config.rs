//! 配置命令：分节读写（tech/04 4.3；T-003 SPEC 1.2）。
//!
//! 命令层只做参数校验（节名枚举、值 Schema 由服务层二次校验），不写日志正文。

use tauri::State;

use crate::error::AppError;
use crate::services::config_store::{AppConfig, ConfigSection, HotkeyConfig};
use crate::state::AppState;

/// 获取完整配置镜像（前端 `api.getConfig()`）。
#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<AppConfig, AppError> {
    Ok(state.config.get().await)
}

/// 更新指定配置节并原子落盘；成功后广播 `config:changed`。
///
/// - `section`：`general` / `hotkey` / `audio` / `output` / `dict` / `about` 等；
/// - `value`：该节的完整 JSON 对象（Schema 见 `services/config_store.rs`，与前端 types.ts 同步）。
///
/// T-543（F-SYS-03）：`hotkey` 节保存前做应用内重复检测，冲突（同一组合键
/// 分配给多个槽位）拒绝保存并指出冲突项；OS 常用组合只预警不拦截（前端录入处提示）。
#[tauri::command]
pub async fn set_config(
    state: State<'_, AppState>,
    section: String,
    value: serde_json::Value,
) -> Result<(), AppError> {
    let section = ConfigSection::parse(&section)
        .ok_or_else(|| AppError::Config(2, format!("未知配置节: {section}")))?;
    if section == ConfigSection::Hotkey {
        // Schema 先校验（错误值不进冲突检测），再做重复检测。
        let hotkey: HotkeyConfig = serde_json::from_value(value.clone())
            .map_err(|err| AppError::Config(4, format!("配置节值不合法: {err}")))?;
        crate::services::hotkey::validate_hotkey_config(&hotkey)
            .map_err(|err| AppError::Hotkey(3, err.to_string()))?;
    }
    state.config.set_section(section, value).await
}
