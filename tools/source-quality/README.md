# source-quality —— 源码质量分析与行数门禁工具

> 供 AI 编程代理与贡献者对仓库源码做「单文件行数 + 函数行数」统计，并执行单文件行数门禁。
> 门禁口径的完整说明、文件拆分方法论见 [doc/quality/源码文件行数门禁与拆分指南](../../doc/quality/源码文件行数门禁与拆分指南.md)。

## 快速使用

| 命令 | 说明 |
| --- | --- |
| `pnpm quality:stats` | 输出统计报告：各语言文件数 / 有效行 / 函数行数 / 长函数预警 / 测试模块偏大清单 |
| `pnpm quality:gate` | 执行门禁：超限且未登记基线、或基线内增长 → 退出码 1 |
| `pnpm quality:baseline` | 重建基线：登记超限文件；只允许条目缩小 / 清理，增大需 `--force` 并说明理由 |

直接调用：`node tools/source-quality/cli.mjs <report|check|update-baseline> [--json] [--top=N] [--no-functions] [--force]`

- `--json`：机器可读输出（AI 编程代理与脚本优先使用）；若要把 JSON 直接管道给其他程序，请使用 `pnpm --silent quality:stats --json`（或直接 `node tools/source-quality/cli.mjs report --json`），避免 pnpm 任务横幅混入 stdout；
- 退出码：`0` 通过 / `1` 门禁未通过 / `2` 用法或运行错误。

## 门禁口径

- **有效行 = 非空行**（注释行计入、空行不计）。报告额外拆分空白 / 注释 / 代码行供分析（近似值，不参与门禁判定）。
- **Rust 测试模块不计入**：以「`#[cfg(test)]` 属性行 + 紧邻的 `mod` 声明」定位文件末尾的内联测试模块；测试行数单独统计，超过 `testModuleWarnLines`（默认 400）进入报告「测试模块偏大」清单。约定测试模块置于文件末尾；其他形式的 `#[cfg(test)]`（如测试专用方法）按代码计（保守）。
- **各语言上限**见 [config.json](./config.json)：基数 500（Rust / TypeScript / JavaScript）；Vue SFC 与 CSS 放宽至 600（理由见指南第二章）。
- **扫描范围**：`src/`、`src-tauri/`、`tools/`；排除 `node_modules` / `target` / `gen` / `dist` / `tmp` 与配置类扩展名（见 `ignoreExt`）；扫描到的未纳管扩展名会在报告末尾提示。
- **基线机制**：历史遗留超限文件登记在 [baseline.json](./baseline.json)（ratchet：允许保持现状、不得增长）。文件拆分达标后运行 `pnpm quality:baseline` 清理条目；条目默认只允许缩小。

## 函数统计

| 语言 | 解析方式 | 统计范围 |
| --- | --- | --- |
| Rust | 掩码文本上正则定位 + 花括号配对（零依赖） | 全部 `fn`（含方法与嵌套函数），并标记是否位于测试模块 |
| TypeScript / JavaScript | typescript 编译器 API（devDependency） | 具名函数、类方法、构造器、访问器、赋值给变量 / 属性的箭头函数与函数表达式；匿名回调不计 |
| Vue | 同上（逐 `<script>` 段解析，行号映射回宿主文件） | 仅 script 段；template / style 不产生函数 |
| CSS | 不解析 | — |

已知边界（统计供分析，允许近似）：注释 / 代码拆分为行级启发式（多行字符串、模板字符串内代码等个别情况会轻微偏斜，不影响门禁口径）；Rust 提取不展开宏，函数签名中出现 const 泛型花括号时跨度可能有偏差；typescript 依赖缺失时函数统计自动跳过（行数统计不受影响）。

## 目录结构

| 路径 | 说明 |
| --- | --- |
| `cli.mjs` | 入口：命令分发、参数解析、退出码 |
| `config.json` | 扫描根、语言与上限、预警阈值、忽略扩展名 |
| `baseline.json` | 历史遗留超限文件基线 |
| `lib/walk.mjs` | 源码文件遍历（排除目录 / 隐藏文件 / 扩展名过滤） |
| `lib/lines.mjs` | 行拆分、注释 / 字符串掩码、行统计、Rust 测试模块定位 |
| `lib/functions.mjs` | TS / JS / Vue 函数提取 |
| `lib/functions-rust.mjs` | Rust 函数提取 |
| `lib/analyze.mjs` | 分析编排、门禁判定、基线计算（纯函数） |
| `lib/render.mjs` | 控制台渲染与 JSON 输出 |

## 扩展新语言

1. 在 `config.json` 的 `languages` 增加条目（`ext`、`limit`）；
2. 行统计需要对应注释语法：在 `lib/lines.mjs` 的 `SYNTAX` 增加条目（行注释 / 块注释 / 字符串引号）；
3. 函数统计（可选）：参照 `lib/functions*.mjs` 实现提取器，并在 `lib/analyze.mjs` 注册。

## 维护约定

- 上限调整（`config.json` 的 `limit`）属于流程变更：须同步更新拆分指南并在提交信息中说明理由；
- `baseline.json` 由 `pnpm quality:baseline` 生成，不要手工编辑；`--force` 仅限确需保留的历史条目，且应在提交信息说明。
