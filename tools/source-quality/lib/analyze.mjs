// 分析编排与门禁判定：遍历 → 行统计 → 函数提取 → 汇总；
// 门禁判定与基线计算为纯函数，便于单测与人工推演。
import { readFileSync } from 'node:fs'
import { scanFiles } from './walk.mjs'
import {
  maskSource,
  splitLines,
  lineStats,
  countEffective,
  findRustTestStartLine,
} from './lines.mjs'
import { collectTsFunctions, collectVueFunctions, loadTypeScript } from './functions.mjs'
import { collectRustFunctions } from './functions-rust.mjs'
import { repoRoot } from './config.mjs'

/** 基线文件默认说明（保留在 baseline.json 中，供后来者理解机制） */
export const DEFAULT_BASELINE_NOTE =
  '历史遗留超限文件基线（引入门禁时登记）：基线内文件行数不得再增长；文件拆分到位后运行 ' +
  'pnpm quality:baseline 清理条目。条目只允许缩小，增大需 --force 并在提交信息中说明理由。'

/**
 * 扫描并分析整个仓库。
 * @param {object} config loadConfig() 的返回值
 * @param {{ withFunctions?: boolean }} opts withFunctions=false 时跳过函数解析（更快）
 */
export async function analyzeRepo(config, { withFunctions = true } = {}) {
  const { files, unclassified } = scanFiles(repoRoot, config)
  const notes = []
  let ts = null
  if (withFunctions) {
    try {
      ts = await loadTypeScript()
    } catch {
      notes.push('未找到 typescript 依赖，已跳过 TS / JS / Vue 函数统计（可先执行 pnpm install）。')
    }
  }

  const records = []
  for (const file of files) {
    let src
    try {
      src = readFileSync(file.abs, 'utf8')
    } catch {
      notes.push(`无法读取：${file.rel}（已跳过）`)
      continue
    }
    if (src.includes('\u0000')) {
      notes.push(`疑似二进制内容：${file.rel}（已跳过）`)
      continue
    }

    const lines = splitLines(src)
    const masked = maskSource(src, file.language)
    const stats = lineStats(lines, splitLines(masked))

    const record = {
      path: file.rel,
      language: file.language,
      limit: file.limit,
      ...stats,
      gated: stats.effective,
      testStartLine: -1,
      testGated: 0,
      functions: [],
    }

    // Rust：内联测试模块不计入门禁（避免惩罚测试覆盖），测试行数单独统计
    if (file.excludeTestModules) {
      const testStart = findRustTestStartLine(lines)
      if (testStart > 0) {
        record.testStartLine = testStart
        record.gated = countEffective(lines.slice(0, testStart - 1))
        record.testGated = stats.effective - record.gated
      }
    }

    // 函数提取
    if (ts) {
      if (file.language === 'rust') {
        record.functions = collectRustFunctions(masked, { testStartLine: record.testStartLine })
      } else if (file.language === 'vue') {
        record.functions = collectVueFunctions(ts, src, file.rel)
      } else if (file.language === 'typescript' || file.language === 'javascript') {
        record.functions = collectTsFunctions(ts, src, { fileName: file.rel, ext: file.ext })
      }
    }

    records.push(record)
  }

  return {
    generatedAt: new Date().toISOString(),
    records,
    ...summarize(records, config),
    unclassified,
    notes,
    options: { withFunctions, functionDataAvailable: Boolean(ts) },
  }
}

