/**
 * 关于 / 应用信息命令封装（T-016 命令契约，后端按 T-016 SPEC 实现）。
 * getAppInfo 同时服务于 T-013 系统与数据页（数据目录展示）。
 */
import { invoke } from '@tauri-apps/api/core'
import type { AppInfo } from './types'

/** 应用信息：版本、构建信息、三个平台标准目录（tech/08 8.1）。 */
export async function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>('get_app_info')
}
