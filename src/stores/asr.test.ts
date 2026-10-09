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

import { useAsrStore } from './asr'
import { EVENTS } from '../api/events'
import type { ModelInfo } from '../api/types'

const model = (patch: Partial<ModelInfo>): ModelInfo => ({
  id: 'paraformer-zh-int8',
  name: 'Paraformer 中文',
  languages: '中文（普通话）',
  tier: 'speed',
  size_bytes: 223_385_835,
  downloaded: false,
  active: false,
  invalid: false,
  progress: null,
  ...patch,
})

const models = (): ModelInfo[] => [
  model({ id: 'paraformer-zh-int8' }),
  model({ id: 'paraformer-zh', tier: 'accuracy' }),
]

describe('stores/asr（T-014 模型管理）', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    handlers.clear()
    setActivePinia(createPinia())
  })

  it('ensureLoaded 拉取模型清单', async () => {
    invokeMock.mockResolvedValue(models())
    const store = useAsrStore()
    await store.ensureLoaded()

    expect(invokeMock).toHaveBeenCalledWith('list_models')
    expect(store.loaded).toBe(true)
    expect(store.models).toHaveLength(2)
    expect(store.hasUsableModel).toBe(false)
  })

  it('statusOf：missing / downloading / downloaded / active / invalid 映射', () => {
    const store = useAsrStore()
    expect(store.statusOf(model({}))).toBe('missing')
    expect(store.statusOf(model({ progress: { downloaded_bytes: 1, total_bytes: 10 } }))).toBe('downloading')
    expect(store.statusOf(model({ downloaded: true }))).toBe('downloaded')
    expect(store.statusOf(model({ downloaded: true, active: true }))).toBe('active')
    expect(store.statusOf(model({ invalid: true }))).toBe('invalid')
  })

  it('下载流转：进度事件更新 progress，completed 后以服务端清单为准', async () => {
    invokeMock.mockResolvedValue(models())
    const store = useAsrStore()
    await store.ensureLoaded()

    const progress = handlers.get(EVENTS.modelProgress)
    expect(progress).toBeDefined()

    invokeMock.mockResolvedValue(undefined)
    expect(await store.download('paraformer-zh-int8')).toBe(true)
    expect(invokeMock).toHaveBeenCalledWith('download_model', { modelId: 'paraformer-zh-int8' })

    progress?.({
      type: 'model_download_progress',
      model_id: 'paraformer-zh-int8',
      status: 'downloading',
      downloaded_bytes: 50,
      total_bytes: 100,
    })
    const target = store.models.find((m) => m.id === 'paraformer-zh-int8')
    expect(target?.progress).toEqual({ downloaded_bytes: 50, total_bytes: 100 })

    // 完成：清空 progress 并重新拉取清单（服务端为准）。
    invokeMock.mockResolvedValue([model({ id: 'paraformer-zh-int8', downloaded: true }), models()[1]])
    progress?.({
      type: 'model_download_progress',
      model_id: 'paraformer-zh-int8',
      status: 'completed',
      downloaded_bytes: 100,
      total_bytes: 100,
    })
    await vi.waitFor(() => {
      expect(store.models.find((m) => m.id === 'paraformer-zh-int8')?.progress).toBeNull()
      expect(store.models.find((m) => m.id === 'paraformer-zh-int8')?.downloaded).toBe(true)
    })
    expect(store.hasUsableModel).toBe(true)
  })

  it('activate 切换成功：invoke 切换命令并刷新激活标记', async () => {
    const downloaded = models().map((m) => ({ ...m, downloaded: true }))
    invokeMock.mockResolvedValue(downloaded)
    const store = useAsrStore()
    await store.ensureLoaded()
    invokeMock.mockClear()

    invokeMock.mockResolvedValue([
      model({ id: 'paraformer-zh-int8', downloaded: true }),
      model({ id: 'paraformer-zh', tier: 'accuracy', downloaded: true, active: true }),
    ])
    invokeMock.mockResolvedValueOnce(undefined)
    expect(await store.activate('paraformer-zh')).toBe(true)
    expect(invokeMock).toHaveBeenCalledWith('set_active_model', { modelId: 'paraformer-zh' })
    expect(store.models.find((m) => m.id === 'paraformer-zh')?.active).toBe(true)
  })

  it('activate 失败（未下载）：记录带编号错误', async () => {
    invokeMock.mockResolvedValue(models())
    const store = useAsrStore()
    await store.ensureLoaded()

    invokeMock.mockRejectedValue('{"code":"E-ASR-001","message":"模型尚未下载，请先下载后再切换"}')
    expect(await store.activate('paraformer-zh')).toBe(false)
    expect(store.lastError?.code).toBe('E-ASR-001')
  })

  it('remove 删除激活模型后消费 asr:notice 提示', async () => {
    const active = [
      model({ id: 'paraformer-zh-int8', downloaded: true, active: true }),
      model({ id: 'paraformer-zh', tier: 'accuracy', downloaded: true }),
    ]
    invokeMock.mockResolvedValue(active)
    const store = useAsrStore()
    await store.ensureLoaded()
    invokeMock.mockClear()

    invokeMock.mockResolvedValue(undefined)
    invokeMock.mockResolvedValueOnce([active[1]])
    expect(await store.remove('paraformer-zh-int8')).toBe(true)
    expect(invokeMock).toHaveBeenCalledWith('delete_model', { modelId: 'paraformer-zh-int8' })

    const notice = handlers.get(EVENTS.asrNotice)
    expect(notice).toBeDefined()
    notice?.({
      type: 'asr_notice',
      code: 'E-ASR-000',
      message: '已删除模型，识别将使用 Paraformer 中文',
    })
    expect(store.notice).toContain('已删除模型')
  })

  it('取消下载：invoke 取消命令并刷新', async () => {
    invokeMock.mockResolvedValue(models())
    const store = useAsrStore()
    await store.ensureLoaded()
    invokeMock.mockClear()

    invokeMock.mockResolvedValue(undefined)
    await store.cancel('paraformer-zh-int8')
    expect(invokeMock).toHaveBeenCalledWith('cancel_download', { modelId: 'paraformer-zh-int8' })
  })
})
