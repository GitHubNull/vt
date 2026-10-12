//! DictService（T-011，F-DICT-01、PRD 3.4.1）：替换词典规则管理 + 词典聚合入口
//! （T-522 专有名词、T-521 技术词库子服务随本服务加载，见 `proper_nouns()` /
//! `tech_lexicon()`）。
//!
//! - 规则持久化：数据目录 `dict/replace.json`（复用 ConfigStore 原子写入工具，
//!   NFR-R-05：tmp → fsync → rename，任何时刻无半写状态）；
//! - 规则约束（PRD 3.4.1 业务规则）：上限 200 条；错误词非空、不重复
//!   （英文不区分大小写去重）；正确词最长 2,000 字，空正确词表示删除该词；
//! - 损坏回退（NFR-S-07 雏形）：文件解析失败回退空词典并广播提示事件，不崩溃；
//! - 变更广播 `dict:changed`（多窗口同步，tech/08 8.2）；
//! - 隐私（NFR-S-02）：日志只记录条数与操作结果，绝不记录规则内容。
//!
//! 替换引擎（长词优先、完整匹配、英文大小写不敏感）见 `ReplaceStage`
//! （`services/post_process.rs`），规则引擎单测逐条覆盖 PRD 3.4 验收用例。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};
use tracing::{info, warn};

use crate::error::AppError;
use crate::events::AppEvent;
use crate::services::config_store::write_json_atomic;
use crate::services::lexicon::TechLexiconService;
use crate::services::proper_nouns::ProperNounService;

/// 词典文件相对数据目录路径（`dict/replace.json`）。
pub const DICT_REL_PATH: &str = "dict/replace.json";
/// 规则条数上限（PRD 3.4.1 业务规则 1，与原型一致）。
pub const MAX_RULES: usize = 200;
/// 正确词最大长度（字符数，PRD 3.4.1 业务规则 4）。
pub const MAX_RIGHT_LEN: usize = 2000;

/// 错误编号：规则数量已达上限。
pub const ERR_RULE_LIMIT: u32 = 1;
/// 错误编号：错误词为空或与已有规则重复。
pub const ERR_BAD_RULE: u32 = 2;

// ---- T-523 统一导入导出（SPEC: doc/specs/v0.5/T-523-dict-import-export.md） ----

/// 统一导入导出文件格式版本；v0.1 的纯 `ReplaceRule` 数组作为「旧版格式」
/// 导入时自动识别（见 `decode_replace_import`）。
pub const EXPORT_FORMAT_VERSION: u32 = 1;
/// 导入文件字节上限（16 MiB）：拒绝超大文件（NFR-S-07；2,000 条专有名词满配上限约 14 MiB）。
pub const MAX_IMPORT_BYTES: usize = 16 * 1024 * 1024;
/// 错误编号 E-CONFIG-003：词典导入导出失败（导入文件格式非法 / 读取失败 /
/// 超限，或导出写入失败）。
pub const ERR_IMPORT_EXPORT: u32 = 3;

/// 词典类型（统一格式 `kind` 字段）。
///
/// 技术词库（T-521）为内置只读数据、无用户条目，当前不参与导入导出；
/// 未来若需要导出用户自定义术语，在此新增变体并同步升级格式版本。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DictKind {
    /// 替换词典（F-DICT-01）。
    Replace,
    /// 专有名词（F-DICT-02）。
    ProperNouns,
}

impl DictKind {
    /// 统一格式 `kind` 字段字符串（前后端与文件格式共用的稳定契约）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::ProperNouns => "proper_nouns",
        }
    }

    /// 中文名称（错误提示用）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Replace => "替换词典",
            Self::ProperNouns => "专有名词",
        }
    }
}

/// 导入冲突策略（导入向导三选一，SPEC T-523）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictStrategy {
    /// 跳过：保留现有条目，冲突条目计入「跳过」。
    #[default]
    Skip,
    /// 覆盖：以导入条目整体覆盖现有条目（保留现有 ID），计入「更新」。
    Overwrite,
    /// 合并：以现有条目为准合并（补全缺失部分，保留现有设置），计入「更新」。
    ///
    /// 各词典合并语义：替换词典——现有正确词为空（删除语义）时采用导入正确词；
    /// 专有名词——别名取并集、备注空则补全；两者均保留现有启用状态。
    Merge,
}

/// 识别出的导入文件格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    /// 统一 v2 格式：`{ version, kind, exported_at, items }`。
    Unified,
    /// v0.1 旧版格式：纯 `ReplaceRule` 数组（仅替换词典支持）。
    LegacyArray,
    /// 制表符分隔纯文本：每行「错误词\t正确词」（仅替换词典支持）。
    Tsv,
}

impl ImportFormat {
    /// 对外展示用短标识（`v2` / `legacy` / `tsv`）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unified => "v2",
            Self::LegacyArray => "legacy",
            Self::Tsv => "tsv",
        }
    }
}

/// 导入前校验结果（向导「校验结果展示」步骤数据源；只读，不写入任何数据）。
#[derive(Debug, Clone, Serialize)]
pub struct ImportInspection {
    /// 识别出的文件格式："v2" / "legacy" / "tsv"。
    pub format: String,
    /// 文件格式版本（旧版数组 / 纯文本为 0）。
    pub version: u32,
    /// 条目总数（文件中的原始条目数）。
    pub total: usize,
    /// 无效条目数（导入时将计入失败并跳过）。
    pub invalid: usize,
    /// 与现有条目冲突的条数（含文件内重复条目；英文不区分大小写）。
    pub conflicts: usize,
}

/// 单条无效条目的说明（序号从 1 起 + 原因；不携带条目原文，NFR-S-02）。
#[derive(Debug, Clone, Serialize)]
pub struct ImportIssue {
    /// 条目序号（1 起）。
    pub index: usize,
    /// 原因描述。
    pub reason: String,
}

/// 导入报告（SPEC 验收标准：成功 / 更新 / 跳过 / 失败条数准确）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportReport {
    /// 新增条数。
    pub imported: usize,
    /// 覆盖 / 合并更新条数。
    pub updated: usize,
    /// 按策略跳过的冲突条数（含文件内重复条目）。
    pub skipped: usize,
    /// 无效条目数（校验不通过，未导入）。
    pub failed: usize,
    /// 无效条目说明（序号 + 原因）。
    pub issues: Vec<ImportIssue>,
}

/// 统一导出文件信封（v2）：`{ version, kind, exported_at, items }`。
#[derive(Debug, Clone, Serialize)]
pub struct UnifiedExportFile<T> {
    /// 格式版本（`EXPORT_FORMAT_VERSION`）。
    pub version: u32,
    /// 词典类型。
    pub kind: DictKind,
    /// 导出时间（UTC RFC 3339，秒精度；信息字段，导入时不参与校验）。
    pub exported_at: String,
    /// 条目数组。
    pub items: Vec<T>,
}

impl<T> UnifiedExportFile<T> {
    /// 以当前版本与时间构造导出信封。
    pub fn new(kind: DictKind, items: Vec<T>) -> Self {
        Self {
            version: EXPORT_FORMAT_VERSION,
            kind,
            exported_at: now_rfc3339(),
            items,
        }
    }
}

/// 替换词典导出条目（不含内部 ID；导入时重新分配 ID）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplaceExportItem {
    /// 错误词。
    pub wrong: String,
    /// 正确词（空 = 删除该词）。
    pub right: String,
    /// 启用开关。
    pub enabled: bool,
}

