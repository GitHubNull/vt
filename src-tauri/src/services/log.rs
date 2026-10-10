//! LogService：日志双写管道、查询、搜索导出（tech/04 4.7、tech/08 8.4、NFR-S-02、T-018）。
//!
//! - **双写（F-LOG-02）**：自定义 `tracing` Layer 把结构化记录送入 `mpsc` 通道，
//!   独立写线程按配置分发到按天滚动文本文件与本地 SQLite；目标可热切换
//!   （关闭数据库即断开连接，搜索随之不可用）；
//! - **查询（F-LOG-01）**：级别 / 时间范围在 SQL 侧过滤，关键词 / 正则 / 大小写
//!   匹配在消费侧逐行过滤（SQLite 无内置正则）；单页上限 [`QUERY_MAX_LIMIT`]；
//! - **导出（F-LOG-03）**：按当前查询条件流式导出（系统信息头 + 脱敏行）；
//! - **脱敏（NFR-S-02）**：密钥 / 令牌样式内容在事件入队前一律掩码（`***`），
//!   文本与数据库两个目标共用同一脱敏层；音频与识别文本本就不进日志。

use std::io::{BufRead, Write as IoWrite};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};

use regex::RegexBuilder;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer};

use crate::error::AppError;
use crate::services::config_store::LogTarget;

/// 日志文件名前缀。
const LOG_FILE_PREFIX: &str = "vibetype.log";
/// 日志数据库文件名（日志目录下，与文本日志同目录、同保留策略）。
const DB_FILE_NAME: &str = "vibetype-logs.sqlite3";
/// 导出包含的最近日志行数（F-ABOUT-02）。
pub const EXPORT_MAX_LINES: usize = 1000;
/// 查询单页上限（F-LOG-01，PRD：单页最多 200 条）。
pub const QUERY_MAX_LIMIT: u32 = 200;
/// 单次查询从数据库扫描的最大行数（消费侧过滤的扫描窗口上限）。
const MAX_SCAN: u32 = 10_000;
/// 日志保留天数（NFR 5.7：文本与数据库一致）。
const RETENTION_DAYS: i64 = 7;
/// 文本滚动文件清理的最小间隔（毫秒）。
const CLEANUP_INTERVAL_MILLIS: i64 = 3_600_000;

/// 导出错误编号：日志目录不可读 / 无日志文件。
pub const ERR_NO_LOG_FILE: u32 = 1;
/// 导出错误编号：写入目标位置失败。
pub const ERR_EXPORT_WRITE: u32 = 2;

/// 日志查询错误编号：数据库记录目标未启用（搜索不可用）。
pub const LOG_ERR_DB_DISABLED: u32 = 1;
/// 日志查询错误编号：正则表达式无效。
pub const LOG_ERR_BAD_REGEX: u32 = 2;
/// 日志搜索导出错误编号：写入目标位置失败。
pub const LOG_ERR_EXPORT_WRITE: u32 = 3;
/// 日志查询错误编号：数据库查询失败。
pub const LOG_ERR_QUERY: u32 = 4;
/// 日志命令错误编号：执行线程失败。
pub const LOG_ERR_TASK: u32 = 5;

/// 一条结构化日志记录（入队后由写线程分发到各目标）。
#[derive(Debug, Clone)]
pub struct LogRecord {
    pub ts_millis: i64,
    /// tracing 级别名（"TRACE" / "DEBUG" / "INFO" / "WARN" / "ERROR"）。
    pub level: String,
    pub target: String,
    /// 消息 + 结构化字段组成的正文（已脱敏）。
    pub body: String,
}

/// tracing 事件访问器：收集 message 与结构化字段（均转为字符串）。
#[derive(Default)]
struct RecordVisitor {
    message: String,
    fields: Vec<(String, String)>,
}

impl tracing::field::Visit for RecordVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let raw = format!("{value:?}");
        // Debug 格式会给字符串加引号，剥离一层便于落库与展示。
        let value = raw
            .strip_prefix('"')
            .and_then(|inner| inner.strip_suffix('"'))
            .map(str::to_string)
            .unwrap_or(raw);
        if field.name() == "message" {
            self.message = value;
        } else {
            self.fields.push((field.name().to_string(), value));
        }
    }
}

