//! AI 供应商与模板命令层（T-531 / T-532 / T-533；前端封装 `src/api/ai.ts`）。
//!
//! 命令名与参数名为前后端契约（tech/03 3.5），修改须同步前端。
//! 返回统一 `Result<T, AppError>`（前端拿到 `{ code, message }` 稳定错误编号）。
//!
//! 安全约定（NFR-S-01/02）：
//! - API Key 只能**写入**（`ai_set_api_key`）与**清除**（`ai_clear_api_key`），
//!   任何命令都不回传 Key 明文；`ai_list_providers` / `ai_key_status` 只回
//!   "已设置"布尔状态；
//! - 模板正文属于用户内容（NFR-S-02），命令层不透传日志、不写日志。

use tauri::State;

use crate::error::AppError;
use crate::platform::credentials::CredentialBackendInfo;
use crate::services::ai::presets::PresetProvider;
use crate::services::ai::service::{AiState, ModelMergeReport, ProviderView, RestoreReport};
use crate::services::ai::templates::AiTemplate;
use crate::services::ai::{CompletionRequest, CompletionResponse, ConnectionTest, ModelInfo};
use crate::services::config_store::AiProviderEntry;

/// 预置供应商清单（设置页"添加供应商"数据源，PRD 3.7 名单）。
#[tauri::command]
pub async fn ai_list_presets() -> Vec<PresetProvider> {
    crate::services::ai::presets::CATALOG.to_vec()
}

/// 供应商清单（首次调用自动灌入预置；含 Key"已设置"状态与推荐 / 本地标记）。
#[tauri::command]
pub async fn ai_list_providers(state: State<'_, AiState>) -> Result<Vec<ProviderView>, AppError> {
    state.list_providers().await
}

/// 恢复预置供应商：按 ID 去重，同名不同 ID 的预置跳过并在报告中列出
/// （PRD 3.7 业务规则 1：设置页据此提示合并或重命名）。
#[tauri::command]
pub async fn ai_restore_presets(state: State<'_, AiState>) -> Result<RestoreReport, AppError> {
    state.restore_presets().await
}

/// 新增或更新供应商（名称唯一、协议与地址合法性校验；不改密钥）。
#[tauri::command]
pub async fn ai_upsert_provider(
    state: State<'_, AiState>,
    entry: AiProviderEntry,
) -> Result<(), AppError> {
    state.upsert_provider(entry).await
}

/// 删除供应商（同时删除其 OS 凭据条目，不留孤儿秘密）。
#[tauri::command]
pub async fn ai_delete_provider(state: State<'_, AiState>, id: String) -> Result<(), AppError> {
    state.delete_provider(&id).await
}

/// 设置 API Key：Key 本体进 OS 凭据管理器（无 Secret Service 时降级加密），
/// 配置文件只写引用键。Key 不出现在任何返回值与日志中。
#[tauri::command]
pub async fn ai_set_api_key(
    state: State<'_, AiState>,
    id: String,
    api_key: String,
) -> Result<(), AppError> {
    state.set_api_key(&id, &api_key).await
}

/// 清除 API Key：删除凭据条目并清掉配置引用（幂等）。
#[tauri::command]
pub async fn ai_clear_api_key(state: State<'_, AiState>, id: String) -> Result<(), AppError> {
    state.clear_api_key(&id).await
}

/// 查询 Key 是否已设置（只回布尔状态，永不回明文，NFR-S-01 界面掩码约定）。
#[tauri::command]
pub async fn ai_key_status(state: State<'_, AiState>, id: String) -> Result<bool, AppError> {
    state.key_status(&id).await
}

/// 连通性测试（服务端发起，PRD 3.7"测试连接"）：成功返回延迟，
/// 失败返回 `ok=false` + 稳定错误编号（Key 无效 / 不可达 / 限流等可读原因）。
#[tauri::command]
pub async fn ai_test_connection(
    state: State<'_, AiState>,
    id: String,
) -> Result<ConnectionTest, AppError> {
    state.test_connection(&id).await
}

