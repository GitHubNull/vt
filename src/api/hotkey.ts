/**
 * 快捷键冲突检测（T-543，F-SYS-03）：后端 `validate_hotkeys` 命令封装
 * （src-tauri/src/commands/hotkey.rs）+ 设置页录入即时反馈的纯逻辑。
 *
 * 三类反馈口径（PRD 3.1.1 规则 7）：
 * - 应用内重复（录音触发键 / 取消键 / 将来模板键）：**拒绝保存**，指出冲突项；
 * - OS 常用组合（Ctrl+C、Alt+F4、Win+组合等三平台清单）：仅警告，允许保存；
 * - 注册失败（被占用 / 平台错误）：保存后由后端 hotkey:error 事件反馈
 *   （E-HOTKEY-001 / E-HOTKEY-004），不在本模块处理。
 *
 * 纯函数与后端 `platform::hotkey::normalize_accelerator` 同一归一化口径：
 * 左右修饰键等价、修饰键顺序无关、Meta/Super 同位；空串 / 非法串不参与比较。
 */
import { invoke } from '@tauri-apps/api/core'

/** 待校验的快捷键槽位（与后端 services/hotkey.rs HotkeySlot 对齐）。 */
export interface HotkeySlot {
  /** 槽位标识（trigger / cancel_trigger；模板键接入后扩展 template:<id>）。 */
  id: string
  /** 快捷键串（录入组件契约格式，如 ControlLeft+ShiftLeft+Space）。 */
  spec: string
}

/** 一组应用内冲突：≥ 2 个槽位归一化后为同一组合。 */
export interface HotkeyConflictGroup {
  normalized: string
  ids: string[]
}

/** OS 常用组合预警（命中仅警告，允许保存）。 */
export interface OsPresetWarning {
  id: string
  normalized: string
  /** 用途标识（i18n 键后缀：hotkey.conflict.osPresets.<usage>）。 */
  usage: string
}

/** 后端校验报告。 */
export interface HotkeyValidationReport {
  conflicts: HotkeyConflictGroup[]
  warnings: OsPresetWarning[]
}

/** 录入反馈（level=error 时保存被拒绝；level=warning 时已保存）。 */
export interface HotkeyFeedback {
  level: 'error' | 'warning'
  text: string
}

/**
 * 调用后端权威校验（应用内重复 + 三平台 OS 预警表）。
 * 只读：不落盘、不注册；保存时的权威校验在后端 set_config 接入点。
 */
export async function validateHotkeys(slots: HotkeySlot[]): Promise<HotkeyValidationReport> {
  return invoke<HotkeyValidationReport>('validate_hotkeys', { slots })
}

/** 修饰键 code → 无左右侧规范名（与后端 canonical_modifier_name 同一映射）。 */
const CANONICAL_MODIFIER: Record<string, string> = {
  ControlLeft: 'Control',
  ControlRight: 'Control',
  AltLeft: 'Alt',
  AltRight: 'Alt',
  ShiftLeft: 'Shift',
  ShiftRight: 'Shift',
  MetaLeft: 'Super',
  MetaRight: 'Super',
  SuperLeft: 'Super',
  SuperRight: 'Super',
}

/**
 * 归一化快捷键串（后端 normalize_accelerator 的前端镜像）：
 * 最后一段为主键（原文保留），其余段须为修饰键（映射规范名、去重、按字母序排序）。
 * 空串 / 空段 / Unidentified / 鼠标侧键 / 主键位出现非修饰键 → null（不参与比较）。
 */
export function normalizeCombo(spec: string): string | null {
  const tokens = spec.split('+').map((part) => part.trim())
  if (tokens.some((token) => token === '' || token === 'Unidentified')) return null
  // 鼠标侧键不是可注册组合键（后端 parse_accelerator 显式不支持），不参与比较。
  if (tokens.some((token) => token === 'MouseBack' || token === 'MouseForward')) return null
  const main = tokens.pop()
  if (main === undefined) return null
  const modifiers: string[] = []
  for (const token of tokens) {
    const name = CANONICAL_MODIFIER[token]
    if (!name) return null
    if (!modifiers.includes(name)) modifiers.push(name)
  }
  modifiers.sort()
  return [...modifiers, main].join('+')
}

