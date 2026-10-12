//! 专有名词服务（T-522，F-DICT-02、SPEC: doc/specs/v0.5/T-522-proper-nouns.md）。
//!
//! - 数据模型：标准写法 + 读音别名（多个）+ 备注 + 启停；
//! - 持久化：数据目录 `dict/proper-nouns.json`（`write_json_atomic` 原子写入，NFR-R-05）；
//! - 约束（PRD 3.4.2）：上限 2,000 条；标准写法不可重复（英文不区分大小写）；
//!   别名可选，未填写时按标准写法本身匹配（英文不区分大小写归一）；
//! - 损坏回退（NFR-S-07）：文件解析失败回退空词典 + 广播提示事件，不崩溃；
//! - 变更广播复用 `dict:changed`（多窗口同步，管线 Stage 据此刷新快照）；
//! - 优先级（PRD 3.1.5）：专有名词（用户手动）> 技术词库，由管线中
//!   ProperNounStage 先于 TechLexiconStage 执行保证；
//! - 隐私（NFR-S-02）：日志只记录条数与操作结果，绝不记录词条内容。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};
use tracing::{info, warn};

use crate::error::AppError;
use crate::events::AppEvent;
use crate::services::config_store::write_json_atomic;
use crate::services::dict::{
    oversize_error, parse_unified_envelope, ConflictStrategy, DecodedImport, DictKind,
    ImportFormat, ImportInspection, ImportIssue, ImportReport, UnifiedExportFile,
    ERR_IMPORT_EXPORT, MAX_IMPORT_BYTES,
};
use crate::services::post_process::CorrectionEntry;

/// 专有名词文件相对数据目录路径（SPEC 1.2：`dict/proper-nouns.json`）。
pub const PROPER_NOUNS_REL_PATH: &str = "dict/proper-nouns.json";
/// 条目上限（PRD 3.4.2 业务规则 1：2,000 条，不含技术词库内置词条）。
pub const MAX_NOUNS: usize = 2000;
/// 标准写法最大长度（字符数）。
pub const MAX_TERM_LEN: usize = 100;
/// 别名条数上限（单条目）。
pub const MAX_ALIASES: usize = 20;
/// 别名最大长度（字符数）。
pub const MAX_ALIAS_LEN: usize = 100;
/// 备注最大长度（字符数）。
pub const MAX_NOTE_LEN: usize = 200;

/// 错误编号：条目数量已达上限。
pub const ERR_NOUN_LIMIT: u32 = 11;
/// 错误编号：标准写法为空 / 超长 / 重复，或别名、备注不合法。
pub const ERR_BAD_NOUN: u32 = 12;

/// 专有名词条目（F-DICT-02）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProperNoun {
    /// 条目 ID（服务内生成，自增序号，持久化保持稳定）。
    pub id: u64,
    /// 标准写法（如 `PostgreSQL`）。
    pub term: String,
    /// 读音别名（可多个，如 `["PG", "波斯格雷"]`；可为空）。
    #[serde(default)]
    pub aliases: Vec<String>,
    /// 备注（可选）。
    #[serde(default)]
    pub note: String,
    /// 启用开关（停用后该条目不参与纠正）。
    pub enabled: bool,
}

/// 磁盘文件形态：版本号 + 条目数组。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProperNounsFile {
    version: u32,
    nouns: Vec<ProperNoun>,
}

/// 专有名词导出条目（T-523 统一格式 items 元素；不含内部 ID，导入时重新分配）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProperNounExportItem {
    /// 标准写法。
    pub term: String,
    /// 读音别名（可多个）。
    #[serde(default)]
    pub aliases: Vec<String>,
    /// 备注。
    #[serde(default)]
    pub note: String,
    /// 启用开关。
    pub enabled: bool,
}

/// 专有名词服务：内存镜像 + 原子持久化 + 变更广播。
#[derive(Clone)]
pub struct ProperNounService {
    path: Arc<PathBuf>,
    nouns: Arc<RwLock<Vec<ProperNoun>>>,
    next_id: Arc<RwLock<u64>>,
    events: Arc<broadcast::Sender<AppEvent>>,
}

impl ProperNounService {
    /// 加载词典：文件不存在 → 空词典；解析失败 → 回退空词典并广播提示
    /// （NFR-S-07：不崩溃、可继续添加），提示事件在本函数内直接发送，
    /// 调用方无需处理恢复标记。
    pub fn load(data_dir: &Path, events: broadcast::Sender<AppEvent>) -> Self {
        let path = data_dir.join(PROPER_NOUNS_REL_PATH);
        let nouns = match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<ProperNounsFile>(&content) {
                Ok(file) => {
                    info!(count = file.nouns.len(), "专有名词加载成功");
                    file.nouns
                }
                Err(err) => {
                    // NFR-S-02：日志只记录解析错误，不记录词条内容。
                    warn!(error = %err, "专有名词文件解析失败，回退空词典");
                    let _ = events.send(AppEvent::AsrNotice {
                        code: "E-CONFIG-002".to_string(),
                        message: "专有名词文件损坏，已回退为空词典，请重新添加条目".to_string(),
                    });
                    Vec::new()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                info!("专有名词文件不存在，使用空词典");
                Vec::new()
            }
            Err(err) => {
                // 读取失败（权限等）：按空词典继续（不致命），仅记录告警。
                warn!(error = %err, "专有名词文件读取失败，使用空词典");
                Vec::new()
            }
        };
        let next_id = nouns.iter().map(|noun| noun.id).max().unwrap_or(0) + 1;
        Self {
            path: Arc::new(path),
            nouns: Arc::new(RwLock::new(nouns)),
            next_id: Arc::new(RwLock::new(next_id)),
            events: Arc::new(events),
        }
    }

