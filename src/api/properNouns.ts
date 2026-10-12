/**
 * 专有名词命令封装（T-522 契约，src-tauri/src/commands/dict.rs）。
 * 命令名与参数名与后端逐字一致（tech/03 3.5）。
 */
import { invoke } from '@tauri-apps/api/core'
import { dictExportUnified, dictImportApply, dictImportInspect } from './dict'
import type { ConflictStrategy, ImportInspection, ImportReport } from './dict'
import type { ProperNoun } from './types'

/** 条目清单。 */
export async function properNounList(): Promise<ProperNoun[]> {
  return invoke<ProperNoun[]>('proper_noun_list')
}

/** 新增条目；标准写法为空 / 重复 / 超上限抛 AppError（E-CONFIG-011/012）。 */
export async function properNounAdd(
  term: string,
  aliases: string[],
  note: string,
  enabled: boolean,
): Promise<ProperNoun> {
  return invoke<ProperNoun>('proper_noun_add', { term, aliases, note, enabled })
}

/** 更新条目（整体替换）。 */
export async function properNounUpdate(noun: ProperNoun): Promise<void> {
  await invoke('proper_noun_update', { noun })
}

/** 删除条目。 */
export async function properNounDelete(id: number): Promise<void> {
  await invoke('proper_noun_delete', { id })
}

/** 启停切换。 */
export async function properNounToggle(id: number, enabled: boolean): Promise<void> {
  await invoke('proper_noun_toggle', { id, enabled })
}

// ---- T-523 统一导入导出（v2 格式；kind 固定为 proper_nouns） ----

/** 导出专有名词为统一格式文件，返回导出条数（目标路径由 plugin-dialog 选定）。 */
export async function properNounExportUnified(targetPath: string): Promise<number> {
  return dictExportUnified('proper_nouns', targetPath)
}

/** 导入前校验专有名词文件（文件级失败抛 AppError E-CONFIG-003）。 */
export async function properNounImportInspect(path: string): Promise<ImportInspection> {
  return dictImportInspect('proper_nouns', path)
}

/** 按冲突策略导入专有名词文件，返回导入报告。 */
export async function properNounImportApply(
  path: string,
  strategy: ConflictStrategy,
): Promise<ImportReport> {
  return dictImportApply('proper_nouns', path, strategy)
}
