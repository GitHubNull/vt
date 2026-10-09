/**
 * 悬浮球动效参数计算（T-008 SPEC 测试要求：纯函数可单测）。
 * 全部函数确定性：同一输入必得同一输出（时间参数由调用方注入，便于测试）。
 */

/** 波形条数（胶囊内 Canvas 宽度按此均分）。 */
export const WAVE_BAR_COUNT = 5
/** 波形条最小间隔（px）。 */
export const WAVE_BAR_GAP = 3
/** 识别中三点加载动画的单点周期（ms）。 */
export const DOT_PERIOD_MS = 900
/** 取消拖动阈值（px）：拖动超过即进入"松开取消"态。 */
export const CANCEL_DRAG_THRESHOLD = 40
/** 有效拖动阈值（px）：超过才视为拖动（区分点击），并触发位置记忆。 */
export const SIGNIFICANT_DRAG_THRESHOLD = 4
/** 即将超时倒计时的触发阈值（剩余秒数，PRD 4.3：距离上限 10 秒）。 */
export const COUNTDOWN_THRESHOLD_SEC = 10

/** 结果反馈停留时长（ms，PRD 4.3：完成 0.8 s、已取消 0.5 s、降级 2 s）。 */
export const FEEDBACK_DURATIONS: Record<string, number> = {
  success: 800,
  cancelled: 500,
  degraded: 2000,
  failed: 2500,
}

/** 归一化电平的一阶平滑（EMA），防止波形抖动刺眼。 */
export function smoothLevel(previous: number, next: number, alpha = 0.35): number {
  const clamped = clamp01(next)
  return clamp01(previous + (clamped - previous) * alpha)
}

/** 静音呼吸系数：0.94 ~ 1.06 间按正弦缓慢变化（PRD 4.5.4：静音时保持低幅度呼吸）。 */
export function breathScale(nowMs: number, periodMs = 1600): number {
  return 1 + 0.06 * Math.sin((nowMs / periodMs) * Math.PI * 2)
}

/**
 * 单条波形高度（归一化 0~1）：静音呼吸底 + 音量驱动 + 逐条伪随机相位。
 * 伪随机用 sin 哈希（确定性，无状态），按 120 ms 跳变保证动画节奏稳定。
 */
export function waveHeight(level: number, index: number, nowMs: number): number {
  const tick = Math.floor(nowMs / 120)
  const jitter = pseudoRandom(index * 73 + tick * 131)
  const idle = 0.08 + 0.04 * Math.sin((nowMs / 1600) * Math.PI * 2 + index)
  return clamp01(idle + clamp01(level) * (0.45 + 0.55 * jitter))
}

/** 时长格式化：毫秒 → "m:ss"（录音时长展示）。 */
export function formatElapsed(milliseconds: number): string {
  const totalSeconds = Math.max(0, Math.floor(milliseconds / 1000))
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  return `${minutes}:${String(seconds).padStart(2, '0')}`
}

/**
 * 即将超时倒计时：剩余时长不足阈值时返回剩余秒数（取整），否则 null。
 * PRD 4.3：计时变为橙色倒计时。
 */
export function remainingSeconds(elapsedMs: number, maxMinutes: number): number | null {
  const remainingMs = maxMinutes * 60 * 1000 - elapsedMs
  const remaining = Math.ceil(remainingMs / 1000)
  return remaining > 0 && remaining <= COUNTDOWN_THRESHOLD_SEC ? remaining : null
}

/** 结果反馈停留时长（未知类型按降级处理，保证必有返回值）。 */
export function feedbackDurationMs(kind: string): number {
  return FEEDBACK_DURATIONS[kind] ?? FEEDBACK_DURATIONS.degraded
}

/** 拖动距离（欧氏）。 */
export function dragDistance(dx: number, dy: number): number {
  return Math.sqrt(dx * dx + dy * dy)
}

/** 是否进入"松开取消"态（拖动到取消区，T-008 SPEC：拖动到取消区取消）。 */
export function isCancelDrag(dx: number, dy: number): boolean {
  return dragDistance(dx, dy) >= CANCEL_DRAG_THRESHOLD
}

/** 是否构成有效拖动（需要移动窗口并记忆位置）。 */
export function isSignificantDrag(dx: number, dy: number): boolean {
  return dragDistance(dx, dy) >= SIGNIFICANT_DRAG_THRESHOLD
}

/** 三点加载动画中第 n 个点（0 起）的不透明度（识别中动效）。 */
export function dotOpacity(dotIndex: number, nowMs: number, periodMs = DOT_PERIOD_MS): number {
  const phase = ((nowMs % periodMs) / periodMs + dotIndex / 3) % 1
  return 0.25 + 0.75 * Math.sin(phase * Math.PI)
}

function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value))
}

/** 确定性伪随机（sin 哈希）：同 seed 同值，范围 0~1。 */
function pseudoRandom(seed: number): number {
  const x = Math.sin(seed) * 10000
  return x - Math.floor(x)
}
