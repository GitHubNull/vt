/**
 * 前后端通信封装（tech/03 3.5）：组件不直接调 invoke，统一走本模块。
 */
import { invoke } from '@tauri-apps/api/core'
import type { AppErrorShape } from './types'

/** 调用后端 `ping` 命令，用于前端联调验证。 */
export async function ping(): Promise<string> {
  return invoke<string>('ping')
}

/** 前端统一错误：后端 AppError { code, message } 的镜像（错误编号进入用户提示，NFR）。 */
export class AppError extends Error implements AppErrorShape {
  readonly code: string

  constructor(code: string, message: string) {
    super(message)
    this.name = 'AppError'
    this.code = code
  }
}

/** 前端本地回退错误编号（后端未返回结构化错误时使用）。 */
export const UNKNOWN_ERROR_CODE = 'E-UNKNOWN'

/**
 * 把 invoke 的拒绝值统一转为 AppError。
 * Tauri 命令返回 Err(AppError) 时，invoke 以 JSON 字符串形式拒绝（{ code, message }）。
 */
export function toAppError(error: unknown): AppError {
  if (error instanceof AppError) return error
  const raw = typeof error === 'string' ? error : error instanceof Error ? error.message : String(error)
  try {
    const parsed = JSON.parse(raw) as Partial<AppErrorShape>
    if (parsed && typeof parsed.code === 'string') {
      return new AppError(parsed.code, typeof parsed.message === 'string' ? parsed.message : raw)
    }
  } catch {
    // 非 JSON：走下方兜底
  }
  return new AppError(UNKNOWN_ERROR_CODE, raw || 'unknown error')
}
