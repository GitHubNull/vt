# 第 3 章 前端架构

> 本章描述 VT 前端（Vue 3 + TypeScript + Vite 6 + PrimeVue 4）的组织方式：多窗口构建、目录划分、状态管理与前后端通信封装。
> 需求依据：PRD 第 4 章（交互与视觉设计）；视觉规范以 PRD 为准，本章只规定工程组织。

## 3.1 多窗口模型与 Vite 多页构建

VT 有三个前端窗口（见 [第 1 章](./01-architecture.md) 1.2 节），在 Vite 中以**多页应用（MPA）**方式构建：

| 页面入口 | 窗口 label | 用途 | 加载时机 |
| --- | --- | --- | --- |
| `index.html` → `src/main.ts` | `main` | 设置主窗口 | 用户打开设置时创建 |
| `mini.html` → `src/mini/main.ts` | `mini` | 语音输入小窗 | 托盘 / 快捷键唤起 |
| `float.html` → `src/float/main.ts` | `float` | 录音悬浮球 | 录音期间动态创建 |

`vite.config.ts` 中使用 `build.rollupOptions.input` 声明三个入口；`src-tauri/tauri.conf.json` 中三个窗口分别指向对应页面。

**悬浮球窗口特殊约束**（产品原则"零打扰"）：

- 创建参数：`decorations: false`、`transparent: true`、`alwaysOnTop: true`、`focusable: false`、`skipTaskbar: true`；
- `focusable: false` 在 Linux 各桌面环境行为不一致，差异处理见 [第 9 章](./09-cross-platform.md)；
- 悬浮球页面只渲染录音反馈（音量动效、时长、模式），**不含任何可聚焦控件**。

## 3.2 前端目录结构

```
src/
├── main.ts                  # 设置窗口入口：createApp + PrimeVue + router + pinia
├── App.vue                  # 设置窗口根组件（三栏布局）
├── router.ts                # 设置中心路由（每个设置模块一条路由）
├── float/                   # 悬浮球窗口（独立入口，不引 router）
│   ├── main.ts
│   └── FloatBall.vue
├── mini/                    # 语音输入小窗（独立入口）
│   ├── main.ts
│   └── MiniWindow.vue
├── views/                   # 设置中心各模块页面（与 PRD 第 2 章导航对应）
│   ├── dict/                # 词典：替换词典 / 专有名词 / 导入导出
│   ├── coding/              # 编程：场景规则 / 口令 / 技术词库
│   ├── ai/                  # AI：模板 / 供应商 / MCP
│   ├── general/             # 通用：界面 / 输入与录音 / 文本输出 / 录音环境 / 系统与数据 / 模型管理
│   ├── stats/               # 统计
│   ├── me/                  # 我的：档案 / 备份恢复
│   ├── faq/                 # 常见问题
│   └── about/               # 关于
├── components/              # 跨模块共享组件
├── stores/                  # Pinia stores（按领域拆分，见 3.4）
├── api/                     # 前后端通信封装（见 3.5）
│   ├── client.ts            # invoke 封装与错误类型
│   ├── events.ts            # 事件名常量与 listen 封装
│   └── types.ts             # 与 Rust 侧 serde 类型对应的 TS 类型
├── composables/             # 组合式函数
├── i18n/                    # 界面文案（zh-CN / en-US）
└── styles/                  # 全局样式与 PrimeVue 主题定制
```

约定：

- 三个窗口入口各自独立打包，**共享代码通过 `src/components`、`src/api`、`src/stores` 引用**，由 Vite 自动拆包；
- 悬浮球与小窗入口保持极简依赖（不引 PrimeVue 全量、不引 router），控制包体与启动耗时（NFR-P-08）；
- 用户可见文案一律走 `src/i18n/`，不散落在组件中（AGENTS.md 3.5）。

## 3.3 UI 组件库使用约定（PrimeVue 4）

- 使用 PrimeVue 4 的 **unstyled + 主题预设**模式之外的默认 styled 模式，主题通过 `@primeuix/themes` 预设定制；明暗主题切换对应 F-SET-01（v0.1 浅色，v1.0 深色）；
- 图标统一使用 primeicons（`pi pi-xxx`），不引入其他图标库；
- 表格、树、标签页、滑块、开关等优先使用 PrimeVue 组件；PrimeVue 没有的（如音量波形）自绘 Canvas 实现，放在 `components/`；
- 对话框（文件选择、确认框）一律使用 `@tauri-apps/plugin-dialog` 的原生对话框，不用浏览器 `confirm`/`alert`；
- 打开外部链接 / 目录一律使用 `@tauri-apps/plugin-opener`。

