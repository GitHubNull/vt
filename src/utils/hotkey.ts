/**
 * 快捷键组合编解码（T-013）。
 *
 * 规范键名格式（与后端 HotkeyService 解析格式一致，见 config_store.rs HotkeyConfig 注释）：
 * `ControlLeft+ShiftLeft+Space`、`AltRight`、`Escape`。
 * 即：完整 KeyboardEvent.code（含左右修饰键）以 `+` 连接，鼠标侧键为 MouseBack / MouseForward。
 */

/** 修饰键的 KeyboardEvent.code（单独按下不构成有效组合）。 */
export const MODIFIER_CODES = [
  'ControlLeft',
  'ControlRight',
  'ShiftLeft',
  'ShiftRight',
  'AltLeft',
  'AltRight',
  'MetaLeft',
  'MetaRight',
] as const

/** 与 KeyboardEvent / MouseEvent 结构兼容的最小输入（便于在 Node 环境测试）。 */
export interface KeyEventLike {
  code: string
  ctrlKey: boolean
  altKey: boolean
  shiftKey: boolean
  metaKey: boolean
}

/**
 * 把按键事件转为规范组合字符串；仅按下修饰键时返回 null（等待主键）。
 * 注意：修饰键的左右侧不在事件中区分（用 AltLeft 代表按住 Alt），
 * 单独的修饰键热键（如默认录音键 AltRight）由 comboFromModifierKeyUp 捕获。
 * code 为空 / Unidentified（部分键盘布局右 Alt 被映射为 AltGr 时会出现）
 * 时不产生组合，避免录入 garbage。
 */
export function comboFromKeyEvent(e: KeyEventLike): string | null {
  if (isModifierCode(e.code)) return null
  if (!isKnownKeyCode(e.code)) return null
  const parts: string[] = []
  if (e.ctrlKey) parts.push('ControlLeft')
  if (e.altKey) parts.push('AltLeft')
  if (e.shiftKey) parts.push('ShiftLeft')
  if (e.metaKey) parts.push('MetaLeft')
  parts.push(e.code)
  return parts.join('+')
}

export function isModifierCode(code: string): boolean {
  return (MODIFIER_CODES as readonly string[]).includes(code)
}

/** code 是否可作为主键录入（排除空串与浏览器无法识别的 Unidentified）。 */
function isKnownKeyCode(code: string): boolean {
  return code !== '' && code !== 'Unidentified'
}

/**
 * 修饰键单独按下并松开（期间未按其他键、未与其他修饰键组合）时，
 * 以该修饰键自身作为热键（如 AltRight = 右 Alt，即默认录音键）。
 */
export function comboFromModifierKeyUp(e: KeyEventLike): string | null {
  if (!isModifierCode(e.code)) return null
  if (e.ctrlKey || e.altKey || e.shiftKey || e.metaKey) return null
  return e.code
}

/** 鼠标侧键（button 3 = 后退，4 = 前进）；其余按钮返回 null。 */
export function comboFromMouseButton(button: number): string | null {
  if (button === 3) return 'MouseBack'
  if (button === 4) return 'MouseForward'
  return null
}

/**
 * 把规范组合字符串转为可显示名称（文案来自 i18n hotkey.keys）。
 * `KeyA` → `A`、`Digit3` → `3`、功能键原样，其余查表。
 */
export function formatCombo(combo: string, names: Record<string, string>): string {
  return combo
    .split('+')
    .map((part) => {
      if (/^Key[A-Z]$/.test(part)) return part.slice(3)
      if (/^Digit[0-9]$/.test(part)) return part.slice(5)
      if (/^Numpad[0-9]$/.test(part)) return `小键盘 ${part.slice(6)}`
      if (/^F([1-9]|1[0-9]|2[0-4])$/.test(part)) return part
      return names[part] ?? part
    })
    .join(' + ')
}
