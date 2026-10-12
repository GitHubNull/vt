/**
 * 场景规则页纯逻辑（T-502，F-CODE-01）：条目草稿归一 / 校验 / 应用列表维护 /
 * 拖动排序移动，独立成模块便于 Vitest 覆盖（Node 环境，不依赖 Tauri）。
 * 常量与后端 src-tauri/src/services/scene.rs 同步（修改须两侧一致）。
 */
import type { SceneEntry, SceneKind } from '../../api/types'

/** 条目总数上限（含内置五条），与后端 scene::MAX_SCENE_ENTRIES 同步。 */
export const MAX_SCENE_ENTRIES = 50
/** 单条目匹配应用数量上限，与后端 scene::MAX_ENTRY_APPS 同步。 */
export const MAX_ENTRY_APPS = 50
/** 场景名称最大长度（字符数），与后端 scene::MAX_ENTRY_NAME_LEN 同步。 */
export const MAX_ENTRY_NAME_LEN = 30
/** 标题关键字最大长度（字符数），与后端 scene::MAX_TITLE_KEYWORD_LEN 同步。 */
export const MAX_TITLE_KEYWORD_LEN = 100
/** 应用标识最大长度（字符数），与后端 scene::MAX_RULE_APP_LEN 同步。 */
export const MAX_APP_LEN = 100

/** 内置条目 ID（与后端 scene::BUILTIN_ENTRY_IDS 同步；内置可编辑不可删除）。 */
export const BUILTIN_ENTRY_IDS = ['ide', 'terminal', 'ai_chat', 'docs', 'default'] as const

/** 可归属的场景（不含 unknown；"默认"对应 other）。 */
export const SCENE_KIND_OPTIONS: SceneKind[] = [
  'ide',
  'terminal',
  'ai_chat',
  'browser',
  'docs',
  'other',
]

/** 新条目图标颜色默认值（中性蓝，用户可在表单中修改）。 */
export const DEFAULT_ICON_COLOR = '#3b82f6'

/** 判断条目是否内置（后端裁决为准；内置 ID 兜底，防御旧数据）。 */
export function isBuiltinEntry(entry: SceneEntry): boolean {
  return entry.builtin || (BUILTIN_ENTRY_IDS as readonly string[]).includes(entry.id)
}

/**
 * 新建自定义条目的默认草稿：规则全部"跟随通用设置"，口令关闭，允许回车；
 * 归属场景默认"默认（other）"，由用户在表单中调整。
 */
export function newEntryDraft(): SceneEntry {
  return {
    id: '',
    name: '',
    icon_color: DEFAULT_ICON_COLOR,
    scene: 'other',
    apps: [],
    title_keyword: '',
    punct_style: 'follow',
    trailing_punct: 'follow',
    cjk_space: 'follow',
    commands: false,
    auto_enter: true,
    ai_template: null,
    builtin: false,
  }
}

/**
 * 条目草稿归一（提交前调用，与后端 sanitize_entry 口径一致）：
 * 名称 / 颜色 / 关键字裁剪、颜色小写、应用列表剔除空串并按大小写不敏感去重（保序）。
 */
export function normalizeEntry(entry: SceneEntry): SceneEntry {
  const apps: string[] = []
  const seen = new Set<string>()
  for (const raw of entry.apps) {
    const app = raw.trim()
    if (!app) continue
    const key = app.toLowerCase()
    if (seen.has(key)) continue
    seen.add(key)
    apps.push(app)
  }
  return {
    ...entry,
    name: entry.name.trim(),
    icon_color: entry.icon_color.trim().toLowerCase(),
    title_keyword: entry.title_keyword.trim(),
    apps,
  }
}

/**
 * 表单前置校验（镜像后端 validate_entry 的用户可见检查）：
 * 返回 i18n 键后缀（`validation.*`）或 null（通过）。
 */
export function validateEntryDraft(entry: SceneEntry): string | null {
  if (!entry.name.trim()) return 'nameRequired'
  if ([...entry.name.trim()].length > MAX_ENTRY_NAME_LEN) return 'nameTooLong'
  if (entry.scene === 'unknown') return 'sceneUnknown'
  if (entry.icon_color && !/^#[0-9a-fA-F]{6}$/.test(entry.icon_color.trim())) {
    return 'colorInvalid'
  }
  if (entry.apps.length > MAX_ENTRY_APPS) return 'appsTooMany'
  if (entry.apps.some((app) => !app.trim() || [...app.trim()].length > MAX_APP_LEN)) {
    return 'appTooLong'
  }
  if ([...entry.title_keyword.trim()].length > MAX_TITLE_KEYWORD_LEN) return 'keywordTooLong'
  return null
}

/**
 * 应用列表追加：裁剪、拒绝空串与重复（大小写不敏感），返回新数组；
 * 未发生变化时返回原数组（调用方可据此跳过状态更新）。
 */
export function addApp(apps: string[], raw: string): string[] {
  const app = raw.trim()
  if (!app) return apps
  if (apps.some((existing) => existing.toLowerCase() === app.toLowerCase())) return apps
  return [...apps, app]
}

/** 应用列表移除（大小写不敏感匹配；不存在时返回原数组）。 */
export function removeApp(apps: string[], app: string): string[] {
  const key = app.toLowerCase()
  const next = apps.filter((existing) => existing.toLowerCase() !== key)
  return next.length === apps.length ? apps : next
}

/**
 * 拖动排序：把 from 位置条目移动到 to 位置并返回新数组；
 * 索引越界或 from === to 时原样返回（调用方据此跳过持久化）。
 */
export function moveEntry<T>(list: T[], from: number, to: number): T[] {
  if (from === to || from < 0 || to < 0 || from >= list.length || to >= list.length) {
    return list
  }
  const next = [...list]
  const [moved] = next.splice(from, 1)
  next.splice(to, 0, moved)
  return next
}
