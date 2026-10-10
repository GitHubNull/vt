/**
 * 配置镜像 store（tech/03 3.4、T-003）：后端 ConfigStore 的前端镜像。
 * 修改 → invoke 写后端 → 成功提交 / 失败回滚并提示；config:changed 事件驱动多窗口同步。
 */
import { defineStore } from 'pinia'
import { ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { getConfig, setConfig } from '../api/config'
import { AppError, toAppError } from '../api/client'
import { EVENTS } from '../api/events'
import type {
  AppConfig,
  ConfigChangedPayload,
  ConfigRecoveredPayload,
  ConfigSection,
  HotkeyErrorPayload,
} from '../api/types'

/**
 * 默认配置（与 Rust 各节 Default 手工同步，src-tauri/src/services/config_store.rs）。
 * 三个系统与数据字段待后端补充（见 api/types.ts TODO）。
 */
export function defaultConfig(): AppConfig {
  return {
    general: {
      language: 'system',
      theme: 'system',
      launch_at_login: false,
      save_voice_dataset: false,
      diagnostic_mode: false,
      float_ball_xy: null,
    },
    hotkey: {
      trigger: 'AltRight',
      mode: 'hold',
      cancel_trigger: 'Escape',
      max_recording_minutes: 5,
      show_float_ball: true,
      float_ball_position: 'cursor',
      sound_hint: false,
    },
    audio: {
      input_device_id: null,
      mute_speaker_while_recording: true,
      keep_warm: true,
    },
    output: {
      smart_number: true,
      filter_fillers: true,
      traditional_chinese: false,
      remove_trailing_punct: true,
      paste_input: true,
      keep_text_in_clipboard: false,
    },
    dict: { enabled: true },
    asr: {
      current_model_id: null,
      models_dir: null,
      download_proxy: null,
      model_source: 'mirror',
    },
    about: { auto_check_update: true },
  }
}

export const useConfigStore = defineStore('config', () => {
  const sections = ref<AppConfig>(defaultConfig())
  const loaded = ref(false)
  const loading = ref(false)
  /** 最近一次操作失败（用户提示：message + 错误编号 code）。 */
  const lastError = ref<AppError | null>(null)
  /** 非错误的提示事件（如配置损坏已回退）。 */
  const notice = ref<'config_recovered' | null>(null)
  const listening = ref(false)

  async function refresh(): Promise<void> {
    sections.value = await getConfig()
  }

  /** 首次进入设置页时拉取完整配置镜像。 */
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

  /**
   * 更新一节：乐观提交，写盘失败回滚并记录错误（错误编号展示给用户）。
   * 返回是否成功。
   */
  async function updateSection<S extends ConfigSection>(
    section: S,
    value: AppConfig[S],
  ): Promise<boolean> {
    const previous = sections.value[section]
    sections.value[section] = value
    try {
      await setConfig(section, value)
      lastError.value = null
      return true
    } catch (error) {
      sections.value[section] = previous
      lastError.value = toAppError(error)
      return false
    }
  }

  /** 订阅后端事件：多窗口配置同步与错误提示。 */
  async function startListening(): Promise<void> {
    if (listening.value) return
    listening.value = true
    try {
      await listen<ConfigChangedPayload>(EVENTS.configChanged, () => {
        // 其他窗口修改了配置：重新拉取镜像（失败时下次操作会再次同步）
        refresh().catch((error: unknown) => {
          lastError.value = toAppError(error)
        })
      })
      await listen<ConfigRecoveredPayload>(EVENTS.configRecovered, () => {
        notice.value = 'config_recovered'
        refresh().catch((error: unknown) => {
          lastError.value = toAppError(error)
        })
      })
      await listen<HotkeyErrorPayload>(EVENTS.hotkeyError, (event) => {
        lastError.value = new AppError(event.payload.code, event.payload.message)
      })
    } catch {
      // Webview 环境外（测试）不支持 listen：忽略
      listening.value = false
    }
  }

  return {
    sections,
    loaded,
    lastError,
    notice,
    ensureLoaded,
    refresh,
    updateSection,
  }
})