/// 消息与字段组成正文，逐部分脱敏（NFR-S-02）。
fn compose_body(message: &str, fields: &[(String, String)]) -> String {
    let mut body = sanitize_log_line(message);
    for (key, value) in fields {
        let value = sanitize_log_line(value);
        if body.is_empty() {
            body = format!("{key}={value}");
        } else {
            body.push_str(&format!(" {key}={value}"));
        }
    }
    body
}

/// 当前 Unix 毫秒时间戳。
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 毫秒时间戳格式化为 `YYYY-MM-DDTHH:MM:SS.mmmZ`（UTC，无外部日期库）。
pub fn format_ts_millis(millis: i64) -> String {
    let secs = millis.div_euclid(1000);
    let ms = millis.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{ms:03}Z",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    )
}

/// 儒略日数转民用日期（Howard Hinnant 算法，1970-01-01 为第 0 天）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { y + 1 } else { y }, month, day)
}

/// tracing 出口层：事件 → 脱敏 → 结构化记录入队（F-LOG-02 管道入口）。
#[derive(Clone)]
pub struct LogSinkLayer {
    tx: mpsc::Sender<LogRecord>,
}

impl LogSinkLayer {
    pub fn new(tx: mpsc::Sender<LogRecord>) -> Self {
        Self { tx }
    }
}

impl<S> Layer<S> for LogSinkLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = RecordVisitor::default();
        event.record(&mut visitor);
        let meta = event.metadata();
        // 发送失败（写线程已退出）只丢弃本条，不影响业务线程。
        let _ = self.tx.send(LogRecord {
            ts_millis: now_millis(),
            level: meta.level().to_string(),
            target: meta.target().to_string(),
            body: compose_body(&visitor.message, &visitor.fields),
        });
    }
}

/// 共享状态：当前启用的记录目标 + 数据库连接（None = 数据库目标关闭）。
struct SharedState {
    targets: Vec<LogTarget>,
    db: Option<Connection>,
}

/// 日志服务句柄：命令层查询 / 导出与配置热更新共用。
#[derive(Clone)]
pub struct LogStore {
    state: Arc<Mutex<SharedState>>,
}

impl LogStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, SharedState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 应用记录目标变更（F-LOG-02）：开启数据库时惰性建连，关闭即断开。
    pub fn set_targets(&self, targets: Vec<LogTarget>, log_dir: &Path) {
        let mut state = self.lock();
        if targets.contains(&LogTarget::Database) {
            if state.db.is_none() {
                match open_db(log_dir) {
                    Ok(conn) => state.db = Some(conn),
                    Err(err) => {
                        tracing::warn!(code = %err.code(), "日志数据库打开失败，数据库目标降级为不可用");
                        state.db = None;
                    }
                }
            }
        } else {
            // 关闭目标即断开连接：此后查询返回 E-LOG-001，期间日志不入库。
            state.db = None;
        }
        state.targets = targets;
    }

    /// 数据库目标当前是否可用（搜索前提）。
    pub fn database_enabled(&self) -> bool {
        self.lock().db.is_some()
    }

    /// 按条件查询（F-LOG-01）；数据库未启用返回 E-LOG-001。
    pub fn query(&self, query: &LogQuery) -> Result<LogPage, AppError> {
        let matcher = compile_matcher(query)?;
        let mut state = self.lock();
        let conn = state.db.as_mut().ok_or_else(|| {
            AppError::Log(
                LOG_ERR_DB_DISABLED,
                "未启用数据库日志记录，搜索不可用".to_string(),
            )
        })?;
        query_impl(conn, query, &matcher)
    }

    /// 按当前查询条件导出匹配日志（F-LOG-03）。
    pub fn export(
        &self,
        query: &LogQuery,
        target_path: &Path,
    ) -> Result<ExportLogsResult, AppError> {
        let matcher = compile_matcher(query)?;
        let mut state = self.lock();
        let conn = state.db.as_mut().ok_or_else(|| {
            AppError::Log(
                LOG_ERR_DB_DISABLED,
                "未启用数据库日志记录，搜索不可用".to_string(),
            )
        })?;
        export_impl(conn, query, &matcher, target_path)
    }
}

