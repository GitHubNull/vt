//! 场景识别与规则命令（T-501 契约 + T-502 扩展，F-CODE-01；前端封装见
//! src/api/scene.ts，命令名与参数名为前后端契约 tech/03 3.5，修改须同步前端）。
//!
//! 手动场景指定（`scene_set_override` / `scene_clear_override`）即 PRD 3.2.1
//! 业务规则 3 的"临时锁定场景"：锁定期间自动识别结果不生效，直到解除；
//! 托盘菜单与设置页"场景规则"共用同一组命令。
//!
//! T-502 新增场景规则条目命令（CRUD / 恢复默认 / 拖动排序持久化）：
//! - `scene_list_entries`：生效条目清单（配置为空 = 内置五场景默认）；
//! - `scene_add_entry` / `scene_update_entry` / `scene_delete_entry`：条目 CRUD
//!   （内置可编辑不可删除，ID 与内置标记由后端裁决）；
//! - `scene_restore_entry_default`：内置条目恢复 PRD 默认规则（保持列表位置）；
//! - `scene_reorder_entries`：持久化拖动排序（清单须为全部条目 ID 的排列）。

use tauri::State;

use crate::error::AppError;
use crate::services::config_store::{SceneEntry, SceneRule};
use crate::services::scene::{SceneKind, SceneService, SceneStatus};
use crate::state::AppState;

/// 获取 `SceneService` 句柄（AppState 聚合，见 state.rs / main.rs 接线）。
fn scene_service<'a>(state: &'a State<'a, AppState>) -> &'a SceneService {
    &state.scene
}

/// 查询当前场景（override 优先，自动识别走短缓存；`app` / `process` 仅用于
/// 设置页"当前识别到：×× → ××场景"展示）。
#[tauri::command]
pub async fn scene_get_current(state: State<'_, AppState>) -> Result<SceneStatus, AppError> {
    Ok(scene_service(&state).status().await)
}

/// 手动指定当前场景（锁定，优先于自动识别；Wayland 等识别受限环境兜底）。
/// 持久化到 `scene.override_scene`，成功后广播 `scene:changed`。
#[tauri::command]
pub async fn scene_set_override(
    state: State<'_, AppState>,
    scene: SceneKind,
) -> Result<(), AppError> {
    scene_service(&state).set_override(scene).await
}

/// 清除手动指定，恢复自动识别；成功后广播 `scene:changed`。
#[tauri::command]
pub async fn scene_clear_override(state: State<'_, AppState>) -> Result<(), AppError> {
    scene_service(&state).clear_override().await
}

/// 自定义规则清单（`scene.custom_rules`，应用标识 → 场景）。
#[tauri::command]
pub async fn scene_list_custom_rules(
    state: State<'_, AppState>,
) -> Result<Vec<SceneRule>, AppError> {
    Ok(scene_service(&state).list_custom_rules().await)
}

/// 新增自定义规则（优先于内置规则库）；应用标识为空 / 过长 / 目标场景为 Unknown /
/// 超上限返回 E-CONFIG-00x。
#[tauri::command]
pub async fn scene_add_custom_rule(
    state: State<'_, AppState>,
    app: String,
    scene: SceneKind,
) -> Result<SceneRule, AppError> {
    scene_service(&state).add_custom_rule(app, scene).await
}

/// 更新自定义规则（按应用标识匹配，整体替换目标场景）。
#[tauri::command]
pub async fn scene_update_custom_rule(
    state: State<'_, AppState>,
    rule: SceneRule,
) -> Result<(), AppError> {
    scene_service(&state).update_custom_rule(rule).await
}

/// 删除自定义规则（按应用标识，大小写不敏感）。
#[tauri::command]
pub async fn scene_delete_custom_rule(
    state: State<'_, AppState>,
    app: String,
) -> Result<(), AppError> {
    scene_service(&state).delete_custom_rule(&app).await
}

/// 场景规则条目清单（T-502）：生效值——配置为空 = 内置五场景默认；
/// 内置条目始终在列（可编辑、不可删除、可恢复默认）。
#[tauri::command]
pub async fn scene_list_entries(state: State<'_, AppState>) -> Result<Vec<SceneEntry>, AppError> {
    Ok(scene_service(&state).list_entries().await)
}

/// 新增自定义场景条目（T-502）：ID 由后端生成（`custom-N`），内置标记恒为 false；
/// 名称 / 应用列表 / 关键字先归一（裁剪、去重）再校验（非法返回 E-CONFIG-004）。
#[tauri::command]
pub async fn scene_add_entry(
    state: State<'_, AppState>,
    entry: SceneEntry,
) -> Result<SceneEntry, AppError> {
    scene_service(&state).add_entry(entry).await
}

/// 更新场景条目（T-502）：内置条目可编辑；ID 与内置标记由后端保持不变，
/// 内置条目的归属场景不可改（编码场景恒映射编码）。
#[tauri::command]
pub async fn scene_update_entry(
    state: State<'_, AppState>,
    entry: SceneEntry,
) -> Result<(), AppError> {
    scene_service(&state).update_entry(entry).await
}

/// 删除场景条目（T-502）：仅自定义条目；内置条目返回 E-CONFIG-005。
#[tauri::command]
pub async fn scene_delete_entry(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<(), AppError> {
    scene_service(&state).delete_entry(&entry_id).await
}

/// 恢复内置场景条目的 PRD 默认规则（T-502）：仅内置条目；
/// 保持列表位置（排序即匹配优先级），返回恢复后的条目。
#[tauri::command]
pub async fn scene_restore_entry_default(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<SceneEntry, AppError> {
    scene_service(&state).restore_entry_default(&entry_id).await
}

/// 持久化场景条目排序（T-502 拖动排序）：`entry_ids` 须为当前全部条目 ID
/// 的排列（长度与集合一致），否则拒绝（E-CONFIG-004）。
#[tauri::command]
pub async fn scene_reorder_entries(
    state: State<'_, AppState>,
    entry_ids: Vec<String>,
) -> Result<(), AppError> {
    scene_service(&state).reorder_entries(entry_ids).await
}
