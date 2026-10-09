/**
 * 文案资源入口：默认 zh-CN，语言切换机制预留（tech/03 3.2）。
 */
import { zhCN } from './zh-CN'

export const t = zhCN

/**
 * 按点分路径取文案（用于路由元信息等动态 key，如 `nav.${key}`）。
 * 找不到时原样返回路径，便于排查缺失 key。
 */
export function getMessage(path: string): string {
  const value = path
    .split('.')
    .reduce<unknown>((obj, key) => (obj as Record<string, unknown> | undefined)?.[key], zhCN)
  return typeof value === 'string' ? value : path
}

export type { Messages } from './zh-CN'
