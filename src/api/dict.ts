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

/**
 * 导出全部规则到 targetPath（plugin-dialog 选定的保存路径，后端 dict_export_file）。
 * 返回导出条数；结构形如 [{ wrong, right, enabled }, ...]。
 */
export async function dictExportFile(targetPath: string): Promise<number> {
  return invoke<number>('dict_export_file', { targetPath })
}

/**
 * 从 path 导入规则（plugin-dialog 选定的文件，后端 dict_import_file）；
 * JSON 结构校验在后端完成，返回导入条数。
 * 结构非法 / 超限等由后端返回 E-CONFIG-00x。
 */
export async function dictImportFile(path: string): Promise<number> {
  return invoke<number>('dict_import_file', { path })
}