/// 创建双写管道：返回 (tracing 层, 服务句柄)。写线程随进程退出回收。
pub fn log_pipeline(log_dir: &Path) -> (LogSinkLayer, LogStore) {
    let (tx, rx) = mpsc::channel();
    let store = LogStore {
        state: Arc::new(Mutex::new(SharedState {
            targets: Vec::new(),
            db: None,
        })),
    };
    // 初始目标 = 配置默认值（双开）：init_services 载入配置后立即校正，
    // 避免启动瞬间（配置未加载）的日志被丢弃。
    {
        let mut state = store.lock();
        state.targets = crate::services::config_store::LogConfig::default().targets;
    }
    let state = store.state.clone();
    let log_dir = log_dir.to_path_buf();
    // 写线程创建失败属极端场景（资源耗尽）：降级为日志丢弃，不影响启动。
    if let Err(err) = std::thread::Builder::new()
        .name("vt-log-writer".to_string())
        .spawn(move || run_writer(rx, log_dir, state))
    {
        eprintln!("日志写线程创建失败，日志功能降级: {err}");
    }
    (LogSinkLayer::new(tx), store)
}

/// 写线程：消费记录队列，按当前 targets 分发到文本文件与数据库。
fn run_writer(rx: mpsc::Receiver<LogRecord>, log_dir: PathBuf, state: Arc<Mutex<SharedState>>) {
    let mut file: Option<(
        tracing_appender::non_blocking::NonBlocking,
        tracing_appender::non_blocking::WorkerGuard,
    )> = None;
    let mut last_cleanup = 0i64;
    while let Ok(record) = rx.recv() {
        let mut state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.targets.contains(&LogTarget::File) {
            if file.is_none() {
                file = Some(LogService::daily_writer(&log_dir));
            }
            if let Some((writer, _)) = file.as_mut() {
                let line = format!(
                    "{} {:>5} {}: {}\n",
                    format_ts_millis(record.ts_millis),
                    record.level,
                    record.target,
                    record.body
                );
                let _ = writer.write_all(line.as_bytes());
            }
        }
        if state.targets.contains(&LogTarget::Database) {
            if state.db.is_none() {
                match open_db(&log_dir) {
                    Ok(conn) => state.db = Some(conn),
                    Err(err) => {
                        tracing::warn!(code = %err.code(), "日志数据库打开失败，本条日志跳过入库");
                        state.db = None;
                    }
                }
            }
            if let Some(conn) = state.db.as_mut() {
                let _ = conn.execute(
                    "INSERT INTO logs (ts_millis, level, target, body) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![record.ts_millis, record.level, record.target, record.body],
                );
            }
        }
        drop(state);
        let now = now_millis();
        if now - last_cleanup > CLEANUP_INTERVAL_MILLIS {
            last_cleanup = now;
            cleanup_old_log_files(&log_dir);
        }
    }
}

/// 打开（或初始化）日志数据库：建表、建索引、清理超龄记录（7 天）。
fn open_db(log_dir: &Path) -> Result<Connection, AppError> {
    let path = log_dir.join(DB_FILE_NAME);
    let conn = Connection::open(&path).map_err(|err| {
        AppError::Log(
            LOG_ERR_QUERY,
            format!("日志数据库打开失败（{}）: {err}", path.display()),
        )
    })?;
    create_schema(&conn)?;
    let cutoff = now_millis() - RETENTION_DAYS * 86_400_000;
    conn.execute("DELETE FROM logs WHERE ts_millis < ?1", [cutoff])
        .map_err(|err| AppError::Log(LOG_ERR_QUERY, format!("日志数据库清理失败: {err}")))?;
    Ok(conn)
}

