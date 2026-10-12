/**
 * 专有名词管理页纯逻辑（T-522，F-DICT-02）：
 * 过滤 / 别名清洗 / 展示格式化，独立成模块便于 Vitest 覆盖（Node 环境）。
 */
import type { ProperNoun } from '../../api/types'

/** 条目上限（PRD 3.4.2 业务规则 1，与后端 MAX_NOUNS、stores/dict 同步）。 */
export const MAX_NOUNS = 2000

/** 按标准写法 / 别名 / 备注过滤（不区分大小写的包含匹配）。 */
export function filterNouns(nouns: ProperNoun[], query: string): ProperNoun[] {
  const q = query.trim().toLowerCase()
  if (!q) return nouns
  return nouns.filter(
    (noun) =>
      noun.term.toLowerCase().includes(q) ||
      noun.aliases.some((alias) => alias.toLowerCase().includes(q)) ||
      noun.note.toLowerCase().includes(q),
  )
}

/**
 * 别名归一清洗：去首尾空白、剔除空串、英文不区分大小写去重（与后端
 * proper_nouns::validate_aliases 口径一致，编辑态即时反馈）。
 */
export function cleanAliases(aliases: string[]): string[] {
  const cleaned: string[] = []
  const seen = new Set<string>()
  for (const raw of aliases) {
    const alias = raw.trim()
    if (!alias) continue
    const key = alias.toLowerCase()
    if (seen.has(key)) continue
    seen.add(key)
    cleaned.push(alias)
  }
  return cleaned
}

/** 列表展示：别名以顿号连接；无别名时给"（按标准写法匹配）"占位说明。 */
export function formatAliases(aliases: string[]): string {
  const cleaned = cleanAliases(aliases)
  return cleaned.length > 0 ? cleaned.join('、') : ''
}

/** 编辑条目归一：标准写法裁剪 + 别名清洗（提交前调用）。 */
export function normalizeNounDraft(draft: {
  id: number | null
  term: string
  aliases: string[]
  note: string
  enabled: boolean
}): { id: number | null; term: string; aliases: string[]; note: string; enabled: boolean } {
  return {
    id: draft.id,
    term: draft.term.trim(),
    aliases: cleanAliases(draft.aliases),
    note: draft.note.trim(),
    enabled: draft.enabled,
  }
}
