//! 使用统计服务（T-541，SPEC: doc/specs/v0.5/T-541-usage-stats.md；F-STAT-01、NFR-PR-04）。
//!
//! - **采集**：订阅链路事件广播（`RecordingState` / `PipelineOutput` / `RecordingResult` /
//!   `SceneChanged` / `SessionFinished`），按本地日历日聚合计数（录音次数与时长、上屏
//!   字数、降级 / 失败次数、场景字数分布）；
//! - **口令 / 模板 / AI 计数**：为 v0.5 后续任务（T-51x / T-534）预留公开 record 方法；
//! - **存储**：数据目录 `stats/usage.json`，按天聚合（NFR-R-05 复用 `write_json_atomic`
//!   原子写入），重启后保持；
//! - **隐私（NFR-S-02 / 产品原则 5）**：Schema 只含计数 / 时长 / 字数——文本本体在
//!   计数后即刻丢弃，绝不持久化；无任何网络上报通道（默认零收集，NFR-PR-04）；
//! - **时区**：本地日历日依赖系统时区偏移，直接 FFI 读取（Unix `localtime_r`、
//!   Windows `_localtime64_s` / `_gmtime64_s`），不新增 chrono 依赖。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::error::AppError;
use crate::events::{AppEvent, RecordingResultEvent, RecordingState};
use crate::services::config_store::write_json_atomic;

/// 统计数据文件相对数据目录路径（`stats/usage.json`）。
pub const STATS_REL_PATH: &str = "stats/usage.json";
/// 磁盘文件形态版本号（未来 Schema 演进预留）。
const FILE_VERSION: u32 = 1;

/// 单日本地统计（隐私安全：只含计数 / 时长 / 字数，不含任何文本内容）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DayStats {
    /// 完成的录音次数（取消的录音不计入，PRD 3.10 业务规则 1）。
    pub recordings: u64,
    /// 录音总时长（毫秒，由 RecordingState 时间戳推算）。
    pub recording_ms: u64,
    /// 上屏字数（后处理管线出口文本的字符数，计数后文本本体即刻丢弃）。
    pub chars: u64,
    /// 降级完成次数（剪贴板兜底等，只计数不记录 reason）。
    pub degraded: u64,
    /// 失败次数（只计数不记录错误详情）。
    pub failed: u64,
    /// AI 后处理次数（T-534 AI Stage 经 `record_ai_use` 计数）。
    pub ai_uses: u64,
    /// 编程口令使用次数（键为口令标识，非用户口述文本）。
    pub command_uses: BTreeMap<String, u64>,
    /// 替换词典命中次数（键为词典规则的"错误词"，PRD 3.10 命中 Top 10 数据源）。
    pub dict_hits: BTreeMap<String, u64>,
    /// 提示词模板使用次数（键为模板名）。
    pub template_uses: BTreeMap<String, u64>,
    /// 上屏字数按场景分布（键为场景标识）。
    pub scene_chars: BTreeMap<String, u64>,
}

/// 磁盘文件形态：版本号 + 按天聚合表（键为本地日历日 `YYYY-MM-DD`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct UsageFile {
    version: u32,
    days: BTreeMap<String, DayStats>,
}

impl UsageFile {
    /// 新建文件形态：携带当前版本号（回退空表 / 清空后重写也走此默认）。
    fn empty() -> Self {
        Self {
            version: FILE_VERSION,
            days: BTreeMap::new(),
        }
    }
}

/// 统计查询时间范围（与前端 `StatsRange` 逐字对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatsRange {
    /// 今天。
    Today,
    /// 本周（周一起）。
    Week,
    /// 本月（自然月）。
    Month,
    /// 全部。
    All,
}

/// 范围聚合总计（概览卡片数据源）。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(default)]
pub struct StatsTotals {
    pub recordings: u64,
    pub recording_ms: u64,
    pub chars: u64,
    pub degraded: u64,
    pub failed: u64,
    pub ai_uses: u64,
    pub command_uses: u64,
    pub template_uses: u64,
}