/// 已解析 / 解码的导入内容。
pub(crate) struct DecodedImport<T> {
    /// 识别出的文件格式。
    pub format: ImportFormat,
    /// 文件格式版本（旧版数组 / 纯文本为 0）。
    pub version: u32,
    /// 条目总数（含无效条目）。
    pub total: usize,
    /// 逐条解码结果；`Err` 为不可导入的原因（进入报告「失败条数」）。
    pub items: Vec<Result<T, String>>,
}

/// 替换规则（F-DICT-01）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplaceRule {
    /// 规则 ID（服务内生成，自增序号，持久化保持稳定）。
    pub id: u64,
    /// 错误词（识别结果中期望被替换的文本）。
    pub wrong: String,
    /// 正确词（替换为）；空字符串表示删除该词（PRD 3.4.1 业务规则 3）。
    pub right: String,
    /// 启用开关（停用后该规则不参与替换）。
    pub enabled: bool,
}

/// 磁盘文件形态：版本号 + 规则数组（未来导入导出格式演进预留）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReplaceDictFile {
    version: u32,
    rules: Vec<ReplaceRule>,
}

/// DictService：词典聚合入口——替换规则内存镜像 + 原子持久化 + 变更广播，
/// 并持有专有名词（T-522）与技术词库（T-521）子服务句柄。
#[derive(Clone)]
pub struct DictService {
    path: Arc<PathBuf>,
    rules: Arc<RwLock<Vec<ReplaceRule>>>,
    next_id: Arc<RwLock<u64>>,
    events: Arc<broadcast::Sender<AppEvent>>,
    /// 词典文件损坏回退标记（加载后由调用方读取并提示）。
    recovered: Arc<RwLock<bool>>,
    /// 专有名词子服务（T-522，F-DICT-02）：`dict/proper-nouns.json`。
    proper_nouns: ProperNounService,
    /// 技术词库子服务（T-521，F-CODE-06）：内置术语包（编译期内嵌）。
    tech_lexicon: TechLexiconService,
}

impl DictService {
    /// 加载词典：文件不存在 → 空词典；解析失败 → 回退空词典并标记
    /// `recovered`（调用方广播提示事件，NFR-S-07 雏形：不崩溃、可继续用）。
    pub fn load(data_dir: &Path, events: broadcast::Sender<AppEvent>) -> Result<Self, AppError> {
        let path = data_dir.join(DICT_REL_PATH);
        let (rules, recovered) = match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<ReplaceDictFile>(&content) {
                Ok(file) => {
                    info!(count = file.rules.len(), "替换词典加载成功");
                    (file.rules, false)
                }
                Err(err) => {
                    // NFR-S-02：日志只记录解析错误，不记录词典内容。
                    warn!(error = %err, "替换词典解析失败，回退空词典");
                    (Vec::new(), true)
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                info!("替换词典不存在，使用空词典");
                (Vec::new(), false)
            }
            Err(err) => {
                return Err(AppError::Config(1, format!("读取替换词典失败: {err}")));
            }
        };
        let next_id = rules.iter().map(|rule| rule.id).max().unwrap_or(0) + 1;
        // T-522 专有名词（损坏自动回退空词典并广播提示，NFR-S-07）
        // 与 T-521 技术词库（内置数据，损坏栈跳过并广播提示）随词典聚合加载。
        let proper_nouns = ProperNounService::load(data_dir, events.clone());
        let tech_lexicon = TechLexiconService::load(events.clone());
        Ok(Self {
            path: Arc::new(path),
            rules: Arc::new(RwLock::new(rules)),
            next_id: Arc::new(RwLock::new(next_id)),
            events: Arc::new(events),
            recovered: Arc::new(RwLock::new(recovered)),
            proper_nouns,
            tech_lexicon,
        })
    }

    /// 专有名词子服务句柄（T-522；命令层与管线 Stage 共用）。
    pub fn proper_nouns(&self) -> &ProperNounService {
        &self.proper_nouns
    }

    /// 技术词库子服务句柄（T-521；命令层与管线 Stage 共用）。
    pub fn tech_lexicon(&self) -> &TechLexiconService {
        &self.tech_lexicon
    }

    /// 词典文件是否因损坏回退过空词典（启动加载后检查一次并提示）。
    pub async fn take_recovered(&self) -> bool {
        let mut recovered = self.recovered.write().await;
        std::mem::take(&mut *recovered)
    }

    /// 规则清单（管理页数据源；引擎快照同用）。
    pub async fn list(&self) -> Vec<ReplaceRule> {
        self.rules.read().await.clone()
    }

    /// 启用规则快照（替换引擎用；按错误词长度降序，长词优先）。
    pub async fn enabled_snapshot(&self) -> Vec<ReplaceRule> {
        let mut rules: Vec<ReplaceRule> = self
            .rules
            .read()
            .await
            .iter()
            .filter(|rule| rule.enabled && !rule.wrong.is_empty())
            .cloned()
            .collect();
        rules.sort_by_key(|rule| std::cmp::Reverse(rule.wrong.chars().count()));
        rules
    }

    /// 新增规则（PRD 3.4.1 业务规则 1/3 校验）。
    pub async fn add_rule(
        &self,
        wrong: String,
        right: String,
        enabled: bool,
    ) -> Result<ReplaceRule, AppError> {
        let mut rules = self.rules.write().await;
        validate_new(&rules, &wrong, &right)?;
        if rules.len() >= MAX_RULES {
            return Err(AppError::Config(
                ERR_RULE_LIMIT,
                format!("替换词典最多 {MAX_RULES} 条，已达上限"),
            ));
        }
        let mut next_id = self.next_id.write().await;
        let rule = ReplaceRule {
            id: *next_id,
            wrong,
            right,
            enabled,
        };
        *next_id += 1;
        rules.push(rule.clone());
        drop(rules);
        self.persist_and_notify().await?;
        Ok(rule)
    }

    /// 更新规则（错误词 / 正确词 / 启停）。
    pub async fn update_rule(&self, rule: ReplaceRule) -> Result<(), AppError> {
        let mut rules = self.rules.write().await;
        let Some(index) = rules.iter().position(|item| item.id == rule.id) else {
            return Err(AppError::Config(ERR_BAD_RULE, "规则不存在".to_string()));
        };
        // 排除自身后校验错误词唯一性。
        let others: Vec<&ReplaceRule> = rules.iter().filter(|item| item.id != rule.id).collect();
        validate_new(&others, &rule.wrong, &rule.right)?;
        rules[index] = rule;
        drop(rules);
        self.persist_and_notify().await
    }

    /// 删除规则。
    pub async fn delete_rule(&self, id: u64) -> Result<(), AppError> {
        let mut rules = self.rules.write().await;
        let before = rules.len();
        rules.retain(|rule| rule.id != id);
        if rules.len() == before {
            return Err(AppError::Config(ERR_BAD_RULE, "规则不存在".to_string()));
        }
        drop(rules);
        self.persist_and_notify().await
    }

    /// 启停切换。
    pub async fn toggle_rule(&self, id: u64, enabled: bool) -> Result<(), AppError> {
        let mut rules = self.rules.write().await;
        let Some(existing) = rules.iter_mut().find(|rule| rule.id == id) else {
            return Err(AppError::Config(ERR_BAD_RULE, "规则不存在".to_string()));
        };
        existing.enabled = enabled;
        drop(rules);
        self.persist_and_notify().await
    }

    /// 导出全部规则（JSON 数组字符串；沿用 `ReplaceRule` 现有 serde 字段，
    /// 与导入对称，便于备份 / 迁移）。
    ///
    /// 隐私（NFR-S-02）：导出内容由前端交由用户处置，服务层不落日志。
    pub async fn export_rules(&self) -> Result<String, AppError> {
        let rules = self.rules.read().await.clone();
        serde_json::to_string(&rules)
            .map_err(|err| AppError::Config(ERR_BAD_RULE, format!("导出序列化失败: {err}")))
    }

