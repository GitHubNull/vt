/**
 * 替换词典命令封装（T-011 契约，src-tauri/src/commands/dict.rs）。
 * 命令名与参数名与后端逐字一致（tech/03 3.5）。
 */
import { invoke } from '@tauri-apps/api/core'
import type { ReplaceRule } from './types'

/** 规则清单。 */
export async function dictListRules(): Promise<ReplaceRule[]> {
  return invoke<ReplaceRule[]>('dict_list_rules')
}

/** 新增规则；错误词重复 / 为空 / 超上限抛 AppError（E-CONFIG-00x）。 */
export async function dictAddRule(
  wrong: string,
  right: string,
  enabled: boolean,
): Promise<ReplaceRule> {
  return invoke<ReplaceRule>('dict_add_rule', { wrong, right, enabled })
}

/** 更新规则（整体替换）。 */
export async function dictUpdateRule(rule: ReplaceRule): Promise<void> {
  await invoke('dict_update_rule', { rule })
}

/** 删除规则。 */
export async function dictDeleteRule(id: number): Promise<void> {
  await invoke('dict_delete_rule', { id })
}

/** 启停切换。 */
export async function dictToggleRule(id: number, enabled: boolean): Promise<void> {
  await invoke('dict_toggle_rule', { id, enabled })
}
