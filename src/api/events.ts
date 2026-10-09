/**
 * 事件名常量（tech/03 3.5）：禁止在组件中写字面量事件名。
 * 与 src-tauri/src/events.rs 的 frontend_event_name 一一对应。
 */
export const EVENTS = {
  recordingState: 'recording:state',
  recordingLevel: 'recording:level',
  recordingResult: 'recording:result',
  deviceChanged: 'device:changed',
  configChanged: 'config:changed',
  configRecovered: 'config:recovered',
  modelProgress: 'model:progress',
  asrNotice: 'asr:notice',
  dictChanged: 'dict:changed',
  hotkeyError: 'hotkey:error',
} as const

export type EventName = (typeof EVENTS)[keyof typeof EVENTS]
