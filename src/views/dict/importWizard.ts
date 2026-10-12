/**
 * 导入向导纯逻辑（T-523）：步骤状态机 / 校验结果与报告文案 / 策略选项 /
 * 文件对话框过滤器与默认文件名。独立成模块便于 Vitest 覆盖（Node 环境）。
 *
 * 文案说明：新提示文案的权威位置是 src/i18n/zh-CN.ts（与 en-US.ts），
 * key 与取值见 T-523 任务报告的 i18n 合并片段；在 i18n 合并前，本模块以
 * `getMessage` + 中文兜底保证界面完整可用——合并后 getMessage 命中 i18n，
 * 兜底自动失效，无需改代码。既有 key（如 t.dict.replace.*）仍直接走 i18n。
 */
import { getMessage } from '../../i18n'
import type { ConflictStrategy, DictKind, ImportInspection, ImportIssue, ImportReport } from '../../api/dict'

/** 取 i18n 文案；key 尚未合并进 i18n 时回退到内置中文兜底。 */
function text(key: string, fallback: string): string {
  const value = getMessage(key)
  return value === key ? fallback : value
}

/**
 * 导出给词典页复用的文案读取（专有名词页导出 / 导入按钮文案等）；
 * 命中 i18n 用 i18n 值，未合并 key 用兜底。
 */
export function i18nText(key: string, fallback: string): string {
  return text(key, fallback)
}

/** 向导步骤：选文件 → 校验结果 → 冲突策略 → 报告（SPEC T-523 步骤 5）。 */
export type WizardStep = 'file' | 'review' | 'strategy' | 'report'

/** 向导状态（纯数据，由 reducer 函数迁移）。 */
export interface WizardState {
  step: WizardStep
  /** 已选定的文件路径（未选择为 null）。 */
  path: string | null
  /** 校验结果（尚未校验为 null）。 */
  inspection: ImportInspection | null
  /** 导入报告（尚未导入为 null）。 */
  report: ImportReport | null
  /** 当前失败 / 提示信息（无则 null）。 */
  error: string | null
}

/** 打开向导 / 重新开始的初始状态。 */
export function initialWizardState(): WizardState {
  return { step: 'file', path: null, inspection: null, report: null, error: null }
}

/**
 * 校验成功：进入「校验结果」步骤；
 * 文件无条目（total 0）时停留在「选择文件」并提示重新选择。
 */
export function wizardInspectSuccess(
  state: WizardState,
  path: string,
  inspection: ImportInspection,
): WizardState {
  if (inspection.total === 0) {
    return {
      ...state,
      step: 'file',
      path,
      inspection: null,
      error: text('dict.importExport.emptyFile', '文件中没有可导入的条目，请重新选择。'),
    }
  }
  return { step: 'review', path, inspection, report: null, error: null }
}

/** 校验失败（后端整体拒绝，E-CONFIG-003）：停留在「选择文件」并展示原因。 */
export function wizardInspectFailure(state: WizardState, message: string): WizardState {
  return { ...state, step: 'file', inspection: null, error: message }
}

/** 校验结果 → 冲突策略。 */
export function wizardToStrategy(state: WizardState): WizardState {
  if (state.step !== 'review') return state
  return { ...state, step: 'strategy', error: null }
}

/** 返回「选择文件」重新选择（丢弃校验结果与报告）。 */
export function wizardBackToFile(state: WizardState): WizardState {
  return { ...state, step: 'file', path: null, inspection: null, report: null, error: null }
}

/** 导入完成（含部分跳过 / 失败）→ 「导入报告」。 */
export function wizardReport(state: WizardState, report: ImportReport): WizardState {
  return { ...state, step: 'report', report, error: null }
}

/** 导入执行失败（文件级拒绝 / 写盘失败）：停留在冲突策略步骤并展示原因。 */
export function wizardApplyFailure(state: WizardState, message: string): WizardState {
  return { ...state, error: message }
}

/** 识别出的文件格式 → 展示文案。 */
export function describeImportFormat(format: string): string {
  switch (format) {
    case 'v2':
      return text('dict.importExport.formatV2', '统一格式')
    case 'legacy':
      return text('dict.importExport.formatLegacy', '旧版 JSON 数组格式')
    case 'tsv':
      return text('dict.importExport.formatTsv', '纯文本（制表符分隔）')
    default:
      return format
  }
}

/** 校验结果展示行（值已字符串化；旧版 / 纯文本无版本号显示「—」）。 */
export interface InspectionSummary {
  format: string
  version: string
  total: string
  invalid: string
  conflicts: string
}

/** 校验结果 → 展示行。 */
export function summarizeInspection(inspection: ImportInspection): InspectionSummary {
  return {
    format: describeImportFormat(inspection.format),
    version: inspection.version > 0 ? String(inspection.version) : '—',
    total: String(inspection.total),
    invalid: String(inspection.invalid),
    conflicts: String(inspection.conflicts),
  }
}

