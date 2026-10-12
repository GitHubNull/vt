// 配置与基线文件的读写：路径一律相对本工具目录解析，保证在任意工作目录下运行结果一致。
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const libDir = dirname(fileURLToPath(import.meta.url))

/** 工具目录：tools/source-quality */
export const toolDir = resolve(libDir, '..')
/** 仓库根目录 */
export const repoRoot = resolve(toolDir, '..', '..')

const CONFIG_PATH = resolve(toolDir, 'config.json')
export const baselinePath = resolve(toolDir, 'baseline.json')

/**
 * 读取 config.json，并把 languages 展开为「扩展名 → 语言规则」映射。
 * @returns {{ roots: string[], excludeDirs: string[], languages: object, defaultFunctionWarnLines: number,
 *   testModuleWarnLines: number, ignoreExt: Set<string>, extMap: Map<string, object> }}
 */
export function loadConfig() {
  const raw = JSON.parse(readFileSync(CONFIG_PATH, 'utf8'))
  const extMap = new Map()
  for (const [language, spec] of Object.entries(raw.languages)) {
    for (const ext of spec.ext) {
      if (extMap.has(ext)) throw new Error(`config.json 中扩展名重复登记：${ext}`)
      extMap.set(ext, {
        language,
        limit: spec.limit,
        excludeTestModules: spec.excludeTestModules === true,
        functionWarnLines: spec.functionWarnLines,
      })
    }
  }
  return {
    ...raw,
    extMap,
    ignoreExt: new Set(raw.ignoreExt.map((e) => e.toLowerCase())),
  }
}

/** 读取基线文件（不存在时返回空基线） */
export function loadBaseline() {
  if (!existsSync(baselinePath)) return { entries: {} }
  const data = JSON.parse(readFileSync(baselinePath, 'utf8'))
  return { ...data, entries: data.entries ?? {} }
}

/** 写入基线文件（由 update-baseline 命令调用） */
export function saveBaseline(data) {
  writeFileSync(baselinePath, `${JSON.stringify(data, null, 2)}\n`, 'utf8')
}
