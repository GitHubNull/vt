import { describe, it, expect } from 'vitest'
import {
  buildHighlightPattern,
  computeSegments,
  formatLocalTime,
  levelClass,
} from './logHighlight'

describe('utils/logHighlight（T-018 语法高亮）', () => {
  it('levelClass：级别着色映射', () => {
    expect(levelClass('DEBUG')).toBe('lv-debug')
    expect(levelClass('info')).toBe('lv-info')
    expect(levelClass('WARN')).toBe('lv-warn')
    expect(levelClass('ERROR')).toBe('lv-error')
    expect(levelClass('OTHER')).toBe('lv-info')
  })

  it('关键词模式按字面量匹配并转义元字符', () => {
    const pattern = buildHighlightPattern('a.b(c)', false, false)
    expect(pattern).not.toBeNull()
    expect(computeSegments('xx a.b(c) yy aXbXc', pattern).filter((s) => s.match).map((s) => s.text)).toEqual([
      'a.b(c)',
    ])
  })

  it('大小写开关：关闭时大小写都命中，开启时严格匹配', () => {
    const insensitive = buildHighlightPattern('error', false, false)
    expect(computeSegments('ERROR Error error', insensitive).filter((s) => s.match)).toHaveLength(3)
    const sensitive = buildHighlightPattern('error', false, true)
    expect(computeSegments('ERROR Error error', sensitive).filter((s) => s.match)).toHaveLength(1)
  })

  it('正则模式按正则命中多段', () => {
    const pattern = buildHighlightPattern(String.raw`elapsed_ms=\d+`, true, false)
    expect(pattern).not.toBeNull()
    const segments = computeSegments('done elapsed_ms=42 then elapsed_ms=7', pattern)
    expect(segments.filter((s) => s.match).map((s) => s.text)).toEqual([
      'elapsed_ms=42',
      'elapsed_ms=7',
    ])
  })

  it('空关键词返回 null（不高亮）；无命中时返回单片段', () => {
    expect(buildHighlightPattern('', false, false)).toBeNull()
    const segments = computeSegments('nothing here', buildHighlightPattern('zzz', false, false))
    expect(segments).toEqual([{ text: 'nothing here', match: false }])
  })

  it('formatLocalTime 输出本地时间格式', () => {
    // 2024-10-15T13:46:40.000Z（UTC），仅校验结构与数字段合法性。
    const text = formatLocalTime(1_729_000_000_000)
    expect(text).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/)
  })
})