    /// 条目清单（管理页数据源）。
    pub async fn list(&self) -> Vec<ProperNoun> {
        self.nouns.read().await.clone()
    }

    /// 新增条目（PRD 3.4.2 业务规则 1/2 校验；别名做归一清洗）。
    pub async fn add(
        &self,
        term: String,
        aliases: Vec<String>,
        note: String,
        enabled: bool,
    ) -> Result<ProperNoun, AppError> {
        let mut nouns = self.nouns.write().await;
        let term = validate_term(&nouns, None, &term)?;
        let aliases = validate_aliases(&aliases)?;
        let note = validate_note(&note)?;
        if nouns.len() >= MAX_NOUNS {
            return Err(AppError::Config(
                ERR_NOUN_LIMIT,
                format!("专有名词最多 {MAX_NOUNS} 条，已达上限"),
            ));
        }
        let mut next_id = self.next_id.write().await;
        let noun = ProperNoun {
            id: *next_id,
            term,
            aliases,
            note,
            enabled,
        };
        *next_id += 1;
        nouns.push(noun.clone());
        drop(nouns);
        drop(next_id);
        self.persist_and_notify().await?;
        Ok(noun)
    }

    /// 更新条目（标准写法 / 别名 / 备注 / 启停整体替换）。
    pub async fn update(&self, noun: ProperNoun) -> Result<(), AppError> {
        let mut nouns = self.nouns.write().await;
        if !nouns.iter().any(|item| item.id == noun.id) {
            return Err(AppError::Config(ERR_BAD_NOUN, "条目不存在".to_string()));
        }
        let term = validate_term(&nouns, Some(noun.id), &noun.term)?;
        let aliases = validate_aliases(&noun.aliases)?;
        let note = validate_note(&noun.note)?;
        if let Some(existing) = nouns.iter_mut().find(|item| item.id == noun.id) {
            existing.term = term;
            existing.aliases = aliases;
            existing.note = note;
            existing.enabled = noun.enabled;
        }
        drop(nouns);
        self.persist_and_notify().await
    }

    /// 删除条目。
    pub async fn delete(&self, id: u64) -> Result<(), AppError> {
        let mut nouns = self.nouns.write().await;
        let before = nouns.len();
        nouns.retain(|noun| noun.id != id);
        if nouns.len() == before {
            return Err(AppError::Config(ERR_BAD_NOUN, "条目不存在".to_string()));
        }
        drop(nouns);
        self.persist_and_notify().await
    }

    /// 启停切换。
    pub async fn toggle(&self, id: u64, enabled: bool) -> Result<(), AppError> {
        let mut nouns = self.nouns.write().await;
        let Some(existing) = nouns.iter_mut().find(|noun| noun.id == id) else {
            return Err(AppError::Config(ERR_BAD_NOUN, "条目不存在".to_string()));
        };
        existing.enabled = enabled;
        drop(nouns);
        self.persist_and_notify().await
    }

    /// 导出统一格式（v2）JSON 字符串（T-523；专有名词）。
    ///
    /// 不含内部 ID（导入端重新分配）；含停用条目（往返一致）。
    /// 隐私（NFR-S-02）：条目内容只写入用户选定的目标文件，不落日志。
    pub async fn export_unified(&self) -> Result<String, AppError> {
        let nouns = self.nouns.read().await.clone();
        let items: Vec<ProperNounExportItem> = nouns
            .iter()
            .map(|noun| ProperNounExportItem {
                term: noun.term.clone(),
                aliases: noun.aliases.clone(),
                note: noun.note.clone(),
                enabled: noun.enabled,
            })
            .collect();
        let file = UnifiedExportFile::new(DictKind::ProperNouns, items);
        serde_json::to_string_pretty(&file)
            .map_err(|err| AppError::Config(ERR_IMPORT_EXPORT, format!("导出序列化失败: {err}")))
    }

    /// 导入前校验（向导「校验结果展示」步骤；只读，不修改任何数据）。
    ///
    /// 返回条目总数、无效条目数（将被跳过并计入失败）与冲突条数
    /// （与现有条目重复或文件内重复；标准写法英文不区分大小写）。
    pub async fn inspect_import(&self, content: &str) -> Result<ImportInspection, AppError> {
        use std::collections::HashSet;
        let decoded = decode_proper_nouns_import(content)?;
        let existing = self.nouns.read().await;
        let mut invalid = 0usize;
        let mut conflicts = 0usize;
        let mut seen: HashSet<String> = HashSet::new();
        for item in &decoded.items {
            let Ok(item) = item else {
                invalid += 1;
                continue;
            };
            let Ok((term, _, _)) = validate_noun_import_item(item) else {
                invalid += 1;
                continue;
            };
            let key = term.to_lowercase();
            let in_existing = existing
                .iter()
                .any(|noun| noun.term.trim().to_lowercase() == key);
            let in_batch = !seen.insert(key);
            if in_existing || in_batch {
                conflicts += 1;
            }
        }
        Ok(ImportInspection {
            format: ImportFormat::Unified.as_str().to_string(),
            version: decoded.version,
            total: decoded.total,
            invalid,
            conflicts,
        })
    }

