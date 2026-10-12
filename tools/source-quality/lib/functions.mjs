// TS / TSX / JS / Vue script 段函数提取：基于 typescript 编译器 API（devDependency，动态加载）。
// 仅统计「具名函数」：函数声明、类方法、构造器、访问器、赋值给变量 / 属性的箭头函数与函数表达式；
// 匿名回调（如 setTimeout(() => {...})）不计入，避免噪音。

/** 动态加载 typescript（未安装 devDependencies 时由调用方降级处理） */
export async function loadTypeScript() {
  const mod = await import('typescript')
  return mod.default ?? mod
}

function scriptKindOf(ts, ext) {
  switch (ext) {
    case '.tsx':
      return ts.ScriptKind.TSX
    case '.js':
    case '.mjs':
    case '.cjs':
      return ts.ScriptKind.JS
    default:
      return ts.ScriptKind.TS
  }
}

/**
 * 提取 TS/JS 源码中的具名函数。
 * @param {object} ts typescript 模块
 * @param {string} source 源码文本
 * @param {{ fileName?: string, ext?: string, lineOffset?: number }} opts
 *   lineOffset：源码在宿主文件中的起始行偏移（Vue script 段用），使行号回到宿主文件坐标系
 * @returns {{ name: string, kind: string, startLine: number, endLine: number, lines: number }[]}
 */
export function collectTsFunctions(
  ts,
  source,
  { fileName = 'source.ts', ext = '.ts', lineOffset = 0 } = {},
) {
  const sf = ts.createSourceFile(
    fileName,
    source,
    ts.ScriptTarget.Latest,
    true,
    scriptKindOf(ts, ext),
  )
  const found = []

  const visit = (node) => {
    const info = nameOfFunctionLike(node)
    if (info) {
      const start = sf.getLineAndCharacterOfPosition(node.getStart(sf))
      const end = sf.getLineAndCharacterOfPosition(node.getEnd())
      found.push({
        name: info.name,
        kind: info.kind,
        startLine: lineOffset + start.line + 1,
        endLine: lineOffset + end.line + 1,
        lines: end.line - start.line + 1,
      })
    }
    ts.forEachChild(node, visit)
  }

  const nameOfFunctionLike = (node) => {
    const text = (idNode) => (idNode ? idNode.getText(sf) : null)
    if (ts.isFunctionDeclaration(node))
      return { name: text(node.name) ?? '(匿名)', kind: 'function' }
    if (ts.isMethodDeclaration(node)) return { name: text(node.name), kind: 'method' }
    if (ts.isConstructorDeclaration(node)) return { name: 'constructor', kind: 'constructor' }
    if (ts.isGetAccessorDeclaration(node)) return { name: text(node.name), kind: 'getter' }
    if (ts.isSetAccessorDeclaration(node)) return { name: text(node.name), kind: 'setter' }
    if (ts.isFunctionExpression(node) || ts.isArrowFunction(node)) {
      const parent = node.parent
      const kind = ts.isArrowFunction(node) ? 'arrow' : 'function'
      if (ts.isVariableDeclaration(parent)) return { name: text(parent.name), kind }
      if (ts.isPropertyDeclaration(parent)) return { name: text(parent.name), kind: 'method' }
      if (ts.isPropertyAssignment(parent)) return { name: text(parent.name), kind: 'method' }
      if (node.name) return { name: text(node.name), kind }
      return null // 匿名回调不计入
    }
    return null
  }

  visit(sf)
  return found
}

/**
 * 提取 Vue SFC 的所有 `<script>` / `<script setup>` 段。
 * @returns {{ content: string, startLine: number }[]} startLine 为段内容首行在宿主文件中的行号
 */
export function extractVueScriptBlocks(source) {
  const blocks = []
  const re = /<script\b[^>]*>/gi
  let m
  while ((m = re.exec(source))) {
    const openEnd = m.index + m[0].length
    const closeIdx = source.indexOf('</script>', openEnd)
    if (closeIdx === -1) break
    const content = source.slice(openEnd, closeIdx)
    let startLine = 1
    for (let k = 0; k < openEnd; k++) {
      if (source[k] === '\n') startLine++
    }
    blocks.push({ content, startLine })
    re.lastIndex = closeIdx + '</script>'.length
  }
  return blocks
}

/** Vue SFC 函数提取：逐 script 段解析后合并（行号映射回宿主文件） */
export function collectVueFunctions(ts, source, fileName = 'source.vue') {
  const found = []
  for (const block of extractVueScriptBlocks(source)) {
    found.push(
      ...collectTsFunctions(ts, block.content, {
        fileName,
        ext: '.ts',
        lineOffset: block.startLine - 1,
      }),
    )
  }
  return found
}