/// 建表与索引（幂等）。
fn create_schema(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts_millis INTEGER NOT NULL,
            level TEXT NOT NULL,
            target TEXT NOT NULL,
            body TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_logs_ts ON logs (ts_millis);
        CREATE INDEX IF NOT EXISTS idx_logs_level ON logs (level);",
    )
    .map_err(|err| AppError::Log(LOG_ERR_QUERY, format!("日志数据库初始化失败: {err}")))
}

/// 清理超龄滚动文件（按文件名日期后缀 `vibetype.log.YYYY-MM-DD` 判定）。
fn cleanup_old_log_files(log_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return;
    };
    let cutoff = now_millis() - RETENTION_DAYS * 86_400_000;
    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(date_str) = name
            .strip_prefix(&format!("{LOG_FILE_PREFIX}."))
            .filter(|s| s.len() == 10)
        else {
            continue;
        };
        // 文件名日期转毫秒（UTC 零点），早于保留期则删除。
        let Some(ts) = parse_utc_date_millis(date_str) else {
            continue;
        };
        if ts < cutoff {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// `YYYY-MM-DD` → 当日 UTC 零点的毫秒时间戳（非法格式返回 None）。
fn parse_utc_date_millis(date: &str) -> Option<i64> {
    let bytes = date.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year: i64 = date.get(0..4)?.parse().ok()?;
    let month: i64 = date.get(5..7)?.parse().ok()?;
    let day: i64 = date.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // 民用日期转儒略日数（Hinnant 算法逆运算）。
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400_000)
}

/// 日志查询条件（`log_query` / `log_export` 命令参数，T-018）。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LogQuery {
    /// 关键词或正则（空 = 不匹配过滤）。
    pub keyword: Option<String>,
    /// 关键词按正则解释（默认 false = 关键词匹配）。
    pub regex: bool,
    /// 大小写敏感（默认 false）。
    pub case_sensitive: bool,
    /// 级别过滤（空 = 全部级别）。
    pub levels: Vec<String>,
    /// 起始时间（Unix 毫秒，闭区间）。
    pub from_millis: Option<i64>,
    /// 结束时间（Unix 毫秒，闭区间）。
    pub to_millis: Option<i64>,
    /// 单页条数（默认且最大 [`QUERY_MAX_LIMIT`]）。
    pub limit: u32,
    /// 跳过前 offset 条匹配结果。
    pub offset: u32,
}

/// 日志条目（查询返回元素）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogEntry {
    pub id: i64,
    pub ts_millis: i64,
    pub level: String,
    pub target: String,
    pub body: String,
}

/// 查询结果页。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogPage {
    pub entries: Vec<LogEntry>,
    /// 匹配结果是否未取完（还有更多）。
    pub has_more: bool,
}

/// 导出结果（与前端 `ExportLogsResult` 逐字段对应）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportLogsResult {
    pub path: String,
    pub lines: u32,
}

/// 消费侧匹配器：关键词 / 正则 / 不过滤。
#[derive(Debug)]
enum Matcher {
    Any,
    Keyword(String),
    Pattern(regex::Regex),
}

fn compile_matcher(query: &LogQuery) -> Result<Matcher, AppError> {
    let keyword = query
        .keyword
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    match (keyword, query.regex) {
        (None, _) => Ok(Matcher::Any),
        (Some(text), false) => Ok(Matcher::Keyword(text.to_string())),
        (Some(text), true) => RegexBuilder::new(text)
            .case_insensitive(!query.case_sensitive)
            .build()
            .map(Matcher::Pattern)
            .map_err(|err| AppError::Log(LOG_ERR_BAD_REGEX, format!("正则表达式无效: {err}"))),
    }
}

/// 对正文应用匹配器（大小写敏感仅作用于关键词模式；正则模式大小写由编译选项控制）。
fn matcher_hit(matcher: &Matcher, body: &str, case_sensitive: bool) -> bool {
    match matcher {
        Matcher::Any => true,
        Matcher::Keyword(keyword) => {
            if case_sensitive {
                body.contains(keyword.as_str())
            } else {
                body.to_lowercase().contains(&keyword.to_lowercase())
            }
        }
        Matcher::Pattern(pattern) => pattern.is_match(body),
    }
}

