/**
 * 模型管理 store（T-014，F-SET-06）：模型清单镜像 + 下载进度 + 切换 / 删除编排。
 * 进度经 model:progress 事件驱动；config:changed(asr) 与 asr:notice 触发刷新。
 */
import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import {
  cancelDownload,
  deleteModel,
  downloadModel,
  listModels,
  setActiveModel,
} from '../api/asr'
import { AppError, toAppError } from '../api/client'
import { EVENTS } from '../api/events'
import type {
  AsrNoticePayload,
  ConfigChangedPayload,
  ModelDownloadProgressPayload,
  ModelInfo,
} from '../api/types'

/** 模型卡片状态（派生自 ModelInfo + 下载进度事件）。 */
export type ModelCardStatus = 'active' | 'downloading' | 'downloaded' | 'invalid' | 'missing'

export const useAsrStore = defineStore('asr', () => {
  const models = ref<ModelInfo[]>([])
  const loaded = ref(false)
  const loading = ref(false)
  /** 最近一次操作失败（用户提示：message + 错误编号 code）。 */
  const lastError = ref<AppError | null>(null)
  /** 后端提示（如删除激活模型后已回退）。 */
  const notice = ref<string | null>(null)
  const listening = ref(false)

  /** 是否有已下载可用的模型（无可用模型时页面给出下载引导）。 */
  const hasUsableModel = computed(() => models.value.some((m) => m.downloaded))

  async function refresh(): Promise<void> {
    models.value = await listModels()
  }

  /** 首次进入模型管理页时拉取清单。 */
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

  /** 模型卡片状态（页面渲染单一来源）。 */
  function statusOf(model: ModelInfo): ModelCardStatus {
    if (model.active && model.downloaded) return 'active'
    if (model.progress) return 'downloading'
    if (model.invalid) return 'invalid'
    if (model.downloaded) return 'downloaded'
    return 'missing'
  }

  async function download(id: string): Promise<boolean> {
    try {
      lastError.value = null
      await downloadModel(id)
      return true
    } catch (error) {
      lastError.value = toAppError(error)
      await refresh().catch(() => undefined)
      return false
    }
  }

  async function cancel(id: string): Promise<void> {
    try {
      await cancelDownload(id)
    } catch (error) {
      lastError.value = toAppError(error)
    }
    await refresh().catch(() => undefined)
  }

  async function activate(id: string): Promise<boolean> {
    try {
      lastError.value = null
      await setActiveModel(id)
      await refresh()
      return true
    } catch (error) {
      lastError.value = toAppError(error)
      return false
    }
  }

  async function remove(id: string): Promise<boolean> {
    try {
      lastError.value = null
      await deleteModel(id)
      await refresh()
      return true
    } catch (error) {
      lastError.value = toAppError(error)
      return false
    }
  }

  /** 订阅后端事件：下载进度、模型提示、激活模型变更。 */
  async function startListening(): Promise<void> {
    if (listening.value) return
    listening.value = true
    try {
      await listen<ModelDownloadProgressPayload>(EVENTS.modelProgress, (event) => {
        const payload = event.payload
        const model = models.value.find((item) => item.id === payload.model_id)
        if (!model) {
          // 进度先于清单加载到达：拉一次清单。
          if (!loaded.value) void refresh().catch(() => undefined)
          return
        }
        if (payload.status === 'downloading') {
          model.progress = {
            downloaded_bytes: payload.downloaded_bytes,
            total_bytes: payload.total_bytes,
          }
        } else {
          model.progress = null
          // 完成 / 失败 / 取消都以服务端清单为准。
          void refresh().catch(() => undefined)
        }
      })
      await listen<AsrNoticePayload>(EVENTS.asrNotice, (event) => {
        notice.value = event.payload.message
        void refresh().catch(() => undefined)
      })
      await listen<ConfigChangedPayload>(EVENTS.configChanged, (event) => {
        if (event.payload.section === 'asr') {
          void refresh().catch(() => undefined)
        }
      })
    } catch {
      // Webview 环境外（测试）不支持 listen：忽略
      listening.value = false
    }
  }

  return {
    models,
    loaded,
    lastError,
    notice,
    hasUsableModel,
    ensureLoaded,
    refresh,
    statusOf,
    download,
    cancel,
    activate,
    remove,
  }
})
