/**
 * 使用统计命令封装（T-541 契约，src-tauri/src/commands/stats.rs）。
 * 命令名与参数名与后端逐字一致（tech/03 3.5）。
 */
import { invoke } from '@tauri-apps/api/core'
import type { StatsRangeOption, StatsSummary } from './types'

/** 范围查询统计（统计页数据源；range 取值 today / week / month / all）。 */
export async function statsSummary(range: StatsRangeOption): Promise<StatsSummary> {
  return invoke<StatsSummary>('stats_summary', { range })
}

/** 清空全部统计数据（调用前须二次确认，PRD 3.10）。 */
export async function statsClear(): Promise<void> {
  await invoke('stats_clear')
}
