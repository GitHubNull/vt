import { describe, expect, it } from 'vitest'
import {
  RING_CIRCUMFERENCE,
  RING_RADIUS,
  resolveRecordAction,
  ringStyle,
  statusKey,
} from './logic'

describe('resolveRecordAction：按住模式', () => {
  it('press 开始、release 结束', () => {
    expect(resolveRecordAction('hold', false, 'press')).toBe('start')
    expect(resolveRecordAction('hold', true, 'release')).toBe('stop')
  })

  it('录音中 press 不重复触发（单会话互斥）', () => {
    expect(resolveRecordAction('hold', true, 'press')).toBeNull()
  })

  it('空闲时 release 无副作用', () => {
    expect(resolveRecordAction('hold', false, 'release')).toBeNull()
  })

  it('click 在按住模式不产生命令', () => {
    expect(resolveRecordAction('hold', false, 'click')).toBeNull()
    expect(resolveRecordAction('hold', true, 'click')).toBeNull()
  })
})

describe('resolveRecordAction：切换模式', () => {
  it('click 翻转开始 / 结束（命令由后端按状态机切换）', () => {
    expect(resolveRecordAction('toggle', false, 'click')).toBe('toggle')
    expect(resolveRecordAction('toggle', true, 'click')).toBe('toggle')
  })

  it('press / release 被忽略（避免与按住手势混淆）', () => {
    expect(resolveRecordAction('toggle', false, 'press')).toBeNull()
    expect(resolveRecordAction('toggle', true, 'release')).toBeNull()
  })
})

describe('statusKey：状态文案分支', () => {
  it('录音 / 识别 / 反馈态与模式无关', () => {
    expect(statusKey('recording', 'hold')).toBe('mini.status.recording')
    expect(statusKey('recording', 'toggle')).toBe('mini.status.recording')
    expect(statusKey('processing', 'toggle')).toBe('mini.status.processing')
    expect(statusKey('feedback', 'hold')).toBe('mini.status.cancelled')
  })

  it('空闲提示随模式变化', () => {
    expect(statusKey('idle', 'hold')).toBe('mini.status.hold')
    expect(statusKey('idle', 'toggle')).toBe('mini.status.toggle')
  })
})

describe('ringStyle：音量环', () => {
  it('几何常量自洽', () => {
    expect(RING_CIRCUMFERENCE).toBeCloseTo(2 * Math.PI * RING_RADIUS)
  })

  it('level 映射为环填充比例', () => {
    expect(ringStyle(0).dashoffset).toBeCloseTo(RING_CIRCUMFERENCE)
    expect(ringStyle(1).dashoffset).toBeCloseTo(0)
    expect(ringStyle(0.5).dashoffset).toBeCloseTo(RING_CIRCUMFERENCE / 2)
  })

  it('越界输入钳制到 0~1', () => {
    expect(ringStyle(-1).dashoffset).toBeCloseTo(ringStyle(0).dashoffset)
    expect(ringStyle(2).dashoffset).toBeCloseTo(ringStyle(1).dashoffset)
  })
})
