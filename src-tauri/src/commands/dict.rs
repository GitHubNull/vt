//! 词典命令（T-011 替换词典、T-521 技术词库、T-522 专有名词；前端封装
//! `src/api/dict.ts` / `src/api/lexicon.ts` / `src/api/properNouns.ts` 逐字对应）。
//!
//! 命令名与参数名为前后端契约（tech/03 3.5），修改须同步前端。

use tauri::State;

use crate::error::AppError;
use crate::services::dict::{
    oversize_error, ConflictStrategy, DictKind, ImportInspection, ImportReport, ReplaceRule,
    ERR_IMPORT_EXPORT, MAX_IMPORT_BYTES,
};
use crate::services::lexicon::TechStackLexicon;
use crate::services::proper_nouns::ProperNoun;
use crate::state::AppState;

/// 规则清单（管理页数据源）。
#[tauri::command]
pub async fn dict_list_rules(state: State<'_, AppState>) -> Result<Vec<ReplaceRule>, AppError> {
    Ok(state.dict.list().await)
}

/// 新增规则；错误词重复 / 为空 / 超上限返回 E-CONFIG-00x。
#[tauri::command]
pub async fn dict_add_rule(
    state: State<'_, AppState>,
    wrong: String,
    right: String,
    enabled: bool,
) -> Result<ReplaceRule, AppError> {
    state.dict.add_rule(wrong, right, enabled).await
}

/// 更新规则（错误词 / 正确词 / 启停整体替换）。
#[tauri::command]
pub async fn dict_update_rule(
    state: State<'_, AppState>,
    rule: ReplaceRule,
) -> Result<(), AppError> {
    state.dict.update_rule(rule).await
}

/// 删除规则。
#[tauri::command]
pub async fn dict_delete_rule(state: State<'_, AppState>, id: u64) -> Result<(), AppError> {
    state.dict.delete_rule(id).await
}

/// 启停切换。
#[tauri::command]
pub async fn dict_toggle_rule(
    state: State<'_, AppState>,
    id: u64,
    enabled: bool,
) -> Result<(), AppError> {
    state.dict.toggle_rule(id, enabled).await
}

/// 导出全部规则（JSON 数组字符串，沿用 ReplaceRule serde 字段；备份 / 迁移用）。
#[tauri::command]
pub async fn dict_export(state: State<'_, AppState>) -> Result<String, String> {
    state
        .dict
        .export_rules()
        .await
        .map_err(|err| err.to_string())
}

/// 导入规则（JSON 数组字符串）：解析校验、去重入库、持久化并触发
/// `dict:changed` 刷新；返回实际导入条数。
#[tauri::command]
pub async fn dict_import(state: State<'_, AppState>, json: String) -> Result<usize, String> {
    state
        .dict
        .import_rules(&json)
        .await
        .map_err(|err| err.to_string())
}

/// 导出词典到文件：取全部规则的 JSON 覆盖写入 target_path，返回导出条数。
/// 供前端"导出到文件"对话框对接（与 `dict_export` 共享序列化逻辑）。
#[tauri::command]
pub async fn dict_export_file(
    state: State<'_, AppState>,
    target_path: String,
) -> Result<usize, String> {
    let json = state
        .dict
        .export_rules()
        .await
        .map_err(|err| err.to_string())?;
    export_rules_to_file(&json, &target_path)
}

/// 从文件导入词典：读取 path 内容后复用 `import_rules` 校验逻辑
/// （解析校验、去重、上限、持久化 + `dict:changed`），返回实际导入条数。
#[tauri::command]
pub async fn dict_import_file(state: State<'_, AppState>, path: String) -> Result<usize, String> {
    let content = read_rules_file(&path)?;
    state
        .dict
        .import_rules(&content)
        .await
        .map_err(|err| err.to_string())
}

// ---- T-523 统一导入导出（v2 格式；替换词典 / 专有名词共用，SPEC T-523） ----

/// 导出统一格式词典文件（该词典类型全部条目，JSON 覆盖写入 target_path），
/// 返回导出条数；供前端「导出」经系统保存对话框（plugin-dialog save）对接。
#[tauri::command]
pub async fn dict_export_unified(
    state: State<'_, AppState>,
    kind: DictKind,
    target_path: String,
) -> Result<usize, AppError> {
    let json = match kind {
        DictKind::Replace => state.dict.export_unified().await?,
        DictKind::ProperNouns => state.dict.proper_nouns().export_unified().await?,
    };
    write_unified_file(&json, kind, &target_path)
}

