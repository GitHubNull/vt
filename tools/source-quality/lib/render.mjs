// 输出渲染：控制台报告 / 门禁结果 / 机器可读 JSON。对齐宽度按中文双宽计算，保证表格整齐。
import process from 'node:process'

const useColor = Boolean(process.stdout.isTTY) && !process.env.NO_COLOR
const wrap = (code) => (s) => (useColor ? `\x1b[${code}m${s}\x1b[0m` : s)

export const red = wrap(31)
export const green = wrap(32)
export const yellow = wrap(33)
export const dim = wrap(2)
export const bold = wrap(1)

const LANG_LABEL = {
  rust: 'Rust',
  typescript: 'TypeScript',
  javascript: 'JavaScript',
  vue: 'Vue SFC',
  css: 'CSS',
}

/** 控制台显示宽度：CJK 等宽字符按 2 计 */
export function displayWidth(str) {
  let width = 0
  for (const ch of String(str)) {
    const cp = ch.codePointAt(0)
    const wide =
      (cp >= 0x1100 && cp <= 0x115f) ||
      (cp >= 0x2e80 && cp <= 0x9fff) ||
      (cp >= 0xac00 && cp <= 0xd7a3) ||
      (cp >= 0xf900 && cp <= 0xfaff) ||
      (cp >= 0xfe30 && cp <= 0xfe6f) ||
      (cp >= 0xff00 && cp <= 0xff60) ||
      (cp >= 0xffe0 && cp <= 0xffe6)
    width += wide ? 2 : 1
  }
  return width
}

