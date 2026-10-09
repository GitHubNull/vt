/**
 * 窗口控制封装（tech/01 1.2）。
 */
import { invoke } from '@tauri-apps/api/core'

/** 回到语音输入小窗：显示（必要时创建）mini 窗口（T-009 后端契约 `show_mini_window`）。 */
export async function showMiniWindow(): Promise<void> {
  await invoke('show_mini_window')
}
