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

import { useDictStore, MAX_RULES } from './dict'
import { EVENTS } from '../api/events'
import type { ReplaceRule } from '../api/types'

const rule = (id: number, wrong: string, right: string, enabled = true): ReplaceRule => ({
  id,
  wrong,
  right,
  enabled,
})

describe('stores/dict（T-011 替换词典）', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    handlers.clear()
    setActivePinia(createPinia())
  })

  it('ensureLoaded 拉取规则镜像', async () => {
    invokeMock.mockResolvedValue([rule(1, 'f二', 'F2')])
    const store = useDictStore()
    await store.ensureLoaded()

    expect(invokeMock).toHaveBeenCalledWith('dict_list_rules')
    expect(store.loaded).toBe(true)
    expect(store.rules).toHaveLength(1)
    expect(store.rules[0].right).toBe('F2')
  })

  it('addRule 成功：invoke 新增命令并刷新镜像', async () => {
    invokeMock.mockResolvedValue([rule(1, 'f二', 'F2')])
    const store = useDictStore()
    await store.ensureLoaded()
    invokeMock.mockClear()

    invokeMock.mockResolvedValue([rule(1, 'f二', 'F2'), rule(2, '我的邮箱', 'dev@example.com')])
    invokeMock.mockResolvedValueOnce(rule(2, '我的邮箱', 'dev@example.com'))
    const created = await store.addRule('我的邮箱', 'dev@example.com')

    expect(created?.id).toBe(2)
    expect(invokeMock).toHaveBeenCalledWith('dict_add_rule', {
      wrong: '我的邮箱',
      right: 'dev@example.com',
      enabled: true,
    })
    expect(store.rules).toHaveLength(2)
    expect(store.lastError).toBeNull()
  })

  it('addRule 失败（重复错误词）：记录带编号错误并返回 null', async () => {
    invokeMock.mockResolvedValue([rule(1, 'f二', 'F2')])
    const store = useDictStore()
    await store.ensureLoaded()
    invokeMock.mockClear()

    invokeMock.mockRejectedValue('{"code":"E-CONFIG-002","message":"错误词已存在，请勿重复添加"}')
    const created = await store.addRule('f二', '其他')

    expect(created).toBeNull()
    expect(store.lastError?.code).toBe('E-CONFIG-002')
    expect(store.rules).toHaveLength(1)
  })

  it('updateRule / removeRule / toggleRule 调用对应命令并刷新', async () => {
    invokeMock.mockResolvedValue([rule(1, 'f二', 'F2')])
    const store = useDictStore()
    await store.ensureLoaded()
    invokeMock.mockClear()

    invokeMock.mockResolvedValue([rule(1, 'f二', 'F2', false)])
    expect(await store.toggleRule(1, false)).toBe(true)
    expect(invokeMock).toHaveBeenCalledWith('dict_toggle_rule', { id: 1, enabled: false })
    expect(store.rules[0].enabled).toBe(false)

    invokeMock.mockResolvedValue([])
    expect(await store.removeRule(1)).toBe(true)
    expect(invokeMock).toHaveBeenCalledWith('dict_delete_rule', { id: 1 })
    expect(store.rules).toHaveLength(0)
  })

  it('removeRule 失败：返回 false 并保留镜像', async () => {
    invokeMock.mockResolvedValue([rule(1, 'f二', 'F2')])
    const store = useDictStore()
    await store.ensureLoaded()
    invokeMock.mockClear()

    invokeMock.mockRejectedValue('{"code":"E-CONFIG-002","message":"规则不存在"}')
    expect(await store.removeRule(99)).toBe(false)
    expect(store.lastError?.code).toBe('E-CONFIG-002')
    expect(store.rules).toHaveLength(1)
  })

  it('达到上限时 isFull 为 true', () => {
    const store = useDictStore()
    expect(store.isFull).toBe(false)
    store.rules = Array.from({ length: MAX_RULES }, (_, i) => rule(i + 1, `词${i}`, `值${i}`))
    expect(store.isFull).toBe(true)
  })

  it('exportRulesTo 传保存路径、返回条数；失败返回 null 并记录错误', async () => {
    invokeMock.mockResolvedValue([rule(1, 'f二', 'F2')])
    const store = useDictStore()
    await store.ensureLoaded()
    invokeMock.mockClear()

    invokeMock.mockResolvedValue(1)
    expect(await store.exportRulesTo('/tmp/vibetype-dict.json')).toBe(1)
    expect(invokeMock).toHaveBeenCalledWith('dict_export_file', {
      targetPath: '/tmp/vibetype-dict.json',
    })

    invokeMock.mockRejectedValue('{"code":"E-UNKNOWN","message":"导出失败"}')
    expect(await store.exportRulesTo('/tmp/vibetype-dict.json')).toBeNull()
    expect(store.lastError?.message).toBe('导出失败')
  })

  it('importRulesFrom 传文件路径、返回条数并刷新镜像；失败返回 null', async () => {
    invokeMock.mockResolvedValue([rule(1, 'f二', 'F2')])
    const store = useDictStore()
    await store.ensureLoaded()
    invokeMock.mockClear()

    invokeMock.mockResolvedValueOnce(1)
    invokeMock.mockResolvedValue([rule(1, 'f二', 'F2'), rule(2, '回车', '\n')])
    expect(await store.importRulesFrom('/tmp/dict.json')).toBe(1)
    expect(invokeMock).toHaveBeenCalledWith('dict_import_file', { path: '/tmp/dict.json' })
    expect(store.rules).toHaveLength(2)

    invokeMock.mockRejectedValue('{"code":"E-CONFIG-003","message":"超出上限"}')
    expect(await store.importRulesFrom('/tmp/dict.json')).toBeNull()
    expect(store.lastError?.code).toBe('E-CONFIG-003')
  })

  it('dict:changed 事件触发重新拉取（多窗口同步）', async () => {
    invokeMock.mockResolvedValue([rule(1, 'f二', 'F2')])
    const store = useDictStore()
    await store.ensureLoaded()
    invokeMock.mockClear()

    invokeMock.mockResolvedValue([rule(1, 'f二', 'F2'), rule(2, '回车', '')])
    const handler = handlers.get(EVENTS.dictChanged)
    expect(handler).toBeDefined()
    handler?.({ type: 'dict_changed' })
    await vi.waitFor(() => expect(store.rules).toHaveLength(2))
    expect(invokeMock).toHaveBeenCalledWith('dict_list_rules')
  })
})
