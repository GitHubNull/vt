//! 预置供应商清单（PRD 3.7 / F-AI-06）与供应商配置校验。
//!
//! 清单为**数据化**预置：名称、协议、默认 base_url、是否本地、是否需要 Key、
//! 是否推荐（PRD：通义千问带"推荐"字样）。渲染侧去重（AGENTS.md 原型问题
//! 修正清单）在 `merge_presets` 落实：ID 已存在或名称与已有供应商重复的预置
//! 不再重复加入。
//!
//! Base URL 为编写时的官方公开接口地址（OpenAI 兼容入口）：
//! - 龙猫（LongCat）：<https://api.longcat.chat/openai/v1>
//! - 小米（MiMo 开放平台）：<https://api.xiaomimimo.com/v1>
//! - Mercury（Inception Labs）：<https://api.inceptionlabs.ai/v1>

use serde::Serialize;

use crate::error::AppError;
use crate::services::config_store::AiProviderEntry;

use super::provider::ProtocolKind;

/// 校验错误编号：与 ConfigStore `ERR_BAD_VALUE` 同号段语义（E-CONFIG-004）。
pub const ERR_AI_BAD_VALUE: u32 = 4;
/// 错误编号：供应商不存在（E-CONFIG-005）。
pub const ERR_AI_NOT_FOUND: u32 = 5;

/// API Key 长度上限（防误粘贴整段文件；真实 Key 远小于该值）。
pub const MAX_API_KEY_LEN: usize = 4096;

/// 单个预置供应商（数据化清单条目）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PresetProvider {
    /// 稳定 ID（配置 `AiProviderEntry.id` 使用同一值）。
    pub id: &'static str,
    /// 显示名（与 PRD 3.7 卡片一致）。
    pub name: &'static str,
    /// 接口协议。
    pub protocol: ProtocolKind,
    /// 默认接口地址（用户可改）。
    pub base_url: &'static str,
    /// 本地供应商（数据不出本机，卡片带"本地"标签）。
    pub local: bool,
    /// 是否需要 API Key（本地供应商免 Key）。
    pub key_required: bool,
    /// 是否推荐（卡片带"推荐"字样）。
    pub recommended: bool,
}

/// 预置供应商清单（顺序即 PRD 3.7 卡片顺序，通义千问推荐置首）。
pub const CATALOG: &[PresetProvider] = &[
    PresetProvider {
        id: "qwen",
        name: "通义千问",
        protocol: ProtocolKind::OpenAiCompatible,
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        local: false,
        key_required: true,
        recommended: true,
    },
    PresetProvider {
        id: "deepseek",
        name: "DeepSeek",
        protocol: ProtocolKind::OpenAiCompatible,
        base_url: "https://api.deepseek.com/v1",
        local: false,
        key_required: true,
        recommended: false,
    },
    PresetProvider {
        id: "zhipu",
        name: "智谱 AI",
        protocol: ProtocolKind::OpenAiCompatible,
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        local: false,
        key_required: true,
        recommended: false,
    },
    PresetProvider {
        id: "doubao",
        name: "豆包",
        protocol: ProtocolKind::OpenAiCompatible,
        base_url: "https://ark.cn-beijing.volces.com/api/v3",
        local: false,
        key_required: true,
        recommended: false,
    },
    PresetProvider {
        id: "siliconflow",
        name: "硅基流动",
        protocol: ProtocolKind::OpenAiCompatible,
        base_url: "https://api.siliconflow.cn/v1",
        local: false,
        key_required: true,
        recommended: false,
    },
    PresetProvider {
        id: "ollama",
        name: "Ollama",
        protocol: ProtocolKind::Ollama,
        base_url: "http://127.0.0.1:11434",
        local: true,
        key_required: false,
        recommended: false,
    },
    PresetProvider {
        id: "lm_studio",
        name: "LM Studio",
        protocol: ProtocolKind::LmStudio,
        base_url: "http://127.0.0.1:1234/v1",
        local: true,
        key_required: false,
        recommended: false,
    },
    PresetProvider {
        id: "kimi",
        name: "Kimi",
        protocol: ProtocolKind::OpenAiCompatible,
        base_url: "https://api.moonshot.cn/v1",
        local: false,
        key_required: true,
        recommended: false,
    },
    PresetProvider {
        id: "gemini",
        name: "Gemini",
        protocol: ProtocolKind::OpenAiCompatible,
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        local: false,
        key_required: true,
        recommended: false,
    },
    PresetProvider {
        id: "longcat",
        name: "龙猫",
        protocol: ProtocolKind::OpenAiCompatible,
        base_url: "https://api.longcat.chat/openai/v1",
        local: false,
        key_required: true,
        recommended: false,
    },
    PresetProvider {
        id: "openai",
        name: "OpenAI",
        protocol: ProtocolKind::OpenAiCompatible,
        base_url: "https://api.openai.com/v1",
        local: false,
        key_required: true,
        recommended: false,
    },
    PresetProvider {
        id: "xiaomi",
        name: "小米",
        protocol: ProtocolKind::OpenAiCompatible,
        base_url: "https://api.xiaomimimo.com/v1",
        local: false,
        key_required: true,
        recommended: false,
    },
    PresetProvider {
        id: "mercury",
        name: "Mercury",
        protocol: ProtocolKind::OpenAiCompatible,
        base_url: "https://api.inceptionlabs.ai/v1",
        local: false,
        key_required: true,
        recommended: false,
    },
    PresetProvider {
        id: "cerebras",
        name: "Cerebras",
        protocol: ProtocolKind::OpenAiCompatible,
        base_url: "https://api.cerebras.ai/v1",
        local: false,
        key_required: true,
        recommended: false,
    },
];

