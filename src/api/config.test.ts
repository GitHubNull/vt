import { describe, it, expect, vi, beforeEach } from 'vitest'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

import { getConfig, setConfig } from './config'
import { defaultConfig } from '../stores/config'

describe('api/config（T-003 契约）', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  it('getConfig 调用 get_config（无参数）并返回完整镜像', async () => {
    const config = defaultConfig()
    invokeMock.mockResolvedValue(config)

    const result = await getConfig()

    expect(invokeMock).toHaveBeenCalledWith('get_config')
    expect(result).toEqual(config)
  })

  it('setConfig 调用 set_config 并透传 section 与完整节值', async () => {
    invokeMock.mockResolvedValue(undefined)
    const value = { ...defaultConfig().audio, keep_warm: false }

    await setConfig('audio', value)

    expect(invokeMock).toHaveBeenCalledWith('set_config', { section: 'audio', value })
  })

  it('setConfig 透传后端错误（如 E-HOTKEY-001 冲突）', async () => {
    invokeMock.mockRejectedValue('{"code":"E-HOTKEY-001","message":"快捷键被占用"}')

    await expect(setConfig('hotkey', defaultConfig().hotkey)).rejects.toBe(
      '{"code":"E-HOTKEY-001","message":"快捷键被占用"}',
    )
  })
})
