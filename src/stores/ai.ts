/**
 * AI 模板管理 store（T-533，F-AI-01/F-AI-03）：模板镜像 + 供应商镜像（绑定下拉数据源）
 * + 展开编辑态（draft）+ CRUD 编排。
 *
 * 管理页的状态机集中在此（页面组件仅做渲染与交互转发），使 Vitest 可在 Node 环境
 * 覆盖“管理页”行为：展开 / 新建草稿 / 保存（新建 → create、已有 → update）/
 * 复制内置 / 删除 / 校验门禁。
 */
import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import {
  aiCopyTemplate,
  aiCreateTemplate,
  aiDeleteTemplate,
  aiListProviders,
  aiListTemplates,
  aiUpdateTemplate,
  type AiTemplate,
  type ProviderView,
} from '../api/ai'
import { AppError, toAppError } from '../api/client'

/** 模板名称长度上限（PRD 3.5.1，与后端 MAX_NAME_CHARS 同步）。 */
export const MAX_TEMPLATE_NAME = 20
/** 提示词内容长度上限（PRD 3.5.1，与后端 MAX_PROMPT_CHARS 同步）。 */
export const MAX_TEMPLATE_PROMPT = 10000
/** 超时时间范围与默认值（毫秒，PRD 3.5.1，与后端同步）。 */
export const MIN_TIMEOUT_MS = 1000
export const MAX_TIMEOUT_MS = 60000
export const DEFAULT_TIMEOUT_MS = 5000
/** 超时时间常用值下拉（PRD 3.5.1）。 */
export const TIMEOUT_PRESETS = [3000, 5000, 10000, 30000]

/** 供应商下拉“无（纯转录）”选项的内部值（Select 需要字符串值）。 */
export const NO_PROVIDER = ''

/** 超时时间归一化：空 / 非数字回退默认值，越界收敛到 [1000, 60000]。 */
export function normalizeTimeout(value: number | string): number {
  if (typeof value === 'string' && value.trim() === '') return DEFAULT_TIMEOUT_MS
  const parsed = Math.round(Number(value))
  if (!Number.isFinite(parsed)) return DEFAULT_TIMEOUT_MS
  return Math.min(MAX_TIMEOUT_MS, Math.max(MIN_TIMEOUT_MS, parsed))
}

/** 编辑草稿（与 AiTemplate 的差异：供应商用 '' 表示“无（纯转录）”，便于下拉绑定）。 */
export interface TemplateDraft {
  /** null = 新建（保存时走 create，ID 由后端生成）。 */
  id: string | null
  name: string
  prompt: string
  /** '' = 无（纯转录）。 */
  providerId: string
  /** '' = 未选择。 */
  model: string
  hotkey: string | null
  /** 可编辑下拉的输入可能是字符串，保存时经 normalizeTimeout 归一。 */
  timeoutMs: number | string
  builtin: boolean
}

function toDraft(template: AiTemplate): TemplateDraft {
  return {
    id: template.id,
    name: template.name,
    prompt: template.prompt,
    providerId: template.provider_id ?? NO_PROVIDER,
    model: template.model ?? '',
    hotkey: template.hotkey,
    timeoutMs: template.timeout_ms,
    builtin: template.builtin,
  }
}

