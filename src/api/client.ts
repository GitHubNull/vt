/**
 * 前后端通信封装（tech/03 3.5）：组件不直接调 invoke，统一走本模块。
 */
import { invoke } from '@tauri-apps/api/core'

/** 调用后端 `ping` 命令，用于前端联调验证。 */
export async function ping(): Promise<string> {
  return invoke<string>('ping')
}