function summarize(records, config) {
  const byLanguage = []
  let gatedTotal = 0
  let functionTotal = 0

  for (const [language, spec] of Object.entries(config.languages)) {
    const rows = records.filter((r) => r.language === language)
    if (rows.length === 0) continue
    const gatedSum = rows.reduce((acc, r) => acc + r.gated, 0)
    const fnSum = rows.reduce((acc, r) => acc + r.functions.length, 0)
    const maxRow = rows.reduce((a, b) => (b.gated > a.gated ? b : a))
    let longestFunction = null
    for (const r of rows) {
      for (const fn of r.functions) {
        if (!longestFunction || fn.lines > longestFunction.lines) {
          longestFunction = {
            name: fn.name,
            lines: fn.lines,
            startLine: fn.startLine,
            path: r.path,
          }
        }
      }
    }
    byLanguage.push({
      language,
      files: rows.length,
      gatedLines: gatedSum,
      avgLines: Math.round(gatedSum / rows.length),
      maxLines: maxRow.gated,
      maxFile: maxRow.path,
      limit: spec.limit,
      overLimit: rows.filter((r) => r.gated > r.limit).length,
      functionCount: fnSum,
      longestFunction,
    })
    gatedTotal += gatedSum
    functionTotal += fnSum
  }

  // 长函数预警（语言条目可用 functionWarnLines 覆盖默认阈值）
  const longFunctions = []
  for (const r of records) {
    const warnLines =
      config.languages[r.language].functionWarnLines ?? config.defaultFunctionWarnLines
    for (const fn of r.functions) {
      if (fn.lines >= warnLines) {
        longFunctions.push({
          path: r.path,
          language: r.language,
          name: fn.name,
          kind: fn.kind,
          startLine: fn.startLine,
          lines: fn.lines,
          inTest: fn.inTest ?? false,
        })
      }
    }
  }
  longFunctions.sort((a, b) => b.lines - a.lines)

  // 内联测试模块偏大（建议拆到独立测试文件）
  const bigTestModules = records
    .filter((r) => r.testGated > config.testModuleWarnLines)
    .map((r) => ({ path: r.path, testGated: r.testGated, testStartLine: r.testStartLine }))
    .sort((a, b) => b.testGated - a.testGated)

  return {
    byLanguage,
    totals: { fileCount: records.length, gatedLines: gatedTotal, functionCount: functionTotal },
    longFunctions,
    bigTestModules,
  }
}

/**
 * 门禁判定：超限且未在基线 → 违规；超限但在基线内且未增长 → 放行；基线过期 → 提示清理。
 * @returns {{ pass: boolean, violations: object[], grandfathered: object[], stale: object[], missing: object[] }}
 */
export function evaluateGate(analysis, baseline) {
  const entries = baseline.entries ?? {}
  const violations = []
  const grandfathered = []
  const stale = []

  for (const r of analysis.records) {
    const base = entries[r.path]
    if (r.gated > r.limit) {
      if (!base) {
        violations.push({ path: r.path, gated: r.gated, limit: r.limit, reason: 'new-over-limit' })
      } else if (r.gated > base.lines) {
        violations.push({
          path: r.path,
          gated: r.gated,
          limit: r.limit,
          baseline: base.lines,
          reason: 'grew-beyond-baseline',
        })
      } else {
        grandfathered.push({ path: r.path, gated: r.gated, limit: r.limit, baseline: base.lines })
      }
    } else if (base) {
      stale.push({ path: r.path, gated: r.gated, limit: r.limit, baseline: base.lines })
    }
  }

  const scanned = new Set(analysis.records.map((r) => r.path))
  const missing = Object.keys(entries)
    .filter((p) => !scanned.has(p))
    .map((p) => ({ path: p, baseline: entries[p].lines }))

  return { pass: violations.length === 0, violations, grandfathered, stale, missing }
}

/**
 * 计算新基线：登记所有超限文件当前行数；默认只允许条目缩小 / 移除，
 * 增大已有条目会被阻止（返回 blocked），需显式 --force。
 */
export function computeBaseline(analysis, oldBaseline, { force = false } = {}) {
  const old = oldBaseline.entries ?? {}
  const entries = {}
  const added = []
  const shrunk = []
  const grown = []
  const removed = []

  for (const r of analysis.records) {
    if (r.gated <= r.limit) continue
    entries[r.path] = { lines: r.gated }
    const prev = old[r.path]
    if (!prev) added.push({ path: r.path, lines: r.gated })
    else if (r.gated < prev.lines) shrunk.push({ path: r.path, from: prev.lines, to: r.gated })
    else if (r.gated > prev.lines) grown.push({ path: r.path, from: prev.lines, to: r.gated })
  }
  for (const p of Object.keys(old)) {
    if (!(p in entries)) removed.push({ path: p, lines: old[p].lines })
  }

  if (grown.length > 0 && !force) {
    return { blocked: grown, added, shrunk, removed }
  }

  const next = {
    note: oldBaseline.note ?? DEFAULT_BASELINE_NOTE,
    generatedAt: analysis.generatedAt.slice(0, 10),
    entries: Object.fromEntries(Object.entries(entries).sort(([a], [b]) => (a < b ? -1 : 1))),
  }
  return { next, added, shrunk, grown, removed }
}
