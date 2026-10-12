// Rust 函数提取：在「掩码文本」（注释 / 字符串已抹掉）上做 fn 定位 + 花括号配对，
// 不依赖 rustc / 第三方解析器；实现细节与已知边界见 tools/source-quality/README.md。

/** 构造换行位置索引，用于字符下标 → 行号的二分查找 */
function buildLineIndex(s) {
  const idx = []
  for (let k = 0; k < s.length; k++) {
    if (s[k] === '\n') idx.push(k)
  }
  return idx
}

function lineAt(index, pos) {
  let lo = 0
  let hi = index.length
  while (lo < hi) {
    const mid = (lo + hi) >> 1
    if (index[mid] <= pos) lo = mid + 1
    else hi = mid
  }
  return lo + 1
}

/** 从 fn 名称之后查找函数体起始 `{`；先遇到 `;` 说明是 trait 声明等无体函数，返回 -1 */
function findBodyStart(s, from) {
  const limit = Math.min(s.length, from + 4000)
  for (let k = from; k < limit; k++) {
    const c = s[k]
    if (c === '{') return k
    if (c === ';') return -1
  }
  return -1
}

/** 花括号配对，返回与 open 匹配的 `}` 下标；未闭合返回 -1 */
function matchBraces(s, open) {
  let depth = 0
  for (let k = open; k < s.length; k++) {
    const c = s[k]
    if (c === '{') depth++
    else if (c === '}') {
      depth--
      if (depth === 0) return k
    }
  }
  return -1
}

/**
 * 提取 Rust 函数（含方法、嵌套函数）。
 * @param {string} masked 掩码后的源码文本（见 lines.mjs maskSource）
 * @param {{ testStartLine?: number }} opts testStartLine：内联测试模块起始行（1 起始），用于标记 inTest
 * @returns {{ name: string, kind: string, startLine: number, endLine: number, lines: number, inTest: boolean }[]}
 */
export function collectRustFunctions(masked, { testStartLine = -1 } = {}) {
  const lineIndex = buildLineIndex(masked)
  const found = []
  const re = /\bfn\s+([A-Za-z_][A-Za-z0-9_]*)/g
  let m
  while ((m = re.exec(masked))) {
    const brace = findBodyStart(masked, m.index + m[0].length)
    if (brace === -1) continue
    const bodyEnd = matchBraces(masked, brace)
    if (bodyEnd === -1) continue
    const startLine = lineAt(lineIndex, m.index)
    const endLine = lineAt(lineIndex, bodyEnd)
    found.push({
      name: m[1],
      kind: 'fn',
      startLine,
      endLine,
      lines: endLine - startLine + 1,
      inTest: testStartLine > 0 && startLine >= testStartLine,
    })
  }
  return found
}
