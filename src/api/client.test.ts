import { describe, it, expect, vi, beforeEach } from 'vitest'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

import { AppError, ping, toAppError, UNKNOWN_ERROR_CODE } from './client'

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

describe('toAppError（后端 AppError { code, message } 归一化）', () => {
  it('解析 JSON 字符串拒绝值', () => {
    const error = toAppError('{"code":"E-HOTKEY-001","message":"快捷键被占用"}')
    expect(error).toBeInstanceOf(AppError)
    expect(error.code).toBe('E-HOTKEY-001')
    expect(error.message).toBe('快捷键被占用')
  })

  it('解析 Error 对象包裹的 JSON', () => {
    const error = toAppError(new Error('{"code":"E-CONFIG-001","message":"写入失败"}'))
    expect(error.code).toBe('E-CONFIG-001')
  })

  it('透传已是 AppError 的值', () => {
    const original = new AppError('E-NET-001', '网络不可达')
    expect(toAppError(original)).toBe(original)
  })

  it('非结构化错误使用兜底编号', () => {
    expect(toAppError('some failure').code).toBe(UNKNOWN_ERROR_CODE)
    expect(toAppError(42).code).toBe(UNKNOWN_ERROR_CODE)
  })
})
