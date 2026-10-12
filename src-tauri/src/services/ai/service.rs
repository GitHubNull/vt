//! AI 供应商服务（T-531 / T-532）：AiState 聚合配置存储、凭据存储与 HTTP 客户端，
//! 提供供应商 CRUD、Key 保管（永不回传明文）、适配器构造与模型列表管理。
//!
//! 装配建议（tech/04 4.1）：`AiState` 独立 `app.manage(...)`（命令层以
//! `State<'_, AiState>` 访问），构造见 [`AiState::new`]，启动时调用一次
//! [`AiState::seed_if_empty`] 灌入预置清单；设置项 [`AiState::with_templates_dir`]
//! 附加提示词模板存储目录（T-533）。
//!
//! 安全（NFR-S-01/02）：
//! - API Key 本体只进 OS 凭据管理器（不可用时降级加密文件，tech/08 8.3），
//!   配置文件只存引用键（`cred:provider:{id}`）；
//! - Key 不写日志、不进错误信息；命令层只能拿到"已设置"布尔状态。

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;

use crate::error::AppError;
use crate::platform::credentials::{backend_info, CredentialBackendInfo, CredentialStore};
use crate::services::config_store::{AiConfig, AiProviderEntry, ConfigSection, ConfigStore};

use super::http::default_client;
use super::ollama::OllamaProvider;
use super::openai_compatible::OpenAiCompatibleProvider;
use super::presets::{
    credential_ref_key, merge_presets, new_custom_id, validate_entry, MergeResult, MAX_API_KEY_LEN,
};
use super::provider::{
    normalize_base_url, AiProvider, CompletionRequest, CompletionResponse, ConnectionTest,
    ModelInfo, ProtocolKind, ERR_NET_NO_KEY,
};
use super::templates::{AiTemplate, TemplatesService};

/// 错误编号：凭据操作任务异常（E-PLAT-023，keyring=20 / 降级 IO=21 / 加密=22 顺延）。
const ERR_CRED_JOIN: u32 = 23;
/// 错误编号：AI 配置序列化失败（与 ConfigStore `ERR_BAD_JSON` 同号，E-CONFIG-002）。
const ERR_AI_SERIALIZE: u32 = 2;
/// 错误编号：模板存储未初始化（E-CONFIG-001；仅应用装配缺失时出现）。
const ERR_TEMPLATES_UNWIRED: u32 = 1;

/// 模型标识长度上限（与 `presets::validate_entry` 的模型校验一致）。
pub const MAX_MODEL_ID_CHARS: usize = 256;

/// AI 服务状态：命令层经 `State<'_, AiState>` 使用。
pub struct AiState {
    config: ConfigStore,
    credentials: Arc<dyn CredentialStore>,
    http: reqwest::Client,
    /// 提示词模板存储目录（配置目录，T-533；未装配时为 None）。
    templates_dir: Option<PathBuf>,
    /// 模板服务（首次访问时从 [`Self::templates_dir`] 加载并缓存）。
    templates: tokio::sync::OnceCell<TemplatesService>,
}

/// 供应商视图（前端列表数据源；**不含任何 Key 内容**，仅"已设置"布尔）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderView {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    pub enabled: bool,
    pub model: Option<String>,
    /// 已拉取 / 手动维护的模型清单（T-532，去重保序；模板绑定下拉的数据源）。
    pub available_models: Vec<String>,
    /// 本地供应商（卡片"本地"标签，数据不出本机）。
    pub local: bool,
    /// 云端供应商需要 Key。
    pub key_required: bool,
    /// Key 是否已设置（只回状态，不回内容，NFR-S-01）。
    pub key_set: bool,
    /// 预置推荐项（卡片"推荐"字样）。
    pub recommended: bool,
}

/// 模型清单拉取合并结果（T-532，F-AI-08）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelMergeReport {
    /// 本次新加入的模型标识（按拉取顺序）。
    pub added: Vec<String>,
    /// 合并后的完整模型清单（去重保序）。
    pub models: Vec<String>,
}

/// 拉取结果规范化：去首尾空白、丢弃空标识、按标识去重保序（保留首个）。
pub fn normalize_model_ids(models: &[ModelInfo]) -> Vec<String> {
    let mut ids: Vec<String> = Vec::with_capacity(models.len());
    for model in models {
        let id = model.id.trim();
        if id.is_empty() || ids.iter().any(|existing| existing == id) {
            continue;
        }
        ids.push(id.to_string());
    }
    ids
}

/// 合并模型清单（T-532）：已存条目保留原位置在前，新模型按拉取顺序追加；
/// 两侧都按标识去重。返回（合并后清单，本次新增条目）。
pub fn merge_models(existing: &[String], fetched: &[String]) -> (Vec<String>, Vec<String>) {
    let mut models: Vec<String> = Vec::with_capacity(existing.len() + fetched.len());
    for id in existing {
        if !models.iter().any(|model| model == id) {
            models.push(id.clone());
        }
    }
    let mut added = Vec::new();
    for id in fetched {
        if !models.iter().any(|model| model == id) {
            models.push(id.clone());
            added.push(id.clone());
        }
    }
    (models, added)
}

/// 恢复预置的结果（设置页据此提示合并 / 重命名，PRD 3.7 业务规则 1）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RestoreReport {
    pub added: Vec<String>,
    pub skipped: Vec<String>,
}

impl AiState {
    /// 构造：配置单例 + 凭据存储（OS 凭据管理器 / 降级加密文件）。
    pub fn new(config: ConfigStore, credentials: Arc<dyn CredentialStore>) -> Self {
        Self {
            config,
            credentials,
            http: default_client(),
            templates_dir: None,
            templates: tokio::sync::OnceCell::new(),
        }
    }

