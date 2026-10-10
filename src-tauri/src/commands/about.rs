//! 关于页命令（T-016 契约，与前端 src/api/about.ts 逐字对齐）：
//! get_app_info / check_update / apply_update / export_logs / get_legal_text。

use serde::Deserialize;
use tauri::Manager;

use crate::error::AppError;
use crate::services::log::{self, LogService};
use crate::services::update::{UpdateCheckResult, UpdateService, ERR_APPLY_UNSUPPORTED};
use crate::state::AppState;

/// 应用信息（与前端 `AppInfo` 逐字段对应）。
#[derive(Debug, serde::Serialize)]
pub struct AppInfo {
    pub product_name: String,
    pub version: String,
    pub build_info: String,
    pub config_dir: String,
    pub data_dir: String,
    pub log_dir: String,
}

/// 法律文本文档（与前端 `LegalDocument` 逐字对应）。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegalDocument {
    /// MIT LICENSE（仓库根，构建时打包进二进制）。
    License,
    /// 法律免责声明（仓库根）。
    Disclaimer,
    /// 隐私说明（Markdown，应用内弹窗展示，PRD v1.7）。
    Privacy,
    /// 第三方组件声明（占位结构，内容随依赖完善）。
    ThirdParty,
}

/// 应用信息：版本、构建信息、三个平台标准目录（tech/08 8.1）。
#[tauri::command]
pub async fn get_app_info(app: tauri::AppHandle) -> Result<AppInfo, AppError> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|err| AppError::Platform(3, format!("配置目录不可用: {err}")))?;
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|err| AppError::Platform(3, format!("数据目录不可用: {err}")))?;
    let log_dir = app
        .path()
        .app_log_dir()
        .map_err(|err| AppError::Platform(3, format!("日志目录不可用: {err}")))?;
    let version = env!("CARGO_PKG_VERSION").to_string();
    Ok(AppInfo {
        product_name: "VibeType 码语".to_string(),
        build_info: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        version,
        config_dir: config_dir.to_string_lossy().into_owned(),
        data_dir: data_dir.to_string_lossy().into_owned(),
        log_dir: log_dir.to_string_lossy().into_owned(),
    })
}

/// 检查更新（F-ABOUT-01、NFR-PR-06：仅发送版本号 / OS / 架构）。
#[tauri::command]
pub async fn check_update(
    state: tauri::State<'_, AppState>,
) -> Result<UpdateCheckResult, AppError> {
    let about = state.config.get().await.about;
    Ok(UpdateService::new()
        .check_update(&about.update_feed_url, env!("CARGO_PKG_VERSION"))
        .await)
}

/// 下载并安装更新。
///
/// v0.1 更新服务器与 Tauri Updater 签名配置属发布流程事项，自动安装暂不提供：
/// 返回明确错误（E-NET-002），不静默忽略（tech/08 8.8、NFR-S-06 随发布流程落地）。
#[tauri::command]
pub async fn apply_update() -> Result<(), AppError> {
    Err(AppError::Network(
        ERR_APPLY_UNSUPPORTED,
        "自动安装更新将在更新服务上线后提供".to_string(),
    ))
}

/// 导出脱敏日志（F-ABOUT-02、NFR-S-02）：最近 1000 行 + 系统信息头，
/// 经脱敏层写入 `target_path`（plugin-dialog 选择的保存位置）。
#[tauri::command]
pub async fn export_logs(
    app: tauri::AppHandle,
    target_path: String,
) -> Result<log::ExportLogsResult, AppError> {
    let log_dir = app
        .path()
        .app_log_dir()
        .map_err(|err| AppError::Platform(3, format!("日志目录不可用: {err}")))?;
    let target = std::path::PathBuf::from(&target_path);
    tokio::task::spawn_blocking(move || {
        LogService::export_recent(&log_dir, &target, env!("CARGO_PKG_VERSION"))
    })
    .await
    .map_err(|err| AppError::Platform(4, format!("导出任务执行失败: {err}")))?
}

/// 法律文本（F-ABOUT-03：LICENSE / DISCLAIMER / 第三方组件声明）。
#[tauri::command]
pub async fn get_legal_text(document: LegalDocument) -> Result<String, AppError> {
    const LICENSE: &str = include_str!("../../../LICENSE");
    const DISCLAIMER: &str = include_str!("../../../DISCLAIMER.md");
    const THIRD_PARTY: &str = "\
VibeType 码语 第三方组件声明（占位结构，内容随依赖完善）

核心依赖（MIT / Apache-2.0 等宽松许可证）：
- Tauri 2（Rust + WebView 跨平台框架）
- Vue 3 / Vite / PrimeVue（前端）
- cpal（跨平台音频采集，Apache-2.0）
- enigo（跨平台输入模拟，MIT）
- tokio（异步运行时，MIT）
- global-hotkey / tauri-plugin-global-shortcut（全局快捷键）
- reqwest（HTTP 客户端，MIT）
- tracing（日志框架，MIT）
";
    const PRIVACY: &str = r#"# 隐私说明

**最后更新：2026-10-10**

## 1. 本地优先，默认零收集

1. 语音识别在本地完成，默认不收集、不上传任何语音或文本数据；断网状态下核心语音输入完整可用。
2. 应用不内置任何遥测、崩溃上报或用户行为统计；检查更新仅发送当前版本号与操作系统类型。

## 2. 数据保存在你的设备上

1. 配置文件、词典、提示词模板等保存在系统标准配置目录；日志、缓存、本地数据库保存在系统标准数据目录，均可在“通用 › 系统与数据”中查看具体位置。
2. “保存语音数据集”等可选功能产生的数据仅保存在你自己的设备上，其保管、备份与销毁由你自行负责。

## 3. 第三方 AI 与 MCP 服务

1. AI 后处理、MCP 服务等能力依赖你自行配置的第三方或本地服务。一旦启用，你口述的内容可能被发送至对应服务，其数据如何处理由该服务的服务条款约束。
2. 向第三方服务发送内容前，应用提供敏感信息检测与打码能力；请自行评估并遵守相关法律法规与所在组织的保密要求。

## 4. 日志与导出

1. 日志在写入前经过脱敏处理：API Key、访问令牌等密钥一律掩码，识别文本、提示词内容、剪贴板内容不会进入日志。
2. 日志默认同时写入本地文本文件与本地数据库，保留 7 天后自动清理；导出文件由你自行选择保存位置并负责保管。
"#;
    Ok(match document {
        LegalDocument::License => LICENSE.to_string(),
        LegalDocument::Disclaimer => DISCLAIMER.to_string(),
        LegalDocument::Privacy => PRIVACY.to_string(),
        LegalDocument::ThirdParty => THIRD_PARTY.to_string(),
    })
}