    /// 导入规则：解析校验（错误词非空、长度合法、正确词不超长）、
    /// 与已有规则及本批内去重（英文不区分大小写）、分配新 ID 入库、
    /// 持久化并广播 `dict:changed`；返回实际导入条数。
    pub async fn import_rules(&self, json: &str) -> Result<usize, AppError> {
        /// 导入输入形态：兼容导出的 ReplaceRule 字段；id 由服务重新分配，忽略。
        #[derive(Debug, Deserialize)]
        struct ImportRule {
            wrong: String,
            right: String,
            #[serde(default)]
            enabled: Option<bool>,
        }
        let items: Vec<ImportRule> = serde_json::from_str(json).map_err(|err| {
            AppError::Config(
                ERR_BAD_RULE,
                format!("导入内容不是合法的规则 JSON 数组: {err}"),
            )
        })?;
        let mut rules = self.rules.write().await;
        let mut pending: Vec<ReplaceRule> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let seq = index + 1;
            let wrong = item.wrong.trim();
            if wrong.is_empty() {
                return Err(AppError::Config(
                    ERR_BAD_RULE,
                    format!("第 {seq} 条规则错误词为空"),
                ));
            }
            if wrong.chars().count() > 100 {
                return Err(AppError::Config(
                    ERR_BAD_RULE,
                    format!("第 {seq} 条规则错误词过长（最多 100 字）"),
                ));
            }
            if item.right.chars().count() > MAX_RIGHT_LEN {
                return Err(AppError::Config(
                    ERR_BAD_RULE,
                    format!("第 {seq} 条规则正确词过长（最多 {MAX_RIGHT_LEN} 字）"),
                ));
            }
            // 去重：与已有规则及本批已收规则比较（英文不区分大小写）。
            let normalized = normalize_wrong(wrong);
            let duplicated = rules
                .iter()
                .chain(pending.iter())
                .any(|rule| normalize_wrong(&rule.wrong) == normalized);
            if duplicated {
                continue;
            }
            pending.push(ReplaceRule {
                id: 0, // 入库前统一分配
                wrong: wrong.to_string(),
                right: item.right.clone(),
                enabled: item.enabled.unwrap_or(true),
            });
        }
        if rules.len() + pending.len() > MAX_RULES {
            return Err(AppError::Config(
                ERR_RULE_LIMIT,
                format!(
                    "导入后将达到 {} 条，超过替换词典上限 {MAX_RULES} 条",
                    rules.len() + pending.len()
                ),
            ));
        }
        let imported = pending.len();
        let mut next_id = self.next_id.write().await;
        for mut rule in pending {
            rule.id = *next_id;
            *next_id += 1;
            rules.push(rule);
        }
        drop(rules);
        if imported > 0 {
            self.persist_and_notify().await?;
        }
        Ok(imported)
    }

    /// 导出统一格式（v2）JSON 字符串（T-523；替换词典）。
    ///
    /// 不含内部 ID（导入端重新分配）；含停用条目（往返一致）。
    /// 隐私（NFR-S-02）：词典内容只写入用户选定的目标文件，不落日志。
    pub async fn export_unified(&self) -> Result<String, AppError> {
        let rules = self.rules.read().await.clone();
        let items: Vec<ReplaceExportItem> = rules
            .iter()
            .map(|rule| ReplaceExportItem {
                wrong: rule.wrong.clone(),
                right: rule.right.clone(),
                enabled: rule.enabled,
            })
            .collect();
        let file = UnifiedExportFile::new(DictKind::Replace, items);
        serde_json::to_string_pretty(&file)
            .map_err(|err| AppError::Config(ERR_IMPORT_EXPORT, format!("导出序列化失败: {err}")))
    }

    /// 导入前校验（向导「校验结果展示」步骤；只读，不修改任何数据）。
    ///
    /// 返回条目总数、无效条目数（将被跳过并计入失败）与冲突条数
    /// （与现有词典重复或文件内重复；英文不区分大小写）。
    pub async fn inspect_import(&self, content: &str) -> Result<ImportInspection, AppError> {
        let decoded = decode_replace_import(content)?;
        let existing = self.rules.read().await;
        let mut invalid = 0usize;
        let mut conflicts = 0usize;
        let mut seen: HashSet<String> = HashSet::new();
        for item in &decoded.items {
            let Ok(item) = item else {
                invalid += 1;
                continue;
            };
            if validate_replace_item(item).is_err() {
                invalid += 1;
                continue;
            }
            let key = normalize_wrong(item.wrong.trim());
            let in_existing = existing
                .iter()
                .any(|rule| normalize_wrong(&rule.wrong) == key);
            let in_batch = !seen.insert(key);
            if in_existing || in_batch {
                conflicts += 1;
            }
        }
        Ok(ImportInspection {
            format: decoded.format.as_str().to_string(),
            version: decoded.version,
            total: decoded.total,
            invalid,
            conflicts,
        })
    }

    /// 执行导入：冲突策略 + 上限检查 + 原子写入 + 导入报告（T-523）。
    ///
    /// - 文件级校验失败（见 `decode_replace_import`）→ 整体拒绝，不产生任何写入；
    /// - 单条无效 → 计入失败条数并跳过，其余条目继续；
    /// - 冲突按策略处理：跳过 / 覆盖（导入为准）/ 合并（现有为准，仅补全空正确词）；
    /// - 导入后超过 200 条 → 整体拒绝（不留半导入状态）；
    /// - 成功写入后持久化并广播 `dict:changed`（多窗口同步）。
    ///
    /// 隐私（NFR-S-02）：日志只记录条数统计，绝不记录规则内容。
    pub async fn apply_import(
        &self,
        content: &str,
        strategy: ConflictStrategy,
    ) -> Result<ImportReport, AppError> {
        let decoded = decode_replace_import(content)?;
        let mut rules = self.rules.write().await;
        let mut report = ImportReport::default();
        // 新增条目（提交时分配 ID）与更新操作（现有索引 → 合并后的条目）。
        let mut pending: Vec<ReplaceRule> = Vec::new();
        let mut updates: Vec<(usize, ReplaceRule)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for (index, result) in decoded.items.iter().enumerate() {
            let seq = index + 1;
            let item = match result {
                Ok(item) => {
                    if let Err(reason) = validate_replace_item(item) {
                        report.failed += 1;
                        report.issues.push(ImportIssue { index: seq, reason });
                        continue;
                    }
                    item
                }
                Err(reason) => {
                    report.failed += 1;
                    report.issues.push(ImportIssue {
                        index: seq,
                        reason: reason.clone(),
                    });
                    continue;
                }
            };
            let key = normalize_wrong(item.wrong.trim());
            if !seen.insert(key.clone()) {
                // 文件内重复：保留先出现的一条（与既有导入行为一致）。
                report.skipped += 1;
                continue;
            }
            let Some(existing_index) = rules
                .iter()
                .position(|rule| normalize_wrong(&rule.wrong) == key)
            else {
                pending.push(ReplaceRule {
                    id: 0,
                    wrong: item.wrong.trim().to_string(),
                    right: item.right.clone(),
                    enabled: item.enabled,
                });
                continue;
            };
            match strategy {
                ConflictStrategy::Skip => report.skipped += 1,
                ConflictStrategy::Overwrite => {
                    updates.push((
                        existing_index,
                        ReplaceRule {
                            id: rules[existing_index].id,
                            wrong: item.wrong.trim().to_string(),
                            right: item.right.clone(),
                            enabled: item.enabled,
                        },
                    ));
                    report.updated += 1;
                }
                ConflictStrategy::Merge => {
                    // 合并语义：以现有条目为准，仅补全缺失——现有正确词为空
                    // （删除语义）时采用导入正确词；启用状态保留现有设置
                    // （导入不改变用户当前选择，结果可预期）。
                    let current = &rules[existing_index];
                    updates.push((
                        existing_index,
                        ReplaceRule {
                            id: current.id,
                            wrong: current.wrong.clone(),
                            right: if current.right.is_empty() {
                                item.right.clone()
                            } else {
                                current.right.clone()
                            },
                            enabled: current.enabled,
                        },
                    ));
                    report.updated += 1;
                }
            }
        }
        // 上限检查：整体拒绝，不留半导入状态（与 v0.1 导入语义一致）。
        if rules.len() + pending.len() > MAX_RULES {
            return Err(AppError::Config(
                ERR_RULE_LIMIT,
                format!(
                    "导入后将达到 {} 条，超过替换词典上限 {MAX_RULES} 条",
                    rules.len() + pending.len()
                ),
            ));
        }
        for (index, rule) in updates {
            rules[index] = rule;
        }
        report.imported = pending.len();
        let mut next_id = self.next_id.write().await;
        for mut rule in pending {
            rule.id = *next_id;
            *next_id += 1;
            rules.push(rule);
        }
        drop(next_id);
        drop(rules);
        if report.imported > 0 || report.updated > 0 {
            self.persist_and_notify().await?;
        }
        // NFR-S-02：只记录条数统计，不记录任何规则内容。
        info!(
            imported = report.imported,
            updated = report.updated,
            skipped = report.skipped,
            failed = report.failed,
            "替换词典导入完成"
        );
        Ok(report)
    }

    /// 持久化 + 广播变更（先落盘再广播，多窗口以广播为准刷新）。
    async fn persist_and_notify(&self) -> Result<(), AppError> {
        let rules = self.rules.read().await.clone();
        let file = ReplaceDictFile { version: 1, rules };
        write_json_atomic(&self.path, &file)?;
        let _ = self.events.send(AppEvent::DictChanged);
        info!(count = file.rules.len(), "替换词典已保存");
        Ok(())
    }
}

