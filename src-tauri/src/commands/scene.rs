//! 场景识别命令（T-501 契约，F-CODE-01；前端封装随 T-502 设置页补充，命令名与
//! 参数名为前后端契约 tech/03 3.5，修改须同步前端）。
//!
//! 手动场景指定（`scene_set_override` / `scene_clear_override`）是 Wayland 等
//! 识别受限环境的兜底入口（SPEC T-501 步骤 4）；托盘菜单项接入时复用同一命令。

use tauri::State;

use crate::error::AppError;
use crate::services::config_store::SceneRule;
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
