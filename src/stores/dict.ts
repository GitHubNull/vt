/**
 * 词典 store（T-011 / T-522，F-DICT-01、F-DICT-02）：
 * 替换规则与专有名词镜像 + CRUD 编排 + dict:changed 多窗口同步。
 */
import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import {
  dictAddRule,
  dictDeleteRule,
  dictExportFile,
  dictImportFile,
  dictListRules,
  dictToggleRule,
  dictUpdateRule,
} from '../api/dict'
import {
  properNounAdd,
  properNounDelete,
  properNounList,
  properNounToggle,
  properNounUpdate,
} from '../api/properNouns'
import { AppError, toAppError } from '../api/client'
import { EVENTS } from '../api/events'
import type { DictChangedPayload, ProperNoun, ReplaceRule } from '../api/types'

/** 规则上限（PRD 3.4.1 业务规则 1，与后端 MAX_RULES 同步）。 */
export const MAX_RULES = 200

/** 专有名词上限（PRD 3.4.2 业务规则 1，与后端 MAX_NOUNS 同步）。 */
export const MAX_NOUNS = 2000

export const useDictStore = defineStore('dict', () => {
  const rules = ref<ReplaceRule[]>([])
  const loaded = ref(false)
  const loading = ref(false)
  const lastError = ref<AppError | null>(null)
  const listening = ref(false)

  /** 专有名词镜像（T-522）。 */
  const nouns = ref<ProperNoun[]>([])
  const nounsLoaded = ref(false)
  const nounsLoading = ref(false)

  /** 是否已达条数上限（添加按钮置灰）。 */
  const isFull = computed(() => rules.value.length >= MAX_RULES)
  /** 专有名词是否已达上限。 */
  const nounsFull = computed(() => nouns.value.length >= MAX_NOUNS)

  async function refresh(): Promise<void> {
    rules.value = await dictListRules()
  }

  async function refreshNouns(): Promise<void> {
    nouns.value = await properNounList()
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

  /** 首次进入专有名词页时拉取条目。 */
  async function ensureNounsLoaded(): Promise<void> {
    if (nounsLoaded.value || nounsLoading.value) return
    nounsLoading.value = true
    try {
      await refreshNouns()
      nounsLoaded.value = true
    } catch (error) {
      lastError.value = toAppError(error)
    } finally {
      nounsLoading.value = false
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

  /** 导出全部规则到 targetPath（plugin-dialog 选定）；失败记录错误并返回 null。 */
  async function exportRulesTo(targetPath: string): Promise<number | null> {
    try {
      lastError.value = null
      return await dictExportFile(targetPath)
    } catch (error) {
      lastError.value = toAppError(error)
      return null
    }
  }

  /** 从 path 导入规则，返回导入条数；失败记录错误并返回 null。 */
  async function importRulesFrom(path: string): Promise<number | null> {
    try {
      lastError.value = null
      const count = await dictImportFile(path)
      // 以服务端广播为准刷新（此处同步刷新保证即时反馈）。
      await refresh()
      return count
    } catch (error) {
      lastError.value = toAppError(error)
      return null
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
        // 专有名词变更同样走 dict:changed（T-522）；未加载过则跳过避免多余拉取。
        if (nounsLoaded.value) {
          refreshNouns().catch((error: unknown) => {
            lastError.value = toAppError(error)
          })
        }
      })
    } catch {
      // Webview 环境外（测试）不支持 listen：忽略
      listening.value = false
    }
  }

  // ---- 专有名词（T-522，F-DICT-02） ----

  async function addNoun(
    term: string,
    aliases: string[],
    note: string,
  ): Promise<ProperNoun | null> {
    try {
      lastError.value = null
      const noun = await properNounAdd(term, aliases, note, true)
      await refreshNouns()
      return noun
    } catch (error) {
      lastError.value = toAppError(error)
      return null
    }
  }

  async function updateNoun(noun: ProperNoun): Promise<boolean> {
    try {
      lastError.value = null
      await properNounUpdate(noun)
      await refreshNouns()
      return true
    } catch (error) {
      lastError.value = toAppError(error)
      return false
    }
  }

  async function removeNoun(id: number): Promise<boolean> {
    try {
      lastError.value = null
      await properNounDelete(id)
      await refreshNouns()
      return true
    } catch (error) {
      lastError.value = toAppError(error)
      return false
    }
  }

  async function toggleNoun(id: number, enabled: boolean): Promise<boolean> {
    try {
      lastError.value = null
      await properNounToggle(id, enabled)
      await refreshNouns()
      return true
    } catch (error) {
      lastError.value = toAppError(error)
      return false
    }
  }

  return {
    rules,
    loaded,
    lastError,
    isFull,
    nouns,
    nounsLoaded,
    nounsFull,
    ensureLoaded,
    ensureNounsLoaded,
    refresh,
    refreshNouns,
    addRule,
    updateRule,
    removeRule,
    toggleRule,
    exportRulesTo,
    importRulesFrom,
    addNoun,
    updateNoun,
    removeNoun,
    toggleNoun,
  }
})