const ANSI_RE = /\x1b\[[0-9;]*m/g

/** 可见显示宽度：剔除 ANSI 颜色转义再计宽 */
function visibleWidth(str) {
  return displayWidth(String(str).replace(ANSI_RE, ''))
}

const pad = (str, width) => String(str) + ' '.repeat(Math.max(0, width - visibleWidth(str)))
const padLeft = (str, width) => ' '.repeat(Math.max(0, width - visibleWidth(str))) + String(str)
const fmt = (n) => String(n).replace(/\B(?=(\d{3})+(?!\d))/g, ',')

const GATE_DOC = 'doc/quality/源码文件行数门禁与拆分指南.md'

/** 渲染统计报告（report 命令） */
export function renderReport(analysis, gate, config, { top = 10 } = {}) {
  const out = []
  const date = analysis.generatedAt.slice(0, 10)
  out.push(`${bold('VibeType 源码质量报告')} · ${date}`)
  out.push(
    `扫描根：${config.roots.join('、')} · 共 ${fmt(analysis.totals.fileCount)} 个源码文件 · ` +
      `有效行合计 ${fmt(analysis.totals.gatedLines)}（Rust 已排除测试模块）`,
  )
  out.push('口径：有效行 = 非空行（注释计入、空行不计）；函数仅统计具名函数（匿名回调不计）')

  // 语言汇总
  out.push('')
  out.push(bold('■ 语言汇总'))
  const header = ['语言', '上限', '文件', '有效行', '平均', '最大', '超限', '函数数', '最长函数']
  const rows = analysis.byLanguage.map((row) => [
    LANG_LABEL[row.language] ?? row.language,
    fmt(row.limit),
    fmt(row.files),
    fmt(row.gatedLines),
    fmt(row.avgLines),
    fmt(row.maxLines),
    row.overLimit > 0 ? `${red(fmt(row.overLimit))}${dim(`/${row.files}`)}` : '0',
    fmt(row.functionCount),
    row.longestFunction
      ? `${row.longestFunction.name}（${fmt(row.longestFunction.lines)} 行）`
      : '—',
  ])
  out.push(...renderTable([header, ...rows]))

  // 行数门禁
  out.push('')
  out.push(bold('■ 行数门禁'))
  if (gate.violations.length === 0) {
    out.push(`  ${green('✔')} 无新增超限文件`)
  } else {
    out.push(
      `  ${red('✖')} 超限违规 ${gate.violations.length} 个（运行 pnpm quality:gate 查看处置要求）`,
    )
    for (const v of gate.violations) out.push(`    - ${violationText(v)}`)
  }
  if (gate.grandfathered.length > 0) {
    out.push(`  ${dim('ℹ')} 基线内遗留 ${gate.grandfathered.length} 个（不得再增长）`)
    for (const g of gate.grandfathered) {
      out.push(`    - ${padLeft(fmt(g.gated), 6)} 行  ${g.path}`)
    }
  }
  if (gate.stale.length > 0) {
    out.push(`  ${dim('ℹ')} 基线已过期 ${gate.stale.length} 个（运行 pnpm quality:baseline 清理）`)
  }

  // 长函数预警
  out.push('')
  const warnLines = config.defaultFunctionWarnLines
  if (!analysis.options.withFunctions) {
    out.push(dim(`■ 长函数预警：已按 --no-functions 跳过`))
  } else if (!analysis.options.functionDataAvailable) {
    out.push(dim(`■ 长函数预警：typescript 依赖不可用，已跳过`))
  } else {
    const shown = Math.min(top, analysis.longFunctions.length)
    out.push(
      bold(`■ 长函数预警（≥ ${warnLines} 行）· 共 ${fmt(analysis.longFunctions.length)} 个`) +
        (analysis.longFunctions.length > shown ? dim(`，显示前 ${shown}`) : ''),
    )
    for (const fn of analysis.longFunctions.slice(0, shown)) {
      const mark = fn.inTest ? dim(' [测试]') : ''
      out.push(`  ${padLeft(fmt(fn.lines), 5)} 行  ${fn.path}:${fn.startLine}  ${fn.name}${mark}`)
    }
  }

  // 测试模块偏大
  if (analysis.bigTestModules.length > 0) {
    out.push('')
    out.push(
      bold(
        `■ 测试模块偏大（> ${config.testModuleWarnLines} 行）· ${analysis.bigTestModules.length} 个`,
      ) + dim('（建议拆到独立测试文件）'),
    )
    for (const m of analysis.bigTestModules) {
      out.push(
        `  ${padLeft(fmt(m.testGated), 5)} 行  ${m.path}${dim(`（测试自第 ${m.testStartLine} 行起）`)}`,
      )
    }
  }

  // 未纳管扩展名
  if (analysis.unclassified.size > 0) {
    out.push('')
    out.push(bold('■ 未纳管扩展名'))
    for (const [ext, count] of [...analysis.unclassified.entries()].sort()) {
      out.push(`  ${ext}（${count} 个文件）${dim('—— 如需纳入门禁请在 config.json 登记')}`)
    }
  }

  for (const note of analysis.notes) out.push(yellow(`ℹ ${note}`))
  out.push('')
  out.push(dim(`提示：门禁 pnpm quality:gate · 拆分指南 ${GATE_DOC}`))
  return out.join('\n')
}

/** 渲染门禁检查结果（check 命令） */
export function renderCheck(gate, analysis, config) {
  const out = []
  const date = analysis.generatedAt.slice(0, 10)
  const limits = Object.entries(config.languages)
    .map(([lang, spec]) => `${LANG_LABEL[lang] ?? lang} ${spec.limit}`)
    .join(' / ')

  out.push(`${bold('源码文件行数门禁')} · ${date}`)
  out.push(`口径：有效行 = 非空行（注释计入）；Rust 不计 #[cfg(test)] 测试模块 · 上限：${limits}`)
  out.push(
    `扫描 ${fmt(analysis.totals.fileCount)} 个文件 · 超限违规 ${gate.violations.length} · ` +
      `基线内 ${gate.grandfathered.length} · 基线过期 ${gate.stale.length}`,
  )
  out.push('')

  if (gate.pass) {
    out.push(`${green('✔ 门禁通过')}：无新增超限文件，无基线内文件增长`)
  } else {
    out.push(
      `${red('✖ 门禁未通过')}：${gate.violations.length} 个文件超限或超过基线，请先拆分再继续加代码` +
        dim(`（指南：${GATE_DOC}）`),
    )
    for (const v of gate.violations) out.push(`  - ${violationText(v)}`)
  }

  if (gate.grandfathered.length > 0) {
    out.push(
      dim(
        `ℹ 基线内遗留文件 ${gate.grandfathered.length} 个：允许保持现状，不得增长；拆分到位后运行 pnpm quality:baseline 清理`,
      ),
    )
  }
  if (gate.stale.length > 0) {
    out.push(dim(`ℹ 基线已过期 ${gate.stale.length} 个：运行 pnpm quality:baseline 清理`))
  }
  if (gate.missing.length > 0) {
    out.push(
      dim(`ℹ 基线中有 ${gate.missing.length} 个文件不在扫描范围：运行 pnpm quality:baseline 清理`),
    )
  }
  if (analysis.options.functionDataAvailable && analysis.longFunctions.length > 0) {
    out.push(
      dim(
        `ℹ 长函数预警 ${analysis.longFunctions.length} 个（≥ ${config.defaultFunctionWarnLines} 行，不影响门禁）：pnpm quality:stats 查看明细`,
      ),
    )
  }
  for (const note of analysis.notes) out.push(yellow(`ℹ ${note}`))
  return out.join('\n')
}

/** 违规描述文案 */
function violationText(v) {
  if (v.reason === 'new-over-limit') {
    return `${v.path}：有效行 ${fmt(v.gated)}，超过上限 ${fmt(v.limit)}（未登记基线的新超限文件）`
  }
  return `${v.path}：有效行 ${fmt(v.gated)}，超过基线 ${fmt(v.baseline)}（+${fmt(v.gated - v.baseline)}，请拆分而不是继续加码）`
}

/** 生成等宽表格（按显示宽度补齐；首行为表头） */
function renderTable(rows) {
  const widths = rows[0].map((_, col) => Math.max(...rows.map((r) => visibleWidth(r[col]))))
  return rows.map((r) =>
    r
      .map((cell, col) => pad(cell, widths[col]))
      .join('  ')
      .trimEnd(),
  )
}

/** 机器可读 JSON（report / check 的 --json 输出） */
export function toJsonPayload(analysis, gate, config) {
  return {
    generatedAt: analysis.generatedAt,
    limits: Object.fromEntries(
      Object.entries(config.languages).map(([lang, spec]) => [lang, spec.limit]),
    ),
    defaultFunctionWarnLines: config.defaultFunctionWarnLines,
    summary: {
      fileCount: analysis.totals.fileCount,
      gatedLines: analysis.totals.gatedLines,
      functionCount: analysis.totals.functionCount,
      byLanguage: analysis.byLanguage,
      violations: gate ? gate.violations.length : undefined,
      grandfathered: gate ? gate.grandfathered.length : undefined,
    },
    files: analysis.records.map((r) => ({
      path: r.path,
      language: r.language,
      limit: r.limit,
      total: r.total,
      blank: r.blank,
      comment: r.comment,
      code: r.code,
      gated: r.gated,
      testStartLine: r.testStartLine > 0 ? r.testStartLine : undefined,
      testGated: r.testGated > 0 ? r.testGated : undefined,
      functionCount: r.functions.length,
      maxFunction: r.functions.length
        ? (() => {
            const fn = r.functions.reduce((a, b) => (b.lines > a.lines ? b : a))
            return { name: fn.name, startLine: fn.startLine, lines: fn.lines }
          })()
        : undefined,
    })),
    longFunctions: analysis.options.withFunctions ? analysis.longFunctions : undefined,
    bigTestModules: analysis.bigTestModules,
    unclassifiedExtensions: [...analysis.unclassified.entries()].map(([ext, count]) => ({
      ext,
      count,
    })),
    notes: analysis.notes,
    check: gate
      ? {
          pass: gate.pass,
          violations: gate.violations,
          grandfathered: gate.grandfathered,
          stale: gate.stale,
          missing: gate.missing,
        }
      : undefined,
  }
}