## 3.4 状态管理（Pinia）

按领域拆分 store，store 是前端状态的唯一来源；后端状态变更通过事件同步进 store：

| Store | 职责 | 主要状态 |
| --- | --- | --- |
| `useConfigStore` | 应用配置镜像 | 全部设置项（与 ConfigStore 同步） |
| `useRecordingStore` | 录音会话 | 状态机（idle/recording/processing）、时长、音量、模式 |
| `useDictStore` | 词典数据 | 替换规则、专有名词列表 |
| `useAiStore` | AI 配置 | 模板列表、供应商列表、当前选择 |
| `useDeviceStore` | 移动设备 | 已配对设备、连接状态、延迟（v1.0） |

约定：

- 配置类状态：store 中的修改 → `invoke` 写后端 → 后端落盘成功后返回 → store 提交变更（失败回滚并提示）；
- 实时状态（录音、音量、设备）：后端 `emit` → `api/events.ts` 统一监听 → 写入对应 store；
- 不在组件内直接调 `invoke` / `listen`，统一走 `api/` 与 store，便于测试与替换。

## 3.5 前后端通信封装（src/api/）

### 命令调用

```ts
// api/client.ts 约定示例（仅示意调用形态，非实现代码）
const result = await api.getDictList();        // 内部 invoke('get_dict_list')
await api.setHotkey(config);                   // 内部 invoke('set_hotkey', { config })
```

- 每个 Tauri Command 在 `api/` 中有一个对应的 TS 函数，函数名与命令名一致（snake_case 命令名 → camelCase 函数名）；
- 参数与返回值的 TS 类型定义在 `api/types.ts`，与 Rust 侧 serde 结构体**手工保持同步**（在 PR 检查清单中核对）；
- 错误统一为 `AppError { code, message }`，`api/client.ts` 负责把后端错误转为带错误编号的用户提示（NFR：错误提示带错误编号）。

### 事件

| 事件名 | 方向 | 载荷 | 用途 |
| --- | --- | --- | --- |
| `recording:state` | 后端→前端 | 状态 + 模式 | 录音状态机变更 |
| `recording:level` | 后端→前端 | 音量电平（节流 50 ms） | 悬浮球 / 小窗动效 |
| `recording:result` | 后端→前端 | 上屏结果 / 降级原因 | 结果反馈 |
| `device:changed` | 后端→前端 | 设备列表快照 | 移动设备状态（v1.0） |
| `config:changed` | 后端→前端 | 变更的配置节 | 多窗口配置同步 |

事件名常量集中在 `api/events.ts`，禁止在组件中写字面量事件名。

## 3.6 路由与设置中心布局

- 设置主窗口为 PRD 第 4 章规定的三栏布局：一级导航（侧边栏）→ 二级导航 / 分组 → 内容区；
- 每个设置模块一条路由（如 `/dict/replace`、`/ai/providers`、`/general/recording`），路由表与 PRD 第 2 章导航体系一一对应；
- 路由懒加载（`defineAsyncComponent` / 动态 import），保证设置窗口打开 ≤500 ms（NFR-P-10）；
- 侧边栏导航项以 PRD v1.3 为准，**不包含"音视频转录"项**（已决策移除，见 AGENTS.md 3.3）。

## 3.7 样式与视觉

- 全局样式入口 `src/styles/`，CSS 变量定义在 `:root`（颜色、间距、圆角），明暗主题通过切换 `data-theme` 属性实现；
- 视觉规范（字号、间距、配色、组件状态）以 PRD 第 4 章为唯一标准；
- 界面实现须对照 `doc/img/语音输入法界面原型截图参考/`，并遵守其中"原型问题修正清单"（侧边栏不重复渲染、供应商列表去重等，见 AGENTS.md 第 4 节）。

## 3.8 前端测试

- 单元测试：Vitest，覆盖 `api/` 封装、store 逻辑、纯函数组件；
- 组件测试：对含复杂交互的组件（如词典编辑表格）做渲染测试；
- 端到端验证以手动为主（Tauri 窗口与系统能力无法纯前端 E2E），验证步骤写在各任务 SPEC 的验收标准中。
