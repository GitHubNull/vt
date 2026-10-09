# T-1038 Wayland 适配方案

| 项目 | 内容 |
| --- | --- |
| 任务编号 | T-1038 |
| 所属阶段 | v1.0 正式版 |
| 优先级 | P1 |
| 覆盖需求 | NFR 兼容性（Linux Wayland 提供替代方案） |
| 前置依赖 | v0.1 T-004、T-007、T-008 |

## 1. 目标与范围

### 1.1 目标

让 VT 在 Wayland 会话下可用：全局快捷键、文本上屏、悬浮球三个受协议限制的能力提供替代方案，达到 PRD"Wayland 提供替代方案"的发布标准。

### 1.2 范围内

- 运行时会话检测：X11 / Wayland 自动识别，能力矩阵按会话切换；
- 全局快捷键替代：XDG GlobalShortcuts portal（支持的 compositor）或引导用户配置桌面环境自定义快捷键调用 `vt --toggle-record` CLI；均不可用时引导使用小窗触发；
- 文本上屏替代：剪贴板粘贴路径为主（portal / ydotool 作为可选后端，用户显式启用）；
- 悬浮球：Wayland 下焦点行为验证与降级（点击穿透 / 位置策略）；
- 设置页与 FAQ：Wayland 环境的能力说明与引导文案；
- 文档：tech/09 更新 Wayland 小节为最终方案。

### 1.3 范围外

- Wayland 下与 X11 完全对等的能力（协议限制，PRD 已接受"替代方案"定位）；
- 场景识别的 Wayland 完整支持（手动场景切换兜底，T-501 已建）。

## 2. 需求依据

- PRD 第 5 章兼容性：X11 完整支持，Wayland 提供替代方案；
- [tech/09](../../tech/09-cross-platform.md) 9.2.1 / 9.2.2 / 9.2.4；
- v1.0 发布标准：目标应用兼容性测试全部通过（含 Wayland 环境的替代方案验证）。

## 3. 技术上下文

- 修改：`platform/hotkey/`、`platform/inject/`（Wayland 后端）、会话检测；
- 新建：CLI 子命令（`--toggle-record` 等，供桌面环境快捷键绑定）；
- 复用：T-004 / T-007 / T-008。

## 4. 任务步骤

1. 会话检测（`XDG_SESSION_TYPE` 等）与能力矩阵；
2. GlobalShortcuts portal 集成（可用性探测 + 实现）；
3. CLI 触发命令与单实例转发（CLI 调用转发给运行中的应用）；
4. 上屏剪贴板路径在 Wayland 的验证与默认化；
5. 悬浮球 Wayland 行为验证与降级；
6. 设置页 / FAQ 引导文案；
7. tech/09 更新为最终方案；
8. Wayland 环境（GNOME / KDE Wayland 会话）手动验证。

## 5. 验收标准

- [ ] Wayland 会话下应用正常运行，能力降级有明确引导；
- [ ] portal 可用的环境全局快捷键生效；不可用时 CLI 绑定方案文档化并验证；
- [ ] Wayland 下文本上屏（剪贴板路径）在目标应用正确；
- [ ] 悬浮球在 Wayland 下不抢焦点或按降级行为工作；
- [ ] X11 会话行为无回归；
- [ ] tech/09 Wayland 小节更新为最终方案；
- [ ] GNOME 与 KDE 的 Wayland 会话手动验证通过。

## 6. 测试要求

- `cargo test`：会话检测、能力矩阵；
- 手动验证：GNOME / KDE Wayland 会话全流程。

## 7. 完成定义（DoD）

- 验收标准全部勾选；
- 提交信息：`feat: Wayland 会话适配方案`。
