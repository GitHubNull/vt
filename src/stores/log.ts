/**
 * 日志中心 store（T-018）：查询条件、结果列表、分页与导出。
 * 搜索前提：配置 log.targets 含 database（后端同时以 E-LOG-001 兜底）。
 */
import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { logExport, logQuery } from '../api/log'
import { AppError, toAppError } from '../api/client'
import type { LogEntry, LogQuery } from '../api/types'
import { useConfigStore } from './config'

/** 单页条数（后端上限 200）。 */
export const PAGE_SIZE = 200

export const useLogStore = defineStore('log', () => {
  const configStore = useConfigStore()

  // ---- 查询条件（PRD 3.13：刷新页面恢复默认） ----
  const keyword = ref('')
  const regex = ref(false)
  const caseSensitive = ref(false)
  const levels = ref<string[]>([])
  const fromLocal = ref<string | null>(null)
  const toLocal = ref<string | null>(null)

  // ---- 结果状态 ----
  const entries = ref<LogEntry[]>([])
  const hasMore = ref(false)
  const loading = ref(false)
  const lastError = ref<AppError | null>(null)
  /** 正则非法等前端可预判的错误（用户提示，不覆盖结果列表）。 */
  const searchHint = ref('')

  /** 数据库记录目标是否启用（搜索前提）。 */
  const dbEnabled = computed(() => configStore.sections.log.targets.includes('database'))

  function buildQuery(offset: number): LogQuery {
    return {
      keyword: keyword.value.trim() ? keyword.value.trim() : null,
      regex: regex.value,
      case_sensitive: caseSensitive.value,
      levels: levels.value,
      from_millis: fromLocal.value ? new Date(fromLocal.value).getTime() : null,
      to_millis: toLocal.value ? new Date(toLocal.value).getTime() : null,
      limit: PAGE_SIZE,
      offset,
    }
  }

  /** 执行查询（reset = 清空已有结果重新查第一页）。 */
  async function search(reset = true): Promise<boolean> {
    if (!dbEnabled.value || loading.value) return false
    // 前端预检非法正则：提示且不发起查询（列表保持现状）。
    if (regex.value && keyword.value.trim()) {
      try {
        new RegExp(keyword.value.trim())
      } catch {
        searchHint.value = 'invalid_regex'
        return false
      }
    }
    searchHint.value = ''
    loading.value = true
    try {
      const page = await logQuery(buildQuery(reset ? 0 : entries.value.length))
      if (reset) {
        entries.value = page.entries
      } else {
        entries.value = [...entries.value, ...page.entries]
      }
      hasMore.value = page.has_more
      lastError.value = null
      return true
    } catch (error) {
      lastError.value = toAppError(error)
      return false
    } finally {
      loading.value = false
    }
  }

  /** 加载下一页（追加）。 */
  async function loadMore(): Promise<void> {
    if (hasMore.value) await search(false)
  }

  /** 按当前条件导出到 target_path，返回导出条数；失败返回 null。 */
  async function exportResults(targetPath: string): Promise<number | null> {
    if (!dbEnabled.value) return null
    loading.value = true
    try {
      const result = await logExport(buildQuery(0), targetPath)
      lastError.value = null
      return result.lines
    } catch (error) {
      lastError.value = toAppError(error)
      return null
    } finally {
      loading.value = false
    }
  }

  /** 更新记录目标（至少保留一项由调用方保证；后端 E-CONFIG-004 兜底）。 */
  async function updateTargets(targets: string[]): Promise<boolean> {
    const ok = await configStore.updateSection('log', { targets: targets as ('file' | 'database')[] })
    // 关闭数据库后清空结果，避免展示过期内容。
    if (ok && !configStore.sections.log.targets.includes('database')) {
      entries.value = []
      hasMore.value = false
    }
    return ok
  }

  return {
    keyword,
    regex,
    caseSensitive,
    levels,
    fromLocal,
    toLocal,
    entries,
    hasMore,
    loading,
    lastError,
    searchHint,
    dbEnabled,
    search,
    loadMore,
    exportResults,
    updateTargets,
  }
})
