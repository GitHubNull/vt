//! AI 供应商服务（T-531）：AiState 聚合配置存储、凭据存储与 HTTP 客户端，
//! 提供供应商 CRUD、Key 保管（永不回传明文）与适配器构造。
//!
//! 装配建议（tech/04 4.1）：`AiState` 独立 `app.manage(...)`（命令层以
//! `State<'_, AiState>` 访问），构造见 [`AiState::new`]，启动时调用一次
//! [`AiState::seed_if_empty`] 灌入预置清单。
//!
//! 安全（NFR-S-01/02）：
//! - API Key 本体只进 OS 凭据管理器（不可用时降级加密文件，tech/08 8.3），
//!   配置文件只存引用键（`cred:provider:{id}`）；
//! - Key 不写日志、不进错误信息；命令层只能拿到"已设置"布尔状态。

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

/// 错误编号：凭据操作任务异常（E-PLAT-023，keyring=20 / 降级 IO=21 / 加密=22 顺延）。
const ERR_CRED_JOIN: u32 = 23;
/// 错误编号：AI 配置序列化失败（与 ConfigStore `ERR_BAD_JSON` 同号，E-CONFIG-002）。
const ERR_AI_SERIALIZE: u32 = 2;

/// AI 服务状态：命令层经 `State<'_, AiState>` 使用。
pub struct AiState {
    config: ConfigStore,
    credentials: Arc<dyn CredentialStore>,
    http: reqwest::Client,
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
    /// 本地供应商（卡片"本地"标签，数据不出本机）。
    pub local: bool,
    /// 云端供应商需要 Key。
    pub key_required: bool,
    /// Key 是否已设置（只回状态，不回内容，NFR-S-01）。
    pub key_set: bool,
    /// 预置推荐项（卡片"推荐"字样）。
    pub recommended: bool,
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
        }
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
            *slot = entry;
        } else {
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
    pub async fn complete(
        &self,
        id: &str,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let provider = self.build_provider(id).await?;
        provider.complete(request).await
    }

    /// 拉取供应商模型清单（F-AI-08）。
    pub async fn fetch_models(&self, id: &str) -> Result<Vec<ModelInfo>, AppError> {
        let provider = self.build_provider(id).await?;
        provider.list_models().await
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
}
