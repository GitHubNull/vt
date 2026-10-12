import { describe, it, expect, vi, beforeEach } from 'vitest'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

import {
  canonicalDisplayNames,
  findDuplicateGroups,
  hotkeyConflictFeedback,
  hotkeyPresetFeedback,
  normalizeCombo,
  validateHotkeys,
} from './hotkey'
import type { HotkeyConflictGroup, OsPresetWarning } from './hotkey'
import { formatCombo } from '../utils/hotkey'

const slot = (id: string, spec: string) => ({ id, spec })

describe('api/hotkey（T-543 快捷键冲突检测）', () => {
  describe('validateHotkeys（命令封装）', () => {
    beforeEach(() => {
      invokeMock.mockReset()
    })

    it('调用 validate_hotkeys 并透传槽位集合', async () => {
      const report = { conflicts: [], warnings: [] }
      invokeMock.mockResolvedValue(report)
      const slots = [slot('trigger', 'AltLeft+KeyR'), slot('cancel_trigger', 'Escape')]

      const result = await validateHotkeys(slots)

      expect(invokeMock).toHaveBeenCalledWith('validate_hotkeys', { slots })
      expect(result).toEqual(report)
    })
  })

  describe('normalizeCombo（与后端 normalize_accelerator 同一口径）', () => {
    it('单键与单修饰键：主键原文保留、无修饰段', () => {
      expect(normalizeCombo('F2')).toBe('F2')
      expect(normalizeCombo('AltRight')).toBe('AltRight')
      expect(normalizeCombo('Escape')).toBe('Escape')
    })

    it('左右修饰键等价（平台不区分修饰位）', () => {
      expect(normalizeCombo('ControlLeft+KeyC')).toBe(normalizeCombo('ControlRight+KeyC'))
      expect(normalizeCombo('AltLeft+KeyR')).toBe('Alt+KeyR')
      expect(normalizeCombo('AltRight+KeyR')).toBe('Alt+KeyR')
    })

    it('修饰键顺序无关且去重，按字母序输出', () => {
      expect(normalizeCombo('ShiftLeft+ControlLeft+Space')).toBe('Control+Shift+Space')
      expect(normalizeCombo('ControlRight+ShiftRight+Space')).toBe('Control+Shift+Space')
      expect(normalizeCombo('ControlLeft+ControlRight+KeyC')).toBe('Control+KeyC')
    })

    it('Meta 与 Super 同一位', () => {
      expect(normalizeCombo('MetaLeft+KeyD')).toBe('Super+KeyD')
      expect(normalizeCombo('SuperRight+KeyD')).toBe('Super+KeyD')
    })

    it('分段两侧空白被忽略', () => {
      expect(normalizeCombo('AltLeft + KeyR')).toBe('Alt+KeyR')
    })

    it('空键 / 空段 / Unidentified / 修饰键占主键位 → null（不参与比较）', () => {
      expect(normalizeCombo('')).toBeNull()
      expect(normalizeCombo('AltLeft+')).toBeNull()
      expect(normalizeCombo('AltLeft++KeyC')).toBeNull()
      expect(normalizeCombo('AltLeft+Unidentified')).toBeNull()
      expect(normalizeCombo('KeyC+KeyD')).toBeNull()
    })
  })

  describe('findDuplicateGroups（应用内重复即时判定）', () => {
    it('无重复：录音键与取消键不同（含默认配置）', () => {
      expect(
        findDuplicateGroups([slot('trigger', 'AltLeft+KeyR'), slot('cancel_trigger', 'Escape')]),
      ).toEqual([])
    })

    it('重复时指出冲突槽位与归一化组合', () => {
      const groups = findDuplicateGroups([
        slot('trigger', 'ControlLeft+ShiftLeft+Space'),
        slot('cancel_trigger', 'ControlLeft+ShiftLeft+Space'),
      ])
      expect(groups).toEqual([{ normalized: 'Control+Shift+Space', ids: ['trigger', 'cancel_trigger'] }])
    })

    it('左右修饰键 / 修饰键顺序的等价表示同样判重', () => {
      expect(
        findDuplicateGroups([slot('trigger', 'AltLeft+KeyR'), slot('cancel_trigger', 'AltRight+KeyR')]),
      ).toHaveLength(1)
      expect(
        findDuplicateGroups([
          slot('trigger', 'ShiftLeft+ControlLeft+Space'),
          slot('cancel_trigger', 'ControlRight+ShiftRight+Space'),
        ]),
      ).toHaveLength(1)
      // 单修饰键作触发键：主键区分左右（物理键不同），同侧才判重。
      expect(findDuplicateGroups([slot('trigger', 'AltRight'), slot('cancel_trigger', 'AltRight')])).toEqual([
        { normalized: 'AltRight', ids: ['trigger', 'cancel_trigger'] },
      ])
      expect(findDuplicateGroups([slot('trigger', 'AltRight'), slot('cancel_trigger', 'AltLeft')])).toEqual([])
    })

    it('空键（未设置）与非法串不参与判重', () => {
      expect(findDuplicateGroups([slot('trigger', ''), slot('cancel_trigger', '')])).toEqual([])
      expect(
        findDuplicateGroups([slot('trigger', 'MouseBack'), slot('cancel_trigger', 'MouseBack')]),
      ).toEqual([])
      expect(
        findDuplicateGroups([
          slot('trigger', 'AltLeft+Unidentified'),
          slot('cancel_trigger', 'AltRight+Unidentified'),
        ]),
      ).toEqual([])
    })

    it('三槽位同键归为一组；无关槽位不误报', () => {
      const groups = findDuplicateGroups([
        slot('a', 'KeyA'),
        slot('b', 'KeyB'),
        slot('c', 'KeyA'),
        slot('d', 'KeyA'),
      ])
      expect(groups).toEqual([{ normalized: 'KeyA', ids: ['a', 'c', 'd'] }])
    })
  })

  describe('canonicalDisplayNames', () => {
    it('规范修饰键名复用现有键位文案，原有条目透传', () => {
      const names = canonicalDisplayNames({ ControlLeft: 'Ctrl', AltLeft: 'Alt', ShiftLeft: 'Shift', MetaLeft: 'Meta' })
      expect(names.Control).toBe('Ctrl')
      expect(names.Alt).toBe('Alt')
      expect(names.Shift).toBe('Shift')
      expect(names.Super).toBe('Meta')
      expect(names.ControlLeft).toBe('Ctrl')
    })

    it('文案缺失时回退规范名本身', () => {
      const names = canonicalDisplayNames({})
      expect(names.Control).toBe('Control')
      expect(names.Super).toBe('Super')
    })
  })

  describe('录入反馈文案（hotkeyConflictFeedback / hotkeyPresetFeedback）', () => {
    const conflictLabels = {
      slotNames: { trigger: '录音快捷键', cancel_trigger: '取消录音快捷键' },
      slotSep: '与',
      duplicate: '{slots}使用了相同的快捷键组合（{combo}），保存被拒绝，请更换按键。',
    }
    const comboLabel = (spec: string) => formatCombo(spec, canonicalDisplayNames({ ControlLeft: 'Ctrl' }))

    it('无冲突返回 null', () => {
      expect(hotkeyConflictFeedback([], conflictLabels, comboLabel)).toBeNull()
    })

    it('冲突反馈 level=error，指出冲突项与归一化组合显示名', () => {
      const groups: HotkeyConflictGroup[] = [{ normalized: 'Control+KeyC', ids: ['trigger', 'cancel_trigger'] }]
      const feedback = hotkeyConflictFeedback(groups, conflictLabels, comboLabel)
      expect(feedback?.level).toBe('error')
      expect(feedback?.text).toBe(
        '录音快捷键与取消录音快捷键使用了相同的快捷键组合（Ctrl + C），保存被拒绝，请更换按键。',
      )
    })

    it('槽位显示名缺失时回退槽位 id；模板缺失时空文本不抛错', () => {
      const groups: HotkeyConflictGroup[] = [{ normalized: 'KeyA', ids: ['template:1'] }]
      expect(hotkeyConflictFeedback(groups, conflictLabels, comboLabel)?.text).toContain('template:1')
      expect(hotkeyConflictFeedback(groups, {}, comboLabel)).toEqual({ level: 'error', text: '' })
    })

    it('无预警返回 null；预警反馈 level=warning 且同一用途去重', () => {
      expect(hotkeyPresetFeedback([], { osPresetWarning: 'x' })).toBeNull()
      const warnings: OsPresetWarning[] = [
        { id: 'trigger', normalized: 'Control+KeyC', usage: 'copy' },
        { id: 'trigger', normalized: 'Control+KeyV', usage: 'paste' },
        { id: 'cancel_trigger', normalized: 'Control+KeyX', usage: 'copy' },
      ]
      const feedback = hotkeyPresetFeedback(warnings, {
        osPresetWarning: '该组合与系统常用快捷键冲突（{usage}），仍将保存。',
        osPresets: { copy: '复制', paste: '粘贴' },
        usageSep: '、',
      })
      expect(feedback?.level).toBe('warning')
      expect(feedback?.text).toBe('该组合与系统常用快捷键冲突（复制、粘贴），仍将保存。')
    })

    it('用途名缺失时回退用途标识', () => {
      const warnings: OsPresetWarning[] = [{ id: 'trigger', normalized: 'F1', usage: 'help' }]
      const feedback = hotkeyPresetFeedback(warnings, { osPresetWarning: '冲突（{usage}）' })
      expect(feedback?.text).toBe('冲突（help）')
    })
  })
})
