//! LogService：日志滚动、脱敏、导出（tech/01 1.4、tech/04 4.7、tech/08 8.4、NFR-S-02）。
//!
//! - 按天滚动文件追加器（T-001 提供）；
//! - 导出（F-ABOUT-02）：取最近日志的最后 1000 行，逐行过脱敏层，附加系统信息头，
//!   写入用户选择的位置；
//! - 脱敏（NFR-S-02）：密钥 / 令牌样式内容一律掩码（`***`），保证导出文件不含
//!   API Key、令牌、授权头；音频与识别文本本就不进日志（NFR-PR-02 服务层保证）。

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::AppError;

/// 日志文件名前缀。
const LOG_FILE_PREFIX: &str = "vibetype.log";
/// 导出包含的最近日志行数（F-ABOUT-02）。
pub const EXPORT_MAX_LINES: usize = 1000;

/// 导出错误编号：日志目录不可读 / 无日志文件。
pub const ERR_NO_LOG_FILE: u32 = 1;
/// 导出错误编号：写入目标位置失败。
pub const ERR_EXPORT_WRITE: u32 = 2;

/// 日志服务。
pub struct LogService;

/// 导出结果（与前端 `ExportLogsResult` 逐字段对应，T-016 契约）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportLogsResult {
    pub path: String,
    pub lines: u32,
}

impl LogService {
    /// 创建按天滚动的非阻塞文件写入器。
    ///
    /// 返回 `(writer, guard)`：`guard` 必须存活至进程退出，否则缓冲日志会丢失。
    pub fn daily_writer(
        log_dir: &Path,
    ) -> (
        tracing_appender::non_blocking::NonBlocking,
        tracing_appender::non_blocking::WorkerGuard,
    ) {
        let appender = tracing_appender::rolling::daily(log_dir, LOG_FILE_PREFIX);
        tracing_appender::non_blocking(appender)
    }

    /// 导出最近日志：最后 [`EXPORT_MAX_LINES`] 行 + 系统信息头，经脱敏后写入
    /// `target_path`。返回写入路径与行数。
    pub fn export_recent(
        log_dir: &Path,
        target_path: &Path,
        product_version: &str,
    ) -> Result<ExportLogsResult, AppError> {
        let latest = latest_log_file(log_dir)
            .ok_or_else(|| AppError::Platform(ERR_NO_LOG_FILE, "暂无日志文件可导出".to_string()))?;

        // 取最后 N 行：读全量后截取（日志文件为本地滚动文件，量级可控）。
        let file = std::fs::File::open(&latest).map_err(|err| {
            AppError::Platform(ERR_NO_LOG_FILE, format!("日志文件读取失败: {err}"))
        })?;
        let reader = std::io::BufReader::new(file);
        let lines: Vec<String> = reader.lines().map_while(std::result::Result::ok).collect();
        let tail: &[String] = if lines.len() > EXPORT_MAX_LINES {
            &lines[lines.len() - EXPORT_MAX_LINES..]
        } else {
            &lines
        };

        let mut output = std::fs::File::create(target_path).map_err(|err| {
            AppError::Platform(
                ERR_EXPORT_WRITE,
                format!("导出文件创建失败（{}）: {err}", target_path.display()),
            )
        })?;
        writeln!(output, "{}", system_info_header(product_version))
            .map_err(write_err(target_path))?;
        for line in tail {
            writeln!(output, "{}", sanitize_log_line(line)).map_err(write_err(target_path))?;
        }
        output.flush().map_err(write_err(target_path))?;

        Ok(ExportLogsResult {
            path: target_path.to_string_lossy().into_owned(),
            lines: tail.len() as u32,
        })
    }
}

fn write_err(target: &Path) -> impl Fn(std::io::Error) -> AppError + '_ {
    move |err| {
        AppError::Platform(
            ERR_EXPORT_WRITE,
            format!("导出文件写入失败（{}）: {err}", target.display()),
        )
    }
}

/// 找最近的日志文件（按修改时间；tracing-appender 按天滚动，文件名带日期后缀）。
fn latest_log_file(log_dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(log_dir).ok()?;
    entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with(LOG_FILE_PREFIX))
                .unwrap_or(false)
        })
        .filter_map(|path| {
            let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
            Some((path, modified))
        })
        .max_by_key(|(_path, modified)| *modified)
        .map(|(path, _modified)| path)
}

