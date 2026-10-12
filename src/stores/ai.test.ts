import { describe, it, expect, vi, beforeEach } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

import {
  DEFAULT_TIMEOUT_MS,
  MAX_TEMPLATE_NAME,
  NO_PROVIDER,
  normalizeTimeout,
  useAiStore,
} from './ai'
import type { AiTemplate, ProviderView } from '../api/ai'

const template = (over: Partial<AiTemplate> = {}): AiTemplate => ({
  id: 'tpl-1',
  name: '我的模板',
  prompt: '请帮我整理：{文本}',
  provider_id: null,
  model: null,
  hotkey: null,
  timeout_ms: 5000,
  builtin: false,
  ...over,
})

const builtin = (over: Partial<AiTemplate> = {}): AiTemplate =>
  template({ id: 'correction', name: '纠错模式', builtin: true, ...over })

const provider = (over: Partial<ProviderView> = {}): ProviderView => ({
  id: 'deepseek',
  name: 'DeepSeek',
  protocol: 'openai_compatible',
  base_url: 'https://api.example.com/v1',
  enabled: false,
  model: null,
  available_models: ['deepseek-chat'],
  local: false,
  key_required: true,
  key_set: false,
  recommended: false,
  ...over,
})

/** 按命令名分发 invoke mock。 */
function mockCommands(handlers: Record<string, (args: unknown) => unknown>) {
  invokeMock.mockImplementation((command: string, args?: unknown) => {
    const handler = handlers[command]
    if (!handler) return Promise.resolve(undefined)
    return Promise.resolve(handler(args))
  })
}

