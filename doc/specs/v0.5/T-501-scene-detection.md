# T-501 前台窗口与场景识别

| 项目 | 内容 |
| --- | --- |
| 任务编号 | T-501 |
| 所属阶段 | v0.5 Beta |
| 优先级 | P1 |
| 覆盖需求 | F-CODE-01（场景规则的识别基础） |
| 前置依赖 | v0.1 完成（T-001、T-003） |

## 1. 目标与范围

### 1.1 目标

实现跨平台前台窗口 / 应用识别能力，把当前焦点应用分类为 PRD 定义的场景（IDE / 终端 / AI 对话 / 浏览器 / 沟通文档 / 其他），供场景规则与终端安全保护使用。

### 1.2 范围内

- `platform/window_info/` 适配层：前台应用名、进程名、窗口标题（三平台）；
- SceneService：应用 → 场景的分类引擎（内置规则库 + 用户自定义规则）；
- 场景判定结果缓存与变更事件（`scene:changed`）；
- 场景信息注入 PipelineContext（供后续口令 Stage 使用）；
- 手动场景切换兜底（Wayland 等识别受限环境）：托盘 / 命令手动指定当前场景。

### 1.3 范围外

- 场景规则的设置页 UI（T-502）；
- 各口令 Stage 本身（T-512 ~ T-516）；
- Wayland 完整适配方案（v1.0 T-1038，本任务手动切换兜底即可）。

## 2. 需求依据

- F-CODE-01：场景规则（PRD 3.2.1 节：自动识别 IDE / 终端 / AI 对话 / 文档沟通场景）；
- PRD 第 5 章兼容性：目标应用五大类须"场景识别正确"；
- [tech/09](../../tech/09-cross-platform.md) 9.2.3 场景识别平台实现；
- [tech/05](../../tech/05-core-pipeline.md) 5.1 PipelineContext 场景字段。

## 3. 技术上下文

- 新建：`src-tauri/src/platform/window_info/`、`src-tauri/src/services/scene.rs`；
- 复用：T-003 ConfigStore（自定义规则持久化）、T-001 事件广播；
- 平台 API：Windows `GetForegroundWindow` + 进程查询；macOS `NSWorkspace` / Accessibility；Linux X11 `_NET_ACTIVE_WINDOW`。

## 4. 任务步骤

1. 实现 `platform/window_info/` trait：`active_app() -> AppInfo { name, process, title }`，三平台实现；
2. 定义场景枚举与内置规则库（进程名 / bundle id / 窗口标题模式 → 场景），规则库数据驱动（JSON，可随版本更新）；
3. 实现 SceneService：查询当前场景（带短缓存）、场景变更广播、用户自定义规则（应用名 → 场景）合并优先级高于内置规则；
4. 手动场景指定：命令 `set_scene_override` / 清除，override 优先于自动识别；
5. 权限缺失（macOS 辅助功能未授权）：降级为"未知场景"并提示引导，不崩溃；
6. 单元测试：规则匹配（内置 + 自定义优先级）、缓存、override；
7. 三平台手动验证：PRD 五大类代表应用各至少 2 个的场景识别正确性。

## 5. 验收标准

- [ ] VS Code / JetBrains IDE 识别为 IDE 场景；Windows Terminal / iTerm2 / GNOME Terminal 识别为终端；浏览器与主流 AI 对话页识别正确（PRD 目标应用清单抽查）；
- [ ] 场景切换时 `scene:changed` 事件正确广播；
- [ ] 用户自定义规则（把某应用指定为某场景）生效且优先于内置规则；
- [ ] 手动场景 override 生效，清除后恢复自动识别；
- [ ] macOS 未授权时降级为未知场景 + 引导提示，不崩溃；
- [ ] 窗口标题等识别信息不写入日志（NFR-S-02）；
- [ ] 三平台手动验证通过。

## 6. 测试要求

- `cargo test`：规则引擎、优先级、override；
- 三平台手动验证清单：五大类应用识别。

## 7. 完成定义（DoD）

- 验收标准全部勾选；
- 平台实现处注释最小权限说明；
- 提交信息：`feat: 前台窗口与场景识别`。
