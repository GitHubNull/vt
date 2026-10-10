/**
 * 日志中心命令（T-018，F-LOG-01 / F-LOG-03）。
 * 后端：src-tauri/src/commands/log.rs（log_query / log_export）。
 */
import { invoke } from '@tauri-apps/api/core'
import type { ExportLogsResult, LogPage, LogQuery } from './types'

/** 按条件查询日志（关键词 / 正则 / 大小写 / 级别 / 时间范围 / 分页）。 */
export async function logQuery(query: LogQuery): Promise<LogPage> {
  return invoke<LogPage>('log_query', { query })
}

/** 按当前查询条件导出匹配日志到 target_path（plugin-dialog 选定）。 */
export async function logExport(query: LogQuery, targetPath: string): Promise<ExportLogsResult> {
  return invoke<ExportLogsResult>('log_export', { query, targetPath })
}
