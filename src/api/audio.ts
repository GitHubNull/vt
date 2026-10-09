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
