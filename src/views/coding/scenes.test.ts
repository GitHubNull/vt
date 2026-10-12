/**
 * 场景规则页纯逻辑测试（T-502）：表单归一 / 校验 / 应用列表 / 拖动排序。
 * 覆盖 SPEC T-502 测试要求"Vitest：设置页表单绑定"。
 */
import { describe, expect, it } from 'vitest'
import type { SceneEntry } from '../../api/types'
import {
  DEFAULT_ICON_COLOR,
  MAX_ENTRY_APPS,
  MAX_ENTRY_NAME_LEN,
  MAX_TITLE_KEYWORD_LEN,
  addApp,
  isBuiltinEntry,
  moveEntry,
  newEntryDraft,
  normalizeEntry,
  removeApp,
  validateEntryDraft,
} from './scenes'

function draft(overrides: Partial<SceneEntry> = {}): SceneEntry {
  return { ...newEntryDraft(), ...overrides }
}

describe('newEntryDraft', () => {
  it('新建草稿：规则跟随通用设置、非内置、无 ID、默认颜色', () => {
    const entry = newEntryDraft()
    expect(entry.id).toBe('')
    expect(entry.name).toBe('')
    expect(entry.builtin).toBe(false)
    expect(entry.icon_color).toBe(DEFAULT_ICON_COLOR)
    expect(entry.punct_style).toBe('follow')
    expect(entry.trailing_punct).toBe('follow')
    expect(entry.cjk_space).toBe('follow')
    expect(entry.commands).toBe(false)
    expect(entry.auto_enter).toBe(true)
    expect(entry.apps).toEqual([])
    expect(entry.title_keyword).toBe('')
    expect(entry.ai_template).toBeNull()
  })
})

describe('normalizeEntry', () => {
  it('裁剪名称 / 关键字 / 颜色并小写化颜色', () => {
    const entry = normalizeEntry(
      draft({ name: '  我的场景 ', title_keyword: '  Pull Request ', icon_color: ' #A1B2C3 ' }),
    )
    expect(entry.name).toBe('我的场景')
    expect(entry.title_keyword).toBe('Pull Request')
    expect(entry.icon_color).toBe('#a1b2c3')
  })

  it('应用列表：剔除空串并按大小写不敏感去重（保序）', () => {
    const entry = normalizeEntry(
      draft({ apps: [' Chrome ', 'chrome', '', 'Code', '  '] }),
    )
    expect(entry.apps).toEqual(['Chrome', 'Code'])
  })

  it('不修改原对象（纯函数）', () => {
    const source = draft({ apps: ['chrome'] })
    normalizeEntry(source)
    expect(source.apps).toEqual(['chrome'])
  })
})

describe('validateEntryDraft', () => {
  it('名称必填', () => {
    expect(validateEntryDraft(draft({ name: '   ' }))).toBe('nameRequired')
  })

  it('名称超长（按字符数，中文计 1）', () => {
    expect(validateEntryDraft(draft({ name: '名'.repeat(MAX_ENTRY_NAME_LEN) }))).toBeNull()
    expect(validateEntryDraft(draft({ name: '名'.repeat(MAX_ENTRY_NAME_LEN + 1) }))).toBe(
      'nameTooLong',
    )
  })

  it('颜色须为 #RRGGBB 或留空', () => {
    expect(validateEntryDraft(draft({ name: 'x', icon_color: '#3b82f6' }))).toBeNull()
    expect(validateEntryDraft(draft({ name: 'x', icon_color: '' }))).toBeNull()
    expect(validateEntryDraft(draft({ name: 'x', icon_color: 'red' }))).toBe('colorInvalid')
    expect(validateEntryDraft(draft({ name: 'x', icon_color: '#12345' }))).toBe('colorInvalid')
  })

  it('应用数量与长度上限', () => {
    const many = Array.from({ length: MAX_ENTRY_APPS + 1 }, (_, i) => `app-${i}`)
    expect(validateEntryDraft(draft({ name: 'x', apps: many }))).toBe('appsTooMany')
    expect(validateEntryDraft(draft({ name: 'x', apps: ['a'.repeat(101)] }))).toBe('appTooLong')
  })

  it('标题关键字超长', () => {
    expect(
      validateEntryDraft(draft({ name: 'x', title_keyword: 'k'.repeat(MAX_TITLE_KEYWORD_LEN + 1) })),
    ).toBe('keywordTooLong')
  })

  it('未知场景拒绝', () => {
    expect(validateEntryDraft(draft({ name: 'x', scene: 'unknown' }))).toBe('sceneUnknown')
  })

  it('合法草稿通过', () => {
    expect(
      validateEntryDraft(
        draft({ name: '合法', scene: 'terminal', apps: ['WindowsTerminal'], title_keyword: 'PR' }),
      ),
    ).toBeNull()
  })
})

describe('addApp / removeApp', () => {
  it('追加：裁剪、拒绝空串与重复（大小写不敏感）', () => {
    expect(addApp([], '  chrome ')).toEqual(['chrome'])
    expect(addApp(['chrome'], '')).toEqual(['chrome'])
    expect(addApp(['chrome'], '  ')).toEqual(['chrome'])
    expect(addApp(['chrome'], 'Chrome')).toEqual(['chrome'])
    expect(addApp(['chrome'], 'code')).toEqual(['chrome', 'code'])
  })

  it('移除：大小写不敏感；不存在时原样返回', () => {
    expect(removeApp(['chrome', 'code'], 'CHROME')).toEqual(['code'])
    const apps = ['chrome']
    expect(removeApp(apps, 'firefox')).toBe(apps)
  })
})

describe('moveEntry（拖动排序）', () => {
  it('把条目从 from 移到 to', () => {
    expect(moveEntry(['a', 'b', 'c', 'd'], 0, 2)).toEqual(['b', 'c', 'a', 'd'])
    expect(moveEntry(['a', 'b', 'c', 'd'], 3, 0)).toEqual(['d', 'a', 'b', 'c'])
  })

  it('越界或原位时原样返回（同一引用，调用方跳过持久化）', () => {
    const list = ['a', 'b', 'c']
    expect(moveEntry(list, 1, 1)).toBe(list)
    expect(moveEntry(list, -1, 0)).toBe(list)
    expect(moveEntry(list, 0, 3)).toBe(list)
    expect(moveEntry(list, 5, 0)).toBe(list)
  })
})

describe('isBuiltinEntry', () => {
  it('内置标记或内置 ID 均判定为内置', () => {
    expect(isBuiltinEntry(draft({ builtin: true }))).toBe(true)
    expect(isBuiltinEntry(draft({ id: 'ide', builtin: false }))).toBe(true)
    expect(isBuiltinEntry(draft({ id: 'terminal', builtin: false }))).toBe(true)
    expect(isBuiltinEntry(draft({ id: 'custom-1', builtin: false }))).toBe(false)
  })
})
