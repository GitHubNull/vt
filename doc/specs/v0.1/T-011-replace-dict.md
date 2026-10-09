# T-011 替换词典与后处理管线

| 项目 | 内容 |
| --- | --- |
| 任务编号 | T-011 |
| 所属阶段 | v0.1 MVP |
| 优先级 | P0 |
| 覆盖需求 | F-DICT-01（替换词典） |
| 前置依赖 | T-006 |

## 1. 目标与范围

### 1.1 目标

建立后处理管线（PostProcess）的 Stage 抽象，并实现第一个 Stage：替换词典（错误词 → 正确词），同时提供词典管理命令与设置页。

### 1.2 范围内

- PostProcess 管线框架：有序 Stage 列表、`PipelineContext`、每 Stage 独立开关（为 v0.5 口令 / AI Stage 预留）；
- DictService：替换规则的增删改查、启用 / 禁用、持久化（配置目录 `dict/replace.json`）；
- 替换规则引擎：长词优先、全词匹配等规则以 PRD 3.4 节为准；
- 词典设置页（`views/dict/`）：规则表格（编辑、删除、启停、搜索），对照原型截图；
- 管线接入：AsrService 结果 → 管线 → 输出（上屏在 T-012 串联）。

### 1.3 范围外

- 专有名词（F-DICT-02，v0.5）、导入导出（F-DICT-03，v0.5）；
- 编程口令 Stage（v0.5）、AI Stage（v0.5）；
- 端到端上屏串联（T-012）。

## 2. 需求依据

- F-DICT-01：替换词典（PRD 3.4 节业务规则与验收标准）；
- 产品原则 3：自动改写可关闭（Stage 开关）、可撤回；
- [tech/05](../../tech/05-core-pipeline.md) 5.1 管线结构、5.3.4 Stage 设计；
- 参考截图：`doc/img/语音输入法界面原型截图参考/PixPin_2026-10-09_21-32-16.png`。

## 3. 技术上下文

- 新建：`src-tauri/src/services/post_process.rs`（管线框架）、`src-tauri/src/services/dict.rs`、`src-tauri/src/commands/dict.rs`、`src/views/dict/`、`src/stores/dict.ts`；
- 复用：T-003 ConfigStore（词典文件持久化可复用原子写入工具函数）、T-006 识别结果事件。

## 4. 任务步骤

1. 定义 `PipelineContext`（原文、各 Stage 输出、开关状态）与 `Stage` trait；
2. 实现 PostProcess：按序执行启用的 Stage，任一 Stage 可短路（v0.5 AI Stage 需要）；
3. 实现 DictService：规则 CRUD、持久化（原子写入）、变更事件；
4. 实现 ReplaceStage：按 PRD 3.4 规则执行替换（长词优先等），Stage 开关绑定配置；
5. 词典命令层：`list_rules` / `add_rule` / `update_rule` / `delete_rule` / `toggle_rule`；
6. 词典设置页：PrimeVue 表格 + 编辑对话框（`@tauri-apps/plugin-dialog` 确认删除），对照截图；
7. 单元测试：替换规则引擎（PRD 3.4 验收用例逐条覆盖）、管线顺序与开关。

## 5. 验收标准

- [ ] PRD 3.4 节 F-DICT-01 验收标准逐条通过；
- [ ] 词典规则增删改查、启停、搜索在设置页可用，重启后保持；
- [ ] 替换 Stage 关闭时识别原文直出（可关闭验证）；
- [ ] 管线框架具备注册多个 Stage 的能力（测试注册两个假 Stage 验证顺序）；
- [ ] 词典文件损坏时回退空词典并提示，不崩溃（NFR-S-07 雏形）；
- [ ] 词典内容与识别文本不写入日志。

## 6. 测试要求

- `cargo test`：规则引擎、管线、持久化；
- Vitest：词典 store；
- 手动验证：设置页操作 + 识别替换效果（可用 T-006 的测试命令驱动）。

## 7. 完成定义（DoD）

- 验收标准全部勾选；
- 提交信息：`feat: 替换词典与后处理管线框架`。