    /// 附加提示词模板存储目录（T-533；装配时传入应用配置目录，`templates.json` 所在）。
    ///
    /// 装配缺失时模板命令返回 E-CONFIG-001；其余命令不受影响。
    pub fn with_templates_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.templates_dir = Some(dir.into());
        self
    }

    /// 模板服务（T-533；首次访问时加载并缓存，内置模板自动补齐）。
    pub async fn templates(&self) -> Result<&TemplatesService, AppError> {
        if let Some(service) = self.templates.get() {
            return Ok(service);
        }
        let dir = self.templates_dir.as_deref().ok_or_else(|| {
            AppError::Config(
                ERR_TEMPLATES_UNWIRED,
                "模板存储目录未初始化（应用启动装配缺失）".to_string(),
            )
        })?;
        let loaded = TemplatesService::load(dir)?;
        // 并发首次访问时可能各自加载一次，内容一致（文件为唯一数据源），后者丢弃。
        Ok(self.templates.get_or_init(|| async { loaded }).await)
    }

    /// 凭据后端信息（设置页明示保护级别，tech/08 8.3）。
    pub fn credential_backend(&self) -> CredentialBackendInfo {
        backend_info(&self.credentials)
    }

    /// 读取供应商配置条目；不存在返回 E-CONFIG-005。
    pub async fn provider_entry(&self, id: &str) -> Result<AiProviderEntry, AppError> {
        let config = self.config.get().await;
        config
            .ai
            .providers
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .ok_or_else(|| {
                AppError::Config(
                    super::presets::ERR_AI_NOT_FOUND,
                    format!("供应商不存在（{id}）"),
                )
            })
    }

    /// 供应商清单（首次为空时自动灌入预置；启用中的供应商排前，PRD 3.7 规则 5）。
    pub async fn list_providers(&self) -> Result<Vec<ProviderView>, AppError> {
        self.seed_if_empty().await?;
        let config = self.config.get().await;
        let mut views = Vec::with_capacity(config.ai.providers.len());
        for entry in &config.ai.providers {
            views.push(self.to_view(entry).await);
        }
        // 稳定排序：启用中在前，其余保持添加顺序。
        views.sort_by_key(|view| !view.enabled);
        Ok(views)
    }

    async fn to_view(&self, entry: &AiProviderEntry) -> ProviderView {
        let protocol = ProtocolKind::parse(&entry.protocol);
        let key_required = protocol.is_some_and(|kind| !kind.is_local());
        let key_set = if key_required && entry.api_key_ref.is_some() {
            self.secret_exists(entry.api_key_ref.as_deref().unwrap_or_default())
                .await
        } else {
            false
        };
        ProviderView {
            id: entry.id.clone(),
            name: entry.name.clone(),
            protocol: entry.protocol.clone(),
            base_url: entry.base_url.clone(),
            enabled: entry.enabled,
            model: entry.model.clone(),
            available_models: entry.available_models.clone(),
            local: entry.local,
            key_required,
            key_set,
            recommended: super::presets::find(&entry.id).is_some_and(|preset| preset.recommended),
        }
    }

    /// 首次使用（清单为空）时灌入预置供应商清单（验收标准 1 的落地入口）。
    ///
    /// 后续版本新增预置的增量合并需要配置版本标记（AiConfig 冻结，改动归 T-536），
    /// 设置页可随时经 [`AiState::restore_presets`] 手动恢复。
    pub async fn seed_if_empty(&self) -> Result<(), AppError> {
        let config = self.config.get().await;
        if !config.ai.providers.is_empty() {
            return Ok(());
        }
        let mut providers = Vec::new();
        let merged = merge_presets(&mut providers);
        if merged.added.is_empty() {
            return Ok(());
        }
        tracing::info!(count = merged.added.len(), "已灌入预置 AI 供应商清单");
        self.write_ai(AiConfig { providers }).await
    }

    /// 恢复预置供应商（按 ID 去重；同名不同 ID 跳过并报告，PRD 3.7 规则 1）。
    pub async fn restore_presets(&self) -> Result<RestoreReport, AppError> {
        let config = self.config.get().await;
        let mut providers = config.ai.providers.clone();
        let MergeResult { added, skipped } = merge_presets(&mut providers);
        if !added.is_empty() {
            self.write_ai(AiConfig { providers }).await?;
        }
        Ok(RestoreReport { added, skipped })
    }

    /// 新增或更新供应商（PRD 3.7 编辑字段校验；名称唯一）。
    ///
    /// Key 引用不受本命令影响：新增条目无 Key，更新条目保留原引用
    /// （Key 走 set/clear 专用命令）；改为本地协议时顺带清理已存 Key。
    ///
    /// T-532：更新时**模型清单与默认模型（`available_models` / `model`）不随
    /// 本命令覆盖**（编辑表单只改名称 / 地址 / 协议等字段；模型列表由拉取合并与
    /// 手动 CRUD 命令管理，默认模型由 `set_default_model` 管理），防止部分
    /// 载荷误清空用户数据。新增（自建）条目：默认模型须先存在于自己的清单，
    /// 因此创建时应为空。
    pub async fn upsert_provider(&self, mut entry: AiProviderEntry) -> Result<(), AppError> {
        // 规整输入（去空白；本地供应商清掉 Key 引用）。
        entry.id = entry.id.trim().to_string();
        entry.name = entry.name.trim().to_string();
        entry.base_url = entry.base_url.trim().to_string();
        entry.protocol = entry.protocol.trim().to_string();
        entry.model = entry
            .model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_string);
        if entry.id.is_empty() {
            entry.id = new_custom_id();
        }
        let mut config = self.config.get().await;
        let protocol = validate_entry(&entry, &config.ai.providers)?;
        if let Some(slot) = config.ai.providers.iter_mut().find(|e| e.id == entry.id) {
            let previous_ref = slot.api_key_ref.take();
            if protocol.is_local() {
                // 本地供应商不持有 Key：清引用并尽力删除已存秘密（防孤儿条目）。
                entry.api_key_ref = None;
                if let Some(ref_key) = previous_ref {
                    if let Err(err) = self.delete_secret(&ref_key).await {
                        tracing::warn!(code = %err.code(), "切换为本地供应商时清理旧 Key 失败");
                    }
                }
            } else if entry.api_key_ref.is_none() {
                // 更新云端条目保留原 Key 引用（本命令不改密钥）。
                entry.api_key_ref = previous_ref;
            }
            // T-532：模型清单 / 默认模型不随编辑覆盖（专用命令管理，见方法注释）。
            entry.available_models = slot.available_models.clone();
            entry.model = slot.model.clone();
            *slot = entry;
        } else {
            // 新增条目：默认模型须在（尚无条目的）模型清单内——只可能是空。
            if let Some(model) = entry.model.as_deref() {
                if !entry
                    .available_models
                    .iter()
                    .any(|available| available == model)
                {
                    return Err(AppError::Config(
                        super::presets::ERR_AI_BAD_VALUE,
                        format!("模型“{model}”不在该供应商的模型列表中，请先拉取或手动添加"),
                    ));
                }
            }
            config.ai.providers.push(entry);
        }
        self.write_ai(config.ai).await
    }

    /// 删除供应商：同时删除其 OS 凭据条目（配置中不留引用，凭据不留孤儿）。
    pub async fn delete_provider(&self, id: &str) -> Result<(), AppError> {
        let mut config = self.config.get().await;
        let mut removed: Option<AiProviderEntry> = None;
        config.ai.providers.retain(|entry| {
            if entry.id == id {
                removed = Some(entry.clone());
                false
            } else {
                true
            }
        });
        let removed = removed.ok_or_else(|| {
            AppError::Config(
                super::presets::ERR_AI_NOT_FOUND,
                format!("供应商不存在（{id}）"),
            )
        })?;
        if let Some(ref_key) = &removed.api_key_ref {
            // 删除凭据失败则整体失败，保持配置与凭据一致。
            self.delete_secret(ref_key).await?;
        }
        self.write_ai(config.ai).await
    }

    /// 设置 API Key：Key 本体进 OS 凭据管理器，配置只写引用键。
    ///
    /// 错误信息与日志只含引用键，绝不含 Key 内容（NFR-S-01/02）。
    pub async fn set_api_key(&self, id: &str, api_key: &str) -> Result<(), AppError> {
        let key = api_key.trim();
        if key.is_empty() {
            return Err(AppError::Config(
                super::presets::ERR_AI_BAD_VALUE,
                "API Key 不能为空".to_string(),
            ));
        }
        if key.len() > MAX_API_KEY_LEN {
            return Err(AppError::Config(
                super::presets::ERR_AI_BAD_VALUE,
                "API Key 过长".to_string(),
            ));
        }
        let entry = self.provider_entry(id).await?;
        let protocol = ProtocolKind::parse(&entry.protocol).ok_or_else(|| {
            AppError::Config(
                super::presets::ERR_AI_BAD_VALUE,
                format!("接口协议不合法: {}", entry.protocol),
            )
        })?;
        if protocol.is_local() {
            return Err(AppError::Config(
                super::presets::ERR_AI_BAD_VALUE,
                "本地供应商（Ollama / LM Studio）无需 API Key".to_string(),
            ));
        }
        let ref_key = credential_ref_key(id);
        self.store_secret(&ref_key, key).await?;
        let mut config = self.config.get().await;
        if let Some(slot) = config.ai.providers.iter_mut().find(|e| e.id == id) {
            slot.api_key_ref = Some(ref_key);
        }
        self.write_ai(config.ai).await
    }

    /// 清除 API Key：删除凭据条目并清掉配置引用（幂等）。
    pub async fn clear_api_key(&self, id: &str) -> Result<(), AppError> {
        let entry = self.provider_entry(id).await?;
        if let Some(ref_key) = &entry.api_key_ref {
            self.delete_secret(ref_key).await?;
        }
        let mut config = self.config.get().await;
        if let Some(slot) = config.ai.providers.iter_mut().find(|e| e.id == id) {
            slot.api_key_ref = None;
        }
        self.write_ai(config.ai).await
    }

    /// Key 是否已设置（命令层唯一可查询的 Key 信息，永不回传明文）。
    pub async fn key_status(&self, id: &str) -> Result<bool, AppError> {
        let entry = self.provider_entry(id).await?;
        let Some(ref_key) = entry.api_key_ref else {
            return Ok(false);
        };
        Ok(self.secret_exists(&ref_key).await)
    }

    /// 构造供应商适配器（trait 对象；Key 在进程内读取，不落日志）。
    pub async fn build_provider(&self, id: &str) -> Result<Box<dyn AiProvider>, AppError> {
        let entry = self.provider_entry(id).await?;
        let protocol = ProtocolKind::parse(&entry.protocol).ok_or_else(|| {
            AppError::Config(
                super::presets::ERR_AI_BAD_VALUE,
                format!("接口协议不合法: {}", entry.protocol),
            )
        })?;
        let base_url = normalize_base_url(&entry.base_url);
        match protocol {
            // Ollama 本地协议（/api/chat + /api/tags），无 Key。
            ProtocolKind::Ollama => Ok(Box::new(OllamaProvider::new(self.http.clone(), base_url))),
            // LM Studio：OpenAI 兼容子集，免 Key。
            ProtocolKind::LmStudio => Ok(Box::new(OpenAiCompatibleProvider::new(
                self.http.clone(),
                base_url,
                None,
                false,
            ))),
            // OpenAI 兼容云端：进程内读取 Key（仅用于请求头）。
            ProtocolKind::OpenAiCompatible => {
                let api_key = match &entry.api_key_ref {
                    Some(ref_key) => self.load_secret(ref_key).await?,
                    None => None,
                };
                Ok(Box::new(OpenAiCompatibleProvider::new(
                    self.http.clone(),
                    base_url,
                    api_key,
                    true,
                )))
            }
        }
    }

    /// 文本补全（F-AI-07 / F-AI-04，服务端联调命令）。
    ///
    /// T-532：请求未显式指定模型时回退供应商默认模型（`entry.model`，
    /// 持久化于配置；模板调用（T-534）同样走此入口）。
    pub async fn complete(
        &self,
        id: &str,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let provider = self.build_provider(id).await?;
        let mut effective = request.clone();
        if effective.model.trim().is_empty() {
            if let Some(model) = self.provider_entry(id).await?.model {
                // 日志不含用户文本（NFR-S-02）；此处不产生任何日志。
                effective.model = model;
            }
        }
        provider.complete(&effective).await
    }

    /// 拉取供应商模型清单（F-AI-08）。
    pub async fn fetch_models(&self, id: &str) -> Result<Vec<ModelInfo>, AppError> {
        let provider = self.build_provider(id).await?;
        provider.list_models().await
    }

    /// 拉取模型清单并与已存清单合并持久化（T-532，F-AI-08；SPEC 的 list_models）。
    ///
    /// 流程：调适配器拉取 → 结果规范化（id 去空白 / 去空 / 去重保序）→ 与
    /// `available_models` 合并（已存条目位置不变，新条目按拉取顺序追加）→
    /// 有变化才原子落盘（NFR-R-05）。
    pub async fn fetch_and_merge_models(&self, id: &str) -> Result<ModelMergeReport, AppError> {
        let provider = self.build_provider(id).await?;
        let fetched = provider.list_models().await?;
        let normalized = normalize_model_ids(&fetched);
        let mut config = self.config.get().await;
        let slot = config
            .ai
            .providers
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| {
                AppError::Config(
                    super::presets::ERR_AI_NOT_FOUND,
                    format!("供应商不存在（{id}）"),
                )
            })?;
        let (models, added) = merge_models(&slot.available_models, &normalized);
        if models != slot.available_models {
            slot.available_models = models.clone();
            self.write_ai(config.ai).await?;
        }
        // 隐私（NFR-S-02）：只记录供应商与数量。
        tracing::info!(provider = %id, added = added.len(), "已合并供应商模型清单");
        Ok(ModelMergeReport { added, models })
    }

    /// 手动添加模型条目（T-532；供应商不支持列表 API 时的兜底）。
    ///
    /// 校验：标识非空、长度上限（[`MAX_MODEL_ID_CHARS`]）、同供应商下唯一。
    pub async fn add_provider_model(&self, id: &str, model: &str) -> Result<(), AppError> {
        let model = model.trim();
        if model.is_empty() {
            return Err(AppError::Config(
                super::presets::ERR_AI_BAD_VALUE,
                "模型标识不能为空".to_string(),
            ));
        }
        if model.chars().count() > MAX_MODEL_ID_CHARS {
            return Err(AppError::Config(
                super::presets::ERR_AI_BAD_VALUE,
                "模型标识过长".to_string(),
            ));
        }
        let mut config = self.config.get().await;
        let slot = config
            .ai
            .providers
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| {
                AppError::Config(
                    super::presets::ERR_AI_NOT_FOUND,
                    format!("供应商不存在（{id}）"),
                )
            })?;
        if slot
            .available_models
            .iter()
            .any(|existing| existing == model)
        {
            return Err(AppError::Config(
                super::presets::ERR_AI_BAD_VALUE,
                format!("模型“{model}”已在列表中"),
            ));
        }
        slot.available_models.push(model.to_string());
        self.write_ai(config.ai).await
    }

    /// 删除模型条目（T-532）。被删除的条目若为当前默认模型，默认选择一并清空。
    pub async fn remove_provider_model(&self, id: &str, model: &str) -> Result<(), AppError> {
        let model = model.trim();
        let mut config = self.config.get().await;
        let slot = config
            .ai
            .providers
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| {
                AppError::Config(
                    super::presets::ERR_AI_NOT_FOUND,
                    format!("供应商不存在（{id}）"),
                )
            })?;
        let before = slot.available_models.len();
        slot.available_models.retain(|existing| existing != model);
        if slot.available_models.len() == before {
            return Err(AppError::Config(
                super::presets::ERR_AI_NOT_FOUND,
                format!("模型不存在（{model}）"),
            ));
        }
        if slot.model.as_deref() == Some(model) {
            slot.model = None;
        }
        self.write_ai(config.ai).await
    }

    /// 设置 / 清除供应商默认模型（T-532；`None` = 清除）。
    ///
    /// 默认模型必须来自该供应商的模型清单（拉取合并或手动添加），
    /// 持久化后被未显式指定模型的后续调用使用（见 [`AiState::complete`]）。
    pub async fn set_default_model(&self, id: &str, model: Option<&str>) -> Result<(), AppError> {
        let mut config = self.config.get().await;
        let slot = config
            .ai
            .providers
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| {
                AppError::Config(
                    super::presets::ERR_AI_NOT_FOUND,
                    format!("供应商不存在（{id}）"),
                )
            })?;
        slot.model = match model.map(str::trim).filter(|value| !value.is_empty()) {
            Some(model) => {
                if !slot
                    .available_models
                    .iter()
                    .any(|existing| existing == model)
                {
                    return Err(AppError::Config(
                        super::presets::ERR_AI_BAD_VALUE,
                        format!("模型“{model}”不在模型列表中，请先拉取或手动添加"),
                    ));
                }
                Some(model.to_string())
            }
            None => None,
        };
        self.write_ai(config.ai).await
    }

    /// 连通性测试（F-AI-08，服务端）：失败以 `ok=false` + 稳定编号返回
    /// （区别于 complete 的 Err 形态，前端统一按结果渲染延迟 / 错误原因）。
    pub async fn test_connection(&self, id: &str) -> Result<ConnectionTest, AppError> {
        let entry = self.provider_entry(id).await?;
        let protocol = ProtocolKind::parse(&entry.protocol).ok_or_else(|| {
            AppError::Config(
                super::presets::ERR_AI_BAD_VALUE,
                format!("接口协议不合法: {}", entry.protocol),
            )
        })?;
        // 云端供应商未配置 Key：直接给出明确编号，不发起无谓请求。
        if !protocol.is_local() && entry.api_key_ref.is_none() {
            return Ok(ConnectionTest {
                ok: false,
                latency_ms: 0,
                error_code: Some(format!("E-NET-{ERR_NET_NO_KEY:03}")),
                detail: Some("尚未配置 API Key，请先填写".to_string()),
            });
        }
        let provider = self.build_provider(id).await?;
        provider.test_connection().await
    }

    /// 模板清单（T-533；首次调用自动初始化内置模板）。
    pub async fn list_templates(&self) -> Result<Vec<AiTemplate>, AppError> {
        Ok(self.templates().await?.list().await)
    }

    /// 新建模板（T-533；名称唯一 / 供应商与模型绑定校验后原子落盘）。
    pub async fn create_template(&self, template: AiTemplate) -> Result<AiTemplate, AppError> {
        let providers = self.config.get().await.ai.providers;
        self.templates().await?.create(&providers, template).await
    }

    /// 更新模板（T-533；内置模板只读，返回 E-CONFIG-004）。
    pub async fn update_template(&self, template: AiTemplate) -> Result<AiTemplate, AppError> {
        let providers = self.config.get().await.ai.providers;
        self.templates().await?.update(&providers, template).await
    }

    /// 删除模板（T-533；内置模板不可删除）。
    pub async fn delete_template(&self, id: &str) -> Result<(), AppError> {
        self.templates().await?.delete(id).await
    }

    /// 复制模板为自定义（T-533；内置模板的唯一定制路径）。
    pub async fn copy_template(&self, id: &str) -> Result<AiTemplate, AppError> {
        self.templates().await?.copy(id).await
    }

    /// 整节写回（ConfigStore 校验 + 原子落盘 + config:changed 广播）。
    async fn write_ai(&self, ai: AiConfig) -> Result<(), AppError> {
        let value = serde_json::to_value(&ai).map_err(|err| {
            AppError::Config(ERR_AI_SERIALIZE, format!("AI 配置序列化失败: {err}"))
        })?;
        // 隐私（NFR-S-02）：日志只含节名，配置值不进日志（ConfigStore 同约定）。
        self.config.set_section(ConfigSection::Ai, value).await
    }

    /// 凭据写入（阻塞系统调用移入阻塞线程池）。
    async fn store_secret(&self, ref_key: &str, secret: &str) -> Result<(), AppError> {
        let store = Arc::clone(&self.credentials);
        let ref_key = ref_key.to_string();
        let secret = secret.to_string();
        tokio::task::spawn_blocking(move || store.set(&ref_key, &secret))
            .await
            .map_err(|err| AppError::Platform(ERR_CRED_JOIN, format!("凭据写入任务异常: {err}")))?
    }

    /// 凭据读取（进程内使用；调用方不得把结果写日志 / 回传前端）。
    async fn load_secret(&self, ref_key: &str) -> Result<Option<String>, AppError> {
        let store = Arc::clone(&self.credentials);
        let ref_key = ref_key.to_string();
        tokio::task::spawn_blocking(move || store.get(&ref_key))
            .await
            .map_err(|err| AppError::Platform(ERR_CRED_JOIN, format!("凭据读取任务异常: {err}")))?
    }

    /// 凭据是否存在（不把秘密内容带出阻塞任务）。
    async fn secret_exists(&self, ref_key: &str) -> bool {
        matches!(self.load_secret(ref_key).await, Ok(Some(_)))
    }

    /// 凭据删除（条目不存在视为成功）。
    async fn delete_secret(&self, ref_key: &str) -> Result<(), AppError> {
        let store = Arc::clone(&self.credentials);
        let ref_key = ref_key.to_string();
        tokio::task::spawn_blocking(move || store.delete(&ref_key))
            .await
            .map_err(|err| AppError::Platform(ERR_CRED_JOIN, format!("凭据删除任务异常: {err}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events;
    use crate::platform::credentials::FallbackStore;
    use crate::services::ai::mock_http::{MockResponse, MockRoute, MockServer};
    use crate::services::config_store::ConfigStore;

    const SECRET: &str = "sk-live-9f8e7d6c5b4a3210feasible";

    struct TestDir(std::path::PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "vt-ai-service-{name}-{}-{:?}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn make_state(dir: &TestDir) -> AiState {
        let (tx, _rx) = events::channel();
        let config = ConfigStore::load(&dir.0, tx).unwrap();
        let credentials: Arc<dyn CredentialStore> = Arc::new(FallbackStore::new(
            dir.0.join("secrets.enc"),
            b"test-machine".to_vec(),
        ));
        AiState::new(config, credentials)
    }

    #[tokio::test]
    async fn seeds_once_and_lists_all_presets() {
        let dir = TestDir::new("seed");
        let state = make_state(&dir).await;
        // 首次清单为空 → 灌入 PRD 3.7 全部预置。
        let views = state.list_providers().await.unwrap();
        assert_eq!(views.len(), super::super::presets::CATALOG.len());
        assert!(views[0].recommended, "通义千问推荐项排首");
        let ollama = views.iter().find(|v| v.id == "ollama").unwrap();
        assert!(ollama.local && !ollama.key_required);
        let deepseek = views.iter().find(|v| v.id == "deepseek").unwrap();
        assert!(!deepseek.local && deepseek.key_required && !deepseek.key_set);
        // 二次列表不重复。
        assert_eq!(state.list_providers().await.unwrap().len(), views.len());
    }

    #[tokio::test]
    async fn upsert_validates_and_preserves_key_ref() {
        let dir = TestDir::new("upsert");
        let state = make_state(&dir).await;
        state.seed_if_empty().await.unwrap();

        // 同名重复（E-CONFIG-004）。
        let mut entry = super::super::presets::find("kimi").unwrap().to_entry();
        entry.name = "DeepSeek".to_string();
        assert_eq!(
            state.upsert_provider(entry).await.unwrap_err().code(),
            "E-CONFIG-004"
        );

        // 更新预置条目（改 base_url）：Key 引用保留。
        state.set_api_key("deepseek", SECRET).await.unwrap();
        let mut entry = super::super::presets::find("deepseek").unwrap().to_entry();
        entry.base_url = "https://api.example.com/v1".to_string();
        entry.enabled = true;
        state.upsert_provider(entry).await.unwrap();
        let config = state.provider_entry("deepseek").await.unwrap();
        assert!(config.enabled);
        assert_eq!(config.base_url, "https://api.example.com/v1");
        assert_eq!(
            config.api_key_ref.as_deref(),
            Some("cred:provider:deepseek")
        );
        assert!(state.key_status("deepseek").await.unwrap());
    }

    #[tokio::test]
    async fn key_lifecycle_never_writes_plaintext_to_config() {
        // 验收标准 2：配置 API Key 后配置文件中无明文 Key（只有引用键）。
        let dir = TestDir::new("key-lifecycle");
        let state = make_state(&dir).await;
        state.seed_if_empty().await.unwrap();

        state.set_api_key("qwen", SECRET).await.unwrap();
        let config_text = std::fs::read_to_string(dir.0.join("config.json")).unwrap();
        assert!(!config_text.contains(SECRET), "配置文件不得出现 Key 明文");
        assert!(config_text.contains("cred:provider:qwen"));
        assert!(state.key_status("qwen").await.unwrap());

        // 覆盖写入（换 Key）与清除。
        state.set_api_key("qwen", "sk-another-key").await.unwrap();
        assert!(state.key_status("qwen").await.unwrap());
        state.clear_api_key("qwen").await.unwrap();
        assert!(!state.key_status("qwen").await.unwrap());
        let config = state.provider_entry("qwen").await.unwrap();
        assert!(config.api_key_ref.is_none());

        // 空值 / 本地供应商 / 不存在的供应商。
        assert_eq!(
            state.set_api_key("qwen", "  ").await.unwrap_err().code(),
            "E-CONFIG-004"
        );
        assert_eq!(
            state
                .set_api_key("ollama", SECRET)
                .await
                .unwrap_err()
                .code(),
            "E-CONFIG-004"
        );
        assert_eq!(
            state.set_api_key("nope", SECRET).await.unwrap_err().code(),
            "E-CONFIG-005"
        );
        assert_eq!(
            state.delete_provider("nope").await.unwrap_err().code(),
            "E-CONFIG-005"
        );
    }

    #[tokio::test]
    async fn delete_provider_removes_credential() {
        let dir = TestDir::new("delete");
        let state = make_state(&dir).await;
        state.seed_if_empty().await.unwrap();
        state.set_api_key("deepseek", SECRET).await.unwrap();
        let ref_key = state
            .provider_entry("deepseek")
            .await
            .unwrap()
            .api_key_ref
            .expect("设置 Key 后应有引用键");
        state.delete_provider("deepseek").await.unwrap();
        assert!(state.provider_entry("deepseek").await.is_err());
        // 凭据随供应商一并删除（供应商条目已不存在，直接查凭据存储）。
        assert!(!state.secret_exists(&ref_key).await, "凭据应随供应商删除");
        // 其余条目不受影响。
        assert!(state.provider_entry("kimi").await.is_ok());
    }

    #[tokio::test]
    async fn restore_presets_reports_merge_and_skips() {
        let dir = TestDir::new("restore");
        let state = make_state(&dir).await;
        state.seed_if_empty().await.unwrap();
        // 删除预置 → 自建同名供应商 → 恢复预置：同名跳过（PRD 3.7 规则 1）。
        state.delete_provider("openai").await.unwrap();
        state
            .upsert_provider(AiProviderEntry {
                id: "custom-x".to_string(),
                name: "OpenAI".to_string(),
                protocol: "openai_compatible".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
                enabled: false,
                model: None,
                available_models: Vec::new(),
                api_key_ref: None,
                local: false,
            })
            .await
            .unwrap();
        let report = state.restore_presets().await.unwrap();
        assert!(report.added.is_empty());
        assert_eq!(report.skipped, vec!["OpenAI".to_string()]);
        // 删除自建同名条目后再恢复 → 预置成功恢复。
        state.delete_provider("custom-x").await.unwrap();
        let report = state.restore_presets().await.unwrap();
        assert_eq!(report.added, vec!["OpenAI".to_string()]);
        assert!(report.skipped.is_empty());
        assert!(state.provider_entry("openai").await.is_ok());
    }

    #[tokio::test]
    async fn cloud_offline_fails_while_local_mock_works() {
        // 验收标准 4：断网时云端供应商返回明确错误编号，本地供应商不受影响。
        let dir = TestDir::new("offline");
        let state = make_state(&dir).await;
        state.seed_if_empty().await.unwrap();
        state.set_api_key("deepseek", SECRET).await.unwrap();

        // 云端：指向无监听端口（连接拒绝 = 断网等价场景）→ E-NET-101。
        let mut entry = super::super::presets::find("deepseek").unwrap().to_entry();
        entry.base_url = "http://127.0.0.1:1/v1".to_string();
        state.upsert_provider(entry).await.unwrap();
        let request = CompletionRequest {
            model: "deepseek-chat".to_string(),
            prompt: "你好".to_string(),
            system: None,
            temperature: None,
        };
        let err = state.complete("deepseek", &request).await.unwrap_err();
        assert_eq!(err.code(), "E-NET-101");
        let test = state.test_connection("deepseek").await.unwrap();
        assert!(!test.ok);
        assert_eq!(test.error_code.as_deref(), Some("E-NET-101"));

        // 本地：mock Ollama 正常补全。
        let server = MockServer::spawn(vec![MockRoute {
            path: "/api/chat",
            delay_ms: 0,
            responses: vec![MockResponse {
                status: 200,
                body: r#"{"model":"qwen3:8b","message":{"role":"assistant","content":"本地应答"},"done":true}"#
                    .to_string(),
            }],
        }])
        .await;
        let mut entry = super::super::presets::find("ollama").unwrap().to_entry();
        entry.base_url = server.base_url.clone();
        state.upsert_provider(entry).await.unwrap();
        let response = state.complete("ollama", &request).await.unwrap();
        assert_eq!(response.text, "本地应答");
    }

    #[tokio::test]
    async fn cloud_without_key_gets_no_key_code_on_test_connection() {
        let dir = TestDir::new("no-key");
        let state = make_state(&dir).await;
        state.seed_if_empty().await.unwrap();
        let test = state.test_connection("deepseek").await.unwrap();
        assert!(!test.ok);
        assert_eq!(test.error_code.as_deref(), Some("E-NET-109"));
    }

    // ---- T-532 模型列表管理（F-AI-07 / F-AI-08） ----

    /// 合并逻辑（纯函数）：去重保序、新条目按拉取顺序追加。
    #[test]
    fn merge_models_dedupes_and_keeps_order() {
        let existing = vec!["a".to_string(), "b".to_string()];
        let fetched = vec![
            "b".to_string(),
            "c".to_string(),
            "a".to_string(),
            "d".to_string(),
        ];
        let (models, added) = merge_models(&existing, &fetched);
        assert_eq!(models, vec!["a", "b", "c", "d"]);
        assert_eq!(added, vec!["c", "d"]);
        // 空集合：拉取结果直接作为清单。
        let (models, added) = merge_models(&[], &["x".to_string()]);
        assert_eq!(models, vec!["x"]);
        assert_eq!(added, vec!["x"]);
    }

    /// 拉取结果规范化：去空白、丢空、去重。
    #[test]
    fn normalize_model_ids_trims_and_dedupes() {
        let fetched = vec![
            ModelInfo {
                id: " qwen3:8b ".to_string(),
                display_name: "x".to_string(),
            },
            ModelInfo {
                id: "qwen3:8b".to_string(),
                display_name: "x".to_string(),
            },
            ModelInfo {
                id: "   ".to_string(),
                display_name: "x".to_string(),
            },
            ModelInfo {
                id: "deepseek-r1:7b".to_string(),
                display_name: "x".to_string(),
            },
        ];
        assert_eq!(
            normalize_model_ids(&fetched),
            vec!["qwen3:8b".to_string(), "deepseek-r1:7b".to_string()]
        );
    }

    /// 验收标准：一键拉取模型列表 → 规范化合并 → 持久化；重复拉取幂等。
    #[tokio::test]
    async fn fetch_and_merge_models_persists_and_is_idempotent() {
        let dir = TestDir::new("merge-models");
        let state = make_state(&dir).await;
        state.seed_if_empty().await.unwrap();
        let server = MockServer::spawn(vec![MockRoute {
            path: "/api/tags",
            delay_ms: 0,
            responses: vec![MockResponse {
                status: 200,
                body: r#"{"models":[
                    {"name":"qwen3:8b"},{"name":" qwen3:8b "},{"name":"  "},
                    {"name":"deepseek-r1:7b"}]}"#
                    .to_string(),
            }],
        }])
        .await;
        let mut entry = super::super::presets::find("ollama").unwrap().to_entry();
        entry.base_url = server.base_url.clone();
        state.upsert_provider(entry).await.unwrap();

        let report = state.fetch_and_merge_models("ollama").await.unwrap();
        assert_eq!(
            report.added,
            vec!["qwen3:8b".to_string(), "deepseek-r1:7b".to_string()]
        );
        assert_eq!(report.models, report.added);

        // 持久化：配置文件内出现清单（重启保持的数据源）。
        let config_text = std::fs::read_to_string(dir.0.join("config.json")).unwrap();
        assert!(config_text.contains("qwen3:8b"));
        assert_eq!(
            state
                .provider_entry("ollama")
                .await
                .unwrap()
                .available_models,
            report.models
        );

        // 再次拉取（mock 重复最后响应）→ 无新增、清单不变。
        let again = state.fetch_and_merge_models("ollama").await.unwrap();
        assert!(again.added.is_empty());
        assert_eq!(again.models, report.models);

        // 不存在的供应商 → E-CONFIG-005。
        assert_eq!(
            state
                .fetch_and_merge_models("nope")
                .await
                .unwrap_err()
                .code(),
            "E-CONFIG-005"
        );
    }

    /// 验收标准：手动添加 / 删除模型；默认模型选择被后续调用使用。
    #[tokio::test]
    async fn manual_model_crud_and_default_model_used_by_complete() {
        let dir = TestDir::new("manual-models");
        let state = make_state(&dir).await;
        state.seed_if_empty().await.unwrap();

        // 手动添加：空 / 重复 / 超长 / 供应商不存在。
        assert_eq!(
            state
                .add_provider_model("deepseek", "  ")
                .await
                .unwrap_err()
                .code(),
            "E-CONFIG-004"
        );
        state
            .add_provider_model("deepseek", "deepseek-chat")
            .await
            .unwrap();
        assert_eq!(
            state
                .add_provider_model("deepseek", " deepseek-chat ")
                .await
                .unwrap_err()
                .code(),
            "E-CONFIG-004"
        );
        let too_long = "m".repeat(MAX_MODEL_ID_CHARS + 1);
        assert_eq!(
            state
                .add_provider_model("deepseek", &too_long)
                .await
                .unwrap_err()
                .code(),
            "E-CONFIG-004"
        );
        assert_eq!(
            state
                .add_provider_model("nope", "m")
                .await
                .unwrap_err()
                .code(),
            "E-CONFIG-005"
        );

        // 默认模型：不在清单 / 设置成功 / 供后续调用使用。
        assert_eq!(
            state
                .set_default_model("deepseek", Some("deepseek-coder"))
                .await
                .unwrap_err()
                .code(),
            "E-CONFIG-004"
        );
        state
            .set_default_model("deepseek", Some("deepseek-chat"))
            .await
            .unwrap();
        assert_eq!(
            state
                .provider_entry("deepseek")
                .await
                .unwrap()
                .model
                .as_deref(),
            Some("deepseek-chat")
        );

        // 后续调用未指定模型 → 使用默认模型（本地 LM Studio 免 Key，走 mock）。
        let server = MockServer::spawn(vec![MockRoute {
            path: "/v1/chat/completions",
            delay_ms: 0,
            responses: vec![MockResponse {
                status: 200,
                body: r#"{"model":"qwen2.5-coder-7b","choices":[{"message":{"content":"ok"}}]}"#
                    .to_string(),
            }],
        }])
        .await;
        let mut entry = super::super::presets::find("lm_studio").unwrap().to_entry();
        entry.base_url = format!("{}/v1", server.base_url);
        state.upsert_provider(entry).await.unwrap();
        state
            .add_provider_model("lm_studio", "qwen2.5-coder-7b")
            .await
            .unwrap();
        state
            .set_default_model("lm_studio", Some("qwen2.5-coder-7b"))
            .await
            .unwrap();
        let request = CompletionRequest {
            model: String::new(),
            prompt: "你好".to_string(),
            system: None,
            temperature: None,
        };
        let response = state.complete("lm_studio", &request).await.unwrap();
        assert_eq!(response.text, "ok");
        let body: serde_json::Value = serde_json::from_str(&server.last_request().body).unwrap();
        assert_eq!(body["model"], "qwen2.5-coder-7b", "应使用默认模型");
        // 显式指定模型时优先于默认。
        let explicit = CompletionRequest {
            model: "other-model".to_string(),
            prompt: "你好".to_string(),
            system: None,
            temperature: None,
        };
        state.complete("lm_studio", &explicit).await.unwrap();
        let body: serde_json::Value = serde_json::from_str(&server.last_request().body).unwrap();
        assert_eq!(body["model"], "other-model");

        // 删除模型条目：删除默认模型时默认选择一并清空。
        state
            .remove_provider_model("lm_studio", "qwen2.5-coder-7b")
            .await
            .unwrap();
        let entry = state.provider_entry("lm_studio").await.unwrap();
        assert!(entry.available_models.is_empty());
        assert_eq!(entry.model, None);
        // 删除不存在的模型 → E-CONFIG-005。
        assert_eq!(
            state
                .remove_provider_model("lm_studio", "qwen2.5-coder-7b")
                .await
                .unwrap_err()
                .code(),
            "E-CONFIG-005"
        );

        // 编辑供应商（upsert）不清空模型清单与默认模型（T-532 数据保护）。
        state.add_provider_model("lm_studio", "m1").await.unwrap();
        state
            .set_default_model("lm_studio", Some("m1"))
            .await
            .unwrap();
        let mut entry = super::super::presets::find("lm_studio").unwrap().to_entry();
        entry.base_url = format!("{}/v1", server.base_url);
        entry.available_models = Vec::new(); // 部分载荷：模型字段缺失
        state.upsert_provider(entry).await.unwrap();
        let entry = state.provider_entry("lm_studio").await.unwrap();
        assert_eq!(entry.available_models, vec!["m1".to_string()]);
        assert_eq!(entry.model.as_deref(), Some("m1"));
    }

    /// 验收标准（NFR-S-02 构造测试）：连通性测试失败原因不含 API Key 明文。
    #[tokio::test]
    async fn connection_test_error_never_leaks_key() {
        let dir = TestDir::new("no-leak");
        let state = make_state(&dir).await;
        state.seed_if_empty().await.unwrap();
        state.set_api_key("deepseek", SECRET).await.unwrap();
        let server = MockServer::spawn(vec![MockRoute {
            path: "/v1/models",
            delay_ms: 0,
            responses: vec![MockResponse {
                status: 401,
                body: format!("invalid api key {SECRET}"),
            }],
        }])
        .await;
        let mut entry = super::super::presets::find("deepseek").unwrap().to_entry();
        entry.base_url = format!("{}/v1", server.base_url);
        state.upsert_provider(entry).await.unwrap();

        let test = state.test_connection("deepseek").await.unwrap();
        assert!(!test.ok);
        assert_eq!(test.error_code.as_deref(), Some("E-NET-103"));
        let detail = test.detail.expect("失败时应带可读原因");
        assert!(!detail.contains(SECRET), "错误原因不得泄露 Key: {detail}");
    }

    // ---- T-533 模板服务装配 ----

    /// 装配缺失时模板命令返回 E-CONFIG-001；装配后 CRUD 可用（内置只读）。
    #[tokio::test]
    async fn templates_use_configured_dir() {
        let dir = TestDir::new("templates-wiring");
        let unwired = make_state(&dir).await;
        assert_eq!(
            unwired.list_templates().await.unwrap_err().code(),
            "E-CONFIG-001"
        );

        let wired = make_state(&dir).await.with_templates_dir(dir.0.clone());
        let list = wired.list_templates().await.unwrap();
        assert_eq!(list.len(), super::super::templates::BUILTIN_CATALOG.len());
        assert!(list.iter().all(|template| template.builtin));

        let created = wired
            .create_template(super::super::templates::AiTemplate {
                name: "自定义模板".to_string(),
                prompt: "请整理：{文本}".to_string(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(created.id.starts_with("tpl-"));

        // 内置只读（不因走 AiState 而放行）。
        let mut builtin = list[0].clone();
        builtin.prompt = "被修改".to_string();
        assert_eq!(
            wired.update_template(builtin).await.unwrap_err().code(),
            "E-CONFIG-004"
        );

        // 复制内置 → 自定义可编辑；删除副本。
        let copy = wired.copy_template(&list[0].id).await.unwrap();
        assert!(!copy.builtin);
        wired.delete_template(&copy.id).await.unwrap();
    }
}
