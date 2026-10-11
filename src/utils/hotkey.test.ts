import { describe, it, expect } from 'vitest'
import {
  comboFromKeyEvent,
  comboFromModifierKeyUp,
  comboFromMouseButton,
  formatCombo,
  isModifierCode,
} from './hotkey'

const key = (code: string, mods: Partial<Record<'ctrl' | 'alt' | 'shift' | 'meta', boolean>> = {}) => ({
  code,
  ctrlKey: mods.ctrl ?? false,
  altKey: mods.alt ?? false,
  shiftKey: mods.shift ?? false,
  metaKey: mods.meta ?? false,
})

describe('utils/hotkey（T-013 快捷键录制）', () => {
  it('仅按下修饰键的 keydown 不构成组合（等待主键或单独成键）', () => {
    expect(comboFromKeyEvent(key('AltRight', { alt: true }))).toBeNull()
    expect(comboFromKeyEvent(key('ControlLeft', { ctrl: true }))).toBeNull()
  })

  it('修饰键单独松开：以自身为热键（默认录音键 AltRight）', () => {
    // 松开右 Alt：keyup 时 altKey 已复位、无其他修饰键 → 单独成键
    expect(comboFromModifierKeyUp(key('AltRight'))).toBe('AltRight')
    expect(isModifierCode('AltRight')).toBe(true)
  })

  it('与其他修饰键组合中的松开：不单独成键', () => {
    expect(comboFromModifierKeyUp(key('AltRight', { ctrl: true }))).toBeNull()
  })

  it('仅按下非修饰主键：直接成组合', () => {
    expect(comboFromKeyEvent(key('F2'))).toBe('F2')
    expect(comboFromKeyEvent(key('Space'))).toBe('Space')
  })

  it('多键组合：ControlLeft+ShiftLeft+Space（后端解析格式）', () => {
    expect(comboFromKeyEvent(key('Space', { ctrl: true, shift: true }))).toBe(
      'ControlLeft+ShiftLeft+Space',
    )
  })

  it('Linux 默认录音键：AltLeft+KeyR（左 Alt 组合，不受右 Alt AltGr 映射影响）', () => {
    expect(comboFromKeyEvent(key('KeyR', { alt: true }))).toBe('AltLeft+KeyR')
  })

  it('code 无法识别（空串 / Unidentified）时不产生组合，避免录入 garbage', () => {
    // 部分键盘布局右 Alt 被映射为 AltGr，webview 上报 code=Unidentified
    expect(comboFromKeyEvent(key('Unidentified', { alt: true }))).toBeNull()
    expect(comboFromKeyEvent(key('', { alt: true }))).toBeNull()
    expect(comboFromModifierKeyUp(key('Unidentified'))).toBeNull()
  })

  it('仅按下修饰键不构成组合（等待主键）', () => {
    expect(comboFromKeyEvent(key('ControlLeft', { ctrl: true }))).toBeNull()
    expect(comboFromKeyEvent(key('ShiftLeft', { shift: true }))).toBeNull()
  })

  it('鼠标侧键：button 3/4 → MouseBack/MouseForward', () => {
    expect(comboFromMouseButton(3)).toBe('MouseBack')
    expect(comboFromMouseButton(4)).toBe('MouseForward')
    expect(comboFromMouseButton(0)).toBeNull()
  })

  it('formatCombo：Key/Digit 前缀剥离，其余查 i18n 名称表', () => {
    const names = {
      AltRight: '右 Alt',
      ControlLeft: 'Ctrl',
      ShiftLeft: 'Shift',
      MouseBack: '鼠标侧键（后退）',
    }
    expect(formatCombo('ControlLeft+ShiftLeft+Space', names)).toBe('Ctrl + Shift + Space')
    expect(formatCombo('AltRight', names)).toBe('右 Alt')
    expect(formatCombo('KeyA', names)).toBe('A')
    expect(formatCombo('Digit3', names)).toBe('3')
    expect(formatCombo('MouseBack', names)).toBe('鼠标侧键（后退）')
    expect(formatCombo('F2', names)).toBe('F2')
  })
})