export const useAiStore = defineStore('ai', () => {
  /** 模板镜像（内置在前）。 */
  const templates = ref<AiTemplate[]>([])
  /** 供应商镜像（模板绑定下拉数据源；含 available_models）。 */
  const providers = ref<ProviderView[]>([])
  const loaded = ref(false)
  const loading = ref(false)
  const saving = ref(false)
  /** 最近一次操作失败（用户提示：message + 错误编号 code）。 */
  const lastError = ref<AppError | null>(null)
  /** 展开中的模板 ID（null = 全部折叠；新建草稿不占此位）。 */
  const expandedId = ref<string | null>(null)
  /** 编辑草稿（null = 未编辑）。 */
  const draft = ref<TemplateDraft | null>(null)

  /** 保存按钮门禁：必填项齐全（供应商已选时模型必填，PRD 3.5.1）。 */
  const draftValid = computed(() => {
    const current = draft.value
    if (!current) return false
    const name = current.name.trim()
    if (!name || name.length > MAX_TEMPLATE_NAME) return false
    const prompt = current.prompt.trim()
    if (!prompt || current.prompt.length > MAX_TEMPLATE_PROMPT) return false
    if (current.providerId !== NO_PROVIDER && !current.model) return false
    return true
  })

  /** 首次进入管理页：并行拉取模板与供应商镜像。 */
  async function ensureLoaded(): Promise<void> {
    if (loaded.value || loading.value) return
    loading.value = true
    try {
      const [templateList, providerList] = await Promise.all([aiListTemplates(), aiListProviders()])
      templates.value = templateList
      providers.value = providerList
      loaded.value = true
    } catch (error) {
      lastError.value = toAppError(error)
    } finally {
      loading.value = false
    }
  }

  /** 以服务端为准刷新模板镜像（CRUD 成功后调用）。 */
  async function refresh(): Promise<void> {
    templates.value = await aiListTemplates()
  }

  /** 按 ID 取供应商（卡片“供应商 · 模型”标签与下拉显示名用）。 */
  function providerById(id: string): ProviderView | undefined {
    return providers.value.find((provider) => provider.id === id)
  }

  /** 某供应商的模型选项；当前绑定不在清单时保留展示（避免静默丢绑定）。 */
  function modelOptions(providerId: string, currentModel = ''): string[] {
    const provider = providerById(providerId)
    const options = provider ? [...provider.available_models] : []
    if (currentModel && !options.includes(currentModel)) {
      options.unshift(currentModel)
    }
    return options
  }

  /** 展开已有模板进入编辑态（内置模板只读展示 + 复制入口）。 */
  function openEditor(template: AiTemplate): void {
    expandedId.value = template.id
    draft.value = toDraft(template)
  }

  /** 新建草稿（展开在最前，保存后落盘）。 */
  function openCreate(): void {
    expandedId.value = null
    draft.value = {
      id: null,
      name: '',
      prompt: '',
      providerId: NO_PROVIDER,
      model: '',
      hotkey: null,
      timeoutMs: DEFAULT_TIMEOUT_MS,
      builtin: false,
    }
  }

  /** 关闭编辑态（取消未保存修改）。 */
  function closeEditor(): void {
    draft.value = null
  }

  /** 点击卡片头：展开 / 折叠（再点当前展开项收起并放弃草稿）。 */
  function toggleExpand(template: AiTemplate): void {
    if (expandedId.value === template.id) {
      expandedId.value = null
      draft.value = null
      return
    }
    openEditor(template)
  }

  /** 草稿 → 命令载荷。 */
  function toPayload(current: TemplateDraft): AiTemplate {
    const providerId = current.providerId || null
    return {
      id: current.id ?? '',
      name: current.name.trim(),
      prompt: current.prompt.trim(),
      provider_id: providerId,
      model: providerId === null ? null : current.model || null,
      hotkey: current.hotkey,
      timeout_ms: normalizeTimeout(current.timeoutMs),
      builtin: false,
    }
  }

  /** 保存草稿：新建走 create、已有走 update；成功后刷新镜像并收起编辑态。 */
  async function saveDraft(): Promise<boolean> {
    const current = draft.value
    if (!current || saving.value) return false
    saving.value = true
    try {
      const payload = toPayload(current)
      if (current.id === null) {
        await aiCreateTemplate(payload)
      } else {
        await aiUpdateTemplate(payload)
      }
      await refresh()
      expandedId.value = null
      draft.value = null
      lastError.value = null
      return true
    } catch (error) {
      lastError.value = toAppError(error)
      return false
    } finally {
      saving.value = false
    }
  }

  /** 复制模板为自定义；成功后直接进入副本编辑态（对照原型“内置复制后即刻定制”）。 */
  async function copyTemplate(id: string): Promise<boolean> {
    try {
      lastError.value = null
      const copied = await aiCopyTemplate(id)
      await refresh()
      openEditor(copied)
      return true
    } catch (error) {
      lastError.value = toAppError(error)
      return false
    }
  }

  /** 删除模板（内置模板由后端拒绝 → E-CONFIG-004）。 */
  async function removeTemplate(id: string): Promise<boolean> {
    try {
      lastError.value = null
      await aiDeleteTemplate(id)
      if (draft.value?.id === id) draft.value = null
      if (expandedId.value === id) expandedId.value = null
      await refresh()
      return true
    } catch (error) {
      lastError.value = toAppError(error)
      return false
    }
  }

  return {
    templates,
    providers,
    loaded,
    loading,
    saving,
    lastError,
    expandedId,
    draft,
    draftValid,
    ensureLoaded,
    refresh,
    providerById,
    modelOptions,
    openEditor,
    openCreate,
    closeEditor,
    toggleExpand,
    saveDraft,
    copyTemplate,
    removeTemplate,
  }
})
