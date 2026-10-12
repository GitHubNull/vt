/**
 * 导入向导纯逻辑测试（T-523，SPEC：Vitest 覆盖向导逻辑）。
 * 文案断言与 i18n 片段保持一致；getMessage 未命中时使用模块内兜底。
 */
import { describe, expect, it } from 'vitest'
import {
  defaultExportName,
  describeImportFormat,
  importFilters,
  initialWizardState,
  issueText,
  reportSummary,
  strategyOptions,
  summarizeInspection,
  wizardApplyFailure,
  wizardBackToFile,
  wizardInspectFailure,
  wizardInspectSuccess,
  wizardReport,
  wizardToStrategy,
  wizardText,
} from './importWizard'
import type { ImportInspection, ImportReport } from '../../api/dict'

const inspection = (over: Partial<ImportInspection> = {}): ImportInspection => ({
  format: 'v2',
  version: 1,
  total: 4,
  invalid: 1,
  conflicts: 2,
  ...over,
})

const report = (over: Partial<ImportReport> = {}): ImportReport => ({
  imported: 3,
  updated: 2,
  skipped: 1,
  failed: 1,
  issues: [{ index: 4, reason: '错误词为空' }],
  ...over,
})

describe('views/dict/importWizard（T-523 导入向导逻辑）', () => {
  it('初始状态停留在选文件步骤', () => {
    const state = initialWizardState()
    expect(state).toEqual({ step: 'file', path: null, inspection: null, report: null, error: null })
  })

  it('校验成功进入「校验结果」步骤', () => {
    const state = wizardInspectSuccess(initialWizardState(), '/tmp/dict.json', inspection())
    expect(state.step).toBe('review')
    expect(state.path).toBe('/tmp/dict.json')
    expect(state.inspection?.total).toBe(4)
    expect(state.error).toBeNull()
  })

  it('文件无条目（total 0）停留选文件并提示', () => {
    const state = wizardInspectSuccess(initialWizardState(), '/tmp/empty.json', inspection({ total: 0 }))
    expect(state.step).toBe('file')
    expect(state.inspection).toBeNull()
    expect(state.error).toBeTruthy()
  })

  it('校验失败停留选文件并展示原因', () => {
    const state = wizardInspectFailure(initialWizardState(), '导入文件不是合法的 JSON')
    expect(state.step).toBe('file')
    expect(state.error).toBe('导入文件不是合法的 JSON')
  })

  it('步骤迁移：review → strategy → report；apply 失败留在 strategy', () => {
    let state = wizardInspectSuccess(initialWizardState(), '/tmp/dict.json', inspection())
    // 非 review 步骤不可进入 strategy。
    expect(wizardToStrategy(initialWizardState()).step).toBe('file')
    state = wizardToStrategy(state)
    expect(state.step).toBe('strategy')

    const failed = wizardApplyFailure(state, '写入词典文件失败')
    expect(failed.step).toBe('strategy')
    expect(failed.error).toBe('写入词典文件失败')

    const done = wizardReport(failed, report())
    expect(done.step).toBe('report')
    expect(done.report?.imported).toBe(3)
    expect(done.error).toBeNull()

    // 重新选择：清空校验结果与报告。
    const back = wizardBackToFile(done)
    expect(back).toEqual({
      step: 'file',
      path: null,
      inspection: null,
      report: null,
      error: null,
    })
  })

  it('describeImportFormat：v2 / legacy / tsv 映射，未知格式原样返回', () => {
    expect(describeImportFormat('v2')).toBe('统一格式')
    expect(describeImportFormat('legacy')).toBe('旧版 JSON 数组格式')
    expect(describeImportFormat('tsv')).toBe('纯文本（制表符分隔）')
    expect(describeImportFormat('future')).toBe('future')
  })

  it('summarizeInspection：旧版 / 纯文本无版本号显示「—」', () => {
    const v2 = summarizeInspection(inspection())
    expect(v2).toEqual({ format: '统一格式', version: '1', total: '4', invalid: '1', conflicts: '2' })
    const legacy = summarizeInspection(inspection({ format: 'legacy', version: 0 }))
    expect(legacy.version).toBe('—')
    expect(legacy.format).toBe('旧版 JSON 数组格式')
  })

  it('reportSummary / issueText：占位符全部替换、计数准确', () => {
    const summary = reportSummary(report())
    expect(summary).not.toContain('{')
    expect(summary).toContain('成功导入 3 条')
    expect(summary).toContain('更新 2 条')
    expect(summary).toContain('跳过 1 条')
    expect(summary).toContain('失败 1 条')

    const issue = issueText({ index: 4, reason: '错误词为空' })
    expect(issue).not.toContain('{')
    expect(issue).toBe('第 4 条：错误词为空')
  })

  it('strategyOptions：三种策略且值为后端契约字面量', () => {
    const options = strategyOptions()
    expect(options.map((option) => option.value)).toEqual(['skip', 'overwrite', 'merge'])
    for (const option of options) {
      expect(option.label.length).toBeGreaterThan(0)
      expect(option.description.length).toBeGreaterThan(0)
    }
  })

  it('importFilters：替换词典支持纯文本，专有名词仅 JSON', () => {
    const replace = importFilters('replace')
    expect(replace[0]).toEqual({ name: 'JSON', extensions: ['json'] })
    expect(replace.flatMap((filter) => filter.extensions)).toContain('txt')
    expect(replace.flatMap((filter) => filter.extensions)).toContain('tsv')
    expect(importFilters('proper_nouns')).toEqual([{ name: 'JSON', extensions: ['json'] }])
  })

  it('defaultExportName：按词典类型给默认文件名', () => {
    expect(defaultExportName('replace')).toBe('vibetype-replace-dict.json')
    expect(defaultExportName('proper_nouns')).toBe('vibetype-proper-nouns.json')
  })

  it('wizardText：关键文案齐全（非空字符串），标题按词典类型区分', () => {
    for (const kind of ['replace', 'proper_nouns'] as const) {
      const txt = wizardText(kind)
      for (const value of Object.values(txt)) {
        expect(typeof value).toBe('string')
        expect(value.length).toBeGreaterThan(0)
      }
    }
    expect(wizardText('replace').title).toBe('导入替换词典')
    expect(wizardText('proper_nouns').title).toBe('导入专有名词')
  })
})
