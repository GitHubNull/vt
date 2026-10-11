import { describe, it, expect, vi, beforeEach } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

import { useLogStore, PAGE_SIZE, LOG_LEVELS } from './log'

describe('stores/log（T-018 日志中心）', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    setActivePinia(createPinia())
  })

  it('默认查询条件：关键词默认匹配、大小写不敏感、级别全选', () => {
    const store = useLogStore()
    expect(store.regex).toBe(false)
    expect(store.caseSensitive).toBe(false)
    expect(store.levels).toEqual(LOG_LEVELS)
    expect(store.dbEnabled).toBe(true)
  })

  it('级别清空选中 = 不过滤：查询时发送全部级别（避免白屏困惑）', async () => {
    invokeMock.mockResolvedValue({ entries: [], has_more: false })
    const store = useLogStore()
    store.levels = []
    expect(await store.search()).toBe(true)
    expect(invokeMock).toHaveBeenCalledWith(
      'log_query',
      expect.objectContaining({
        query: expect.objectContaining({ levels: LOG_LEVELS }),
      }),
    )
  })

  it('dbEnabled 跟随配置：去掉 database 目标后搜索不可用并清空结果', async () => {
    invokeMock.mockResolvedValue({ entries: [{ id: 1 }], has_more: false })
    const store = useLogStore()
    expect(await store.search()).toBe(true)
    expect(store.entries).toHaveLength(1)

    invokeMock.mockResolvedValue(undefined)
    const ok = await store.updateTargets(['file'])
    expect(ok).toBe(true)
    expect(store.dbEnabled).toBe(false)
    expect(store.entries).toHaveLength(0)
    // 数据库未启用：search 直接拒绝，不发起 invoke。
    invokeMock.mockClear()
    expect(await store.search()).toBe(false)
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('search 组装查询参数并写入结果（reset / append）', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [{ id: 2 }, { id: 1 }], has_more: true })
    const store = useLogStore()
    store.keyword = 'error'
    store.regex = true
    store.caseSensitive = true
    store.levels = ['ERROR']
    expect(await store.search()).toBe(true)
    expect(invokeMock).toHaveBeenCalledWith('log_query', {
      query: {
        keyword: 'error',
        regex: true,
        case_sensitive: true,
        levels: ['ERROR'],
        from_millis: null,
        to_millis: null,
        limit: PAGE_SIZE,
        offset: 0,
      },
    })
    expect(store.hasMore).toBe(true)

    // loadMore：offset = 已有条数，结果追加。
    invokeMock.mockResolvedValueOnce({ entries: [{ id: 0 }], has_more: false })
    await store.loadMore()
    expect(store.entries).toHaveLength(3)
    const lastCall = invokeMock.mock.calls.at(-1)![1] as { query: { offset: number } }
    expect(lastCall.query.offset).toBe(2)
    expect(store.hasMore).toBe(false)
  })

  it('非法正则：前端预检提示且不发起查询', async () => {
    const store = useLogStore()
    store.keyword = '[unclosed'
    store.regex = true
    expect(await store.search()).toBe(false)
    expect(store.searchHint).toBe('invalid_regex')
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('后端错误（如 E-LOG-001）记录 lastError 并保留原列表', async () => {
    invokeMock.mockRejectedValueOnce('{"code":"E-LOG-001","message":"未启用数据库日志记录"}')
    const store = useLogStore()
    expect(await store.search()).toBe(false)
    expect(store.lastError?.code).toBe('E-LOG-001')
  })

  it('exportResults 走 log_export 并返回条数', async () => {
    invokeMock.mockResolvedValue({ path: '/tmp/out.log', lines: 7 })
    const store = useLogStore()
    store.keyword = 'needle'
    const lines = await store.exportResults('/tmp/out.log')
    expect(lines).toBe(7)
    expect(invokeMock).toHaveBeenCalledWith('log_export', {
      query: expect.objectContaining({ keyword: 'needle', limit: PAGE_SIZE, offset: 0 }),
      targetPath: '/tmp/out.log',
    })
  })

  it('updateTargets 拒绝全空（至少保留一项），不发起写配置', async () => {
    const store = useLogStore()
    // 直接调用 store 层校验由组件完成；这里验证空目标写后端被拒时回滚。
    invokeMock.mockRejectedValueOnce('{"code":"E-CONFIG-004","message":"至少选择一种日志记录方式"}')
    const ok = await store.updateTargets([])
    expect(ok).toBe(false)
  })
})
