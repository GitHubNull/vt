// 源码文件遍历：按配置扫描各根目录，跳过排除目录、隐藏文件与已登记的忽略扩展名。
import { readdirSync } from 'node:fs'
import { extname, resolve } from 'node:path'

/**
 * @param {string} repoRoot 仓库根目录绝对路径
 * @param {object} config loadConfig() 的返回值
 * @returns {{ files: object[], unclassified: Map<string, number> }}
 *   files：已纳管语言的源码文件（rel 为 POSIX 风格相对路径）
 *   unclassified：扫描到但未在 config.json 登记、也未被忽略的扩展名统计
 */
export function scanFiles(repoRoot, config) {
  const files = []
  const unclassified = new Map()
  const excludeDirs = new Set(config.excludeDirs)

  for (const root of config.roots) {
    walk(resolve(repoRoot, root), root)
  }

  files.sort((a, b) => (a.rel < b.rel ? -1 : 1))
  return { files, unclassified }

  function walk(absDir, relDir) {
    let entries
    try {
      entries = readdirSync(absDir, { withFileTypes: true })
    } catch {
      return // 根目录可能不存在（如 tools 尚未创建），静默跳过
    }
    entries.sort((a, b) => (a.name < b.name ? -1 : 1))
    for (const entry of entries) {
      const name = entry.name
      if (name.startsWith('.')) continue
      if (entry.isDirectory()) {
        if (excludeDirs.has(name)) continue
        walk(resolve(absDir, name), `${relDir}/${name}`)
        continue
      }
      if (!entry.isFile()) continue
      const ext = extname(name).toLowerCase()
      if (config.ignoreExt.has(ext)) continue
      const mapped = config.extMap.get(ext)
      if (!mapped) {
        unclassified.set(ext, (unclassified.get(ext) ?? 0) + 1)
        continue
      }
      files.push({
        rel: `${relDir}/${name}`,
        abs: resolve(absDir, name),
        ext,
        language: mapped.language,
        limit: mapped.limit,
        excludeTestModules: mapped.excludeTestModules,
        functionWarnLines: mapped.functionWarnLines,
      })
    }
  }
}
