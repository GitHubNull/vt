/**
 * 技术词库管理页纯逻辑（T-521，F-CODE-06）：
 * 栈卡片 / 词条过滤、启用栈切换（纯函数），独立成模块便于 Vitest 覆盖（Node 环境）。
 */
import type { TechStackLexicon, TechTerm } from '../../api/types'

/** 栈展示顺序（与后端 LEXICON_STACKS canonical 顺序一致）。 */
export const STACK_ORDER = [
  'general',
  'java',
  'frontend',
  'python',
  'go',
  'rust',
  'cloudnative',
  'database',
  'ai',
  'mobile',
  'desktop',
] as const

/** 技术栈标识 → i18n 显示名映射键（coding.glossary.stacks.<id>）。 */
export function stackNameKey(id: string): string {
  return `coding.glossary.stacks.${id}`
}

/** 按显示名或包含的术语 / 别名过滤栈卡片（空查询返回原顺序全量）。 */
export function filterStacks(
  stacks: TechStackLexicon[],
  names: Record<string, string>,
  query: string,
): TechStackLexicon[] {
  const q = query.trim().toLowerCase()
  if (!q) return stacks
  return stacks.filter((stack) => {
    const name = (names[stack.id] ?? stack.id).toLowerCase()
    if (name.includes(q)) return true
    return stack.terms.some(
      (term) =>
        term.term.toLowerCase().includes(q) ||
        term.aliases.some((alias) => alias.toLowerCase().includes(q)),
    )
  })
}

/** 词条对话框内过滤：按标准写法 / 别名包含匹配。 */
export function filterTerms(terms: TechTerm[], query: string): TechTerm[] {
  const q = query.trim().toLowerCase()
  if (!q) return terms
  return terms.filter(
    (term) =>
      term.term.toLowerCase().includes(q) ||
      term.aliases.some((alias) => alias.toLowerCase().includes(q)),
  )
}

/** 启停一个技术栈（纯函数，返回新数组；幂等）。 */
export function toggleStack(enabled: string[], stack: string, on: boolean): string[] {
  const without = enabled.filter((id) => id !== stack)
  return on ? [...without, stack] : without
}