/// 导入前校验（T-523 向导「校验结果展示」步骤）：读取文件 → 文件级校验
/// （格式 / 版本兼容 / 条目上限；非法整体拒绝，E-CONFIG-003）→ 统计无效与
/// 冲突条数；不写入任何数据，不执行文件内任何内容（NFR-S-07）。
#[tauri::command]
pub async fn dict_import_inspect(
    state: State<'_, AppState>,
    kind: DictKind,
    path: String,
) -> Result<ImportInspection, AppError> {
    let content = read_import_file(&path)?;
    match kind {
        DictKind::Replace => state.dict.inspect_import(&content).await,
        DictKind::ProperNouns => state.dict.proper_nouns().inspect_import(&content).await,
    }
}

/// 执行导入（T-523 向导「冲突处理」步骤）：按冲突策略（跳过 / 覆盖 / 合并）
/// 写入并返回导入报告（成功 / 更新 / 跳过 / 失败条数）。
#[tauri::command]
pub async fn dict_import_apply(
    state: State<'_, AppState>,
    kind: DictKind,
    path: String,
    strategy: ConflictStrategy,
) -> Result<ImportReport, AppError> {
    let content = read_import_file(&path)?;
    match kind {
        DictKind::Replace => state.dict.apply_import(&content, strategy).await,
        DictKind::ProperNouns => {
            state
                .dict
                .proper_nouns()
                .apply_import(&content, strategy)
                .await
        }
    }
}

// ---- T-521 技术词库（内置术语包，只读；启停经 config dict 节） ----

/// 技术词库全部栈清单（管理页数据源；栈显示名由前端 i18n 映射）。
#[tauri::command]
pub async fn tech_lexicon_stacks(
    state: State<'_, AppState>,
) -> Result<Vec<TechStackLexicon>, AppError> {
    Ok(state.dict.tech_lexicon().stacks().await)
}

// ---- T-522 专有名词（F-DICT-02；持久化 dict/proper-nouns.json） ----

/// 专有名词条目清单（管理页数据源）。
#[tauri::command]
pub async fn proper_noun_list(state: State<'_, AppState>) -> Result<Vec<ProperNoun>, AppError> {
    Ok(state.dict.proper_nouns().list().await)
}

/// 新增条目；标准写法为空 / 重复 / 超上限返回 E-CONFIG-011/012。
#[tauri::command]
pub async fn proper_noun_add(
    state: State<'_, AppState>,
    term: String,
    aliases: Vec<String>,
    note: String,
    enabled: bool,
) -> Result<ProperNoun, AppError> {
    state
        .dict
        .proper_nouns()
        .add(term, aliases, note, enabled)
        .await
}

/// 更新条目（标准写法 / 别名 / 备注 / 启停整体替换）。
#[tauri::command]
pub async fn proper_noun_update(
    state: State<'_, AppState>,
    noun: ProperNoun,
) -> Result<(), AppError> {
    state.dict.proper_nouns().update(noun).await
}

/// 删除条目。
#[tauri::command]
pub async fn proper_noun_delete(state: State<'_, AppState>, id: u64) -> Result<(), AppError> {
    state.dict.proper_nouns().delete(id).await
}

/// 启停切换。
#[tauri::command]
pub async fn proper_noun_toggle(
    state: State<'_, AppState>,
    id: u64,
    enabled: bool,
) -> Result<(), AppError> {
    state.dict.proper_nouns().toggle(id, enabled).await
}

/// 导出 JSON 覆盖写入目标文件，返回写入条数。
///
/// 条数从导出 JSON 自身解析（与序列化同一份数据，无并发漂移）；
/// 隐私（NFR-S-02）：词典内容只写文件，不进日志。
fn export_rules_to_file(json: &str, target_path: &str) -> Result<usize, String> {
    let count = serde_json::from_str::<Vec<ReplaceRule>>(json)
        .map(|rules| rules.len())
        .map_err(|err| format!("导出内容序列化异常: {err}"))?;
    std::fs::write(target_path, json)
        .map_err(|err| format!("写入词典文件失败（{target_path}）: {err}"))?;
    Ok(count)
}

/// 读取词典文件内容（导入用）；IO 错误包装为清晰中文错误。
fn read_rules_file(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|err| format!("读取词典文件失败（{path}）: {err}"))
}

/// 读取导入文件（T-523）：先按文件元数据长度拒绝超大文件（防超大文件
/// 攻击），再读入并复核字节数（读取期间被增大的防御）；非 UTF-8 文本
/// 同样拒绝。文件级问题返回 E-CONFIG-003。
/// 隐私（NFR-S-02）：文件内容只交给服务层校验，绝不进入日志。
fn read_import_file(path: &str) -> Result<String, AppError> {
    let metadata = std::fs::metadata(path).map_err(|err| {
        AppError::Config(
            ERR_IMPORT_EXPORT,
            format!("读取导入文件失败（{path}）: {err}"),
        )
    })?;
    if metadata.len() > MAX_IMPORT_BYTES as u64 {
        return Err(oversize_error());
    }
    let content = std::fs::read_to_string(path).map_err(|err| {
        AppError::Config(
            ERR_IMPORT_EXPORT,
            format!("读取导入文件失败（{path}）: {err}"),
        )
    })?;
    if content.len() > MAX_IMPORT_BYTES {
        return Err(oversize_error());
    }
    Ok(content)
}