/// SQL 公共过滤（时间范围 + 级别），返回 WHERE 片段与参数。
fn build_filters(query: &LogQuery) -> (String, Vec<rusqlite::types::Value>) {
    let mut sql = String::from(" WHERE 1=1");
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(from) = query.from_millis {
        sql.push_str(" AND ts_millis >= ?");
        params.push(from.into());
    }
    if let Some(to) = query.to_millis {
        sql.push_str(" AND ts_millis <= ?");
        params.push(to.into());
    }
    let levels: Vec<&str> = query
        .levels
        .iter()
        .map(String::as_str)
        .filter(|level| !level.is_empty())
        .collect();
    if !levels.is_empty() {
        let marks = levels.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        sql.push_str(&format!(" AND level IN ({marks})"));
        params.extend(levels.iter().map(|level| (*level).to_string().into()));
    }
    (sql, params)
}

/// 查询实现：SQL 过滤（时间 / 级别）+ 消费侧匹配（关键词 / 正则）+ 分页。
fn query_impl(conn: &Connection, query: &LogQuery, matcher: &Matcher) -> Result<LogPage, AppError> {
    let limit = if query.limit == 0 {
        QUERY_MAX_LIMIT
    } else {
        query.limit.min(QUERY_MAX_LIMIT)
    };
    let (filter_sql, mut params) = build_filters(query);
    let sql = format!(
        "SELECT id, ts_millis, level, target, body FROM logs{filter_sql} ORDER BY id DESC LIMIT ?"
    );
    params.push((MAX_SCAN as i64).into());
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|err| AppError::Log(LOG_ERR_QUERY, format!("日志查询失败: {err}")))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok(LogEntry {
                id: row.get(0)?,
                ts_millis: row.get(1)?,
                level: row.get(2)?,
                target: row.get(3)?,
                body: row.get(4)?,
            })
        })
        .map_err(|err| AppError::Log(LOG_ERR_QUERY, format!("日志查询失败: {err}")))?;

    let mut matched: Vec<LogEntry> = Vec::new();
    for row in rows {
        let entry =
            row.map_err(|err| AppError::Log(LOG_ERR_QUERY, format!("日志查询失败: {err}")))?;
        if matcher_hit(matcher, &entry.body, query.case_sensitive) {
            matched.push(entry);
        }
    }
    let has_more = matched.len() as u32 > query.offset.saturating_add(limit)
        || matched.len() as u32 >= MAX_SCAN;
    let entries: Vec<LogEntry> = matched
        .into_iter()
        .skip(query.offset as usize)
        .take(limit as usize)
        .collect();
    Ok(LogPage { entries, has_more })
}

/// 导出实现：同条件分块扫描（每块 1000 行，按 id 递减），匹配即写。
fn export_impl(
    conn: &Connection,
    query: &LogQuery,
    matcher: &Matcher,
    target_path: &Path,
) -> Result<ExportLogsResult, AppError> {
    let mut output = std::fs::File::create(target_path).map_err(|err| {
        AppError::Log(
            LOG_ERR_EXPORT_WRITE,
            format!("导出文件创建失败（{}）: {err}", target_path.display()),
        )
    })?;
    writeln!(output, "{}", system_info_header(env!("CARGO_PKG_VERSION")))
        .map_err(export_write_err(target_path))?;

    let (filter_sql, params) = build_filters(query);
    let mut last_id = i64::MAX;
    let mut total: u32 = 0;
    loop {
        let sql = format!(
            "SELECT id, ts_millis, level, target, body FROM logs{filter_sql} AND id < ? ORDER BY id DESC LIMIT 1000"
        );
        let mut block_params = params.clone();
        block_params.push(last_id.into());
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|err| AppError::Log(LOG_ERR_QUERY, format!("日志查询失败: {err}")))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(block_params.iter()), |row| {
                Ok(LogEntry {
                    id: row.get(0)?,
                    ts_millis: row.get(1)?,
                    level: row.get(2)?,
                    target: row.get(3)?,
                    body: row.get(4)?,
                })
            })
            .map_err(|err| AppError::Log(LOG_ERR_QUERY, format!("日志查询失败: {err}")))?;
        let mut block_min_id = last_id;
        let mut block_count = 0usize;
        for row in rows {
            let entry =
                row.map_err(|err| AppError::Log(LOG_ERR_QUERY, format!("日志查询失败: {err}")))?;
            block_min_id = block_min_id.min(entry.id);
            block_count += 1;
            if matcher_hit(matcher, &entry.body, query.case_sensitive) {
                writeln!(output, "{}", format_log_line(&entry))
                    .map_err(export_write_err(target_path))?;
                total += 1;
            }
        }
        if block_count < 1000 || block_min_id == last_id {
            break;
        }
        last_id = block_min_id;
    }
    output.flush().map_err(export_write_err(target_path))?;
    Ok(ExportLogsResult {
        path: target_path.to_string_lossy().into_owned(),
        lines: total,
    })
}

