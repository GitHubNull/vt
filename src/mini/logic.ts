/**
 * 语音输入小窗交互逻辑（T-009 SPEC 测试要求：按钮交互模式分支可单测）。
 * 纯函数：同一输入必得同一输出，不依赖 DOM / Tauri 运行时。
 */

/** 麦克风按钮手势。 */
export type MicGesture = 'press' | 'release' | 'click'
/** 录音控制命令（与后端 control_recording 契约一致）。 */
export type RecordAction = 'start' | 'stop' | 'toggle'

/** 音量环几何：半径 44 的圆周（SVG ring around 88px 按钮，PRD 4.2.2）。 */
export const RING_RADIUS = 44
export const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS

/**
 * 麦克风按钮交互 → 录音控制命令（T-009 SPEC 步骤 2）。
 *
 * - 按住（hold）：press 开始、release 结束；
 * - 切换（toggle）：click 翻转开始 / 结束，press / release 忽略；
 * - 单会话互斥：录音中 hold 的 press 不重复触发（T-009 验收 4；
 *   后端 AudioService 同样忽略重复 start，双保险）。
 *
 * @param mode 触发模式（配置 hotkey.mode）
 * @param recording 当前是否录音中（recording:state 驱动）
 * @param gesture 指针手势
 * @returns 录音控制命令；无需动作时返回 null
 */
export function resolveRecordAction(
  mode: 'hold' | 'toggle',
  recording: boolean,
  gesture: MicGesture,
): RecordAction | null {
  if (mode === 'hold') {
    if (gesture === 'press') return recording ? null : 'start'
    if (gesture === 'release') return recording ? 'stop' : null
    return null
  }
  // 切换模式：仅 click 翻转。
  if (gesture === 'click') return 'toggle'
  return null
}

/** 页面阶段：idle（空闲）→ recording → processing → feedback。 */
export type MiniPhase = 'idle' | 'recording' | 'processing' | 'feedback'

/**
 * 状态文案的 i18n key（T-009 状态文案）。
 * 空闲提示随模式变化：hold 提示"按住说话"，toggle 提示"点击开始"。
 */
export function statusKey(phase: MiniPhase, mode: 'hold' | 'toggle'): string {
  switch (phase) {
    case 'recording':
      return 'mini.status.recording'
    case 'processing':
      return 'mini.status.processing'
    case 'feedback':
      return 'mini.status.cancelled'
    default:
      return mode === 'toggle' ? 'mini.status.toggle' : 'mini.status.hold'
  }
}

/**
 * 音量环样式：返回 SVG stroke-dasharray / stroke-dashoffset，
 * level 0~1 映射为环的填充比例（录音音量反馈，T-009）。
 */
export function ringStyle(level: number): { dasharray: number; dashoffset: number } {
  const clamped = Math.min(1, Math.max(0, level))
  return {
    dasharray: RING_CIRCUMFERENCE,
    dashoffset: RING_CIRCUMFERENCE * (1 - clamped),
  }
}
