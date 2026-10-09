import { describe, expect, it } from 'vitest'
import {
  CANCEL_DRAG_THRESHOLD,
  COUNTDOWN_THRESHOLD_SEC,
  DOT_PERIOD_MS,
  FEEDBACK_DURATIONS,
  SIGNIFICANT_DRAG_THRESHOLD,
  WAVE_BAR_COUNT,
  breathScale,
  dotOpacity,
  dragDistance,
  feedbackDurationMs,
  formatElapsed,
  isCancelDrag,
  isSignificantDrag,
  remainingSeconds,
  smoothLevel,
  waveHeight,
} from './animation'

describe('smoothLevel', () => {
  it('EMA 向目标值靠近且不越界', () => {
    expect(smoothLevel(0, 1)).toBeCloseTo(0.35)
    expect(smoothLevel(0.8, 0.2)).toBeCloseTo(0.59, 2)
    // 越界输入先被钳制到 0~1，再参与平滑。
    expect(smoothLevel(0, 2)).toBeCloseTo(0.35)
    expect(smoothLevel(0.5, -1)).toBeCloseTo(0.5 - 0.5 * 0.35)
  })

  it('相同输入结果稳定', () => {
    expect(smoothLevel(0.3, 0.7)).toBe(smoothLevel(0.3, 0.7))
  })
})

describe('breathScale', () => {
  it('在 0.94 ~ 1.06 之间周期性变化', () => {
    const samples = Array.from({ length: 40 }, (_, i) => breathScale(i * 100))
    expect(Math.min(...samples)).toBeGreaterThanOrEqual(1 - 0.06 - 1e-9)
    expect(Math.max(...samples)).toBeLessThanOrEqual(1.06 + 1e-9)
    // 四分之一个周期后到达峰值：值不同（动画确实在动）。
    expect(breathScale(0)).not.toBeCloseTo(breathScale(400), 5)
    // 整周期回到同值。
    expect(breathScale(0)).toBeCloseTo(breathScale(1600), 9)
  })
})

describe('waveHeight', () => {
  it('确定性：同参数同结果', () => {
    expect(waveHeight(0.5, 2, 1234)).toBe(waveHeight(0.5, 2, 1234))
  })

  it('音量越大高度越高（整体趋势）', () => {
    const loud = Array.from({ length: WAVE_BAR_COUNT }, (_, i) => waveHeight(1, i, 500))
    const quiet = Array.from({ length: WAVE_BAR_COUNT }, (_, i) => waveHeight(0, i, 500))
    const avg = (xs: number[]) => xs.reduce((a, b) => a + b, 0) / xs.length
    expect(avg(loud)).toBeGreaterThan(avg(quiet) + 0.3)
  })

  it('输出始终在 0~1', () => {
    for (const level of [0, 0.3, 1, 5]) {
      for (let i = 0; i < 20; i += 1) {
        const h = waveHeight(level, i % WAVE_BAR_COUNT, i * 97)
        expect(h).toBeGreaterThanOrEqual(0)
        expect(h).toBeLessThanOrEqual(1)
      }
    }
  })

  it('静音时保持低幅度呼吸（> 0）', () => {
    const heights = Array.from({ length: WAVE_BAR_COUNT }, (_, i) => waveHeight(0, i, 0))
    expect(Math.min(...heights)).toBeGreaterThan(0)
    expect(Math.max(...heights)).toBeLessThan(0.2)
  })
})

describe('formatElapsed', () => {
  it('格式化为 m:ss', () => {
    expect(formatElapsed(0)).toBe('0:00')
    expect(formatElapsed(999)).toBe('0:00')
    expect(formatElapsed(60_000)).toBe('1:00')
    expect(formatElapsed(65_000)).toBe('1:05')
    expect(formatElapsed(599_000)).toBe('9:59')
    expect(formatElapsed(3_723_000)).toBe('62:03')
  })

  it('负值按 0 处理', () => {
    expect(formatElapsed(-5)).toBe('0:00')
  })
})

describe('remainingSeconds', () => {
  it('剩余充足时返回 null', () => {
    expect(remainingSeconds(0, 5)).toBeNull()
    // 剩余 11 秒（> 阈值）不触发倒计时。
    expect(remainingSeconds(5 * 60 * 1000 - 11_000, 5)).toBeNull()
  })

  it('临近上限时返回剩余秒数', () => {
    expect(remainingSeconds(5 * 60 * 1000 - 10_000, 5)).toBe(10)
    expect(remainingSeconds(5 * 60 * 1000 - 10_000 + 1, 5)).toBe(10)
    expect(remainingSeconds(5 * 60 * 1000 - 500, 5)).toBe(1)
  })

  it('已超限返回 null（由自动结束流程接管）', () => {
    expect(remainingSeconds(5 * 60 * 1000, 5)).toBeNull()
    expect(remainingSeconds(6 * 60 * 1000, 5)).toBeNull()
  })

  it('阈值与常量一致', () => {
    expect(COUNTDOWN_THRESHOLD_SEC).toBe(10)
  })
})

describe('feedbackDurationMs', () => {
  it('按 PRD 4.3 取值', () => {
    expect(feedbackDurationMs('success')).toBe(800)
    expect(feedbackDurationMs('cancelled')).toBe(500)
    expect(feedbackDurationMs('degraded')).toBe(2000)
    expect(feedbackDurationMs('failed')).toBe(2500)
  })

  it('覆盖全部反馈类型', () => {
    for (const kind of ['success', 'cancelled', 'degraded', 'failed']) {
      expect(FEEDBACK_DURATIONS[kind]).toBeGreaterThan(0)
    }
  })

  it('未知类型回退到降级时长', () => {
    expect(feedbackDurationMs('whatever')).toBe(feedbackDurationMs('degraded'))
  })
})

describe('拖动判定', () => {
  it('dragDistance 欧氏距离', () => {
    expect(dragDistance(3, 4)).toBe(5)
    expect(dragDistance(0, 0)).toBe(0)
  })

  it('超过取消阈值进入取消态', () => {
    expect(isCancelDrag(CANCEL_DRAG_THRESHOLD, 0)).toBe(true)
    expect(isCancelDrag(CANCEL_DRAG_THRESHOLD - 1, 0)).toBe(false)
    expect(isCancelDrag(30, 30)).toBe(true)
  })

  it('有效拖动阈值区分点击', () => {
    expect(isSignificantDrag(0, 0)).toBe(false)
    expect(isSignificantDrag(SIGNIFICANT_DRAG_THRESHOLD - 1, 0)).toBe(false)
    expect(isSignificantDrag(SIGNIFICANT_DRAG_THRESHOLD, 0)).toBe(true)
  })
})

describe('dotOpacity', () => {
  it('范围 0.25 ~ 1 且三点相位错开', () => {
    for (let i = 0; i < 30; i += 1) {
      for (let d = 0; d < 3; d += 1) {
        const opacity = dotOpacity(d, i * 137)
        expect(opacity).toBeGreaterThanOrEqual(0.25)
        expect(opacity).toBeLessThanOrEqual(1)
      }
    }
    expect(dotOpacity(0, 0)).not.toBeCloseTo(dotOpacity(1, 0), 5)
  })

  it('整周期稳定', () => {
    expect(dotOpacity(0, 0)).toBeCloseTo(dotOpacity(0, DOT_PERIOD_MS), 9)
  })
})