/// 统一格式导出内容写入目标文件（覆盖写），返回条目数。
/// 条数从导出 JSON 自身解析（与序列化同一份数据，无并发漂移）。
/// 隐私（NFR-S-02）：导出内容只写文件，不进日志。
fn write_unified_file(json: &str, kind: DictKind, target_path: &str) -> Result<usize, AppError> {
    let count = serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| {
            value
                .get("items")
                .and_then(|items| items.as_array().map(Vec::len))
        })
        .ok_or_else(|| AppError::Config(ERR_IMPORT_EXPORT, "导出内容序列化异常".to_string()))?;
    std::fs::write(target_path, json).map_err(|err| {
        AppError::Config(
            ERR_IMPORT_EXPORT,
            format!("写入{}文件失败（{target_path}）: {err}", kind.label()),
        )
    })?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 临时目录（进程 + 纳秒唯一，析构清理）。
    struct TestDir(std::path::PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "vt-dict-cmd-test-{}-{}-{:?}",
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

    #[test]
    fn export_rules_to_file_writes_and_counts() {
        let dir = TestDir::new("export");
        let target = dir.0.join("backup.json");
        let json = r#"[{"id":1,"wrong":"f二","right":"F2","enabled":true}]"#;
        let count = export_rules_to_file(json, &target.to_string_lossy()).unwrap();
        assert_eq!(count, 1);
        // 覆盖写：内容与导出 JSON 完全一致。
        assert_eq!(std::fs::read_to_string(&target).unwrap(), json);
    }

    #[test]
    fn export_rules_to_file_io_error_is_chinese() {
        let dir = TestDir::new("export-io-err");
        // 目标路径落在已存在的文件之下：必然写失败。
        let blocked = dir.0.join("blocked");
        std::fs::write(&blocked, b"x").unwrap();
        let err =
            export_rules_to_file("[]", &blocked.join("sub.json").to_string_lossy()).unwrap_err();
        assert!(err.contains("写入词典文件失败"), "错误应为中文描述: {err}");
    }

    #[test]
    fn read_rules_file_ok_and_error() {
        let dir = TestDir::new("read");
        let file = dir.0.join("dict.json");
        std::fs::write(&file, "[]").unwrap();
        assert_eq!(read_rules_file(&file.to_string_lossy()).unwrap(), "[]");
        let err = read_rules_file(&dir.0.join("missing.json").to_string_lossy()).unwrap_err();
        assert!(err.contains("读取词典文件失败"), "错误应为中文描述: {err}");
    }

    #[tokio::test]
    async fn export_file_then_import_file_roundtrip() {
        // 命令层文件导入 / 导出与 DictService 组合链路：
        // 导出 JSON → 写文件 → 读文件 → 导入新服务（复用 import_rules 校验）。
        let dir = TestDir::new("roundtrip");
        let (tx, _rx) = crate::events::channel();
        let service = crate::services::dict::DictService::load(&dir.0, tx).unwrap();
        service
            .add_rule("f二".to_string(), "F2".to_string(), true)
            .await
            .unwrap();
        service
            .add_rule("我的邮箱".to_string(), "dev@example.com".to_string(), false)
            .await
            .unwrap();

        // 模拟 dict_export_file 的写盘部分。
        let json = service.export_rules().await.unwrap();
        let backup = dir.0.join("backup.json");
        let exported = export_rules_to_file(&json, &backup.to_string_lossy()).unwrap();
        assert_eq!(exported, 2);

        // 模拟 dict_import_file 的读盘 + 导入部分（目标服务已有 1 条重复规则）。
        let dir2 = TestDir::new("roundtrip-2");
        let (tx2, mut rx2) = crate::events::channel();
        let service2 = crate::services::dict::DictService::load(&dir2.0, tx2).unwrap();
        service2
            .add_rule("f二".to_string(), "旧值".to_string(), true)
            .await
            .unwrap();
        let content = read_rules_file(&backup.to_string_lossy()).unwrap();
        let imported = service2.import_rules(&content).await.unwrap();
        // 重复规则被去重，仅导入 1 条。
        assert_eq!(imported, 1);
        assert!(matches!(
            rx2.try_recv().unwrap(),
            crate::events::AppEvent::DictChanged
        ));
        assert_eq!(service2.list().await.len(), 2);
    }

    // ---- T-523 统一格式（v2）命令层 ----

    #[tokio::test]
    async fn unified_export_write_read_apply_flow() {
        let dir = TestDir::new("v2-flow");
        let (tx, _rx) = crate::events::channel();
        let service = crate::services::dict::DictService::load(&dir.0, tx).unwrap();
        service
            .add_rule("f二".to_string(), "F2".to_string(), true)
            .await
            .unwrap();
        service
            .add_rule("回车".to_string(), String::new(), false)
            .await
            .unwrap();

        // 模拟 dict_export_unified 的序列化 + write_unified_file 写盘。
        let json = service.export_unified().await.unwrap();
        let backup = dir.0.join("vibetype-replace-dict.json");
        let exported =
            write_unified_file(&json, DictKind::Replace, &backup.to_string_lossy()).unwrap();
        assert_eq!(exported, 2);

        // 模拟 dict_import_apply 的读盘 + 导入。
        let dir2 = TestDir::new("v2-flow-2");
        let (tx2, mut rx2) = crate::events::channel();
        let service2 = crate::services::dict::DictService::load(&dir2.0, tx2).unwrap();
        let content = read_import_file(&backup.to_string_lossy()).unwrap();
        let report = service2
            .apply_import(&content, ConflictStrategy::Skip)
            .await
            .unwrap();
        assert_eq!(report.imported, 2);
        assert!(matches!(
            rx2.try_recv().unwrap(),
            crate::events::AppEvent::DictChanged
        ));
        assert_eq!(service2.list().await.len(), 2);
    }

    #[tokio::test]
    async fn proper_nouns_export_write_read_flow() {
        let dir = TestDir::new("nouns-flow");
        let (tx, _rx) = crate::events::channel();
        let dict = crate::services::dict::DictService::load(&dir.0, tx).unwrap();
        dict.proper_nouns()
            .add(
                "PostgreSQL".to_string(),
                vec!["PG".to_string()],
                String::new(),
                true,
            )
            .await
            .unwrap();

        let json = dict.proper_nouns().export_unified().await.unwrap();
        let backup = dir.0.join("vibetype-proper-nouns.json");
        let exported =
            write_unified_file(&json, DictKind::ProperNouns, &backup.to_string_lossy()).unwrap();
        assert_eq!(exported, 1);

        let dir2 = TestDir::new("nouns-flow-2");
        let (tx2, _rx2) = crate::events::channel();
        let dict2 = crate::services::dict::DictService::load(&dir2.0, tx2).unwrap();
        let content = read_import_file(&backup.to_string_lossy()).unwrap();
        let report = dict2
            .proper_nouns()
            .apply_import(&content, ConflictStrategy::Merge)
            .await
            .unwrap();
        assert_eq!(report.imported, 1);
        assert_eq!(dict2.proper_nouns().list().await[0].aliases, vec!["PG"]);
    }

    #[test]
    fn read_import_file_rejects_oversize_missing_and_non_utf8() {
        let dir = TestDir::new("read-import");
        // 超大文件（上限 + 1 字节）：拒绝。
        let big = dir.0.join("big.json");
        std::fs::write(&big, vec![b'x'; MAX_IMPORT_BYTES + 1]).unwrap();
        let err = read_import_file(&big.to_string_lossy()).unwrap_err();
        assert_eq!(err.code(), "E-CONFIG-003");
        assert!(err.message().contains("过大"), "{err}");

        // 非 UTF-8 文本：拒绝。
        let bad = dir.0.join("bad.json");
        std::fs::write(&bad, [0xff_u8, 0xfe, 0x00]).unwrap();
        let err = read_import_file(&bad.to_string_lossy()).unwrap_err();
        assert_eq!(err.code(), "E-CONFIG-003");
        assert!(err.message().contains("读取导入文件失败"), "{err}");

        // 不存在的文件：拒绝。
        let err = read_import_file(&dir.0.join("missing.json").to_string_lossy()).unwrap_err();
        assert_eq!(err.code(), "E-CONFIG-003");

        // 正常小文件：读入。
        let ok = dir.0.join("ok.json");
        std::fs::write(&ok, "{}").unwrap();
        assert_eq!(read_import_file(&ok.to_string_lossy()).unwrap(), "{}");
    }

    #[test]
    fn write_unified_file_writes_counts_and_errors() {
        let dir = TestDir::new("write-unified");
        let target = dir.0.join("export.json");
        let json =
            r#"{"version":1,"kind":"replace","items":[{"wrong":"a","right":"b","enabled":true}]}"#;
        let count = write_unified_file(json, DictKind::Replace, &target.to_string_lossy()).unwrap();
        assert_eq!(count, 1);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), json);

        // 目标落在已存在文件之下：写失败，中文错误 + E-CONFIG-003。
        let blocked = dir.0.join("blocked");
        std::fs::write(&blocked, b"x").unwrap();
        let err = write_unified_file(
            json,
            DictKind::ProperNouns,
            &blocked.join("sub.json").to_string_lossy(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "E-CONFIG-003");
        assert!(err.message().contains("写入专有名词文件失败"), "{err}");
    }
}
