//! 替换词典命令（T-011 契约，F-DICT-01；前端封装 `src/api/dict.ts` 逐字对应）。
//!
//! 命令名与参数名为前后端契约（tech/03 3.5），修改须同步前端。

use tauri::State;

use crate::error::AppError;
use crate::services::dict::ReplaceRule;
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
}
