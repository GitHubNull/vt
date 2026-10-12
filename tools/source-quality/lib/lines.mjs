// 行统计与掩码：把注释与字符串「抹掉」（等长替换、保留换行），用于把非空行拆分为纯注释行与代码行。
// 门禁口径只使用「非空行数」（精确值）；注释 / 代码拆分供报告分析（容忍近似，不参与门禁判定）。

const SYNTAX = {
  rust: { lineComment: true, block: 'nested', quotes: ['"', "'"], rustLiterals: true },
  typescript: { lineComment: true, block: 'flat', quotes: ['"', "'", '`'] },
  javascript: { lineComment: true, block: 'flat', quotes: ['"', "'", '`'] },
  vue: { lineComment: true, block: 'flat', quotes: ['"', "'", '`'] },
  css: { lineComment: false, block: 'flat', quotes: ['"', "'"] },
}

/** 拆分物理行：兼容 LF / CRLF / CR；文件末尾换行符不额外计行；空文件返回空数组 */
export function splitLines(src) {
  if (src === '') return []
  const lines = src.split(/\r\n|\r|\n/)
  if (lines[lines.length - 1] === '') lines.pop()
  return lines
}

/** 等长抹字：把 [from, to] 区间替换为 ch，保留换行符 */
function blank(out, from, to, ch) {
  for (let k = from; k <= to && k < out.length; k++) {
    if (out[k] !== '\n' && out[k] !== '\r') out[k] = ch
  }
}

/**
 * 掩码整份源码：注释 → 空格，字符串内容 → 'x'（两者都不含换行）。
 * 返回与原文等长的字符串；掩码后仍为非空的行即为代码行。
 */
export function maskSource(src, language) {
  const syn = SYNTAX[language]
  if (!syn || src === '') return src
  const out = src.split('')
  const n = src.length
  let i = 0

  while (i < n) {
    const c = src[i]
    if (c === '\n' || c === '\r') {
      i++
      continue
    }

    // 行注释 //（CSS 无行注释）
    if (syn.lineComment && c === '/' && src[i + 1] === '/') {
      while (i < n && src[i] !== '\n') {
        out[i] = ' '
        i++
      }
      continue
    }

    // 块注释 /* */（Rust 允许嵌套）
    if (syn.block && c === '/' && src[i + 1] === '*') {
      let depth = 1
      out[i] = ' '
      out[i + 1] = ' '
      i += 2
      while (i < n && depth > 0) {
        if (src[i] === '\n') {
          i++
          continue
        }
        if (syn.block === 'nested' && src[i] === '/' && src[i + 1] === '*') {
          depth++
          out[i] = ' '
          out[i + 1] = ' '
          i += 2
          continue
        }
        if (src[i] === '*' && src[i + 1] === '/') {
          depth--
          out[i] = ' '
          out[i + 1] = ' '
          i += 2
          continue
        }
        out[i] = ' '
        i++
      }
      continue
    }

    // Rust 原始字符串：r"…" / r#"…"# / br#"…"# / cr#"…"#
    if (syn.rustLiterals) {
      const after = tryRustRawString(src, out, i)
      if (after > i) {
        i = after
        continue
      }
      // Rust 字符字面量 / 生命周期
      if (c === "'") {
        if (src[i + 1] === '\\') {
          let j = i + 2
          while (j < n && src[j] !== "'" && src[j] !== '\n') j++
          if (j < n && src[j] === "'") {
            blank(out, i, j, 'x')
            i = j + 1
            continue
          }
        } else if (src[i + 2] === "'" && src[i + 1] !== '\n') {
          blank(out, i, i + 2, 'x')
          i += 3
          continue
        }
        i++ // 生命周期（'a）等：原样保留
        continue
      }
    }

    // 字符串字面量（普通引号不允许跨行；模板字符串除外）
    if (syn.quotes.includes(c)) {
      let j = i + 1
      while (j < n) {
        if (src[j] === '\\') {
          j += 2
          continue
        }
        if (src[j] === c) break
        if (src[j] === '\n' && c !== '`') break
        j++
      }
      const end = j < n && src[j] === c ? j : Math.min(j, n - 1)
      blank(out, i, end, 'x')
      i = end + 1
      continue
    }

    i++
  }
  return out.join('')
}

/** 识别位于 i 处的 Rust 原始字符串；返回字符串结束后的下标（未识别时返回 i 本身） */
function tryRustRawString(src, out, i) {
  const n = src.length
  const prevIsIdent = i > 0 && /[A-Za-z0-9_]/.test(src[i - 1])
  let j = i
  if (src[j] === 'b' || src[j] === 'c') {
    if (prevIsIdent) return i
    j++
  }
  if (src[j] !== 'r') return i
  if (j === i && prevIsIdent) return i
  j++
  let hashes = 0
  while (src[j] === '#') {
    hashes++
    j++
  }
  if (src[j] !== '"') return i
  const closer = `"${'#'.repeat(hashes)}`
  const endIdx = src.indexOf(closer, j + 1)
  if (endIdx === -1) return i
  blank(out, i, endIdx + closer.length - 1, 'x')
  return endIdx + closer.length
}

/**
 * 行统计：total / blank / comment / code / effective（effective = 非空行 = 门禁口径）。
 * @param {string[]} lines 原始物理行
 * @param {string[]} maskedLines 与 lines 对齐的掩码行
 */
export function lineStats(lines, maskedLines) {
  let blank = 0
  let comment = 0
  let code = 0
  for (let k = 0; k < lines.length; k++) {
    if (lines[k].trim() === '') {
      blank++
    } else if (maskedLines[k].trim() === '') {
      comment++
    } else {
      code++
    }
  }
  return { total: lines.length, blank, comment, code, effective: lines.length - blank }
}

/** 统计物理行数组中非空行数量 */
export function countEffective(lines) {
  let n = 0
  for (const line of lines) {
    if (line.trim() !== '') n++
  }
  return n
}

/**
 * 定位 Rust 文件内联测试模块：约定测试模块位于文件末尾，形如
 *   #[cfg(test)]
 *   mod tests {
 * 只有「`#[cfg(test)]` 属性行 + 紧邻（可隔空行 / 注释）的 mod 声明」才认定为测试区起点，
 * 避免把文档注释中的字样、或修饰单个测试专用方法的 #[cfg(test)] 误当成分界。
 * 返回 1 起始的行号（属性行）；未找到返回 -1。
 */
export function findRustTestStartLine(lines) {
  for (let k = 0; k < lines.length; k++) {
    if (!/^\s*#\[cfg\(test\)\]\s*$/.test(lines[k])) continue
    let j = k + 1
    while (j < lines.length && (lines[j].trim() === '' || /^\s*\/\//.test(lines[j]))) j++
    if (j < lines.length && /^\s*(pub(\([^)]*\))?\s+)?mod\s+[A-Za-z_]/.test(lines[j])) {
      return k + 1
    }
  }
  return -1
}
