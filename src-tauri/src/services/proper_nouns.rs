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
}
