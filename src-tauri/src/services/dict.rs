//! DictService（T-011，F-DICT-01、PRD 3.4.1）：替换词典规则管理。
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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};
use tracing::{info, warn};

use crate::error::AppError;
use crate::events::AppEvent;
use crate::services::config_store::write_json_atomic;

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

/// DictService：规则内存镜像 + 原子持久化 + 变更广播。
#[derive(Clone)]
pub struct DictService {
    path: Arc<PathBuf>,
    rules: Arc<RwLock<Vec<ReplaceRule>>>,
    next_id: Arc<RwLock<u64>>,
    events: Arc<broadcast::Sender<AppEvent>>,
    /// 词典文件损坏回退标记（加载后由调用方读取并提示）。
    recovered: Arc<RwLock<bool>>,
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
        Ok(Self {
            path: Arc::new(path),
            rules: Arc::new(RwLock::new(rules)),
            next_id: Arc::new(RwLock::new(next_id)),
            events: Arc::new(events),
            recovered: Arc::new(RwLock::new(recovered)),
        })
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
}