/**
 * 应用内重复分组（每组 ≥ 2 个槽位，T-543 应用内冲突校验的本地即时判定）。
 * 空串（未设置）与非法串（鼠标侧键 / Unidentified 等）不参与比较。
 */
export function findDuplicateGroups(slots: HotkeySlot[]): HotkeyConflictGroup[] {
  const groups = new Map<string, string[]>()
  for (const slot of slots) {
    const normalized = normalizeCombo(slot.spec)
    if (!normalized) continue
    const ids = groups.get(normalized)
    if (ids) {
      ids.push(slot.id)
    } else {
      groups.set(normalized, [slot.id])
    }
  }
  return [...groups.entries()]
    .filter(([, ids]) => ids.length > 1)
    .map(([normalized, ids]) => ({ normalized, ids }))
}

/**
 * 归一化组合（Control/Alt/Shift/Super 形态）的显示名称表：
 * 规范修饰键名复用现有左右侧键名文案（如 Control → "Ctrl"），不新增键位词条。
 */
export function canonicalDisplayNames(keyNames: Record<string, string>): Record<string, string> {
  return {
    ...keyNames,
    Control: keyNames.ControlLeft ?? 'Control',
    Alt: keyNames.AltLeft ?? 'Alt',
    Shift: keyNames.ShiftLeft ?? 'Shift',
    Super: keyNames.MetaLeft ?? 'Super',
  }
}

/** 文案模板占位符替换（{name} 形式，缺失 key 保留原文便于排查）。 */
function fill(template: string, params: Record<string, string>): string {
  return template.replace(/\{(\w+)\}/g, (raw, key: string) => params[key] ?? raw)
}

/** 应用内冲突的 i18n 文案资源（t.hotkey.conflict 的冲突部分）。 */
export interface HotkeyConflictLabels {
  /** 槽位 id → 显示名（slotNames）。 */
  slotNames?: Record<string, string>
  /** 多个槽位名的连接词（slotSep，如 "与"）。 */
  slotSep?: string
  /** 冲突提示模板（duplicate，占位符 {slots} {combo}）。 */
  duplicate?: string
}

/**
 * 应用内冲突 → 录入反馈（level=error，保存被拒绝）。
 * 文案缺失（i18n 未同步）时返回空文本而非抛错，避免界面崩溃。
 */
export function hotkeyConflictFeedback(
  groups: HotkeyConflictGroup[],
  labels: HotkeyConflictLabels,
  formatComboFn: (spec: string) => string,
): HotkeyFeedback | null {
  const group = groups[0]
  if (!group) return null
  const sep = labels.slotSep ?? ''
  const slots = group.ids.map((id) => labels.slotNames?.[id] ?? id).join(sep)
  return {
    level: 'error',
    text: fill(labels.duplicate ?? '', { slots, combo: formatComboFn(group.normalized) }),
  }
}

/** OS 常用组合预警的 i18n 文案资源（t.hotkey.conflict 的预警部分）。 */
export interface HotkeyPresetLabels {
  /** 预警提示模板（osPresetWarning，占位符 {usage}）。 */
  osPresetWarning?: string
  /** 用途标识 → 显示名（osPresets）。 */
  osPresets?: Record<string, string>
  /** 多个用途的连接符（usageSep，如 "、"）。 */
  usageSep?: string
}

/**
 * OS 常用组合预警 → 录入反馈（level=warning，允许保存）。
 * 同一用途多组键（如 macOS 截图）只提示一次；槽位 id 过滤由调用方完成。
 */
export function hotkeyPresetFeedback(
  warnings: OsPresetWarning[],
  labels: HotkeyPresetLabels,
): HotkeyFeedback | null {
  if (warnings.length === 0) return null
  const usages = [...new Set(warnings.map((warning) => labels.osPresets?.[warning.usage] ?? warning.usage))]
  return {
    level: 'warning',
    text: fill(labels.osPresetWarning ?? '', { usage: usages.join(labels.usageSep ?? '') }),
  }
}