/// 从供应商拉取可用模型清单（F-AI-08"获取模型列表"，OpenAI 兼容 /models、
/// Ollama /api/tags）。
#[tauri::command]
pub async fn ai_fetch_models(
    state: State<'_, AiState>,
    id: String,
) -> Result<Vec<ModelInfo>, AppError> {
    state.fetch_models(&id).await
}

/// 文本补全联调命令（F-AI-07 / F-AI-04 服务端链路）：模板引擎（T-533）与
/// AI Stage（T-534）落地前的前端真机联调入口。
///
/// 断网时云端供应商返回 E-NET-101 等稳定编号，本地供应商不受影响
/// （超时降级为原文上屏的策略在 T-534 管线侧实现）。
#[tauri::command]
pub async fn ai_complete(
    state: State<'_, AiState>,
    id: String,
    request: CompletionRequest,
) -> Result<CompletionResponse, AppError> {
    state.complete(&id, &request).await
}

/// 当前凭据存储后端与保护级别（设置页按 tech/08 8.3 明示，
/// Linux 无 Secret Service 时展示降级加密说明）。
#[tauri::command]
pub async fn ai_credential_backend(
    state: State<'_, AiState>,
) -> Result<CredentialBackendInfo, AppError> {
    Ok(state.credential_backend())
}

/// 拉取供应商模型清单并合并持久化（T-532，F-AI-08；SPEC 的 list_models）：
/// 支持列表 API 的供应商（OpenAI 兼容 /models、Ollama /api/tags）一键拉取；
/// 结果规范化后与已存清单合并（去重保序）并原子落盘。
#[tauri::command]
pub async fn ai_fetch_and_merge_models(
    state: State<'_, AiState>,
    id: String,
) -> Result<ModelMergeReport, AppError> {
    state.fetch_and_merge_models(&id).await
}

/// 手动添加模型条目（T-532；供应商不支持列表 API 时的兜底；同供应商下标识唯一）。
#[tauri::command]
pub async fn ai_add_provider_model(
    state: State<'_, AiState>,
    id: String,
    model: String,
) -> Result<(), AppError> {
    state.add_provider_model(&id, &model).await
}

/// 删除模型条目（T-532；删除当前默认模型时默认选择一并清空）。
#[tauri::command]
pub async fn ai_remove_provider_model(
    state: State<'_, AiState>,
    id: String,
    model: String,
) -> Result<(), AppError> {
    state.remove_provider_model(&id, &model).await
}

/// 设置 / 清除供应商默认模型（T-532；`model=null` 清除；
/// 持久化后供未显式指定模型的后续调用使用）。
#[tauri::command]
pub async fn ai_set_provider_model(
    state: State<'_, AiState>,
    id: String,
    model: Option<String>,
) -> Result<(), AppError> {
    state.set_default_model(&id, model.as_deref()).await
}

/// 模板清单（T-533，F-AI-01；首次调用自动初始化内置模板，内置在前）。
#[tauri::command]
pub async fn ai_list_templates(state: State<'_, AiState>) -> Result<Vec<AiTemplate>, AppError> {
    state.list_templates().await
}

/// 新建模板（T-533；名称唯一、供应商与模型绑定校验；内置标记由服务端固定为 false）。
#[tauri::command]
pub async fn ai_create_template(
    state: State<'_, AiState>,
    template: AiTemplate,
) -> Result<AiTemplate, AppError> {
    state.create_template(template).await
}

/// 更新模板（T-533；内置模板只读 → E-CONFIG-004）。
#[tauri::command]
pub async fn ai_update_template(
    state: State<'_, AiState>,
    template: AiTemplate,
) -> Result<AiTemplate, AppError> {
    state.update_template(template).await
}

/// 删除模板（T-533；内置模板不可删除）。
#[tauri::command]
pub async fn ai_delete_template(state: State<'_, AiState>, id: String) -> Result<(), AppError> {
    state.delete_template(&id).await
}

/// 复制模板为自定义（T-533；内置模板的唯一定制路径，名称自动唯一化）。
#[tauri::command]
pub async fn ai_copy_template(
    state: State<'_, AiState>,
    id: String,
) -> Result<AiTemplate, AppError> {
    state.copy_template(&id).await
}
