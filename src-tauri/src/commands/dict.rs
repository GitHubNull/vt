//! 替换词典命令（T-011 契约，F-DICT-01；前端封装 `src/api/dict.ts` 逐字对应）。
//!
//! 命令名与参数名为前后端契约（tech/03 3.5），修改须同步前端。

use tauri::State;

use crate::error::AppError;
use crate::services::dict::ReplaceRule;
use crate::state::AppState;

/// 规则清单（管理页数据源）。
#[tauri::command]
pub async fn dict_list_rules(state: State<'_, AppState>) -> Result<Vec<ReplaceRule>, AppError> {
    Ok(state.dict.list().await)
}

/// 新增规则；错误词重复 / 为空 / 超上限返回 E-CONFIG-00x。
#[tauri::command]
pub async fn dict_add_rule(
    state: State<'_, AppState>,
    wrong: String,
    right: String,
    enabled: bool,
) -> Result<ReplaceRule, AppError> {
    state.dict.add_rule(wrong, right, enabled).await
}

/// 更新规则（错误词 / 正确词 / 启停整体替换）。
#[tauri::command]
pub async fn dict_update_rule(
    state: State<'_, AppState>,
    rule: ReplaceRule,
) -> Result<(), AppError> {
    state.dict.update_rule(rule).await
}

/// 删除规则。
#[tauri::command]
pub async fn dict_delete_rule(state: State<'_, AppState>, id: u64) -> Result<(), AppError> {
    state.dict.delete_rule(id).await
}

/// 启停切换。
#[tauri::command]
pub async fn dict_toggle_rule(
    state: State<'_, AppState>,
    id: u64,
    enabled: bool,
) -> Result<(), AppError> {
    state.dict.toggle_rule(id, enabled).await
}