impl StatsRange {
    /// 从字符串解析统计范围（命令层参数校验）。
    pub fn parse_range(name: &str) -> Option<Self> {
        match name {
            "today" => Some(Self::Today),
            "week" => Some(Self::Week),
            "month" => Some(Self::Month),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

/// 单日的趋势点（按天柱状图数据源，日期升序）。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(default)]
pub struct StatsDayPoint {
    pub date: String,
    pub chars: u64,
    pub recordings: u64,
    pub recording_ms: u64,
}

/// 命名计数项（场景分布 / 口令 Top / 模板 Top；`name` 为标识符，非用户内容）。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(default)]
pub struct NamedCount {
    pub name: String,
    pub count: u64,
}

/// 统计查询结果（统计页整体数据源）。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(default)]
pub struct StatsSummary {
    pub totals: StatsTotals,
    /// 范围内逐日序列（日期升序，趋势图）。
    pub days: Vec<StatsDayPoint>,
    /// 场景字数分布（降序）。
    pub scenes: Vec<NamedCount>,
    /// 最常用编程口令 Top 5（降序）。
    pub commands: Vec<NamedCount>,
    /// 最常用 AI 模板 Top 5（降序）。
    pub templates: Vec<NamedCount>,
    /// 替换词典命中 Top 10（降序，PRD 3.10；点击可跳转词典编辑）。
    pub dict_hits: Vec<NamedCount>,
}

/// StatsService：链路事件聚合计数 + 本地持久化 + 范围查询。
///
/// 句柄 `Clone`（内部 `Arc`），命令层经 `AppState` 共享。
#[derive(Clone)]
pub struct StatsService {
    path: Arc<PathBuf>,
    file: Arc<Mutex<UsageFile>>,
    /// 录音开始时刻（Unix 毫秒）：Recording 状态进入时记录，Processing 结算时长。
    recording_started_ms: Arc<Mutex<Option<i64>>>,
    /// 当前场景（`SceneChanged` 维护）：上屏字数按场景归属。
    current_scene: Arc<Mutex<Option<String>>>,
}

impl StatsService {
    /// 加载统计：文件不存在 → 空表；解析失败 → 回退空表（NFR-S-07 雏形：
    /// 不崩溃、可继续用，日志只记录错误不记录内容）。
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join(STATS_REL_PATH);
        let file = match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<UsageFile>(&content) {
                Ok(file) => {
                    info!(days = file.days.len(), "使用统计数据加载成功");
                    file
                }
                Err(err) => {
                    warn!(error = %err, "使用统计数据解析失败，回退为空统计");
                    UsageFile::empty()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => UsageFile::empty(),
            Err(err) => {
                warn!(error = %err, "使用统计数据读取失败，回退为空统计");
                UsageFile::empty()
            }
        };
        Self {
            path: Arc::new(path),
            file: Arc::new(Mutex::new(file)),
            recording_started_ms: Arc::new(Mutex::new(None)),
            current_scene: Arc::new(Mutex::new(None)),
        }
    }

    /// 订阅链路事件广播并持续聚合（main.rs 接线后调用）。
    ///
    /// 广播通道容量 256（tech/04 4.2）：消费过慢触发 `Lagged` 时跳过丢失段继续
    /// 聚合（统计为旁路能力，丢段可接受，但任务循环绝不能退出）。
    pub fn start(self: &Arc<Self>, mut rx: broadcast::Receiver<AppEvent>) {
        let service = self.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => service.apply_event(&event, now_ms()),
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "统计事件消费滞后，跳过丢失段继续聚合");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    /// 编程口令使用计数（T-51x 口令任务经此方法接入，v0.5 预留）。
    /// `command` 为口令标识符（如 `camel_case`），不含用户口述文本。
    #[allow(dead_code)]
    pub fn record_command_use(&self, command: &str) {
        self.add_command_use(command, now_ms());
    }

    /// 提示词模板使用计数（T-534 模板快捷键触发经此方法接入，v0.5 预留）。
    #[allow(dead_code)]
    pub fn record_template_use(&self, name: &str) {
        self.add_template_use(name, now_ms());
    }

    /// AI 后处理次数计数（T-534 AI Stage 经此方法接入，v0.5 预留）。
    #[allow(dead_code)]
    pub fn record_ai_use(&self) {
        self.mutate_day(now_ms(), |day| day.ai_uses += 1);
    }

    /// 替换词典命中计数（替换词典 Stage 命中规则时调用，v0.5 预留）。
    /// `wrong` 为词典规则键（用户在词典中维护的"错误词"，PRD 3.10 要求展示命中排行）。
    #[allow(dead_code)]
    pub fn record_dict_hit(&self, wrong: &str) {
        let key = wrong.to_string();
        self.mutate_day(now_ms(), move |day| {
            *day.dict_hits.entry(key).or_default() += 1;
        });
    }

    /// 范围查询（统计页数据源；聚合在 `spawn_blocking` 中执行）。
    pub fn summary(&self, range: StatsRange) -> StatsSummary {
        let today = day_key(now_ms(), local_offset_seconds());
        self.summary_for(range, &today)
    }

    /// 清空全部统计数据（PRD 3.10：需前端二次确认后调用）。
    pub fn clear(&self) -> Result<(), AppError> {
        let mut file = self.file.lock().map_err(|_| poisoned())?;
        file.days.clear();
        write_json_atomic(&self.path, &*file)?;
        info!("使用统计数据已清空");
        Ok(())
    }

    /// 事件聚合适配：按事件类型更新对应计数。
    ///
    /// 隐私（NFR-S-02）：`PipelineOutput` / `RecordingResult` 携带的文本与原因
    /// 只用于内存计数，绝不落入统计文件与日志。
    fn apply_event(&self, event: &AppEvent, at_ms: i64) {
        match event {
            AppEvent::RecordingState { state, .. } => self.on_recording_state(*state, at_ms),
            AppEvent::PipelineOutput { text, .. } => {
                let len = text.chars().count() as u64;
                let scene = self
                    .current_scene
                    .lock()
                    .ok()
                    .and_then(|guard| (*guard).clone());
                self.mutate_day(at_ms, move |day| {
                    day.chars += len;
                    if let Some(scene) = scene {
                        *day.scene_chars.entry(scene).or_default() += len;
                    }
                });
            }
            AppEvent::RecordingResult(RecordingResultEvent::Degraded { .. }) => {
                self.mutate_day(at_ms, |day| day.degraded += 1);
            }
            AppEvent::RecordingResult(RecordingResultEvent::Failed { .. }) => {
                self.mutate_day(at_ms, |day| day.failed += 1);
            }
            AppEvent::SceneChanged { scene } => {
                if let Ok(mut guard) = self.current_scene.lock() {
                    *guard = Some(scene.clone());
                }
            }
            AppEvent::SessionFinished { .. } => {
                // 状态机收尾兜底：异常路径下清掉悬挂的录音起点。
                if let Ok(mut guard) = self.recording_started_ms.lock() {
                    *guard = None;
                }
            }
            _ => {}
        }
    }

    /// 录音状态机聚合：Recording 记录起点；Processing 结算次数与时长；
    /// Cancelled 与其他状态清理起点（取消的录音不计入，PRD 3.10 业务规则 1）。
    fn on_recording_state(&self, state: RecordingState, at_ms: i64) {
        match state {
            RecordingState::Recording => {
                if let Ok(mut guard) = self.recording_started_ms.lock() {
                    *guard = Some(at_ms);
                }
            }
            RecordingState::Processing => {
                let start = self
                    .recording_started_ms
                    .lock()
                    .ok()
                    .and_then(|mut guard| guard.take());
                if let Some(start) = start {
                    // 系统时钟回拨等异常下时长不为负。
                    let duration = (at_ms - start).max(0) as u64;
                    self.mutate_day(at_ms, |day| {
                        day.recordings += 1;
                        day.recording_ms += duration;
                    });
                }
            }
            RecordingState::Cancelled | RecordingState::Idle | RecordingState::Warming => {
                if let Ok(mut guard) = self.recording_started_ms.lock() {
                    *guard = None;
                }
            }
        }
    }

    /// 编程口令计数（内部：注入时间戳便于单测跨天边界）。
    fn add_command_use(&self, command: &str, at_ms: i64) {
        let name = command.to_string();
        self.mutate_day(at_ms, move |day| {
            *day.command_uses.entry(name).or_default() += 1;
        });
    }

    /// 提示词模板计数（内部：注入时间戳便于单测跨天边界）。
    fn add_template_use(&self, name: &str, at_ms: i64) {
        let key = name.to_string();
        self.mutate_day(at_ms, move |day| {
            *day.template_uses.entry(key).or_default() += 1;
        });
    }

    /// 更新当天聚合并原子落盘；持久化失败仅告警（统计不得影响核心链路）。
    fn mutate_day(&self, at_ms: i64, mutate: impl FnOnce(&mut DayStats)) {
        let key = day_key(at_ms, local_offset_seconds());
        let mut file = match self.file.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        let day = file.days.entry(key).or_default();
        mutate(day);
        if let Err(err) = write_json_atomic(&self.path, &*file) {
            warn!(error = %err, "使用统计持久化失败");
        }
    }

    /// 范围聚合（`today` 键注入便于单测；生产经 `summary` 取真实今天）。
    fn summary_for(&self, range: StatsRange, today: &str) -> StatsSummary {
        let bounds = range_bounds(range, today);
        let file = match self.file.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => return StatsSummary::default(),
        };
        let mut summary = StatsSummary::default();
        let mut scenes: BTreeMap<String, u64> = BTreeMap::new();
        let mut commands: BTreeMap<String, u64> = BTreeMap::new();
        let mut templates: BTreeMap<String, u64> = BTreeMap::new();
        let mut dict_hits: BTreeMap<String, u64> = BTreeMap::new();
        for (date, day) in &file.days {
            if let Some((from, to)) = &bounds {
                if date < from || date > to {
                    continue;
                }
            }
            summary.totals.recordings += day.recordings;
            summary.totals.recording_ms += day.recording_ms;
            summary.totals.chars += day.chars;
            summary.totals.degraded += day.degraded;
            summary.totals.failed += day.failed;
            summary.totals.ai_uses += day.ai_uses;
            summary.totals.command_uses += day.command_uses.values().sum::<u64>();
            summary.totals.template_uses += day.template_uses.values().sum::<u64>();
            summary.days.push(StatsDayPoint {
                date: date.clone(),
                chars: day.chars,
                recordings: day.recordings,
                recording_ms: day.recording_ms,
            });
            for (scene, chars) in &day.scene_chars {
                *scenes.entry(scene.clone()).or_default() += chars;
            }
            for (name, count) in &day.command_uses {
                *commands.entry(name.clone()).or_default() += count;
            }
            for (name, count) in &day.template_uses {
                *templates.entry(name.clone()).or_default() += count;
            }
            for (wrong, count) in &day.dict_hits {
                *dict_hits.entry(wrong.clone()).or_default() += count;
            }
        }
        // 场景分布不截断（占比图需要全集）；口令 / 模板按 PRD 3.10 只取 Top 5，
        // 词典命中取 Top 10。
        summary.scenes = top_named(map_to_named(scenes), usize::MAX);
        summary.commands = top_named(map_to_named(commands), 5);
        summary.templates = top_named(map_to_named(templates), 5);
        summary.dict_hits = top_named(map_to_named(dict_hits), 10);
        summary
    }
}

/// 互斥锁中毒错误。
fn poisoned() -> AppError {
    AppError::Config(5, "统计数据状态损坏".to_string())
}

/// 当前 Unix 毫秒。
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// 本地时区偏移（秒）：Unix 走 `localtime_r.tm_gmtoff`（glibc / macOS / musl
/// 布局一致）；Windows 走 `_localtime64_s` 与 `_gmtime64_s` 的字段差值。
/// 纯本地计算，无任何网络访问；读取失败回退 UTC（仅影响按天归属，不丢数据）。
#[cfg(unix)]
fn local_offset_seconds() -> i32 {
    use std::os::raw::{c_char, c_long};

    #[repr(C)]
    struct Tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
        tm_gmtoff: c_long,
        tm_zone: *const c_char,
    }

    extern "C" {
        fn localtime_r(timep: *const c_long, result: *mut Tm) -> *mut Tm;
    }

    let now = now_ms() / 1000;
    let mut tm = std::mem::MaybeUninit::<Tm>::uninit();
    let result = unsafe { localtime_r(&now, tm.as_mut_ptr()) };
    if result.is_null() {
        return 0;
    }
    let tm = unsafe { tm.assume_init() };
    tm.tm_gmtoff as i32
}

/// Windows 实现：msvcrt 的 `struct tm` 不含 gmtoff，用本地 / UTC 分解值求差。
#[cfg(windows)]
fn local_offset_seconds() -> i32 {
    #[repr(C)]
    struct Tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
    }