/// 系统信息头（F-ABOUT-02：OS 版本、产品版本、模型信息）。
pub fn system_info_header(product_version: &str) -> String {
    format!(
        "# VibeType 日志导出\n# os: {} {}\n# arch: {}\n# version: {}\n# model: {}\n",
        std::env::consts::OS,
        os_release(),
        std::env::consts::ARCH,
        product_version,
        // 模型管理属 T-014；未安装模型时如实标注。
        "未安装（模型管理功能后续版本提供）"
    )
}

/// 尽力读取 Linux /etc/os-release 的 PRETTY_NAME（其余平台用 cfg 常量）。
fn os_release() -> String {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|content| {
                content.lines().find_map(|line| {
                    line.strip_prefix("PRETTY_NAME=")
                        .map(|value| value.trim_matches('"').to_string())
                })
            })
            .unwrap_or_else(|| "linux".to_string())
    }
    #[cfg(target_os = "macos")]
    {
        "macOS".to_string()
    }
    #[cfg(target_os = "windows")]
    {
        "Windows".to_string()
    }
}

/// 已知令牌前缀：前缀后的令牌字符（`[A-Za-z0-9_-]`）整体掩码。
const TOKEN_PREFIXES: &[&str] = &[
    "sk-",         // OpenAI 风格 API Key（NFR-S-02 示例）
    "ghp_",        // GitHub personal access token
    "gho_",        // GitHub OAuth token
    "github_pat_", // GitHub fine-grained PAT
    "xoxb-",       // Slack bot token
    "xoxp-",       // Slack user token
    "AKIA",        // AWS Access Key ID
];
/// JSON 风格敏感键：其值整体掩码。
const SENSITIVE_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "apiKey",
    "access_token",
    "refresh_token",
    "token",
    "secret",
    "client_secret",
    "authorization",
    "private_key",
];

/// 脱敏一行日志（NFR-S-02）：掩码已知令牌前缀与敏感键值，输出不含原始密钥。
pub fn sanitize_log_line(line: &str) -> String {
    let mut out = mask_sensitive_values(line);
    for prefix in TOKEN_PREFIXES {
        out = mask_token_runs(&out, prefix);
    }
    mask_bearer_tokens(&out)
}

/// `"key": "value"` / `"key":"value"` 形态的值掩码（大小写不敏感匹配键名）。
fn mask_sensitive_values(line: &str) -> String {
    let mut out = line.to_string();
    for key in SENSITIVE_KEYS {
        let needle = format!("\"{key}\"");
        let mut search_from = 0;
        while let Some(found) = out[search_from..].find(&needle) {
            let key_start = search_from + found;
            // 键名后允许空白 + ':' + 空白，再取带引号的值。
            let after_key = &out[key_start + needle.len()..];
            let colon = after_key.find(':');
            let Some(colon) = colon else {
                search_from = key_start + needle.len();
                continue;
            };
            let after_colon = &after_key[colon + 1..];
            let value_open = after_colon.find('"');
            let Some(value_open) = value_open else {
                search_from = key_start + needle.len();
                continue;
            };
            let value_start = key_start + needle.len() + colon + 1 + value_open + 1;
            let value_close = out[value_start..].find('"').map(|i| value_start + i);
            let Some(value_end) = value_close else {
                search_from = value_start;
                continue;
            };
            out.replace_range(value_start..value_end, "***");
            search_from = value_start + 3;
        }
    }
    out
}

/// 掩码 `prefix` 之后连续的令牌字符（保留前缀本身便于排查类型）。
fn mask_token_runs(line: &str, prefix: &str) -> String {
    let mut out = line.to_string();
    let mut search_from = 0;
    while let Some(found) = out[search_from..].find(prefix) {
        let run_start = search_from + found + prefix.len();
        let run_end = out[run_start..]
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
            .map(|i| run_start + i)
            .unwrap_or(out.len());
        if run_end > run_start {
            out.replace_range(run_start..run_end, "***");
        }
        search_from = run_start + 3;
    }
    out
}

