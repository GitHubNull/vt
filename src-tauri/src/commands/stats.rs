//! 使用统计命令层（T-541，F-STAT-01；前端封装 `src/api/stats.ts` 逐字对应）。
//!
//! 命令名与参数名为前后端契约（tech/03 3.5），修改须同步前端。

use tauri::State;

use crate::error::AppError;
use crate::services::usage_stats::{StatsRange, StatsSummary};
use crate::state::AppState;

/// 范围参数非法的错误编号（E-CONFIG-004 语义沿用：参数值不合法）。
const ERR_BAD_RANGE: u32 = 4;

/// 统计范围查询（统计页数据源；聚合为同步 CPU 操作，放阻塞线程池执行）。
#[tauri::command]
pub async fn stats_summary(
    state: State<'_, AppState>,
    range: String,
) -> Result<StatsSummary, AppError> {
    let range = StatsRange::parse_range(&range).ok_or_else(|| {
        AppError::Config(
            ERR_BAD_RANGE,
            "统计范围须为 today / week / month / all".to_string(),
        )
    })?;
    let stats = state.stats.clone();
    tauri::async_runtime::spawn_blocking(move || stats.summary(range))
        .await
        .map_err(|_| AppError::Config(5, "统计聚合任务执行失败".to_string()))
}

/// 清空全部统计数据（前端须二次确认后调用，PRD 3.10）。
#[tauri::command]
pub async fn stats_clear(state: State<'_, AppState>) -> Result<(), AppError> {
    state.stats.clear()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_range_parse_accepts_all_variants() {
        assert!(StatsRange::parse_range("today").is_some());
        assert!(StatsRange::parse_range("week").is_some());
        assert!(StatsRange::parse_range("month").is_some());
        assert!(StatsRange::parse_range("all").is_some());
        assert!(StatsRange::parse_range("year").is_none());
        assert!(StatsRange::parse_range("").is_none());
    }
}