    /// 执行导入：冲突策略 + 上限检查 + 原子写入 + 导入报告（T-523）。
    ///
    /// - 文件级校验失败 → 整体拒绝，不产生任何写入；
    /// - 单条无效 → 计入失败条数并跳过，其余条目继续；
    /// - 冲突按策略处理：跳过 / 覆盖（导入为准）/ 合并（现有为准——别名取并集、备注空则补全、标准写法与启用状态保留现有；并集超过别名上限时该条计入失败且现有条目不变，不静默截断）；
    /// - 导入后超过 2,000 条 → 整体拒绝（不留半导入状态）；
    /// - 成功写入后持久化并广播 `dict:changed`（多窗口同步）。
    ///
    /// 隐私（NFR-S-02）：日志只记录条数统计，绝不记录条目内容。
    pub async fn apply_import(
        &self,
        content: &str,
        strategy: ConflictStrategy,
    ) -> Result<ImportReport, AppError> {
        use std::collections::HashSet;
        let decoded = decode_proper_nouns_import(content)?;
        let mut nouns = self.nouns.write().await;
        let mut report = ImportReport::default();
        let mut pending: Vec<ProperNoun> = Vec::new();
        let mut updates: Vec<(usize, ProperNoun)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for (index, result) in decoded.items.iter().enumerate() {
            let seq = index + 1;
            let item = match result {
                Ok(item) => item,
                Err(reason) => {
                    report.failed += 1;
                    report.issues.push(ImportIssue {
                        index: seq,
                        reason: reason.clone(),
                    });
                    continue;
                }
            };
            let (term, aliases, note) = match validate_noun_import_item(item) {
                Ok(valid) => valid,
                Err(reason) => {
                    report.failed += 1;
                    report.issues.push(ImportIssue { index: seq, reason });
                    continue;
                }
            };
            let key = term.to_lowercase();
            if !seen.insert(key.clone()) {
                // 文件内重复：保留先出现的一条（与既有导入行为一致）。
                report.skipped += 1;
                continue;
            }
            let Some(existing_index) = nouns
                .iter()
                .position(|noun| noun.term.trim().to_lowercase() == key)
            else {
                pending.push(ProperNoun {
                    id: 0,
                    term,
                    aliases,
                    note,
                    enabled: item.enabled,
                });
                continue;
            };
            match strategy {
                ConflictStrategy::Skip => report.skipped += 1,
                ConflictStrategy::Overwrite => {
                    updates.push((
                        existing_index,
                        ProperNoun {
                            id: nouns[existing_index].id,
                            term,
                            aliases,
                            note,
                            enabled: item.enabled,
                        },
                    ));
                    report.updated += 1;
                }
                ConflictStrategy::Merge => {
                    // 合并语义（T-523）：以现有条目为准——标准写法与启用状态保留
                    // 现有（导入不改变用户当前选择）；别名取并集（现有在前，
                    // 英文不区分大小写去重）；备注现有非空则保留，否则补全。
                    let current = &nouns[existing_index];
                    let mut merged_aliases = current.aliases.clone();
                    let mut seen_alias: HashSet<String> = merged_aliases
                        .iter()
                        .map(|alias| alias.to_lowercase())
                        .collect();
                    for alias in &aliases {
                        if seen_alias.insert(alias.to_lowercase()) {
                            merged_aliases.push(alias.clone());
                        }
                    }
                    if merged_aliases.len() > MAX_ALIASES {
                        report.failed += 1;
                        report.issues.push(ImportIssue {
                            index: seq,
                            reason: format!("合并后别名超过上限 {MAX_ALIASES} 个，未合并"),
                        });
                        continue;
                    }
                    updates.push((
                        existing_index,
                        ProperNoun {
                            id: current.id,
                            term: current.term.clone(),
                            aliases: merged_aliases,
                            note: if current.note.is_empty() {
                                note.clone()
                            } else {
                                current.note.clone()
                            },
                            enabled: current.enabled,
                        },
                    ));
                    report.updated += 1;
                }
            }
        }
        // 上限检查：整体拒绝，不留半导入状态。
        if nouns.len() + pending.len() > MAX_NOUNS {
            return Err(AppError::Config(
                ERR_NOUN_LIMIT,
                format!(
                    "导入后将达到 {} 条，超过专有名词上限 {MAX_NOUNS} 条",
                    nouns.len() + pending.len()
                ),
            ));
        }
        for (index, noun) in updates {
            nouns[index] = noun;
        }
        report.imported = pending.len();
        let mut next_id = self.next_id.write().await;
        for mut noun in pending {
            noun.id = *next_id;
            *next_id += 1;
            nouns.push(noun);
        }
        drop(next_id);
        drop(nouns);
        if report.imported > 0 || report.updated > 0 {
            self.persist_and_notify().await?;
        }
        // NFR-S-02：只记录条数统计，不记录任何条目内容。
        info!(
            imported = report.imported,
            updated = report.updated,
            skipped = report.skipped,
            failed = report.failed,
            "专有名词导入完成"
        );
        Ok(report)
    }

    /// 纠正条目快照（Stage 用）：启用条目的别名 → 标准写法，按别名长度降序
    /// （长别名优先）；别名留空的条目按标准写法自身匹配（PRD 3.4.2 业务规则 2）。
    pub async fn enabled_snapshot(&self) -> Vec<CorrectionEntry> {
        use std::collections::HashSet;
        let nouns = self.nouns.read().await;
        let mut entries: Vec<CorrectionEntry> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut push = |entries: &mut Vec<CorrectionEntry>, alias: &str, term: &str| {
            let alias = alias.trim();
            if alias.is_empty() {
                return;
            }
            let key = alias.to_lowercase();
            if !seen.insert(key) {
                return;
            }
            entries.push(CorrectionEntry {
                alias: alias.to_string(),
                term: term.to_string(),
            });
        };
        for noun in nouns
            .iter()
            .filter(|noun| noun.enabled && !noun.term.is_empty())
        {
            if noun.aliases.is_empty() {
                // 未填写别名：按标准写法的常见读法匹配（英文不区分大小写）。
                push(&mut entries, &noun.term, &noun.term);
            } else {
                for alias in &noun.aliases {
                    push(&mut entries, alias, &noun.term);
                }
            }
        }
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.alias.chars().count()));
        entries
    }

    /// 持久化 + 广播变更（复用 `dict:changed`，管线与多窗口据此刷新）。
    async fn persist_and_notify(&self) -> Result<(), AppError> {
        let nouns = self.nouns.read().await.clone();
        let file = ProperNounsFile {
            version: 1,
            nouns: nouns.clone(),
        };
        write_json_atomic(&self.path, &file)?;
        let _ = self.events.send(AppEvent::DictChanged);
        info!(count = nouns.len(), "专有名词已保存");
        Ok(())
    }
}