/// 错误词规范化：英文不区分大小写（PRD 3.4.1 业务规则 2），去除首尾空白。
fn normalize_wrong(wrong: &str) -> String {
    wrong.trim().to_lowercase()
}

/// 新增 / 更新规则校验（PRD 3.4.1 业务规则 1/3/4）。
fn validate_new(
    existing: &[impl std::borrow::Borrow<ReplaceRule>],
    wrong: &str,
    right: &str,
) -> Result<(), AppError> {
    let wrong = wrong.trim();
    if wrong.is_empty() {
        return Err(AppError::Config(ERR_BAD_RULE, "错误词不能为空".to_string()));
    }
    if wrong.chars().count() > 100 {
        return Err(AppError::Config(
            ERR_BAD_RULE,
            "错误词过长（最多 100 字）".to_string(),
        ));
    }
    if right.chars().count() > MAX_RIGHT_LEN {
        return Err(AppError::Config(
            ERR_BAD_RULE,
            format!("正确词过长（最多 {MAX_RIGHT_LEN} 字）"),
        ));
    }
    let normalized = normalize_wrong(wrong);
    let duplicated = existing
        .iter()
        .any(|rule| normalize_wrong(&rule.borrow().wrong) == normalized);
    if duplicated {
        return Err(AppError::Config(
            ERR_BAD_RULE,
            "错误词已存在，请勿重复添加".to_string(),
        ));
    }
    Ok(())
}

// ---- T-523 导入导出：解析 / 解码 / 校验（纯函数，便于单测） ----

