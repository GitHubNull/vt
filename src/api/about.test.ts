import { describe, it, expect, vi, beforeEach } from 'vitest'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

import { applyUpdate, checkUpdate, exportLogs, getAppInfo, getLegalText } from './about'

describe('api/about（T-016 命令契约）', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  it('getAppInfo 调用 get_app_info', async () => {
    const info = {
      product_name: 'VibeType',
      version: '0.1.0',
      build_info: 'dev',
      config_dir: '/home/user/.config/vibetype',
      data_dir: '/home/user/.local/share/vibetype',
      log_dir: '/home/user/.local/state/vibetype/logs',
    }
    invokeMock.mockResolvedValue(info)

    const result = await getAppInfo()

    expect(invokeMock).toHaveBeenCalledWith('get_app_info')
    expect(result).toEqual(info)
  })

  it('checkUpdate 调用 check_update', async () => {
    invokeMock.mockResolvedValue({
      status: 'available',
      version: '0.2.0',
      notes: '修复与改进',
      error_code: null,
    })

    const result = await checkUpdate()

    expect(invokeMock).toHaveBeenCalledWith('check_update')
    expect(result.status).toBe('available')
    expect(result.version).toBe('0.2.0')
  })

  it('applyUpdate 调用 apply_update', async () => {
    invokeMock.mockResolvedValue(undefined)

    await applyUpdate()

    expect(invokeMock).toHaveBeenCalledWith('apply_update')
  })

  it('exportLogs 调用 export_logs 并透传保存路径', async () => {
    invokeMock.mockResolvedValue({ path: '/tmp/vibetype-logs.txt', lines: 1000 })

    const result = await exportLogs('/tmp/vibetype-logs.txt')

    expect(invokeMock).toHaveBeenCalledWith('export_logs', { targetPath: '/tmp/vibetype-logs.txt' })
    expect(result.lines).toBe(1000)
  })

  it('getLegalText 调用 get_legal_text 并透传文档类型', async () => {
    invokeMock.mockResolvedValue('MIT License\n...')

    const result = await getLegalText('license')

    expect(invokeMock).toHaveBeenCalledWith('get_legal_text', { document: 'license' })
    expect(result).toContain('MIT')
  })
})
