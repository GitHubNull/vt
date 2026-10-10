/**
 * 日志语法高亮工具（T-018，F-LOG-01）。
 * 命中片段切分与级别着色由纯函数完成，便于 Vitest 直接覆盖。
 */

/** 正文高亮片段：match = 命中关键词 / 正则的部分。 */
export interface HighlightSegment {
  text: string
  match: boolean
}

/** 日志级别 → CSS 类名（级别着色）。 */
export function levelClass(level: string): string {
  switch (level.toUpperCase()) {
    case 'DEBUG':
    case 'TRACE':
      return 'lv-debug'
    case 'INFO':
      return 'lv-info'
    case 'WARN':
      return 'lv-warn'
    case 'ERROR':
      return 'lv-error'
    default:
      return 'lv-info'
  }
}

/** 转义正则元字符（关键词模式按字面量匹配）。 */
function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/**
 * 构造高亮匹配器：keyword 为空返回 null（不高亮）；
 * regex = true 时 keyword 按正则解释（非法正则由调用方捕获），否则按字面量。
 */
export function buildHighlightPattern(
  keyword: string,
  regex: boolean,
  caseSensitive: boolean,
): RegExp | null {
  const source = regex ? keyword : escapeRegExp(keyword)
  if (!source) return null
  return new RegExp(source, caseSensitive ? 'g' : 'gi')
}

/** 把正文按匹配器切分为高亮片段（无匹配器时返回单片段）。 */
export function computeSegments(text: string, pattern: RegExp | null): HighlightSegment[] {
  if (!pattern) return [{ text, match: false }]
  const segments: HighlightSegment[] = []
  let last = 0
  for (const match of text.matchAll(pattern)) {
    const index = match.index ?? 0
    if (index > last) segments.push({ text: text.slice(last, index), match: false })
    segments.push({ text: match[0], match: true })
    last = index + match[0].length
  }
  if (last < text.length) segments.push({ text: text.slice(last), match: false })
  return segments.length > 0 ? segments : [{ text, match: false }]
}

/** 本地时间格式化（`YYYY-MM-DD HH:mm:ss`）。 */
export function formatLocalTime(tsMillis: number): string {
  const d = new Date(tsMillis)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

/** datetime-local 输入值（本地时区）→ Unix 毫秒；空值返回 null。 */
export function datetimeLocalToMillis(value: string | null): number | null {
  if (!value) return null
  const millis = new Date(value).getTime()
  return Number.isNaN(millis) ? null : millis
}