    extern "C" {
        fn _localtime64_s(tm: *mut Tm, time: *const i64) -> i32;
        fn _gmtime64_s(tm: *mut Tm, time: *const i64) -> i32;
    }

    fn day_seconds(tm: &Tm) -> i32 {
        tm.tm_yday * 86_400 + tm.tm_hour * 3_600 + tm.tm_min * 60 + tm.tm_sec
    }

    let now = now_ms() / 1000;
    let mut local = std::mem::MaybeUninit::<Tm>::uninit();
    let mut utc = std::mem::MaybeUninit::<Tm>::uninit();
    let rc_local = unsafe { _localtime64_s(local.as_mut_ptr(), &now) };
    let rc_utc = unsafe { _gmtime64_s(utc.as_mut_ptr(), &now) };
    if rc_local != 0 || rc_utc != 0 {
        return 0;
    }
    let local = unsafe { local.assume_init() };
    let utc = unsafe { utc.assume_init() };
    day_seconds(&local) - day_seconds(&utc)
}

/// 纪元日 → 公历年月日（Howard Hinnant 算法，纯算术，无时区依赖）。
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (y as i32 + i32::from(m <= 2), m, d)
}

/// 公历年月日 → 纪元日（`civil_from_days` 的逆运算）。
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 本地日历日键 `YYYY-MM-DD`（统计按天聚合的桶键；`ms` 为 Unix 毫秒）。
fn day_key(ms: i64, offset_seconds: i32) -> String {
    let local_seconds = ms.div_euclid(1000) + i64::from(offset_seconds);
    let (y, m, d) = civil_from_days(local_seconds.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// 范围 → 闭区间日期键（`YYYY-MM-DD` 字典序比较即日期比较）；`All` 返回 `None`。
fn range_bounds(range: StatsRange, today: &str) -> Option<(String, String)> {
    let (y, m, d) = parse_day_key(today)?;
    match range {
        StatsRange::Today => Some((today.to_string(), today.to_string())),
        StatsRange::Week => {
            // 周一为一周起点（ISO）；1970-01-01（纪元日 0）为周四。
            let days = days_from_civil(y, m, d);
            let dow_monday_zero = (days + 3).rem_euclid(7);
            let (sy, sm, sd) = civil_from_days(days - dow_monday_zero);
            Some((format!("{sy:04}-{sm:02}-{sd:02}"), today.to_string()))
        }
        StatsRange::Month => Some((format!("{y:04}-{m:02}-01"), today.to_string())),
        StatsRange::All => None,
    }
}

/// 解析 `YYYY-MM-DD`；非法输入返回 `None`（调用方按最窄范围兜底）。
fn parse_day_key(key: &str) -> Option<(i32, u32, u32)> {
    let mut parts = key.split('-');
    let y = parts.next()?.parse().ok()?;
    let m = parts.next()?.parse().ok()?;
    let d = parts.next()?.parse().ok()?;
    (parts.next().is_none() && (1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

/// 命名计数聚合排序：按次数降序（同名并列按标识升序），截断前 `n`。
fn top_named(mut list: Vec<NamedCount>, n: usize) -> Vec<NamedCount> {
    list.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    list.truncate(n);
    list
}

/// `BTreeMap` → `NamedCount` 列表。
fn map_to_named(map: BTreeMap<String, u64>) -> Vec<NamedCount> {
    map.into_iter()
        .map(|(name, count)| NamedCount { name, count })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "vt-stats-test-{}-{}-{}",
                name,
                std::process::id(),
                now_ms()
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

    /// 指定本地日历日 00:00 对应的 Unix 毫秒（测试用时间锚点）。
    fn local_midnight_ms(y: i32, m: u32, d: u32) -> i64 {
        (days_from_civil(y, m, d) * 86_400 - i64::from(local_offset_seconds())) * 1000
    }

    fn make_service(dir: &TestDir) -> StatsService {
        StatsService::load(&dir.0)
    }

    fn recording_event(state: RecordingState) -> AppEvent {
        AppEvent::RecordingState { state, mode: None }
    }

    #[test]
    fn civil_date_roundtrip() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(days_from_civil(2000, 2, 29)), (2000, 2, 29));
        assert_eq!(
            civil_from_days(days_from_civil(2026, 10, 11)),
            (2026, 10, 11)
        );
        assert_eq!(
            civil_from_days(days_from_civil(1999, 12, 31)),
            (1999, 12, 31)
        );
    }

    #[test]
    fn day_key_respects_offset_and_crosses_day_boundary() {
        // 1970-01-01 23:00 UTC：东一区已是 1 月 2 日，西一区仍是 1 月 1 日。
        let ms = (86_400 - 3_600) * 1000;
        assert_eq!(day_key(ms, 0), "1970-01-01");
        assert_eq!(day_key(ms, 3_600), "1970-01-02");
        assert_eq!(day_key(ms, -3_600), "1970-01-01");
    }

    #[tokio::test]
    async fn recording_across_midnight_aggregates_into_end_day() {
        let dir = TestDir::new("cross-day");
        let service = make_service(&dir);
        let start = local_midnight_ms(2026, 10, 11) - 30_000; // 10-10 23:59:30
        let end = local_midnight_ms(2026, 10, 11) + 30_000; // 10-11 00:00:30
        service.apply_event(&recording_event(RecordingState::Recording), start);
        service.apply_event(&recording_event(RecordingState::Processing), end);

        let summary = service.summary_for(StatsRange::All, "2026-10-11");
        assert_eq!(summary.totals.recordings, 1);
        assert_eq!(summary.totals.recording_ms, 60_000);
        assert_eq!(summary.days.len(), 1);
        assert_eq!(summary.days[0].date, "2026-10-11");
    }

    #[tokio::test]
    async fn cancelled_recording_not_counted() {
        let dir = TestDir::new("cancelled");
        let service = make_service(&dir);
        let base = local_midnight_ms(2026, 10, 11);
        service.apply_event(&recording_event(RecordingState::Recording), base);
        service.apply_event(&recording_event(RecordingState::Cancelled), base + 10_000);
        // 未配对 Processing 也不产生计数。
        service.apply_event(&recording_event(RecordingState::Processing), base + 20_000);

        let summary = service.summary_for(StatsRange::All, "2026-10-11");
        assert_eq!(summary.totals.recordings, 0);
        assert_eq!(summary.totals.recording_ms, 0);
    }

    #[tokio::test]
    async fn pipeline_output_counts_chars_and_never_persists_text() {
        let dir = TestDir::new("privacy");
        let service = make_service(&dir);
        let base = local_midnight_ms(2026, 10, 11);
        let text = "我的秘密口令内容 abc";
        service.apply_event(
            &AppEvent::PipelineOutput {
                session_id: 1,
                text: text.to_string(),
            },
            base,
        );
        service.apply_event(
            &AppEvent::RecordingResult(RecordingResultEvent::Degraded {
                reason: "clipboard-fallback".to_string(),
            }),
            base,
        );
        service.apply_event(
            &AppEvent::RecordingResult(RecordingResultEvent::Failed {
                code: "E-INJECT-001".to_string(),
                message: "boom".to_string(),
            }),
            base,
        );

        let summary = service.summary_for(StatsRange::All, "2026-10-11");
        assert_eq!(summary.totals.chars, text.chars().count() as u64);
        assert_eq!(summary.totals.degraded, 1);
        assert_eq!(summary.totals.failed, 1);

        // Schema 隐私核查：统计文件不含任何识别文本 / 原因 / 错误内容。
        let persisted = std::fs::read_to_string(dir.0.join(STATS_REL_PATH)).unwrap();
        assert!(!persisted.contains("秘密"));
        assert!(!persisted.contains("clipboard-fallback"));
        assert!(!persisted.contains("boom"));
        assert!(!persisted.contains("abc"));
    }

    #[tokio::test]
    async fn scene_changed_attributes_chars_by_scene() {
        let dir = TestDir::new("scene");
        let service = make_service(&dir);
        let base = local_midnight_ms(2026, 10, 11);
        service.apply_event(
            &AppEvent::SceneChanged {
                scene: "terminal".to_string(),
            },
            base,
        );
        service.apply_event(
            &AppEvent::PipelineOutput {
                session_id: 1,
                text: "四字内容".to_string(),
            },
            base,
        );

        let summary = service.summary_for(StatsRange::All, "2026-10-11");
        assert_eq!(summary.scenes.len(), 1);
        assert_eq!(summary.scenes[0].name, "terminal");
        assert_eq!(summary.scenes[0].count, 4);
    }

    #[tokio::test]
    async fn named_counters_and_ai_use_rollup() {
        let dir = TestDir::new("named");
        let service = make_service(&dir);
        let base = local_midnight_ms(2026, 10, 11);
        service.add_command_use("camel_case", base);
        service.add_command_use("camel_case", base);
        service.add_command_use("snake_case", base);
        service.add_template_use("重构说明", base);
        service.mutate_day(base, |day| {
            day.ai_uses += 3;
            *day.dict_hits.entry("f二".to_string()).or_default() += 2;
        });

        let summary = service.summary_for(StatsRange::All, "2026-10-11");
        assert_eq!(summary.totals.command_uses, 3);
        assert_eq!(summary.totals.template_uses, 1);
        assert_eq!(summary.totals.ai_uses, 3);
        assert_eq!(summary.commands.len(), 2);
        assert_eq!(summary.commands[0].name, "camel_case");
        assert_eq!(summary.commands[0].count, 2);
        assert_eq!(summary.templates[0].name, "重构说明");
        assert_eq!(summary.dict_hits.len(), 1);
        assert_eq!(summary.dict_hits[0].name, "f二");
        assert_eq!(summary.dict_hits[0].count, 2);
    }

    #[tokio::test]
    async fn persistence_roundtrip_across_restart() {
        let dir = TestDir::new("restart");
        let service = make_service(&dir);
        let base = local_midnight_ms(2026, 10, 11);
        service.apply_event(&recording_event(RecordingState::Recording), base);
        service.apply_event(&recording_event(RecordingState::Processing), base + 5_000);
        service.apply_event(
            &AppEvent::PipelineOutput {
                session_id: 1,
                text: "hello".to_string(),
            },
            base,
        );

        // 模拟重启：新实例加载同一目录，统计保持。
        let restarted = make_service(&dir);
        let summary = restarted.summary_for(StatsRange::All, "2026-10-11");
        assert_eq!(summary.totals.recordings, 1);
        assert_eq!(summary.totals.recording_ms, 5_000);
        assert_eq!(summary.totals.chars, 5);
    }

    #[tokio::test]
    async fn range_filtering_today_week_month() {
        let dir = TestDir::new("range");
        let service = make_service(&dir);
        // 2026-10-05（周一）~ 2026-10-11（周日，今天）：三天有数据。
        for (y, m, d, chars) in [(2026, 10, 4, 100), (2026, 10, 5, 200), (2026, 10, 11, 300)] {
            let base = local_midnight_ms(y, m, d);
            service.mutate_day(base, |day| day.chars += chars);
        }

        let today = service.summary_for(StatsRange::Today, "2026-10-11");
        assert_eq!(today.totals.chars, 300);
        let week = service.summary_for(StatsRange::Week, "2026-10-11");
        assert_eq!(week.totals.chars, 500);
        assert_eq!(week.days.len(), 2);
        let month = service.summary_for(StatsRange::Month, "2026-10-11");
        assert_eq!(month.totals.chars, 600);
        let all = service.summary_for(StatsRange::All, "2026-10-11");
        assert_eq!(all.totals.chars, 600);
        assert_eq!(all.days.len(), 3);
    }

    #[tokio::test]
    async fn top_lists_sorted_and_capped() {
        let dir = TestDir::new("top");
        let service = make_service(&dir);
        let base = local_midnight_ms(2026, 10, 11);
        for (name, times) in [("a", 1), ("b", 9), ("c", 5), ("d", 7), ("e", 3), ("f", 8)] {
            for _ in 0..times {
                service.add_command_use(name, base);
            }
        }
        let summary = service.summary_for(StatsRange::All, "2026-10-11");
        assert_eq!(summary.commands.len(), 5);
        assert_eq!(summary.commands[0].name, "b");
        assert_eq!(summary.commands[1].name, "f");
    }

    #[tokio::test]
    async fn clear_empties_persisted_stats() {
        let dir = TestDir::new("clear");
        let service = make_service(&dir);
        let base = local_midnight_ms(2026, 10, 11);
        service.mutate_day(base, |day| day.chars += 42);
        service.clear().unwrap();

        let restarted = make_service(&dir);
        let summary = restarted.summary_for(StatsRange::All, "2026-10-11");
        assert_eq!(summary.totals.chars, 0);
        assert!(summary.days.is_empty());
    }

    #[tokio::test]
    async fn corrupted_file_falls_back_to_empty() {
        let dir = TestDir::new("corrupt");
        let stats_dir = dir.0.join("stats");
        std::fs::create_dir_all(&stats_dir).unwrap();
        std::fs::write(stats_dir.join("usage.json"), "{ broken !!!").unwrap();
        let service = make_service(&dir);
        let summary = service.summary_for(StatsRange::All, "2026-10-11");
        assert_eq!(summary.totals.chars, 0);
    }
}