impl PresetProvider {
    /// 预置条目 → 配置条目（默认不启用、未选模型、无 Key 引用）。
    pub fn to_entry(&self) -> AiProviderEntry {
        AiProviderEntry {
            id: self.id.to_string(),
            name: self.name.to_string(),
            protocol: self.protocol.as_config_str().to_string(),
            base_url: self.base_url.to_string(),
            enabled: false,
            model: None,
            api_key_ref: None,
            local: self.local,
        }
    }
}

/// 按 ID 查找预置。
pub fn find(id: &str) -> Option<&'static PresetProvider> {
    CATALOG.iter().find(|preset| preset.id == id)
}

/// OS 凭据存储引用键（tech/08 8.3：配置文件只存引用键，如 `cred:provider:deepseek`）。
pub fn credential_ref_key(provider_id: &str) -> String {
    format!("cred:provider:{provider_id}")
}

/// 生成用户自建供应商 ID（`custom-` + 16 位随机 hex）。
pub fn new_custom_id() -> String {
    let bytes: [u8; 8] = rand::random();
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("custom-{hex}")
}

/// 预置合并结果。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MergeResult {
    /// 新加入的预置显示名。
    pub added: Vec<String>,
    /// 因同名供应商已存在而跳过的预置显示名（PRD 3.7 业务规则 1：名称唯一）。
    pub skipped: Vec<String>,
}

/// 把预置清单合并进现有供应商列表（按 ID 去重；同名不同 ID 的预置跳过）。
///
/// 返回合并明细；`providers` 无变化时 `added` 为空。
pub fn merge_presets(providers: &mut Vec<AiProviderEntry>) -> MergeResult {
    let mut result = MergeResult::default();
    for preset in CATALOG {
        if providers.iter().any(|entry| entry.id == preset.id) {
            continue; // 已存在（含用户改过配置的同 ID 条目）：绝不覆盖用户配置
        }
        if providers
            .iter()
            .any(|entry| entry.name.trim() == preset.name)
        {
            // PRD 3.7 业务规则 1：供应商名称唯一——同名条目已存在时跳过，
            // 交由设置页提示用户合并或重命名（修正原型的重复项问题）。
            result.skipped.push(preset.name.to_string());
            continue;
        }
        providers.push(preset.to_entry());
        result.added.push(preset.name.to_string());
    }
    result
}

fn bad_value(message: impl Into<String>) -> AppError {
    AppError::Config(ERR_AI_BAD_VALUE, message.into())
}

