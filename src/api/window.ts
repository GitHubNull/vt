/**
 * 窗口控制封装（tech/01 1.2）。
 */
import { invoke } from '@tauri-apps/api/core'

/** 回到语音输入小窗：显示（必要时创建）mini 窗口（T-009 后端契约 `show_mini_window`）。 */
export async function showMiniWindow(): Promise<void> {
  await invoke('show_mini_window')
}

/** 打开设置主窗口（T-009 小窗"设置"按钮，后端契约 `show_settings_window`）。 */
export async function showSettingsWindow(): Promise<void> {
  await invoke('show_settings_window')
}

/** 显示悬浮球（T-008 后端契约 `show_float`；主入口为后端录音状态联动）。 */
export async function showFloat(): Promise<void> {
  await invoke('show_float')
}

/** 隐藏悬浮球（T-008 后端契约 `hide_float`；结果反馈结束后由悬浮球页调用）。 */
export async function hideFloat(): Promise<void> {
  await invoke('hide_float')
}

/** 移动悬浮球并持久化位置（T-008 后端契约 `move_float`；拖动结束时调用）。 */
export async function moveFloat(x: number, y: number): Promise<void> {
  await invoke('move_float', { x, y })
}
