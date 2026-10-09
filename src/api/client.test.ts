import { describe, it, expect, vi, beforeEach } from 'vitest'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

import { ping } from './client'

describe('api/client', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  it('ping 调用后端 ping 命令并透传返回值', async () => {
    invokeMock.mockResolvedValue('pong')

    const result = await ping()

    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('ping')
    expect(result).toBe('pong')
  })

  it('ping 透传后端错误', async () => {
    invokeMock.mockRejectedValue(new Error('backend down'))

    await expect(ping()).rejects.toThrow('backend down')
  })
})