/// 校验供应商配置条目（PRD 3.7 编辑字段与业务规则；CRUD 命令入口统一走此校验）。
///
/// `others` 为除本条目外的现有清单（名称唯一性检查）。
/// 返回解析后的协议类型供调用方使用。
pub fn validate_entry(
    entry: &AiProviderEntry,
    others: &[AiProviderEntry],
) -> Result<ProtocolKind, AppError> {
    let id = entry.id.trim();
    if id.is_empty() {
        return Err(bad_value("供应商 ID 不能为空"));
    }
    if id.chars().count() > 128 {
        return Err(bad_value("供应商 ID 过长"));
    }
    let name = entry.name.trim();
    if name.is_empty() {
        return Err(bad_value("供应商名称不能为空"));
    }
    if name.chars().count() > 100 {
        return Err(bad_value("供应商名称过长（不超过 100 字）"));
    }
    if others
        .iter()
        .any(|other| other.id != id && other.name.trim() == name)
    {
        return Err(bad_value(format!("供应商名称“{name}”与已有供应商重复")));
    }
    let protocol = ProtocolKind::parse(entry.protocol.trim())
        .ok_or_else(|| bad_value(format!("接口协议不合法: {}", entry.protocol)))?;
    let base_url = entry.base_url.trim();
    if base_url.is_empty() {
        return Err(bad_value("接口地址不能为空"));
    }
    let url = reqwest::Url::parse(base_url)
        .map_err(|_| bad_value("接口地址须为合法 URL（如 https://api.example.com/v1）"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(bad_value("接口地址须以 http:// 或 https:// 开头"));
    }
    if url.host_str().is_none() {
        return Err(bad_value("接口地址缺少主机名"));
    }
    match protocol {
        ProtocolKind::Ollama | ProtocolKind::LmStudio => {
            if !entry.local {
                return Err(bad_value("Ollama / LM Studio 为本地供应商，须标记 local"));
            }
            if entry.api_key_ref.is_some() {
                return Err(bad_value("本地供应商无需 API Key"));
            }
        }
        ProtocolKind::OpenAiCompatible => {
            if entry.local {
                return Err(bad_value("OpenAI 兼容协议为云端供应商，不可标记 local"));
            }
        }
    }
    if let Some(model) = entry
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        if model.chars().count() > 256 {
            return Err(bad_value("模型标识过长"));
        }
    }
    Ok(protocol)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验收标准 1：预置供应商清单与 PRD 3.7 节一致。
    #[test]
    fn catalog_matches_prd_3_7() {
        let names: Vec<_> = CATALOG.iter().map(|preset| preset.name).collect();
        assert_eq!(
            names,
            vec![
                "通义千问",
                "DeepSeek",
                "智谱 AI",
                "豆包",
                "硅基流动",
                "Ollama",
                "LM Studio",
                "Kimi",
                "Gemini",
                "龙猫",
                "OpenAI",
                "小米",
                "Mercury",
                "Cerebras",
            ]
        );
        // 推荐项：仅通义千问；本地项：Ollama / LM Studio 且免 Key。
        assert!(find("qwen").unwrap().recommended);
        assert_eq!(CATALOG.iter().filter(|p| p.recommended).count(), 1);
        for preset in CATALOG {
            assert_eq!(preset.local, preset.protocol.is_local());
            assert_eq!(preset.key_required, !preset.local);
        }
    }

    #[test]
    fn ids_unique_and_base_urls_wellformed() {
        let mut ids: Vec<_> = CATALOG.iter().map(|preset| preset.id).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "预置 ID 不得重复");
        for preset in CATALOG {
            let url = reqwest::Url::parse(preset.base_url).expect("base_url 合法");
            assert!(matches!(url.scheme(), "http" | "https"));
            assert!(url.host_str().is_some());
            assert_eq!(preset.to_entry().id, preset.id);
        }
    }

    #[test]
    fn merge_is_idempotent_and_respects_name_uniqueness() {
        let mut providers: Vec<AiProviderEntry> = Vec::new();
        let first = merge_presets(&mut providers);
        assert_eq!(first.added.len(), CATALOG.len());
        assert!(first.skipped.is_empty());

        // 幂等：再次合并不新增。
        let second = merge_presets(&mut providers);
        assert!(second.added.is_empty() && second.skipped.is_empty());
        assert_eq!(providers.len(), CATALOG.len());

        // 删除一个预置后合并 → 恢复该预置。
        providers.retain(|entry| entry.id != "deepseek");
        let third = merge_presets(&mut providers);
        assert_eq!(third.added, vec!["DeepSeek".to_string()]);

        // 用户自建同名供应商 → 预置跳过（不产生重复项）。
        let mut providers = vec![AiProviderEntry {
            id: new_custom_id(),
            name: "DeepSeek".to_string(),
            ..AiProviderEntry::default()
        }];
        let fourth = merge_presets(&mut providers);
        assert_eq!(fourth.skipped, vec!["DeepSeek".to_string()]);
        // 仅同名预置跳过，其余预置照常合并（名称唯一性只挡同名）。
        assert!(!providers.iter().any(|entry| entry.id == "deepseek"));
        assert_eq!(providers.len(), CATALOG.len());
    }

    #[test]
    fn validate_rejects_duplicates_and_bad_fields() {
        let existing = vec![find("deepseek").unwrap().to_entry()];
        // 名称重复（与不同 ID 条目重名）。
        let mut entry = find("kimi").unwrap().to_entry();
        entry.name = "DeepSeek".to_string();
        assert_eq!(
            validate_entry(&entry, &existing).unwrap_err().code(),
            "E-CONFIG-004"
        );
        // 协议非法。
        let mut entry = find("kimi").unwrap().to_entry();
        entry.protocol = "grpc".to_string();
        assert!(validate_entry(&entry, &existing).is_err());
        // 地址非法。
        let mut entry = find("kimi").unwrap().to_entry();
        entry.base_url = "ftp://x".to_string();
        assert!(validate_entry(&entry, &existing).is_err());
        // 本地标记与协议矛盾。
        let mut entry = find("ollama").unwrap().to_entry();
        entry.local = false;
        assert!(validate_entry(&entry, &existing).is_err());
        // 本地供应商带 Key 引用。
        let mut entry = find("lm_studio").unwrap().to_entry();
        entry.api_key_ref = Some(credential_ref_key("lm_studio"));
        assert!(validate_entry(&entry, &existing).is_err());
        // 合法条目通过。
        let entry = find("ollama").unwrap().to_entry();
        assert!(validate_entry(&entry, &existing).is_ok());
    }

    #[test]
    fn ref_key_format_follows_tech_08() {
        assert_eq!(credential_ref_key("deepseek"), "cred:provider:deepseek");
        assert!(new_custom_id().starts_with("custom-"));
        assert_eq!(new_custom_id().len(), "custom-".len() + 16);
        assert_ne!(new_custom_id(), new_custom_id());
    }
}