/** 导入报告摘要文案（成功 / 更新 / 跳过 / 失败条数）。 */
export function reportSummary(report: ImportReport): string {
  return text(
    'dict.importExport.reportSummary',
    '成功导入 {imported} 条，更新 {updated} 条，跳过 {skipped} 条，失败 {failed} 条。',
  )
    .replace('{imported}', String(report.imported))
    .replace('{updated}', String(report.updated))
    .replace('{skipped}', String(report.skipped))
    .replace('{failed}', String(report.failed))
}

/** 单条无效条目展示文案（序号 + 原因）。 */
export function issueText(issue: ImportIssue): string {
  return text('dict.importExport.issueLine', '第 {index} 条：{reason}')
    .replace('{index}', String(issue.index))
    .replace('{reason}', issue.reason)
}

/** 冲突策略选项（值为后端 ConflictStrategy 契约字面量）。 */
export interface StrategyOption {
  value: ConflictStrategy
  label: string
  description: string
}

/** 三种冲突策略的展示选项（按 UI 展示顺序）。 */
export function strategyOptions(): StrategyOption[] {
  return [
    {
      value: 'skip',
      label: text('dict.importExport.strategySkip', '跳过'),
      description: text(
        'dict.importExport.strategySkipDesc',
        '保留现有条目，冲突条目计入「跳过」。',
      ),
    },
    {
      value: 'overwrite',
      label: text('dict.importExport.strategyOverwrite', '覆盖'),
      description: text(
        'dict.importExport.strategyOverwriteDesc',
        '冲突条目以文件内容整体覆盖现有条目。',
      ),
    },
    {
      value: 'merge',
      label: text('dict.importExport.strategyMerge', '合并'),
      description: text(
        'dict.importExport.strategyMergeDesc',
        '以现有条目为准合并：补全缺失信息，保留现有设置。',
      ),
    },
  ]
}

/** 导入文件对话框过滤器：替换词典额外支持纯文本（PRD 3.4.1 业务规则 5）。 */
export function importFilters(kind: DictKind): { name: string; extensions: string[] }[] {
  const json = { name: 'JSON', extensions: ['json'] }
  if (kind === 'replace') {
    return [json, { name: text('dict.importExport.filterText', '纯文本（制表符分隔）'), extensions: ['txt', 'tsv'] }]
  }
  return [json]
}

/** 导出默认文件名（按词典类型）。 */
export function defaultExportName(kind: DictKind): string {
  return kind === 'proper_nouns' ? 'vibetype-proper-nouns.json' : 'vibetype-replace-dict.json'
}

/** 向导文案（模板直接绑定；键值详见报告中的 i18n 片段）。 */
export function wizardText(kind: DictKind) {
  return {
    // 标题复用既有 i18n key（替换词典）；专有名词 key 列入 i18n 合并片段。
    title:
      kind === 'proper_nouns'
        ? text('dict.properNouns.importTitle', '导入专有名词')
        : text('dict.replace.importTitle', '导入替换词典'),
    stepFile: text('dict.importExport.stepFile', '选择文件'),
    stepReview: text('dict.importExport.stepReview', '格式校验'),
    stepStrategy: text('dict.importExport.stepStrategy', '冲突处理'),
    stepReport: text('dict.importExport.stepReport', '导入报告'),
    pickHint: text(
      'dict.importExport.pickHint',
      '选择要导入的文件：支持本产品导出的词典文件；替换词典还支持「每行一条、错误词与正确词用制表符分隔」的纯文本。',
    ),
    pick: text('dict.importExport.pick', '选择文件'),
    repick: text('dict.importExport.repick', '重新选择'),
    next: text('dict.importExport.next', '下一步'),
    back: text('dict.importExport.back', '上一步'),
    apply: text('dict.importExport.apply', '开始导入'),
    close: text('dict.importExport.close', '完成'),
    inspecting: text('dict.importExport.inspecting', '校验中…'),
    applying: text('dict.importExport.applying', '导入中…'),
    needFile: text('dict.importExport.needFile', '请先选择要导入的文件。'),
    formatLabel: text('dict.importExport.formatLabel', '文件格式'),
    versionLabel: text('dict.importExport.versionLabel', '格式版本'),
    totalLabel: text('dict.importExport.totalLabel', '条目总数'),
    invalidLabel: text('dict.importExport.invalidLabel', '无效条目'),
    conflictsLabel: text('dict.importExport.conflictsLabel', '与现有条目冲突'),
    invalidHint: text(
      'dict.importExport.invalidHint',
      '无效条目将在导入时计入失败并跳过，其余条目继续导入。',
    ),
    strategyHint: text('dict.importExport.strategyHint', '与现有条目冲突时的处理方式：'),
  }
}