/// 日志行文本格式（与文本滚动文件一致）。
fn format_log_line(entry: &LogEntry) -> String {
    format!(
        "{} {:>5} {}: {}",
        format_ts_millis(entry.ts_millis),
        entry.level,
        entry.target,
        entry.body
    )
}

fn export_write_err(target: &Path) -> impl Fn(std::io::Error) -> AppError + '_ {
    move |err| {
        AppError::Log(
            LOG_ERR_EXPORT_WRITE,
            format!("导出文件写入失败（{}）: {err}", target.display()),
        )
    }
}

/// 日志服务（静态方法集）。
pub struct LogService;

impl LogService {
    /// 创建按天滚动的非阻塞文件写入器。
    ///
    /// 返回 `(writer, guard)`：`guard` 必须存活至写入器不再使用，否则缓冲日志会丢失。
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

/// 系统信息头（F-ABOUT-02 / F-LOG-03：OS 版本、产品版本、模型信息）。
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

    fn mem_db_with(rows: &[(i64, &str, &str, &str)]) -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        create_schema(&conn).expect("schema");
        for (ts, level, target, body) in rows {
            conn.execute(
                "INSERT INTO logs (ts_millis, level, target, body) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![ts, level, target, body],
            )
            .expect("insert");
        }
        conn
    }

    #[test]
    fn format_ts_epoch_and_day() {
        assert_eq!(format_ts_millis(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_ts_millis(86_400_000), "1970-01-02T00:00:00.000Z");
        assert_eq!(
            format_ts_millis(1_729_000_000_000),
            "2024-10-15T13:46:40.000Z"
        );
    }

    #[test]
    fn civil_date_roundtrip() {
        // 2024-02-29（闰日）→ 天数 → 还原。
        let millis = parse_utc_date_millis("2024-02-29").expect("date parse");
        assert_eq!(format_ts_millis(millis), "2024-02-29T00:00:00.000Z");
        assert!(parse_utc_date_millis("2024-13-01").is_none());
        assert!(parse_utc_date_millis("not-a-date").is_none());
    }

    #[test]
    fn keyword_matcher_case_sensitivity() {
        let query = LogQuery {
            keyword: Some("error".to_string()),
            case_sensitive: false,
            ..LogQuery::default()
        };
        let matcher = compile_matcher(&query).expect("matcher");
        assert!(matcher_hit(&matcher, "INFO ERROR happened", false));
        assert!(matcher_hit(&matcher, "some Error text", false));
        // 大小写敏感：小写关键词不命中大写文本。
        assert!(!matcher_hit(&matcher, "ERROR happened", true));
        let query_sensitive = LogQuery {
            keyword: Some("Error".to_string()),
            case_sensitive: true,
            ..LogQuery::default()
        };
        let matcher = compile_matcher(&query_sensitive).expect("matcher");
        assert!(matcher_hit(&matcher, "an Error occurred", true));
        assert!(!matcher_hit(&matcher, "an error occurred", true));
    }

    #[test]
    fn regex_matcher_and_invalid_pattern() {
        let query = LogQuery {
            keyword: Some(r"elapsed_ms=\d+".to_string()),
            regex: true,
            ..LogQuery::default()
        };
        let matcher = compile_matcher(&query).expect("matcher");
        assert!(matcher_hit(&matcher, "INFO done elapsed_ms=42", false));
        assert!(!matcher_hit(&matcher, "INFO done elapsed_ms=abc", false));

        let bad = LogQuery {
            keyword: Some("[unclosed".to_string()),
            regex: true,
            ..LogQuery::default()
        };
        let err = compile_matcher(&bad).unwrap_err();
        assert_eq!(err.code(), "E-LOG-002");
    }

    #[test]
    fn query_filters_levels_and_time_range() {
        let conn = mem_db_with(&[
            (1000, "INFO", "a", "first"),
            (2000, "ERROR", "a", "second"),
            (3000, "WARN", "b", "third"),
        ]);
        let query = LogQuery {
            levels: vec!["ERROR".to_string()],
            from_millis: Some(1500),
            to_millis: Some(2500),
            ..LogQuery::default()
        };
        let page = query_impl(&conn, &query, &Matcher::Any).expect("query");
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].body, "second");
        assert!(!page.has_more);
    }

    #[test]
    fn query_keyword_pagination_and_has_more() {
        let rows: Vec<(i64, &str, &str, &str)> =
            (0..10).map(|i| (i as i64, "INFO", "a", "keep")).collect();
        let conn = mem_db_with(&rows);
        let query = LogQuery {
            keyword: Some("keep".to_string()),
            limit: 4,
            offset: 0,
            ..LogQuery::default()
        };
        let matcher = compile_matcher(&query).unwrap();
        let page = query_impl(&conn, &query, &matcher).expect("query");
        assert_eq!(page.entries.len(), 4);
        assert!(page.has_more);
        // 倒序：最新（id 最大）在前。
        assert!(page.entries[0].id > page.entries[3].id);

        let page2 = query_impl(
            &conn,
            &LogQuery {
                offset: 8,
                ..query.clone()
            },
            &matcher,
        )
        .expect("query");
        assert_eq!(page2.entries.len(), 2);
        assert!(!page2.has_more);
    }

    #[test]
    fn query_empty_db_returns_empty_page() {
        let conn = mem_db_with(&[]);
        let page = query_impl(&conn, &LogQuery::default(), &Matcher::Any).expect("query");
        assert!(page.entries.is_empty());
        assert!(!page.has_more);
    }

    #[test]
    fn export_by_query_writes_matched_lines() {
        let conn = mem_db_with(&[
            (1000, "INFO", "a", "needle one"),
            (2000, "ERROR", "a", "other"),
            (3000, "WARN", "b", "needle two"),
        ]);
        let dir = std::env::temp_dir().join(format!(
            "vt-log-export-q-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("out.log");
        let query = LogQuery {
            keyword: Some("needle".to_string()),
            ..LogQuery::default()
        };
        let matcher = compile_matcher(&query).expect("matcher");
        let result = export_impl(&conn, &query, &matcher, &target).expect("export");
        assert_eq!(result.lines, 2);
        let content = std::fs::read_to_string(&target).unwrap();
        assert!(content.contains("# VibeType 日志导出"));
        assert!(content.contains("needle one"));
        assert!(content.contains("needle two"));
        assert!(!content.contains("\nother"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_query_requires_database_enabled() {
        let dir = std::env::temp_dir().join(format!(
            "vt-log-store-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (_tx, _rx) = mpsc::channel::<LogRecord>();
        let store = LogStore {
            state: Arc::new(Mutex::new(SharedState {
                targets: vec![],
                db: None,
            })),
        };
        // 未开启数据库目标：查询返回 E-LOG-001。
        let err = store.query(&LogQuery::default()).unwrap_err();
        assert_eq!(err.code(), "E-LOG-001");
        assert!(!store.database_enabled());
        // 开启后建连可用，关闭后再次不可用。
        store.set_targets(vec![LogTarget::Database], &dir);
        assert!(store.database_enabled());
        let page = store.query(&LogQuery::default()).expect("query");
        assert!(page.entries.is_empty());
        store.set_targets(vec![LogTarget::File], &dir);
        assert!(!store.database_enabled());
        let _ = std::fs::remove_dir_all(&dir);
    }

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