// ---- T-523 导入导出：解析 / 解码 / 校验（纯函数，便于单测） ----

/// 解码单条专有名词条目；结构问题（字段缺失 / 类型错）返回中文原因文本。
fn decode_noun_item(value: &serde_json::Value) -> Result<ProperNounExportItem, String> {
    let Some(object) = value.as_object() else {
        return Err("条目必须是对象".to_string());
    };
    let term = match object.get("term") {
        Some(serde_json::Value::String(term)) => term.clone(),
        Some(_) => return Err("term 字段类型非法（应为字符串）".to_string()),
        None => return Err("缺少 term 字段".to_string()),
    };
    let aliases = match object.get("aliases") {
        None => Vec::new(),
        Some(serde_json::Value::Array(list)) => {
            let mut aliases = Vec::with_capacity(list.len());
            for alias in list {
                match alias {
                    serde_json::Value::String(alias) => aliases.push(alias.clone()),
                    _ => return Err("aliases 元素类型非法（应为字符串）".to_string()),
                }
            }
            aliases
        }
        Some(_) => return Err("aliases 字段类型非法（应为字符串数组）".to_string()),
    };
    let note = match object.get("note") {
        Some(serde_json::Value::String(note)) => note.clone(),
        Some(_) => return Err("note 字段类型非法（应为字符串）".to_string()),
        None => String::new(),
    };
    let enabled = match object.get("enabled") {
        Some(serde_json::Value::Bool(enabled)) => *enabled,
        Some(_) => return Err("enabled 字段类型非法（应为布尔值）".to_string()),
        // 缺省启用（与替换词典导入一致）。
        None => true,
    };
    Ok(ProperNounExportItem {
        term,
        aliases,
        note,
        enabled,
    })
}

/// 单条专有名词内容校验（与手动添加同一口径）。
///
/// 返回（裁剪后标准写法, 清洗后别名, 裁剪后备注）；别名清洗去空 / 去重 /
/// 英文不区分大小写，与 `validate_aliases` 一致。
fn validate_noun_import_item(
    item: &ProperNounExportItem,
) -> Result<(String, Vec<String>, String), String> {
    let term = item.term.trim();
    if term.is_empty() {
        return Err("标准写法为空".to_string());
    }
    if term.chars().count() > MAX_TERM_LEN {
        return Err(format!("标准写法过长（最多 {MAX_TERM_LEN} 字）"));
    }
    let aliases = validate_aliases(&item.aliases).map_err(|err| err.message().to_string())?;
    let note = validate_note(&item.note).map_err(|err| err.message().to_string())?;
    Ok((term.to_string(), aliases, note))
}

/// 解析并解码专有名词导入内容（T-523）。
///
/// 仅接受统一 v2 格式（`{ version, kind, exported_at, items }`）；v0.1 旧版
/// 数组与纯文本仅适用于替换词典，此处整体拒绝并给出指引。
/// 文件级问题 → 整体拒绝（Err）；单条问题 → 进入该条的 `Err`（失败条数）。
pub(crate) fn decode_proper_nouns_import(
    content: &str,
) -> Result<DecodedImport<ProperNounExportItem>, AppError> {
    // 防御性体积复查（命令层已按文件元数据先行拦截，见 commands/dict.rs）。
    if content.len() > MAX_IMPORT_BYTES {
        return Err(oversize_error());
    }
    let trimmed = content.trim_start_matches('\u{feff}').trim_start();
    if trimmed.is_empty() {
        return Err(AppError::Config(
            ERR_IMPORT_EXPORT,
            "导入文件为空".to_string(),
        ));
    }
    if !trimmed.starts_with('{') {
        return Err(AppError::Config(
            ERR_IMPORT_EXPORT,
            "专有名词导入仅支持统一格式文件（{ version, kind, exported_at, items }）；\
             旧版数组与纯文本仅适用于替换词典"
                .to_string(),
        ));
    }
    let (version, raw_items) = parse_unified_envelope(trimmed, DictKind::ProperNouns, MAX_NOUNS)?;
    let items = raw_items.iter().map(decode_noun_item).collect();
    Ok(DecodedImport {
        format: ImportFormat::Unified,
        version,
        total: raw_items.len(),
        items,
    })
}

/// 标准写法校验（PRD 3.4.2 业务规则 2：不可重复；英文不区分大小写）。
fn validate_term(
    existing: &[ProperNoun],
    exclude_id: Option<u64>,
    term: &str,
) -> Result<String, AppError> {
    let term = term.trim();
    if term.is_empty() {
        return Err(AppError::Config(
            ERR_BAD_NOUN,
            "标准写法不能为空".to_string(),
        ));
    }
    if term.chars().count() > MAX_TERM_LEN {
        return Err(AppError::Config(
            ERR_BAD_NOUN,
            format!("标准写法过长（最多 {MAX_TERM_LEN} 字）"),
        ));
    }
    let normalized = term.to_lowercase();
    let duplicated = existing
        .iter()
        .filter(|noun| Some(noun.id) != exclude_id)
        .any(|noun| noun.term.trim().to_lowercase() == normalized);
    if duplicated {
        return Err(AppError::Config(
            ERR_BAD_NOUN,
            "标准写法已存在，请勿重复添加".to_string(),
        ));
    }
    Ok(term.to_string())
}

