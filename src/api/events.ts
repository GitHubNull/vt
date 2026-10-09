/**
 * 事件名常量（tech/03 3.5）：禁止在组件中写字面量事件名。
 */
export const EVENTS = {
  recordingState: 'recording:state',
  recordingLevel: 'recording:level',
  recordingResult: 'recording:result',
  deviceChanged: 'device:changed',
  configChanged: 'config:changed',
} as const

export type EventName = (typeof EVENTS)[keyof typeof EVENTS]
