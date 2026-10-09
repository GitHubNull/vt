import { describe, it, expect, vi, beforeEach } from 'vitest'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

import { listAudioDevices } from './audio'

describe('api/audio（T-005 list_audio_devices 契约）', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  it('listAudioDevices 调用 list_audio_devices 并返回设备列表', async () => {
    const devices = [
      { id: 'default', name: '内置麦克风', is_default: true },
      { id: 'usb-046d:0a4d', name: 'USB 麦克风', is_default: false },
    ]
    invokeMock.mockResolvedValue(devices)

    const result = await listAudioDevices()

    expect(invokeMock).toHaveBeenCalledWith('list_audio_devices')
    expect(result).toEqual(devices)
  })
})
