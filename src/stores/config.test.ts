import { describe, it, expect, vi, beforeEach } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const { invokeMock, listenMock, handlers } = vi.hoisted(() => {
  const invokeMock = vi.fn()
  const handlers = new Map<string, (payload: unknown) => void>()
  const listenMock = vi.fn((event: string, handler: (payload: { payload: unknown }) => void) => {
    handlers.set(event, (payload: unknown) => handler({ payload }))
    return Promise.resolve(() => handlers.delete(event))
  })
  return { invokeMock, listenMock, handlers }
})

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }))

import { useConfigStore, defaultConfig } from './config'
import { EVENTS } from '../api/events'

describe('stores/config（T-003 useConfigStore）', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    handlers.clear()
    setActivePinia(createPinia())
  })

  it('ensureLoaded 拉取完整配置镜像', async () => {
    const config = defaultConfig()
    config.output.smart_number = false
    invokeMock.mockResolvedValue(config)

    const store = useConfigStore()
    await store.ensureLoaded()

    expect(invokeMock).toHaveBeenCalledWith('get_config')
    expect(store.loaded).toBe(true)
    expect(store.sections.output.smart_number).toBe(false)
  })

  it('updateSection 成功：invoke 写盘并提交', async () => {
    invokeMock.mockResolvedValue(defaultConfig())
    const store = useConfigStore()
    await store.ensureLoaded()
    invokeMock.mockClear()

    const next = { ...store.sections.audio, keep_warm: false }
    const ok = await store.updateSection('audio', next)

    expect(ok).toBe(true)
    expect(invokeMock).toHaveBeenCalledWith('set_config', { section: 'audio', value: next })
    expect(store.sections.audio.keep_warm).toBe(false)
    expect(store.lastError).toBeNull()
  })

  it('updateSection 失败：回滚镜像并记录带编号的错误', async () => {
    invokeMock.mockResolvedValue(defaultConfig())
    const store = useConfigStore()
    await store.ensureLoaded()
    invokeMock.mockClear()

    invokeMock.mockRejectedValue('{"code":"E-HOTKEY-001","message":"快捷键被占用"}')
    const previous = store.sections.hotkey.trigger
    const ok = await store.updateSection('hotkey', { ...store.sections.hotkey, trigger: 'F2' })

    expect(ok).toBe(false)
    // 失败回滚：配置恢复原值
    expect(store.sections.hotkey.trigger).toBe(previous)
    expect(store.lastError?.code).toBe('E-HOTKEY-001')
    expect(store.lastError?.message).toBe('快捷键被占用')
  })

  it('config:changed 事件触发重新拉取（多窗口同步）', async () => {
    invokeMock.mockResolvedValue(defaultConfig())
    const store = useConfigStore()
    await store.ensureLoaded()
    invokeMock.mockClear()

    const changed = defaultConfig()
    changed.general.theme = 'light'
    invokeMock.mockResolvedValue(changed)

    const handler = handlers.get(EVENTS.configChanged)
    expect(handler).toBeDefined()
    handler?.({ type: 'config_changed', section: 'general' })
    await vi.waitFor(() => expect(store.sections.general.theme).toBe('light'))
    expect(invokeMock).toHaveBeenCalledWith('get_config')
  })

  it('hotkey:error 事件记录错误编号提示', async () => {
    invokeMock.mockResolvedValue(defaultConfig())
    const store = useConfigStore()
    await store.ensureLoaded()

    handlers.get(EVENTS.hotkeyError)?.({
      type: 'hotkey_error',
      code: 'E-HOTKEY-001',
      message: '快捷键被占用',
    })

    expect(store.lastError?.code).toBe('E-HOTKEY-001')
  })
})
