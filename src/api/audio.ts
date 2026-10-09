/**
 * 音频设备枚举封装（T-013 调 T-005 枚举命令）。
 * TODO(后端契约)：命令 `list_audio_devices` 由 T-005 实现，返回 AudioDeviceInfo[]。
 */
import { invoke } from '@tauri-apps/api/core'
import type { AudioDeviceInfo } from './types'

/** 列出系统输入设备（麦克风）；设备 ID 用于 AudioConfig.input_device_id。 */
export async function listAudioDevices(): Promise<AudioDeviceInfo[]> {
  return invoke<AudioDeviceInfo[]>('list_audio_devices')
}

/** 录音控制命令（T-008 悬浮球取消区、T-009 小窗麦克风按钮、T-010 取消入口）。 */
export type RecordCommandName = 'start' | 'stop' | 'toggle' | 'cancel'

/**
 * 发送录音控制命令（后端契约 `control_recording`）：与快捷键共用同一状态机，
 * 录音中重复 start 由后端忽略（单会话互斥）。
 */
export async function controlRecording(command: RecordCommandName): Promise<void> {
  await invoke('control_recording', { command })
}