describe('stores/ai（T-533 模板管理页状态）', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    setActivePinia(createPinia())
  })

  it('normalizeTimeout：空值回退默认、越界收敛、取整', () => {
    expect(normalizeTimeout('')).toBe(DEFAULT_TIMEOUT_MS)
    expect(normalizeTimeout('abc')).toBe(DEFAULT_TIMEOUT_MS)
    expect(normalizeTimeout(200)).toBe(1000)
    expect(normalizeTimeout(999999)).toBe(60000)
    expect(normalizeTimeout('8000')).toBe(8000)
    expect(normalizeTimeout(4999.6)).toBe(5000)
  })

  it('ensureLoaded 并行拉取模板与供应商镜像', async () => {
    mockCommands({
      ai_list_templates: () => [builtin(), template()],
      ai_list_providers: () => [provider()],
    })
    const store = useAiStore()
    await store.ensureLoaded()

    expect(store.loaded).toBe(true)
    expect(store.templates).toHaveLength(2)
    expect(store.templates[0].builtin).toBe(true)
    expect(store.providers).toHaveLength(1)
    expect(store.providers[0].available_models).toEqual(['deepseek-chat'])
  })

  it('ensureLoaded 失败：记录带编号错误且不置 loaded', async () => {
    invokeMock.mockRejectedValue('{"code":"E-CONFIG-001","message":"读取模板文件失败"}')
    const store = useAiStore()
    await store.ensureLoaded()

    expect(store.loaded).toBe(false)
    expect(store.lastError?.code).toBe('E-CONFIG-001')
  })

  it('展开 / 折叠：draft 随展开项生成，再点收起并放弃草稿', () => {
    const store = useAiStore()
    const item = builtin()
    store.toggleExpand(item)
    expect(store.expandedId).toBe('correction')
    expect(store.draft?.name).toBe('纠错模式')
    expect(store.draft?.builtin).toBe(true)
    expect(store.draft?.providerId).toBe(NO_PROVIDER)

    store.toggleExpand(item)
    expect(store.expandedId).toBeNull()
    expect(store.draft).toBeNull()
  })

  it('openCreate：新草稿默认值；draftValid 门禁（必填项与供应商模型联动）', () => {
    const store = useAiStore()
    store.openCreate()
    expect(store.draft?.id).toBeNull()
    expect(store.draft?.timeoutMs).toBe(DEFAULT_TIMEOUT_MS)
    expect(store.draftValid).toBe(false)

    store.draft!.name = '新模板'
    expect(store.draftValid).toBe(false)
    store.draft!.prompt = '提示词'
    expect(store.draftValid).toBe(true)

    // 选择供应商后模型必填（PRD 3.5.1）。
    store.draft!.providerId = 'deepseek'
    expect(store.draftValid).toBe(false)
    store.draft!.model = 'deepseek-chat'
    expect(store.draftValid).toBe(true)

    // 名称超长拒绝（与服务端一致）。
    store.draft!.name = '名'.repeat(MAX_TEMPLATE_NAME + 1)
    expect(store.draftValid).toBe(false)
  })

  it('saveDraft 新建：payload 规整（trim / 无供应商置 null / 超时归一）并刷新镜像', async () => {
    mockCommands({
      ai_create_template: () => template({ id: 'tpl-new', name: '新模板' }),
      ai_list_templates: () => [builtin(), template({ id: 'tpl-new', name: '新模板' })],
    })
    const store = useAiStore()
    store.openCreate()
    store.draft!.name = '  新模板  '
    store.draft!.prompt = '  提示词  '
    store.draft!.timeoutMs = '8000'

    expect(await store.saveDraft()).toBe(true)
    expect(invokeMock).toHaveBeenCalledWith('ai_create_template', {
      template: {
        id: '',
        name: '新模板',
        prompt: '提示词',
        provider_id: null,
        model: null,
        hotkey: null,
        timeout_ms: 8000,
        builtin: false,
      },
    })
    expect(store.draft).toBeNull()
    expect(store.templates).toHaveLength(2)
    expect(store.lastError).toBeNull()
  })

  it('saveDraft 更新：按 ID 走 update 并保留绑定（供应商 · 模型）', async () => {
    mockCommands({
      ai_update_template: () => template({ name: '改名' }),
      ai_list_templates: () => [template({ name: '改名' })],
    })
    const store = useAiStore()
    store.providers = [provider()]
    store.openEditor(
      template({ provider_id: 'deepseek', model: 'deepseek-chat', hotkey: 'CtrlRight' }),
    )
    store.draft!.name = '改名'

    expect(await store.saveDraft()).toBe(true)
    expect(invokeMock).toHaveBeenCalledWith('ai_update_template', {
      template: {
        id: 'tpl-1',
        name: '改名',
        prompt: '请帮我整理：{文本}',
        provider_id: 'deepseek',
        model: 'deepseek-chat',
        hotkey: 'CtrlRight',
        timeout_ms: 5000,
        builtin: false,
      },
    })
  })

  it('saveDraft 失败（名称重复）：记录 E-CONFIG-004 并保留草稿', async () => {
    invokeMock.mockRejectedValue('{"code":"E-CONFIG-004","message":"模板名称与已有模板重复"}')
    const store = useAiStore()
    store.openCreate()
    store.draft!.name = '重复名'
    store.draft!.prompt = 'p'

    expect(await store.saveDraft()).toBe(false)
    expect(store.lastError?.code).toBe('E-CONFIG-004')
    expect(store.draft?.name).toBe('重复名')
  })

  it('copyTemplate：复制内置后进入副本编辑态', async () => {
    mockCommands({
      ai_copy_template: () => template({ id: 'tpl-copy', name: '纠错模式（副本）' }),
      ai_list_templates: () => [builtin(), template({ id: 'tpl-copy', name: '纠错模式（副本）' })],
    })
    const store = useAiStore()
    expect(await store.copyTemplate('correction')).toBe(true)

    expect(invokeMock).toHaveBeenCalledWith('ai_copy_template', { id: 'correction' })
    expect(store.expandedId).toBe('tpl-copy')
    expect(store.draft?.id).toBe('tpl-copy')
    expect(store.draft?.builtin).toBe(false)
  })

  it('removeTemplate：删除后清空同 ID 的编辑态并刷新', async () => {
    mockCommands({
      ai_delete_template: () => undefined,
      ai_list_templates: () => [],
    })
    const store = useAiStore()
    store.openEditor(template())
    expect(await store.removeTemplate('tpl-1')).toBe(true)

    expect(invokeMock).toHaveBeenCalledWith('ai_delete_template', { id: 'tpl-1' })
    expect(store.draft).toBeNull()
    expect(store.expandedId).toBeNull()
    expect(store.templates).toHaveLength(0)
  })

  it('removeTemplate 失败（内置不可删）：记录错误并保留镜像', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'ai_list_templates') return Promise.resolve([builtin()])
      return Promise.reject('{"code":"E-CONFIG-004","message":"内置模板不可删除"}')
    })
    const store = useAiStore()
    store.templates = [builtin()]
    expect(await store.removeTemplate('correction')).toBe(false)
    expect(store.lastError?.code).toBe('E-CONFIG-004')
    expect(store.templates).toHaveLength(1)
  })

  it('modelOptions：来自供应商清单；当前绑定不在清单时保留展示', () => {
    const store = useAiStore()
    store.providers = [provider()]
    expect(store.modelOptions('deepseek')).toEqual(['deepseek-chat'])
    expect(store.modelOptions('deepseek', 'legacy-model')).toEqual([
      'legacy-model',
      'deepseek-chat',
    ])
    expect(store.modelOptions('no-such')).toEqual([])
    expect(store.providerById('deepseek')?.name).toBe('DeepSeek')
  })
})
