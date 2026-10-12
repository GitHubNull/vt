# VibeType（码语）源码质量规范

> 本目录收录面向 AI 编程代理与贡献者的源码质量规范与操作指南。
> 配套工具位于 [tools/source-quality/](../../tools/source-quality/)（统计报告、行数门禁、基线维护）。

| 项目 | 内容 |
| --- | --- |
| 适用范围 | 仓库内全部源码：`src/`、`src-tauri/`、`tools/` |
| 第一读者 | AI 编程代理（配合 [AGENTS.md](../../AGENTS.md) 使用）；人类贡献者同样适用 |
| 文档性质 | 工程规范文档：规定门禁口径与拆分方法，不描述产品需求（见 [PRD](../prd/README.md)） |

---

## 一、文档目录

| 文档 | 说明 |
| --- | --- |
| [源码文件行数门禁与拆分指南](./源码文件行数门禁与拆分指南.md) | 门禁口径与命令、AI 编程代理的分步骤拆分工作流、各语言拆分手法、反模式与 FAQ |

## 二、速查

| 项 | 内容 |
| --- | --- |
| 门禁指标 | 单文件**有效行**（非空行；注释计入；Rust 不计 `#[cfg(test)] mod` 测试模块） |
| 各语言上限 | Rust / TypeScript / JavaScript 500；Vue SFC / CSS 600（定义于 [tools/source-quality/config.json](../../tools/source-quality/config.json)） |
| 提交前必跑 | `pnpm quality:gate`（必须退出码 0，见 AGENTS.md 3.5） |
| 统计报告 | `pnpm quality:stats`（`--json` 输出机器可读格式） |
| 历史遗留 | [tools/source-quality/baseline.json](../../tools/source-quality/baseline.json) 登记的历史超限文件：允许保持现状、不得增长；拆分到位后 `pnpm quality:baseline` 清理 |

## 三、维护约定

- 上限与基线的任何调整须在提交信息中说明理由；**AI 编程代理不得自行放宽上限或使用 `--force` 增大基线**；
- 修改口径、上限或工具行为时，同步更新本文档、拆分指南与 [工具 README](../../tools/source-quality/README.md)；
- 本目录文档使用简体中文，中英文之间加空格（与 [doc/tech](../tech/README.md) 约定一致）。

## 四、修订记录

| 版本 | 日期 | 修订说明 |
| --- | --- | --- |
| v1.0 | 2026-10-12 | 初版：引入单文件行数门禁（含历史遗留基线）、源码质量分析工具与拆分指南 |