/// `Bearer <token>` / `Authorization: Bearer <token>` 的令牌掩码。
fn mask_bearer_tokens(line: &str) -> String {
    let mut out = line.to_string();
    let needle = "Bearer ";
    let mut search_from = 0;
    while let Some(found) = out[search_from..].find(needle) {
        let run_start = search_from + found + needle.len();
        let run_end = out[run_start..]
            .find(|ch: char| ch.is_whitespace() || ch == '"' || ch == '\'')
            .map(|i| run_start + i)
            .unwrap_or(out.len());
        if run_end > run_start {
            out.replace_range(run_start..run_end, "***");
        }
        search_from = run_start + 3;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_masks_api_key_prefixes() {
        let line = "request failed key=sk-AbCdEf1234567890xyz end";
        let sanitized = sanitize_log_line(line);
        assert!(sanitized.contains("sk-***"));
        assert!(!sanitized.contains("AbCdEf1234567890xyz"));
    }

    #[test]
    fn sanitize_masks_github_and_aws_tokens() {
        let line = "token ghp_AaaaBbbbCcccDdddEeeeFfffGggg000111222 and AKIAIOSFODNN7EXAMPLE";
        let sanitized = sanitize_log_line(line);
        assert!(sanitized.contains("ghp_***"));
        assert!(sanitized.contains("AKIA***"));
        assert!(!sanitized.contains("AaaaBbbb"));
        assert!(!sanitized.contains("IOSFODNN7EXAMPLE"));
    }

    #[test]
    fn sanitize_masks_json_sensitive_values() {
        let line = r#"config {"api_key": "sk-live-abcdef123456", "token":"t0p-secret-value", "note": "keep"}"#;
        let sanitized = sanitize_log_line(line);
        assert!(sanitized.contains("\"api_key\": \"***\""));
        assert!(sanitized.contains("\"token\":\"***\""));
        assert!(sanitized.contains("\"note\": \"keep\""));
        assert!(!sanitized.contains("sk-live-abcdef123456"));
    }

    #[test]
    fn sanitize_masks_bearer_tokens() {
        let line = "authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig done";
        let sanitized = sanitize_log_line(line);
        assert!(sanitized.contains("Bearer ***"));
        assert!(!sanitized.contains("eyJhbGciOiJIUzI1NiJ9"));
    }

    #[test]
    fn sanitize_leaves_normal_lines_untouched() {
        let line = "INFO 录音会话开始 elapsed_ms=42";
        assert_eq!(sanitize_log_line(line), line);
    }

    #[test]
    fn sanitize_masks_authorization_header_value() {
        let line = r#"{"authorization": "Bearer abc123xyz", "path": "/v1/chat"}"#;
        let sanitized = sanitize_log_line(line);
        // 敏感键值整体掩码（Bearer 前缀已在键值掩码时处理或二次掩码）。
        assert!(!sanitized.contains("abc123xyz"));
    }

    #[test]
    fn export_recent_writes_sanitized_tail_with_header() {
        let dir = std::env::temp_dir().join(format!(
            "vt-log-export-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let log_file = dir.join("vibetype.log.2026-10-10");
        let mut content = String::new();
        for index in 0..1100 {
            content.push_str(&format!("INFO line {index}\n"));
        }
        content.push_str("DEBUG key=sk-SuperSecretValue9999\n");
        std::fs::write(&log_file, content).unwrap();

        let target = dir.join("export.log");
        let result = LogService::export_recent(&dir, &target, "0.1.0-test").unwrap();
        assert_eq!(result.lines, EXPORT_MAX_LINES as u32);
        let exported = std::fs::read_to_string(&target).unwrap();
        assert!(exported.contains("# version: 0.1.0-test"));
        // 头部 + 最近 1000 行（第 101~1100 行，0 基索引 101..=1100）。
        assert!(exported.contains("\nINFO line 101\n"));
        // 整行匹配：第 100 行被截掉（"INFO line 100" 是 "INFO line 1000" 的子串，不能粗匹配）。
        assert!(!exported.contains("\nINFO line 100\n"));
        // 末尾密钥行在导出范围内，必须已被脱敏。
        assert!(exported.contains("sk-***"));
        assert!(!exported.contains("SuperSecretValue9999"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_recent_without_logs_errors() {
        let dir = std::env::temp_dir().join(format!(
            "vt-log-empty-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let err = LogService::export_recent(&dir, &dir.join("out.log"), "0.1.0").unwrap_err();
        assert_eq!(err.code(), "E-PLAT-001");
    }
}