/// 当前 UTC 时间（RFC 3339，秒精度；不引入时间库依赖）。
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let (year, month, day) = civil_from_days((secs / 86_400) as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        (secs % 86_400) / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// 纪元天数 →（年, 月, 日）（Howard Hinnant 的 civil_from_days 算法）。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// 解析统一 v2 信封并做文件级校验（NFR-S-07；不修改任何数据）。
///
/// 文件级校验失败 → 整体拒绝（E-CONFIG-003）：畸形 JSON / 顶层非对象 /
/// `version` 缺失或非法 / 版本高于当前支持 / `kind` 缺失或非法 / 类型不匹配 /
/// `items` 缺失或非数组 / 条目数超上限。深层嵌套（超过 serde_json 默认
/// 128 层递归上限）同样以解析错误拒绝，不递归展开、不崩溃。
///
/// `exported_at` 为信息字段（缺省 / 类型异常不参与校验）。
pub(crate) fn parse_unified_envelope(
    content: &str,
    expect: DictKind,
    max_items: usize,
) -> Result<(u32, Vec<serde_json::Value>), AppError> {
    let value: serde_json::Value = serde_json::from_str(content).map_err(|err| {
        AppError::Config(ERR_IMPORT_EXPORT, format!("导入文件不是合法的 JSON：{err}"))
    })?;
    let serde_json::Value::Object(object) = value else {
        return Err(AppError::Config(
            ERR_IMPORT_EXPORT,
            "导入文件顶层必须是对象（{ version, kind, exported_at, items }）".to_string(),
        ));
    };
    let version = match object.get("version") {
        Some(serde_json::Value::Number(number)) => match number.as_u64() {
            Some(version) if version >= 1 => version,
            _ => {
                return Err(AppError::Config(
                    ERR_IMPORT_EXPORT,
                    "version 字段不合法（应为不小于 1 的整数）".to_string(),
                ));
            }
        },
        Some(_) => {
            return Err(AppError::Config(
                ERR_IMPORT_EXPORT,
                "version 字段类型非法（应为整数）".to_string(),
            ));
        }
        None => {
            return Err(AppError::Config(
                ERR_IMPORT_EXPORT,
                "缺少 version 字段".to_string(),
            ));
        }
    };
    if version > u64::from(EXPORT_FORMAT_VERSION) {
        return Err(AppError::Config(
            ERR_IMPORT_EXPORT,
            format!(
                "文件版本 {version} 高于当前支持的版本 {EXPORT_FORMAT_VERSION}，请升级应用后再导入"
            ),
        ));
    }
    let kind = match object.get("kind") {
        Some(serde_json::Value::String(kind)) => kind.as_str(),
        Some(_) => {
            return Err(AppError::Config(
                ERR_IMPORT_EXPORT,
                "kind 字段类型非法（应为字符串）".to_string(),
            ));
        }
        None => {
            return Err(AppError::Config(
                ERR_IMPORT_EXPORT,
                "缺少 kind 字段".to_string(),
            ));
        }
    };
    let file_kind = match kind {
        "replace" => DictKind::Replace,
        "proper_nouns" => DictKind::ProperNouns,
        other => {
            return Err(AppError::Config(
                ERR_IMPORT_EXPORT,
                format!("未知的词典类型「{other}」"),
            ));
        }
    };
    if file_kind != expect {
        return Err(AppError::Config(
            ERR_IMPORT_EXPORT,
            format!(
                "该文件是{}文件，请在「{}」页面导入",
                file_kind.label(),
                expect.label()
            ),
        ));
    }
    let items = match object.get("items") {
        Some(serde_json::Value::Array(items)) => items,
        Some(_) => {
            return Err(AppError::Config(
                ERR_IMPORT_EXPORT,
                "items 字段类型非法（应为数组）".to_string(),
            ));
        }
        None => {
            return Err(AppError::Config(
                ERR_IMPORT_EXPORT,
                "缺少 items 字段".to_string(),
            ));
        }
    };
    if items.len() > max_items {
        return Err(AppError::Config(
            ERR_IMPORT_EXPORT,
            format!("条目数量 {} 超过上限 {max_items} 条", items.len()),
        ));
    }
    Ok((version as u32, items.clone()))
}

/// 解码单条替换条目（v2 / 旧版数组共用）；
/// 结构问题（字段缺失 / 类型错 → 中文原因文本）不整体拒绝，计入「失败条数」。
fn decode_replace_item(value: &serde_json::Value) -> Result<ReplaceExportItem, String> {
    let Some(object) = value.as_object() else {
        return Err("条目必须是对象".to_string());
    };
    let wrong = match object.get("wrong") {
        Some(serde_json::Value::String(wrong)) => wrong.clone(),
        Some(_) => return Err("wrong 字段类型非法（应为字符串）".to_string()),
        None => return Err("缺少 wrong 字段".to_string()),
    };
    let right = match object.get("right") {
        Some(serde_json::Value::String(right)) => right.clone(),
        Some(_) => return Err("right 字段类型非法（应为字符串）".to_string()),
        None => return Err("缺少 right 字段".to_string()),
    };
    let enabled = match object.get("enabled") {
        Some(serde_json::Value::Bool(enabled)) => *enabled,
        Some(_) => return Err("enabled 字段类型非法（应为布尔值）".to_string()),
        // 缺省启用（与 v0.1 导入行为一致）。
        None => true,
    };
    Ok(ReplaceExportItem {
        wrong,
        right,
        enabled,
    })
}

/// 单条替换条目的内容校验（与手动添加同一口径，PRD 3.4.1 业务规则 3/4）。
fn validate_replace_item(item: &ReplaceExportItem) -> Result<(), String> {
    let wrong = item.wrong.trim();
    if wrong.is_empty() {
        return Err("错误词为空".to_string());
    }
    if wrong.chars().count() > 100 {
        return Err("错误词过长（最多 100 字）".to_string());
    }
    if item.right.chars().count() > MAX_RIGHT_LEN {
        return Err(format!("正确词过长（最多 {MAX_RIGHT_LEN} 字）"));
    }
    Ok(())
}

/// 解析制表符分隔纯文本（PRD 3.4.1 业务规则 5）：每行「错误词\t正确词」，
/// 正确词留空表示删除该词；空行忽略；行数超上限整体拒绝；
/// 缺少制表符的行与空错误词行计入「失败条数」。
fn parse_replace_tsv(content: &str) -> Result<DecodedImport<ReplaceExportItem>, AppError> {
    let mut items: Vec<Result<ReplaceExportItem, String>> = Vec::new();
    for raw in content.lines() {
        // `str::lines` 已剥离 \r\n 的 \r；此处兜底剥离行尾残余 \r。
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        match line.split_once('\t') {
            Some((wrong, right)) => items.push(Ok(ReplaceExportItem {
                wrong: wrong.trim().to_string(),
                right: right.trim().to_string(),
                enabled: true,
            })),
            None => items.push(Err(
                "缺少制表符分隔符（每行应为「错误词\t正确词」）".to_string()
            )),
        }
    }
    if items.len() > MAX_RULES {
        return Err(AppError::Config(
            ERR_IMPORT_EXPORT,
            format!("条目数量 {} 超过上限 {MAX_RULES} 条", items.len()),
        ));
    }
    Ok(DecodedImport {
        format: ImportFormat::Tsv,
        version: 0,
        total: items.len(),
        items,
    })
}

/// 解析并解码替换词典导入内容（统一 v2 / v0.1 旧版数组 / 纯文本三种格式自动识别）。
///
/// 文件级问题（超大 / 空文件 / 畸形 JSON / 信封字段非法 / 版本不兼容 /
/// 类型不匹配 / 条目数超上限）→ 整体拒绝（Err），不产生任何写入；
/// 单条问题 → 进入该条的 `Err`，由导入报告汇总为「失败条数」。
pub(crate) fn decode_replace_import(
    content: &str,
) -> Result<DecodedImport<ReplaceExportItem>, AppError> {
    // 防御性体积复查（命令层已按文件元数据先行拦截，见 commands/dict.rs）。
    if content.len() > MAX_IMPORT_BYTES {
        return Err(oversize_error());
    }
    let stripped = content.trim_start_matches('\u{feff}');
    let probe = stripped.trim_start();
    if probe.is_empty() {
        return Err(AppError::Config(
            ERR_IMPORT_EXPORT,
            "导入文件为空".to_string(),
        ));
    }
    if probe.starts_with('{') {
        let (version, raw_items) = parse_unified_envelope(probe, DictKind::Replace, MAX_RULES)?;
        let items = raw_items.iter().map(decode_replace_item).collect();
        return Ok(DecodedImport {
            format: ImportFormat::Unified,
            version,
            total: raw_items.len(),
            items,
        });
    }
    if probe.starts_with('[') {
        // v0.1 旧版格式：纯 ReplaceRule 数组。
        let raw_items: Vec<serde_json::Value> = serde_json::from_str(probe).map_err(|err| {
            AppError::Config(
                ERR_IMPORT_EXPORT,
                format!("旧版词典文件不是合法的 JSON 数组：{err}"),
            )
        })?;
        if raw_items.len() > MAX_RULES {
            return Err(AppError::Config(
                ERR_IMPORT_EXPORT,
                format!("条目数量 {} 超过上限 {MAX_RULES} 条", raw_items.len()),
            ));
        }
        let items = raw_items.iter().map(decode_replace_item).collect();
        return Ok(DecodedImport {
            format: ImportFormat::LegacyArray,
            version: 0,
            total: raw_items.len(),
            items,
        });
    }
    // 既非 JSON 对象 / 数组：仅当内容含制表符时按纯文本处理，否则视为
    // 无法识别的格式整体拒绝（防裸标量等“伪纯文本”混入）。
    if !stripped.contains('\t') {
        return Err(AppError::Config(
            ERR_IMPORT_EXPORT,
            "无法识别的文件格式：应为统一格式 JSON（{ version, kind, items }）\
             或每行「错误词\t正确词」的纯文本"
                .to_string(),
        ));
    }
    parse_replace_tsv(stripped)
}

/// 超大文件统一错误（服务层与命令层共用文案）。
pub(crate) fn oversize_error() -> AppError {
    AppError::Config(
        ERR_IMPORT_EXPORT,
        format!(
            "导入文件过大（超过 {} MiB 上限），已拒绝导入",
            MAX_IMPORT_BYTES / (1024 * 1024)
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "vt-dict-test-{}-{}-{:?}",
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

    fn make_service(dir: &TestDir) -> DictService {
        DictService::load(&dir.0, crate::events::channel().0).unwrap()
    }

    #[tokio::test]
    async fn add_and_persist_roundtrip() {
        let dir = TestDir::new("roundtrip");
        let service = make_service(&dir);
        let rule = service
            .add_rule("f二".to_string(), "F2".to_string(), true)
            .await
            .unwrap();
        assert_eq!(rule.id, 1);

        // 重启（重新 load）后规则保持。
        let service2 = make_service(&dir);
        let rules = service2.list().await;
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].wrong, "f二");
        assert_eq!(rules[0].right, "F2");
        assert!(rules[0].enabled);
        // ID 续接，不重复。
        let rule2 = service2
            .add_rule("我的邮箱".to_string(), "dev@example.com".to_string(), true)
            .await
            .unwrap();
        assert_eq!(rule2.id, 2);
    }

    #[tokio::test]
    async fn duplicate_wrong_rejected_case_insensitive() {
        let dir = TestDir::new("dup");
        let service = make_service(&dir);
        service
            .add_rule("F二".to_string(), "F2".to_string(), true)
            .await
            .unwrap();
        // 英文不区分大小写重复 → 拒绝。
        let err = service
            .add_rule("f二".to_string(), "Fx".to_string(), true)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "E-CONFIG-002");
        // 中文规则不受大小写归一影响之外的影响。
        let err = service
            .add_rule("F二".to_string(), "F3".to_string(), true)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "E-CONFIG-002");
    }

    #[tokio::test]
    async fn empty_wrong_and_empty_right_semantics() {
        let dir = TestDir::new("empty");
        let service = make_service(&dir);
        let err = service
            .add_rule("  ".to_string(), "x".to_string(), true)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "E-CONFIG-002");
        // 正确词为空 = 删除该词（合法）。
        let rule = service
            .add_rule("回车".to_string(), String::new(), true)
            .await
            .unwrap();
        assert_eq!(rule.right, "");
    }

    #[tokio::test]
    async fn rule_limit_enforced() {
        let dir = TestDir::new("limit");
        let service = make_service(&dir);
        for i in 0..MAX_RULES {
            service
                .add_rule(format!("词{i}"), format!("值{i}"), true)
                .await
                .unwrap();
        }
        let err = service
            .add_rule("超出".to_string(), "x".to_string(), true)
            .await
            .unwrap_err();
        assert_eq!(err.code(), format!("E-CONFIG-{ERR_RULE_LIMIT:03}"));
        assert_eq!(service.list().await.len(), MAX_RULES);
    }

    #[tokio::test]
    async fn update_delete_toggle() {
        let dir = TestDir::new("crud");
        let service = make_service(&dir);
        let rule = service
            .add_rule("a".to_string(), "b".to_string(), true)
            .await
            .unwrap();

        service.toggle_rule(rule.id, false).await.unwrap();
        assert!(!service.list().await[0].enabled);
        // 停用的规则不出现在引擎快照。
        assert!(service.enabled_snapshot().await.is_empty());

        service
            .update_rule(ReplaceRule {
                right: "c".to_string(),
                ..rule.clone()
            })
            .await
            .unwrap();
        assert_eq!(service.list().await[0].right, "c");

        service.delete_rule(rule.id).await.unwrap();
        assert!(service.list().await.is_empty());
        // 删除不存在 → 报错。
        assert!(service.delete_rule(rule.id).await.is_err());
    }

    #[tokio::test]
    async fn corrupted_dict_falls_back_to_empty() {
        let dir = TestDir::new("corrupt");
        let dict_dir = dir.0.join("dict");
        std::fs::create_dir_all(&dict_dir).unwrap();
        std::fs::write(dict_dir.join("replace.json"), "{ broken !!!").unwrap();
        let service = make_service(&dir);
        // 不崩溃：空词典 + recovered 标记。
        assert!(service.list().await.is_empty());
        assert!(service.take_recovered().await);
        assert!(!service.take_recovered().await, "标记只消费一次");
    }

    #[tokio::test]
    async fn long_right_rejected() {
        let dir = TestDir::new("long");
        let service = make_service(&dir);
        let err = service
            .add_rule("长".to_string(), "字".repeat(MAX_RIGHT_LEN + 1), true)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "E-CONFIG-002");
    }

    #[tokio::test]
    async fn change_event_broadcast() {
        let dir = TestDir::new("event");
        let (tx, mut rx) = crate::events::channel();
        let service = DictService::load(&dir.0, tx).unwrap();
        service
            .add_rule("x".to_string(), "y".to_string(), true)
            .await
            .unwrap();
        let event = rx.try_recv().expect("应广播 dict:changed");
        assert!(matches!(event, AppEvent::DictChanged));
    }

    #[tokio::test]
    async fn export_import_roundtrip() {
        let dir = TestDir::new("roundtrip-io");
        let service = make_service(&dir);
        service
            .add_rule("f二".to_string(), "F2".to_string(), true)
            .await
            .unwrap();
        service
            .add_rule("我的邮箱".to_string(), "dev@example.com".to_string(), false)
            .await
            .unwrap();

        // 导出 → 新服务导入（模拟备份 / 迁移）。
        let json = service.export_rules().await.unwrap();
        let dir2 = TestDir::new("roundtrip-io-2");
        let (tx, mut rx) = crate::events::channel();
        let service2 = DictService::load(&dir2.0, tx).unwrap();
        let imported = service2.import_rules(&json).await.unwrap();
        assert_eq!(imported, 2);
        assert!(matches!(rx.try_recv().unwrap(), AppEvent::DictChanged));
        let rules = service2.list().await;
        assert_eq!(rules.len(), 2);
        // 字段完整保留（含启停状态）。
        assert_eq!(rules[0].wrong, "f二");
        assert_eq!(rules[0].right, "F2");
        assert!(rules[0].enabled);
        assert!(!rules[1].enabled);
        // ID 重新分配且持久化续接。
        let rule = service2
            .add_rule("新词".to_string(), "新值".to_string(), true)
            .await
            .unwrap();
        assert!(rule.id > rules[1].id);
    }

    #[tokio::test]
    async fn import_dedupes_against_existing_and_batch() {
        let dir = TestDir::new("import-dedup");
        let service = make_service(&dir);
        service
            .add_rule("F二".to_string(), "F2".to_string(), true)
            .await
            .unwrap();
        let json = r#"[
            {"wrong": "f二", "right": "Fx", "enabled": true},
            {"wrong": "新词一", "right": "一", "enabled": true},
            {"wrong": "新词一", "right": "重复批内", "enabled": true},
            {"wrong": "新词二", "right": "二"}
        ]"#;
        let imported = service.import_rules(json).await.unwrap();
        // 已有（大小写归一重复）+ 批内重复被跳过，导入 2 条。
        assert_eq!(imported, 2);
        let rules = service.list().await;
        assert_eq!(rules.len(), 3);
        assert!(rules.iter().any(|r| r.wrong == "新词二" && r.enabled));
    }

    #[tokio::test]
    async fn import_invalid_json_and_empty_wrong() {
        let dir = TestDir::new("import-bad");
        let service = make_service(&dir);
        // 结构非法：非 JSON。
        let err = service.import_rules("not json").await.unwrap_err();
        assert_eq!(err.code(), "E-CONFIG-002");
        // 结构非法：JSON 但非数组。
        let err = service.import_rules("{}").await.unwrap_err();
        assert_eq!(err.code(), "E-CONFIG-002");
        // 错误词为空。
        let err = service
            .import_rules(r#"[{"wrong": "  ", "right": "x"}]"#)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "E-CONFIG-002");
        assert!(err.to_string().contains("错误词为空"));
        // 错误消息不含导入文本内容之外的泄漏（仅结构描述）。
        assert!(service.list().await.is_empty());
    }

    #[tokio::test]
    async fn import_respects_rule_limit() {
        let dir = TestDir::new("import-limit");
        let service = make_service(&dir);
        // 先填满到上限 - 2。
        for i in 0..MAX_RULES - 2 {
            service
                .add_rule(format!("词{i}"), format!("值{i}"), true)
                .await
                .unwrap();
        }
        let items: Vec<String> = (0..3)
            .map(|i| format!(r#"{{"wrong": "导入{i}", "right": "v"}}"#))
            .collect();
        let json = format!("[{}]", items.join(","));
        let err = service.import_rules(&json).await.unwrap_err();
        assert_eq!(err.code(), format!("E-CONFIG-{ERR_RULE_LIMIT:03}"));
        // 超限整体拒绝，不产生半导入状态。
        assert_eq!(service.list().await.len(), MAX_RULES - 2);
    }

    // ---- T-523 统一导入导出（v2 格式） ----

    fn exported_value(json: &str) -> serde_json::Value {
        serde_json::from_str(json).expect("导出内容应为合法 JSON")
    }

    #[tokio::test]
    async fn unified_export_shape_and_roundtrip() {
        let dir = TestDir::new("v2-export");
        let service = make_service(&dir);
        service
            .add_rule("f二".to_string(), "F2".to_string(), true)
            .await
            .unwrap();
        service
            .add_rule("回车".to_string(), String::new(), false)
            .await
            .unwrap();

        let json = service.export_unified().await.unwrap();
        let value = exported_value(&json);
        // 信封字段：version / kind / exported_at / items。
        assert_eq!(value["version"], 1);
        assert_eq!(value["kind"], "replace");
        let exported_at = value["exported_at"]
            .as_str()
            .expect("exported_at 应为字符串");
        assert_eq!(exported_at.len(), 20, "{exported_at}");
        assert!(exported_at.ends_with('Z') && exported_at.contains('T'));
        let items = value["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        // 不含内部 ID（导入端重新分配）。
        assert!(items[0].get("id").is_none());
        assert_eq!(items[1]["right"], "");

        // 往返一致：导出 → 新服务导入，字段完整保留（含删除语义与停用状态）。
        let dir2 = TestDir::new("v2-import");
        let (tx, mut rx) = crate::events::channel();
        let service2 = DictService::load(&dir2.0, tx).unwrap();
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
        assert!(matches!(rx.try_recv().unwrap(), AppEvent::DictChanged));
        let rules = service2.list().await;
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].wrong, "f二");
        assert_eq!(rules[0].right, "F2");
        assert!(rules[0].enabled);
        assert_eq!(rules[1].wrong, "回车");
        assert_eq!(rules[1].right, "");
        assert!(!rules[1].enabled);
    }

    #[tokio::test]
    async fn conflict_strategy_skip_keeps_existing() {
        let dir = TestDir::new("strategy-skip");
        let service = make_service(&dir);
        service
            .add_rule("f二".to_string(), "旧值".to_string(), true)
            .await
            .unwrap();
        // 冲突键大小写不敏感（F二 ≡ f二）；无 version 之外的字段也可缺省。
        let file = r#"{"version":1,"kind":"replace","items":[{"wrong":"F二","right":"新值","enabled":false}]}"#;
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
            (0, 0, 1, 0)
        );
        let rules = service.list().await;
        assert_eq!(rules[0].right, "旧值");
        assert!(rules[0].enabled);
    }

    #[tokio::test]
    async fn conflict_strategy_overwrite_replaces() {
        let dir = TestDir::new("strategy-overwrite");
        let service = make_service(&dir);
        let rule = service
            .add_rule("f二".to_string(), "旧值".to_string(), true)
            .await
            .unwrap();
        let file = r#"{"version":1,"kind":"replace","items":[{"wrong":"f二","right":"新值","enabled":false}]}"#;
        let report = service
            .apply_import(file, ConflictStrategy::Overwrite)
            .await
            .unwrap();
        assert_eq!(
            (
                report.imported,
                report.updated,
                report.skipped,
                report.failed
            ),
            (0, 1, 0, 0)
        );
        let rules = service.list().await;
        assert_eq!(rules[0].id, rule.id, "覆盖保留现有 ID");
        assert_eq!(rules[0].right, "新值");
        assert!(!rules[0].enabled, "覆盖采用导入的启用状态");
        // 持久化：重新加载后仍为新值。
        let service2 = make_service(&dir);
        assert_eq!(service2.list().await[0].right, "新值");
    }

    #[tokio::test]
    async fn conflict_strategy_merge_fills_missing_only() {
        let dir = TestDir::new("strategy-merge");
        let service = make_service(&dir);
        service
            .add_rule("f二".to_string(), "旧值".to_string(), true)
            .await
            .unwrap();
        let file = r#"{"version":1,"kind":"replace","items":[{"wrong":"f二","right":"新值","enabled":false},{"wrong":"回车","right":"","enabled":true}]}"#;
        let report = service
            .apply_import(file, ConflictStrategy::Merge)
            .await
            .unwrap();
        assert_eq!(
            (
                report.imported,
                report.updated,
                report.skipped,
                report.failed
            ),
            (1, 1, 0, 0)
        );
        let rules = service.list().await;
        let hit = rules.iter().find(|rule| rule.wrong == "f二").unwrap();
        assert_eq!(hit.right, "旧值", "现有正确词非空：保留现有");
        assert!(hit.enabled, "合并保留现有启用状态");

        // 现有正确词为空（删除语义）：补全导入正确词。
        let dir2 = TestDir::new("strategy-merge-fill");
        let service2 = make_service(&dir2);
        service2
            .add_rule("回车".to_string(), String::new(), true)
            .await
            .unwrap();
        let file2 = r#"{"version":1,"kind":"replace","items":[{"wrong":"回车","right":"Enter","enabled":true}]}"#;
        let report2 = service2
            .apply_import(file2, ConflictStrategy::Merge)
            .await
            .unwrap();
        assert_eq!(report2.updated, 1);
        assert_eq!(service2.list().await[0].right, "Enter");
    }

    #[tokio::test]
    async fn legacy_v01_array_import() {
        let dir = TestDir::new("legacy");
        let service = make_service(&dir);
        // v0.1 导出形态：含 id、enabled 可缺省。
        let file = r#"[{"id":7,"wrong":"f二","right":"F2","enabled":true},{"wrong":"我的邮箱","right":"dev@example.com"}]"#;
        let inspection = service.inspect_import(file).await.unwrap();
        assert_eq!(inspection.format, "legacy");
        assert_eq!(inspection.version, 0);
        assert_eq!(
            (inspection.total, inspection.invalid, inspection.conflicts),
            (2, 0, 0)
        );
        let report = service
            .apply_import(file, ConflictStrategy::Skip)
            .await
            .unwrap();
        assert_eq!((report.imported, report.failed), (2, 0));
        let rules = service.list().await;
        assert_eq!(rules[0].wrong, "f二");
        assert!(rules[0].enabled);
        assert!(rules[1].enabled, "缺省 enabled → 启用");
        assert_eq!(rules[0].id, 1, "ID 由服务重新分配");
    }

    #[tokio::test]
    async fn tsv_plain_text_import() {
        let dir = TestDir::new("tsv");
        let service = make_service(&dir);
        let file = "f二\tF2\n\n我的邮箱\tdev@example.com\n回车\t\nbad_line\r\n";
        let inspection = service.inspect_import(file).await.unwrap();
        assert_eq!(inspection.format, "tsv");
        assert_eq!(inspection.version, 0);
        assert_eq!((inspection.total, inspection.invalid), (4, 1));
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
            (3, 0, 0, 1)
        );
        assert_eq!(report.issues[0].index, 4);
        assert!(report.issues[0].reason.contains("制表符"));
        let rules = service.list().await;
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[2].right, "", "正确词留空 = 删除该词");
    }

    #[tokio::test]
    async fn tsv_empty_wrong_counts_failed() {
        let dir = TestDir::new("tsv-empty");
        let service = make_service(&dir);
        let report = service
            .apply_import("\tX\n有效\t值\n", ConflictStrategy::Skip)
            .await
            .unwrap();
        assert_eq!((report.imported, report.failed), (1, 1));
        assert_eq!(report.issues[0].index, 1);
        assert!(report.issues[0].reason.contains("错误词为空"));
    }

    #[tokio::test]
    async fn abnormal_file_set_rejected_safely() {
        let dir = TestDir::new("abnormal");
        let service = make_service(&dir);
        // 预置一条现有规则并记录磁盘内容（用于断言「无写入」）。
        service
            .add_rule("既有".to_string(), "值".to_string(), true)
            .await
            .unwrap();
        let dict_file = dir.0.join("dict").join("replace.json");
        let before = std::fs::read_to_string(&dict_file).unwrap();

        // 文件级异常（整体拒绝，E-CONFIG-003）。
        let file_level: Vec<(&str, &str)> = vec![
            ("畸形 JSON", "{ \"version\": 1, kind"),
            ("空文件", "   "),
            ("顶层为数字", "42"),
            ("顶层为字符串", "\"hello\""),
            ("缺 version", r#"{"kind":"replace","items":[]}"#),
            (
                "version 类型错",
                r#"{"version":"1","kind":"replace","items":[]}"#,
            ),
            (
                "version 非法（0）",
                r#"{"version":0,"kind":"replace","items":[]}"#,
            ),
            ("版本不兼容", r#"{"version":2,"kind":"replace","items":[]}"#),
            ("缺 kind", r#"{"version":1,"items":[]}"#),
            ("kind 未知", r#"{"version":1,"kind":"life","items":[]}"#),
            ("kind 类型错", r#"{"version":1,"kind":5,"items":[]}"#),
            (
                "kind 不匹配",
                r#"{"version":1,"kind":"proper_nouns","items":[]}"#,
            ),
            ("缺 items", r#"{"version":1,"kind":"replace"}"#),
            (
                "items 非数组",
                r#"{"version":1,"kind":"replace","items":{}}"#,
            ),
        ];
        for (name, content) in &file_level {
            let err = service.inspect_import(content).await.unwrap_err();
            assert_eq!(err.code(), "E-CONFIG-003", "{name} 应整体拒绝");
            let err = service
                .apply_import(content, ConflictStrategy::Skip)
                .await
                .unwrap_err();
            assert_eq!(err.code(), "E-CONFIG-003", "{name} 应整体拒绝");
        }

        // 条目数超上限（201 条超过 200）与超长数组：整体拒绝。
        let items: Vec<String> = (0..=MAX_RULES)
            .map(|i| format!(r#"{{"wrong":"词{i}","right":"v"}}"#))
            .collect();
        let too_many = format!(
            r#"{{"version":1,"kind":"replace","items":[{}]}}"#,
            items.join(",")
        );
        assert_eq!(
            service.inspect_import(&too_many).await.unwrap_err().code(),
            "E-CONFIG-003"
        );
        let long_array = format!("[{}]", vec!["1"; 100_000].join(","));
        assert_eq!(
            service
                .inspect_import(&long_array)
                .await
                .unwrap_err()
                .code(),
            "E-CONFIG-003"
        );

        // 极深嵌套（600 层 > serde_json 递归上限）：安全拒绝，不栈溢出。
        let deep = format!("{}1{}", "[".repeat(600), "]".repeat(600));
        assert!(service.inspect_import(&deep).await.is_err());

        // 单条异常（结构 / 内容问题）：计入失败条数，不导入。
        let item_level: [(&str, usize, &str); 6] = [
            (r#"[{"wrong":"a"}]"#, 1, "缺少 right 字段"),
            (r#"[{"right":"x"}]"#, 1, "缺少 wrong 字段"),
            (r#"[{"wrong":123,"right":"x"}]"#, 1, "wrong 字段类型非法"),
            (
                r#"[{"wrong":"a","right":"x","enabled":"yes"}]"#,
                1,
                "enabled 字段类型非法",
            ),
            (r#"[{"wrong":"  ","right":"x"}]"#, 1, "错误词为空"),
            (r#"[5]"#, 1, "条目必须是对象"),
        ];
        for (content, expected_failed, hint) in item_level {
            let report = service
                .apply_import(content, ConflictStrategy::Skip)
                .await
                .unwrap();
            assert_eq!(report.imported, 0, "{hint}");
            assert_eq!(report.failed, expected_failed, "{hint}");
            assert!(!report.issues.is_empty(), "{hint}");
        }

        // NFR-S-02 对照：失败原因不携带条目原文。
        let report = service
            .apply_import(
                r#"[{"wrong":"秘密内容","right":42}]"#,
                ConflictStrategy::Skip,
            )
            .await
            .unwrap();
        assert_eq!(report.failed, 1);
        let reason = &report.issues[0].reason;
        assert!(reason.contains("right 字段类型非法"), "{reason}");
        assert!(!reason.contains("秘密内容"), "失败原因不得携带内容");

        // 全程无写入：内存与磁盘均保持原样。
        assert_eq!(service.list().await.len(), 1);
        assert_eq!(std::fs::read_to_string(&dict_file).unwrap(), before);
    }

    #[tokio::test]
    async fn inspect_reports_counts_and_batch_dedup_skips() {
        let dir = TestDir::new("inspect");
        let service = make_service(&dir);
        service
            .add_rule("f二".to_string(), "F2".to_string(), true)
            .await
            .unwrap();
        let file = r#"{"version":1,"kind":"replace","items":[
            {"wrong":"f二","right":"F3"},
            {"wrong":"新一","right":"一"},
            {"wrong":"新一","right":"一重复"},
            {"wrong":"","right":"无效"}
        ]}"#;
        let inspection = service.inspect_import(file).await.unwrap();
        assert_eq!(inspection.format, "v2");
        assert_eq!(inspection.version, 1);
        assert_eq!(inspection.total, 4);
        assert_eq!(inspection.invalid, 1);
        // f二 与现有冲突；新一 批内重复。
        assert_eq!(inspection.conflicts, 2);
        // 校验不写入数据。
        assert_eq!(service.list().await.len(), 1);

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
    async fn import_over_capacity_rejects_without_partial_write() {
        let dir = TestDir::new("cap");
        let service = make_service(&dir);
        for i in 0..(MAX_RULES - 1) {
            service
                .add_rule(format!("词{i}"), "值".to_string(), true)
                .await
                .unwrap();
        }
        // 文件 2 条均为新词：合计 201 > 200 → 整体拒绝。
        let file = r#"{"version":1,"kind":"replace","items":[{"wrong":"新一","right":"a"},{"wrong":"新二","right":"b"}]}"#;
        let err = service
            .apply_import(file, ConflictStrategy::Skip)
            .await
            .unwrap_err();
        assert_eq!(err.code(), format!("E-CONFIG-{ERR_RULE_LIMIT:03}"));
        assert_eq!(service.list().await.len(), MAX_RULES - 1);

        // 冲突覆盖不占新增容量：全部冲突时不受上限影响。
        let file2 = r#"{"version":1,"kind":"replace","items":[{"wrong":"词0","right":"覆盖值"}]}"#;
        let report = service
            .apply_import(file2, ConflictStrategy::Overwrite)
            .await
            .unwrap();
        assert_eq!(report.updated, 1);
        assert_eq!(service.list().await.len(), MAX_RULES - 1);

        // 恰好达到 200 条：允许（跳过冲突 + 导入 1 条新增）。
        let file3 = r#"{"version":1,"kind":"replace","items":[{"wrong":"词0","right":"x"},{"wrong":"新三","right":"c"}]}"#;
        let report = service
            .apply_import(file3, ConflictStrategy::Skip)
            .await
            .unwrap();
        assert_eq!((report.imported, report.skipped), (1, 1));
        assert_eq!(service.list().await.len(), MAX_RULES);
    }

    #[test]
    fn civil_days_conversion() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_738), (2026, 10, 12));
    }

    #[test]
    fn rfc3339_format_shape() {
        let ts = now_rfc3339();
        assert_eq!(ts.len(), 20, "{ts}");
        assert!(ts.ends_with('Z'));
        assert_eq!(ts.as_bytes()[4], b'-');
        assert_eq!(ts.as_bytes()[10], b'T');
    }
}