/// 别名归一清洗：去首尾空白、去空、去重（英文不区分大小写）、限量限长。
fn validate_aliases(aliases: &[String]) -> Result<Vec<String>, AppError> {
    let mut cleaned: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for alias in aliases {
        let alias = alias.trim();
        if alias.is_empty() {
            continue;
        }
        if alias.chars().count() > MAX_ALIAS_LEN {
            return Err(AppError::Config(
                ERR_BAD_NOUN,
                format!("别名过长（最多 {MAX_ALIAS_LEN} 字）"),
            ));
        }
        let key = alias.to_lowercase();
        if seen.insert(key) {
            cleaned.push(alias.to_string());
        }
    }
    if cleaned.len() > MAX_ALIASES {
        return Err(AppError::Config(
            ERR_BAD_NOUN,
            format!("别名最多 {MAX_ALIASES} 个"),
        ));
    }
    Ok(cleaned)
}

/// 备注校验（可选，限长）。
fn validate_note(note: &str) -> Result<String, AppError> {
    let note = note.trim();
    if note.chars().count() > MAX_NOTE_LEN {
        return Err(AppError::Config(
            ERR_BAD_NOUN,
            format!("备注过长（最多 {MAX_NOTE_LEN} 字）"),
        ));
    }
    Ok(note.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::post_process::apply_alias_corrections;

    /// 临时目录（进程 + 纳秒唯一，析构清理）。
    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "vt-pn-test-{}-{}-{:?}",
                name,
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

    fn make_service(dir: &TestDir) -> ProperNounService {
        ProperNounService::load(&dir.0, crate::events::channel().0)
    }

    #[tokio::test]
    async fn add_and_persist_roundtrip() {
        let dir = TestDir::new("roundtrip");
        let service = make_service(&dir);
        let noun = service
            .add(
                "PostgreSQL".to_string(),
                vec!["PG".to_string(), "波斯格雷".to_string()],
                "数据库".to_string(),
                true,
            )
            .await
            .unwrap();
        assert_eq!(noun.id, 1);

        // 重启（重新 load）后条目保持（SPEC：条目与别名管理可用，重启后保持）。
        let service2 = make_service(&dir);
        let nouns = service2.list().await;
        assert_eq!(nouns.len(), 1);
        assert_eq!(nouns[0].term, "PostgreSQL");
        assert_eq!(nouns[0].aliases, vec!["PG", "波斯格雷"]);
        assert_eq!(nouns[0].note, "数据库");
        assert!(nouns[0].enabled);
        // ID 续接。
        let next = service2
            .add("码语".to_string(), vec![], String::new(), true)
            .await
            .unwrap();
        assert_eq!(next.id, 2);
    }

    #[tokio::test]
    async fn duplicate_term_rejected_case_insensitive() {
        let dir = TestDir::new("dup");
        let service = make_service(&dir);
        service
            .add("PostgreSQL".to_string(), vec![], String::new(), true)
            .await
            .unwrap();
        // 标准写法不可重复（PRD 3.4.2 业务规则 2，英文不区分大小写）。
        let err = service
            .add("postgresql".to_string(), vec![], String::new(), true)
            .await
            .unwrap_err();
        assert_eq!(err.code(), format!("E-CONFIG-{ERR_BAD_NOUN:03}"));
    }

    #[tokio::test]
    async fn validation_rules() {
        let dir = TestDir::new("validate");
        let service = make_service(&dir);
        // 空标准写法拒绝。
        let err = service
            .add("  ".to_string(), vec![], String::new(), true)
            .await
            .unwrap_err();
        assert_eq!(err.code(), format!("E-CONFIG-{ERR_BAD_NOUN:03}"));
        // 超长标准写法拒绝。
        let err = service
            .add("字".repeat(MAX_TERM_LEN + 1), vec![], String::new(), true)
            .await
            .unwrap_err();
        assert_eq!(err.code(), format!("E-CONFIG-{ERR_BAD_NOUN:03}"));
        // 超长备注拒绝。
        let err = service
            .add("x".to_string(), vec![], "字".repeat(MAX_NOTE_LEN + 1), true)
            .await
            .unwrap_err();
        assert_eq!(err.code(), format!("E-CONFIG-{ERR_BAD_NOUN:03}"));
        // 别名清洗：空白别名剔除、英文不区分大小写去重、首尾空白裁剪。
        let noun = service
            .add(
                "Kubernetes".to_string(),
                vec![
                    " k8s ".to_string(),
                    String::new(),
                    "K8S".to_string(),
                    "K 八 S".to_string(),
                ],
                String::new(),
                true,
            )
            .await
            .unwrap();
        assert_eq!(noun.aliases, vec!["k8s", "K 八 S"]);
    }

    #[tokio::test]
    async fn noun_limit_enforced() {
        let dir = TestDir::new("limit");
        let service = make_service(&dir);
        for i in 0..MAX_NOUNS {
            service
                .add(format!("词{i}"), vec![], String::new(), true)
                .await
                .unwrap();
        }
        let err = service
            .add("超出".to_string(), vec![], String::new(), true)
            .await
            .unwrap_err();
        assert_eq!(err.code(), format!("E-CONFIG-{ERR_NOUN_LIMIT:03}"));
        assert_eq!(service.list().await.len(), MAX_NOUNS);
    }

    #[tokio::test]
    async fn update_delete_toggle() {
        let dir = TestDir::new("crud");
        let service = make_service(&dir);
        let noun = service
            .add(
                "Foo".to_string(),
                vec!["福".to_string()],
                String::new(),
                true,
            )
            .await
            .unwrap();

        service.toggle(noun.id, false).await.unwrap();
        assert!(!service.list().await[0].enabled);
        // 停用条目不进入纠正快照。
        assert!(service.enabled_snapshot().await.is_empty());

        service
            .update(ProperNoun {
                term: "Bar".to_string(),
                aliases: vec!["巴".to_string()],
                note: "改过".to_string(),
                ..noun.clone()
            })
            .await
            .unwrap();
        let updated = &service.list().await[0];
        assert_eq!(updated.term, "Bar");
        assert_eq!(updated.note, "改过");

        service.delete(noun.id).await.unwrap();
        assert!(service.list().await.is_empty());
        // 删除不存在 → 报错。
        assert!(service.delete(noun.id).await.is_err());
        assert!(service.update(noun).await.is_err());
    }

    #[tokio::test]
    async fn corrupted_file_falls_back_to_empty_with_notice() {
        // NFR-S-07 / SPEC：文件损坏回退空词典并提示，不崩溃。
        let (tx, mut rx) = crate::events::channel();
        let dir = TestDir::new("corrupt");
        let file = dir.0.join("dict").join("proper-nouns.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{ broken !!!").unwrap();
        let service = ProperNounService::load(&dir.0, tx);
        assert!(service.list().await.is_empty());
        // 损坏提示事件已广播。
        let event = rx.try_recv().expect("损坏应广播提示事件");
        assert!(matches!(event, AppEvent::AsrNotice { .. }));
        // 回退后可继续正常添加。
        service
            .add("Redis".to_string(), vec![], String::new(), true)
            .await
            .unwrap();
        assert_eq!(service.list().await.len(), 1);
    }

    #[tokio::test]
    async fn change_event_broadcast() {
        let dir = TestDir::new("event");
        let (tx, mut rx) = crate::events::channel();
        let service = ProperNounService::load(&dir.0, tx);
        service
            .add("x".to_string(), vec![], String::new(), true)
            .await
            .unwrap();
        let event = rx.try_recv().expect("应广播 dict:changed");
        assert!(matches!(event, AppEvent::DictChanged));
    }

    #[tokio::test]
    async fn snapshot_alias_priority_and_term_fallback() {
        let dir = TestDir::new("snapshot");
        let service = make_service(&dir);
        // 多别名条目。
        service
            .add(
                "PostgreSQL".to_string(),
                vec!["PG".to_string(), "波斯格雷".to_string()],
                String::new(),
                true,
            )
            .await
            .unwrap();
        // 无别名条目：按标准写法自身匹配。
        service
            .add("Kubernetes".to_string(), vec![], String::new(), true)
            .await
            .unwrap();
        // 停用条目不进快照。
        service
            .add(
                "停用词".to_string(),
                vec!["别名".to_string()],
                String::new(),
                false,
            )
            .await
            .unwrap();

        let entries = service.enabled_snapshot().await;
        // 长别名优先：波斯格雷(4) > Kubernetes(10)？——按字符数：Kubernetes 10 最长。
        assert_eq!(entries[0].alias, "Kubernetes");
        assert_eq!(entries[0].term, "Kubernetes");
        assert!(entries
            .iter()
            .any(|e| e.alias == "波斯格雷" && e.term == "PostgreSQL"));
        assert!(entries
            .iter()
            .any(|e| e.alias == "PG" && e.term == "PostgreSQL"));
        assert!(!entries.iter().any(|e| e.term == "停用词"));
        assert_eq!(entries.len(), 3);
    }

    #[tokio::test]
    async fn prd_acceptance_bosigrey_to_postgresql() {
        // PRD 3.4.2 验收：添加 PostgreSQL（别名 PG、波斯格雷）后，
        // 说"把数据迁移到波斯格雷"上屏为"把数据迁移到 PostgreSQL"。
        let dir = TestDir::new("prd");
        let service = make_service(&dir);
        service
            .add(
                "PostgreSQL".to_string(),
                vec!["PG".to_string(), "波斯格雷".to_string()],
                String::new(),
                true,
            )
            .await
            .unwrap();
        let entries = service.enabled_snapshot().await;
        assert_eq!(
            apply_alias_corrections("把数据迁移到波斯格雷", &entries),
            "把数据迁移到PostgreSQL"
        );
        assert_eq!(
            apply_alias_corrections("把数据迁移到 PostgreSQL", &entries),
            "把数据迁移到 PostgreSQL"
        );
    }

    // ---- T-523 统一导入导出（v2 格式） ----

    #[tokio::test]
    async fn unified_export_import_roundtrip() {
        let dir = TestDir::new("v2-export");
        let service = make_service(&dir);
        service
            .add(
                "PostgreSQL".to_string(),
                vec!["PG".to_string(), "波斯格雷".to_string()],
                "数据库".to_string(),
                true,
            )
            .await
            .unwrap();
        service
            .add("Kubernetes".to_string(), vec![], String::new(), false)
            .await
            .unwrap();

        let json = service.export_unified().await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["kind"], "proper_nouns");
        assert_eq!(
            value["exported_at"].as_str().unwrap().len(),
            20,
            "exported_at 应为 RFC 3339 秒精度"
        );
        let items = value["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert!(items[0].get("id").is_none(), "导出不含内部 ID");

        // 往返一致：导出 → 新服务导入，字段完整保留。
        let dir2 = TestDir::new("v2-import");
        let service2 = make_service(&dir2);
        let report = service2
            .apply_import(&json, ConflictStrategy::Skip)
            .await
            .unwrap();
        assert_eq!(
            (
                report.imported,
                report.updated,
                report.skipped,
                report.failed
            ),
            (2, 0, 0, 0)
        );
        let nouns = service2.list().await;
        assert_eq!(nouns.len(), 2);
        assert_eq!(nouns[0].term, "PostgreSQL");
        assert_eq!(nouns[0].aliases, vec!["PG", "波斯格雷"]);
        assert_eq!(nouns[0].note, "数据库");
        assert!(nouns[0].enabled);
        assert_eq!(nouns[1].term, "Kubernetes");
        assert!(nouns[1].aliases.is_empty());
        assert!(!nouns[1].enabled, "停用状态往返保留");
    }

    #[tokio::test]
    async fn import_inspect_reports_counts() {
        let dir = TestDir::new("inspect");
        let service = make_service(&dir);
        service
            .add("PostgreSQL".to_string(), vec![], String::new(), true)
            .await
            .unwrap();
        let file = r#"{"version":1,"kind":"proper_nouns","items":[
            {"term":"postgresql","aliases":["Pg"]},
            {"term":"Redis","aliases":["瑞迪斯"]},
            {"term":"Redis","aliases":["重复批内"]},
            {"term":"","aliases":[]}
        ]}"#;
        let inspection = service.inspect_import(file).await.unwrap();
        assert_eq!(inspection.format, "v2");
        assert_eq!(inspection.version, 1);
        assert_eq!(inspection.total, 4);
        assert_eq!(inspection.invalid, 1);
        // postgresql 与现有冲突（大小写不敏感）+ Redis 批内重复。
        assert_eq!(inspection.conflicts, 2);
        assert_eq!(service.list().await.len(), 1, "校验不写入数据");

        let report = service
            .apply_import(file, ConflictStrategy::Skip)
            .await
            .unwrap();
        assert_eq!(
            (
                report.imported,
                report.updated,
                report.skipped,
                report.failed
            ),
            (1, 0, 2, 1)
        );
        assert_eq!(service.list().await.len(), 2);
    }

    #[tokio::test]
    async fn import_conflict_strategies() {
        // 跳过：保留现有条目。
        let dir = TestDir::new("strategy-skip");
        let service = make_service(&dir);
        service
            .add(
                "PostgreSQL".to_string(),
                vec!["PG".to_string()],
                "旧备注".to_string(),
                true,
            )
            .await
            .unwrap();
        let file = r#"{"version":1,"kind":"proper_nouns","items":[{"term":"postgresql","aliases":["Postgres"],"note":"新备注","enabled":false}]}"#;
        let report = service
            .apply_import(file, ConflictStrategy::Skip)
            .await
            .unwrap();
        assert_eq!((report.updated, report.skipped), (0, 1));
        let nouns = service.list().await;
        assert_eq!(nouns[0].aliases, vec!["PG"], "跳过不修改现有");

        // 覆盖：整体采用导入内容，保留现有 ID。
        let dir2 = TestDir::new("strategy-overwrite");
        let service2 = make_service(&dir2);
        let noun = service2
            .add(
                "PostgreSQL".to_string(),
                vec!["PG".to_string()],
                "旧备注".to_string(),
                true,
            )
            .await
            .unwrap();
        let report2 = service2
            .apply_import(file, ConflictStrategy::Overwrite)
            .await
            .unwrap();
        assert_eq!((report2.updated, report2.skipped), (1, 0));
        let nouns2 = service2.list().await;
        assert_eq!(nouns2[0].id, noun.id, "覆盖保留现有 ID");
        assert_eq!(nouns2[0].term, "postgresql".to_string().trim());
        assert_eq!(nouns2[0].aliases, vec!["Postgres"]);
        assert_eq!(nouns2[0].note, "新备注");
        assert!(!nouns2[0].enabled);

        // 合并：别名取并集、备注现有非空保留、启用状态保留现有。
        let dir3 = TestDir::new("strategy-merge");
        let service3 = make_service(&dir3);
        service3
            .add(
                "PostgreSQL".to_string(),
                vec!["PG".to_string(), "Postgres".to_string()],
                "旧备注".to_string(),
                true,
            )
            .await
            .unwrap();
        let report3 = service3
            .apply_import(file, ConflictStrategy::Merge)
            .await
            .unwrap();
        assert_eq!((report3.updated, report3.failed), (1, 0));
        let nouns3 = service3.list().await;
        assert_eq!(nouns3[0].term, "PostgreSQL", "合并保留现有标准写法");
        // 并集且去重（Postgres 与现有重复，大小写不敏感）。
        assert_eq!(nouns3[0].aliases, vec!["PG", "Postgres"]);
        assert_eq!(nouns3[0].note, "旧备注", "备注现有非空保留");
        assert!(nouns3[0].enabled);
    }

    #[tokio::test]
    async fn merge_fills_empty_note_and_rejects_alias_overflow() {
        // 备注为空 → 合并补全。
        let dir = TestDir::new("merge-note");
        let service = make_service(&dir);
        service
            .add(
                "Redis".to_string(),
                vec!["瑞迪斯".to_string()],
                String::new(),
                true,
            )
            .await
            .unwrap();
        let file = r#"{"version":1,"kind":"proper_nouns","items":[{"term":"Redis","aliases":[],"note":"缓存","enabled":true}]}"#;
        let report = service
            .apply_import(file, ConflictStrategy::Merge)
            .await
            .unwrap();
        assert_eq!(report.updated, 1);
        assert_eq!(service.list().await[0].note, "缓存");

        // 别名并集超上限（20）→ 该条失败，现有条目不变（不静默截断）。
        let dir2 = TestDir::new("merge-overflow");
        let service2 = make_service(&dir2);
        let aliases: Vec<String> = (0..MAX_ALIASES).map(|i| format!("别名{i}")).collect();
        service2
            .add("Redis".to_string(), aliases.clone(), String::new(), true)
            .await
            .unwrap();
        let file2 = r#"{"version":1,"kind":"proper_nouns","items":[{"term":"Redis","aliases":["新别名"],"enabled":true}]}"#;
        let report2 = service2
            .apply_import(file2, ConflictStrategy::Merge)
            .await
            .unwrap();
        assert_eq!((report2.updated, report2.failed), (0, 1));
        assert!(report2.issues[0].reason.contains("别名"));
        assert_eq!(service2.list().await[0].aliases, aliases, "失败不修改现有");
    }

    #[tokio::test]
    async fn nouns_import_abnormal_files_rejected_safely() {
        let dir = TestDir::new("abnormal");
        let service = make_service(&dir);
        service
            .add("PostgreSQL".to_string(), vec![], String::new(), true)
            .await
            .unwrap();
        let file_path = dir.0.join("dict").join("proper-nouns.json");
        let before = std::fs::read_to_string(&file_path).unwrap();

        // 文件级异常：整体拒绝。
        let file_level: Vec<(&str, &str)> = vec![
            ("畸形 JSON", "{ broken"),
            ("空文件", "\n\n"),
            ("顶层为数字", "7"),
            ("缺 version", r#"{"kind":"proper_nouns","items":[]}"#),
            (
                "版本不兼容",
                r#"{"version":9,"kind":"proper_nouns","items":[]}"#,
            ),
            (
                "kind 不匹配（替换词典文件）",
                r#"{"version":1,"kind":"replace","items":[]}"#,
            ),
            ("未知 kind", r#"{"version":1,"kind":"others","items":[]}"#),
            ("缺 items", r#"{"version":1,"kind":"proper_nouns"}"#),
            (
                "items 非数组",
                r#"{"version":1,"kind":"proper_nouns","items":1}"#,
            ),
            ("旧版数组格式", r#"[{"term":"Redis"}]"#),
            ("纯文本", "Redis\t瑞迪斯"),
        ];
        for (name, content) in &file_level {
            let err = service.inspect_import(content).await.unwrap_err();
            assert_eq!(err.code(), "E-CONFIG-003", "{name} 应整体拒绝");
        }

        // 条目数超上限（2,001 条）：整体拒绝。
        let items: Vec<String> = (0..=MAX_NOUNS)
            .map(|i| format!(r#"{{"term":"词{i}"}}"#))
            .collect();
        let too_many = format!(
            r#"{{"version":1,"kind":"proper_nouns","items":[{}]}}"#,
            items.join(",")
        );
        assert_eq!(
            service.inspect_import(&too_many).await.unwrap_err().code(),
            "E-CONFIG-003"
        );

        // 超大文件（> 16 MiB）：服务层防御性拒绝。
        let oversize = format!(
            r#"{{"version":1,"kind":"proper_nouns","items":["{}"]}}"#,
            "x".repeat(MAX_IMPORT_BYTES)
        );
        assert_eq!(
            service.inspect_import(&oversize).await.unwrap_err().code(),
            "E-CONFIG-003"
        );

        // 极深嵌套：安全拒绝，不栈溢出。
        let deep = format!("{}1{}", "[".repeat(600), "]".repeat(600));
        assert!(service.inspect_import(&deep).await.is_err());

        // 单条异常：计入失败条数，不导入。
        let item_level: [(&str, usize, &str); 5] = [
            (
                r#"{"version":1,"kind":"proper_nouns","items":[{}]}"#,
                1,
                "缺少 term 字段",
            ),
            (
                r#"{"version":1,"kind":"proper_nouns","items":[{"term":3}]}"#,
                1,
                "term 字段类型非法",
            ),
            (
                r#"{"version":1,"kind":"proper_nouns","items":[{"term":"R","aliases":"PG"}]}"#,
                1,
                "aliases 字段类型非法",
            ),
            (
                r#"{"version":1,"kind":"proper_nouns","items":[{"term":"R","aliases":["a",7]}]}"#,
                1,
                "aliases 元素类型非法",
            ),
            (
                r#"{"version":1,"kind":"proper_nouns","items":[{"term":"  "}]}"#,
                1,
                "标准写法为空",
            ),
        ];
        for (content, expected_failed, hint) in item_level {
            let report = service
                .apply_import(content, ConflictStrategy::Skip)
                .await
                .unwrap();
            assert_eq!(report.failed, expected_failed, "{hint}");
            assert_eq!(report.imported, 0, "{hint}");
            assert!(!report.issues.is_empty(), "{hint}");
        }

        // 全程无写入：磁盘文件保持原样（失败条目与恶意文件均不落盘）。
        let nouns = service.list().await;
        assert_eq!(nouns.len(), 1);
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), before);
    }

    #[tokio::test]
    async fn nouns_import_over_capacity_rejects_without_partial_write() {
        let dir = TestDir::new("cap");
        // 直接构造接近上限的磁盘文件（避免 2,000 次逐条写入），再加载服务。
        let nouns: Vec<String> = (0..MAX_NOUNS - 1)
            .map(|i| {
                format!(r#"{{"id":{i},"term":"词{i}","aliases":[],"note":"","enabled":true}}"#)
            })
            .collect();
        let file_content = format!(r#"{{"version":1,"nouns":[{}]}}"#, nouns.join(","));
        let path = dir.0.join("dict").join("proper-nouns.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &file_content).unwrap();
        let service = make_service(&dir);
        assert_eq!(service.list().await.len(), MAX_NOUNS - 1);

        // 2 条新增 → 2,001 > 2,000：整体拒绝，不留半导入状态。
        let file =
            r#"{"version":1,"kind":"proper_nouns","items":[{"term":"新一"},{"term":"新二"}]}"#;
        let err = service
            .apply_import(file, ConflictStrategy::Skip)
            .await
            .unwrap_err();
        assert_eq!(err.code(), format!("E-CONFIG-{ERR_NOUN_LIMIT:03}"));
        assert_eq!(service.list().await.len(), MAX_NOUNS - 1);

        // 冲突跳过不占新增容量；恰好补足到 2,000 条允许（导入 1 条新增）。
        let file2 = r#"{"version":1,"kind":"proper_nouns","items":[{"term":"词0","aliases":["首词"]},{"term":"新三"}]}"#;
        let report = service
            .apply_import(file2, ConflictStrategy::Skip)
            .await
            .unwrap();
        assert_eq!((report.imported, report.skipped), (1, 1));
        assert_eq!(service.list().await.len(), MAX_NOUNS);
    }
}
