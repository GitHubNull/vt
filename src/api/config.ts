/**
 * 配置命令封装（T-003 契约，src-tauri/src/commands/config.rs）。
 */
import { invoke } from '@tauri-apps/api/core'
import type { AppConfig, ConfigSection } from './types'

/**
 * 获取完整配置镜像。
 * 注意：后端 `get_config` 无参数、返回完整 AppConfig（分节由前端/store 消费）。
 */
export async function getConfig(): Promise<AppConfig> {
  return invoke<AppConfig>('get_config')
}

/** 更新指定配置节并原子落盘；成功后后端广播 config:changed。失败抛 AppError（如 E-HOTKEY-001 冲突回滚）。 */
export async function setConfig<S extends ConfigSection>(
  section: S,
  value: AppConfig[S],
): Promise<void> {
  await invoke('set_config', { section, value })
}
