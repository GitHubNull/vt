#!/usr/bin/env node
// 源码质量工具入口：report（统计报告）/ check（单文件行数门禁）/ update-baseline（基线维护）。
// 完整用法与口径见 tools/source-quality/README.md 与 doc/quality/源码文件行数门禁与拆分指南.md。
import process from 'node:process'
import { loadConfig, loadBaseline, saveBaseline } from './lib/config.mjs'
import { analyzeRepo, evaluateGate, computeBaseline } from './lib/analyze.mjs'
import { renderReport, renderCheck, toJsonPayload, bold, dim, red } from './lib/render.mjs'

const USAGE = `用法：node tools/source-quality/cli.mjs <命令> [参数]
（等价的 pnpm 脚本：quality:stats / quality:gate / quality:baseline）

命令：
  report            输出源码质量统计报告（各语言文件行数、函数行数、长函数预警；默认命令）
  check             执行单文件行数门禁：超限且未登记基线、或基线内继续增长 → 退出码 1
  update-baseline   按当前状态重建基线（默认只允许条目缩小；增大需 --force 并说明理由）

参数：
  --json            以 JSON 输出（便于 AI 编程代理与脚本消费）
  --top=N           控制台展示的长函数条数（默认 10）
  --no-functions    跳过函数解析（更快，仅行数统计）
  --force           update-baseline：允许基线条目增大（谨慎使用）
  --help            显示本帮助

退出码：0 通过 / 1 门禁未通过 / 2 用法或运行错误`

const KNOWN_FLAGS = new Set(['--json', '--no-functions', '--force', '--help'])
const KNOWN_OPTS = new Set(['--top'])

async function main() {
  const args = process.argv.slice(2)
  if (args.includes('--help') || args.includes('-h')) {
    console.log(USAGE)
    return 0
  }

  const flags = new Set(args.filter((a) => a.startsWith('--') && !a.includes('=')))
  const opts = Object.fromEntries(
    args
      .filter((a) => /^--[^=]+=/.test(a))
      .map((a) => {
        const eq = a.indexOf('=')
        return [a.slice(2, eq), a.slice(eq + 1)]
      }),
  )
  for (const flag of flags) {
    if (!KNOWN_FLAGS.has(flag)) {
      console.error(red(`未知参数：${flag}`))
      console.log(USAGE)
      return 2
    }
  }
  for (const key of Object.keys(opts)) {
    if (!KNOWN_OPTS.has(`--${key}`)) {
      console.error(red(`未知参数：--${key}`))
      console.log(USAGE)
      return 2
    }
  }

  const command = args.find((a) => !a.startsWith('-')) ?? 'report'
  const asJson = flags.has('--json')
  const withFunctions = !flags.has('--no-functions')
  const top = Number.parseInt(opts.top ?? '10', 10) || 10
  const config = loadConfig()

  if (command === 'report') {
    const analysis = await analyzeRepo(config, { withFunctions })
    const gate = evaluateGate(analysis, loadBaseline())
    print(
      asJson
        ? toJsonPayload(analysis, gate, config)
        : renderReport(analysis, gate, config, { top }),
      asJson,
    )
    return 0
  }

  if (command === 'check') {
    const analysis = await analyzeRepo(config, { withFunctions })
    const gate = evaluateGate(analysis, loadBaseline())
    print(
      asJson ? toJsonPayload(analysis, gate, config) : renderCheck(gate, analysis, config),
      asJson,
    )
    return gate.pass ? 0 : 1
  }

  if (command === 'update-baseline') {
    const analysis = await analyzeRepo(config, { withFunctions: false })
    const result = computeBaseline(analysis, loadBaseline(), { force: flags.has('--force') })
    if (result.blocked) {
      console.error(red('拒绝更新：以下基线条目将增大（默认只允许缩小）：'))
      for (const g of result.blocked) {
        console.error(`  - ${g.path}：${g.from} → ${g.to}（+${g.to - g.from}）`)
      }
      console.error(dim('请先拆分文件；如确需增大（不推荐），加 --force 并在提交信息中说明理由。'))
      return 2
    }
    saveBaseline(result.next)
    console.log(`${bold('基线已更新')} · ${result.next.generatedAt}`)
    console.log(
      `  登记超限文件：${Object.keys(result.next.entries).length} 个` +
        `（新增 ${result.added.length} · 缩小 ${result.shrunk.length} · 清理 ${result.removed.length}）`,
    )
    for (const a of result.added) console.log(dim(`  + 新增登记：${a.path}（${a.lines} 行）`))
    for (const s of result.shrunk) console.log(dim(`  ↓ 缩小：${s.path}（${s.from} → ${s.to}）`))
    for (const r of result.removed) console.log(dim(`  - 清理：${r.path}（已达标或不在扫描范围）`))
    return 0
  }

  console.error(red(`未知命令：${command}`))
  console.log(USAGE)
  return 2
}

function print(content, asJson) {
  if (asJson) console.log(JSON.stringify(content, null, 2))
  else console.log(content)
}

main()
  .then((code) => {
    process.exitCode = code
  })
  .catch((err) => {
    console.error(red(`[source-quality] ${err instanceof Error ? err.message : String(err)}`))
    process.exitCode = 2
  })
