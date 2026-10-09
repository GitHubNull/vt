/**
 * 替换词典 store（T-011，F-DICT-01）：规则镜像 + CRUD 编排 + dict:changed 多窗口同步。
 */
import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import {
  dictAddRule,
  dictDeleteRule,
  dictListRules,
  dictToggleRule,
  dictUpdateRule,
} from '../api/dict'
import { AppError, toAppError } from '../api/client'
import { EVENTS } from '../api/events'
import type { DictChangedPayload, ReplaceRule } from '../api/types'

/** 规则上限（PRD 3.4.1 业务规则 1，与后端 MAX_RULES 同步）。 */
export const MAX_RULES = 200

export const useDictStore = defineStore('dict', () => {
  const rules = ref<ReplaceRule[]>([])
  const loaded = ref(false)
  const loading = ref(false)
  const lastError = ref<AppError | null>(null)
  const listening = ref(false)

  /** 是否已达条数上限（添加按钮置灰）。 */
  const isFull = computed(() => rules.value.length >= MAX_RULES)

  async function refresh(): Promise<void> {
    rules.value = await dictListRules()
  }

  /** 首次进入词典页时拉取规则。 */
  async function ensureLoaded(): Promise<void> {
    if (loaded.value || loading.value) return
    loading.value = true
    try {
      await refresh()
      loaded.value = true
    } catch (error) {
      lastError.value = toAppError(error)
    } finally {
      loading.value = false
    }
    void startListening()
  }

  async function addRule(wrong: string, right: string): Promise<ReplaceRule | null> {
    try {
      lastError.value = null
      const rule = await dictAddRule(wrong, right, true)
      // 以服务端广播为准刷新（此处同步刷新保证即时反馈）。
      await refresh()
      return rule
    } catch (error) {
      lastError.value = toAppError(error)
      return null
    }
  }

  async function updateRule(rule: ReplaceRule): Promise<boolean> {
    try {
      lastError.value = null
      await dictUpdateRule(rule)
      await refresh()
      return true
    } catch (error) {
      lastError.value = toAppError(error)
      return false
    }
  }

  async function removeRule(id: number): Promise<boolean> {
    try {
      lastError.value = null
      await dictDeleteRule(id)
      await refresh()
      return true
    } catch (error) {
      lastError.value = toAppError(error)
      return false
    }
  }

  async function toggleRule(id: number, enabled: boolean): Promise<boolean> {
    try {
      lastError.value = null
      await dictToggleRule(id, enabled)
      await refresh()
      return true
    } catch (error) {
      lastError.value = toAppError(error)
      return false
    }
  }

  /** 订阅词典变更（本窗口命令触发外的所有路径：其他窗口 / 服务内部）。 */
  async function startListening(): Promise<void> {
    if (listening.value) return
    listening.value = true
    try {
      await listen<DictChangedPayload>(EVENTS.dictChanged, () => {
        refresh().catch((error: unknown) => {
          lastError.value = toAppError(error)
        })
      })
    } catch {
      // Webview 环境外（测试）不支持 listen：忽略
      listening.value = false
    }
  }

  return {
    rules,
    loaded,
    lastError,
    isFull,
    ensureLoaded,
    refresh,
    addRule,
    updateRule,
    removeRule,
    toggleRule,
  }
})
