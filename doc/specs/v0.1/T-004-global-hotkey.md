# T-004 全局快捷键模块

| 项目 | 内容 |
| --- | --- |
| 任务编号 | T-004 |
| 所属阶段 | v0.1 MVP |
| 优先级 | P0 |
| 覆盖需求 | F-INPUT-01（全局快捷键录音）、F-INPUT-02（按住说话 / 切换模式） |
| 前置依赖 | T-001 |

## 1. 目标与范围

### 1.1 目标

实现三平台全局快捷键捕获，支持"按住说话"与"切换模式"两种触发方式，输出标准化的录音控制事件（开始 / 结束）。

### 1.2 范围内

- HotkeyService：基于 Tauri `global-shortcut` 的注册 / 注销 / 重注册；
- 默认快捷键：右 Alt；可在配置中修改（本任务只实现机制，设置 UI 在 T-013）；
- 两种模式：按住（Pressed 开始 / Released 结束）、切换（Pressed 翻转）；
- 注册失败（被占用）检测与错误上报（错误编号 + 提示事件）；
- 录音控制事件出口：向 AudioService 发 `RecordCommand`（本任务只到通道出口，T-005 消费）；
- `platform/hotkey/` 适配层封装（右 Alt 等特殊键的平台差异收敛）。

### 1.3 范围外

- 快捷键冲突的系统性检测界面（F-SYS-03，v0.5 T-543）；
- 模板快捷键（F-AI-02，v0.5）；
- 录音状态机与 UI 反馈（T-008 / T-009 / T-012）。

## 2. 需求依据

- F-INPUT-01：全局快捷键录音（默认右 Alt，可修改）；
- F-INPUT-02：按住说话 / 切换模式；
- NFR-P-01/02：快捷键事件到录音启动的延迟预算入口；
- [tech/04](../../tech/04-backend.md) 4.4 HotkeyService；
- [tech/05](../../tech/05-core-pipeline.md) 5.3.1；
- [tech/09](../../tech/09-cross-platform.md) 9.2.1（含 Wayland 限制说明）。

## 3. 技术上下文

- 新建：`src-tauri/src/services/hotkey.rs`、`src-tauri/src/platform/hotkey/`；
- 复用：T-001 AppState / 事件广播；T-003 配置（快捷键定义、模式）；
- 注意：macOS 需辅助功能权限，权限缺失时错误处理（tech/09 9.4）。

## 4. 任务步骤

1. 定义 `RecordCommand { Start, Stop, Toggle, Cancel }` 与快捷键配置结构（键名、模式）；
2. 实现 `platform/hotkey/` trait（`register` / `unregister` / 事件回调），三平台默认实现基于 `global-shortcut`；
3. 实现 HotkeyService：按配置注册；Pressed / Released 事件按模式映射为 RecordCommand；通过 mpsc 通道出口；
4. 配置变更（`config:changed` 的 hotkey 节）时平滑重注册（先注销再注册，失败回滚旧配置并提示）；
5. 注册失败路径：错误编号 `E-HOTKEY-001`，事件通知前端；
6. 单元测试：模式映射逻辑（按住 / 切换的状态推导）用模拟事件覆盖；
7. 临时联调：命令 `trigger_test_hotkey` 或日志输出来验证事件流（正式 UI 在后续任务）。

## 5. 验收标准

- [ ] 默认右 Alt 按住说话：按下产生 Start，松开产生 Stop（日志验证）；
- [ ] 切换模式：按一次 Start，再按一次 Stop；
- [ ] 修改配置后快捷键热更新生效，无需重启；
- [ ] 快捷键被其他应用占用时，前端收到带错误编号的提示事件；
- [ ] 三平台手动验证：Windows / macOS / Linux X11 事件均正确（macOS 权限缺失时给出引导提示而非崩溃）；
- [ ] 单元测试通过；`cargo clippy` 无警告。

## 6. 测试要求

- `cargo test`：模式映射、重注册逻辑；
- 三平台手动验证清单：默认键、自定义键（如 Ctrl+Shift+Space）、占用冲突。

## 7. 完成定义（DoD）

- 验收标准全部勾选；
- 平台实现处注释最小权限说明（AGENTS.md 3.5）；
- 提交信息：`feat: 全局快捷键模块（按住/切换双模式）`。
