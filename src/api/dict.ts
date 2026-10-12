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

// ---- T-523 统一导入导出（v2 格式，替换词典 / 专有名词共用） ----

/**
 * 词典类型（统一格式 kind 字段，后端 services/dict.rs DictKind 逐字对应）。
 * 技术词库为内置只读、无用户条目，暂不参与导入导出。
 */
export type DictKind = 'replace' | 'proper_nouns'

/** 导入冲突策略（后端 ConflictStrategy 逐字对应；skip 为默认）。 */
export type ConflictStrategy = 'skip' | 'overwrite' | 'merge'

/** 单条无效条目说明（序号从 1 起 + 原因；原因不携带条目原文）。 */
export interface ImportIssue {
  index: number
  reason: string
}

/** 导入前校验结果（向导「校验结果展示」步骤）。 */
export interface ImportInspection {
  /** 识别出的文件格式："v2" / "legacy" / "tsv"。 */
  format: string
  /** 文件格式版本（旧版数组 / 纯文本为 0）。 */
  version: number
  /** 条目总数。 */
  total: number
  /** 无效条目数（导入时计入失败并跳过）。 */
  invalid: number
  /** 与现有条目冲突条数（含文件内重复）。 */
  conflicts: number
}

/** 导入报告（成功 / 更新 / 跳过 / 失败条数）。 */
export interface ImportReport {
  imported: number
  updated: number
  skipped: number
  failed: number
  issues: ImportIssue[]
}

/**
 * 导出指定词典为统一格式（v2）文件，返回导出条数；
 * 目标路径由 plugin-dialog save 选定（后端 dict_export_unified）。
 */
export async function dictExportUnified(kind: DictKind, targetPath: string): Promise<number> {
  return invoke<number>('dict_export_unified', { kind, targetPath })
}

/**
 * 导入前校验（后端 dict_import_inspect）：文件级校验失败抛 AppError
 * （E-CONFIG-003）；成功返回格式 / 版本 / 条目与冲突统计，不写入数据。
 */
export async function dictImportInspect(
  kind: DictKind,
  path: string,
): Promise<ImportInspection> {
  return invoke<ImportInspection>('dict_import_inspect', { kind, path })
}

/**
 * 执行导入（后端 dict_import_apply）：按冲突策略写入并返回导入报告；
 * 文件级校验失败抛 AppError（E-CONFIG-003），越权内容绝不执行（NFR-S-07）。
 */
export async function dictImportApply(
  kind: DictKind,
  path: string,
  strategy: ConflictStrategy,
): Promise<ImportReport> {
  return invoke<ImportReport>('dict_import_apply', { kind, path, strategy })
}
