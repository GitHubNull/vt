# 第 2 章 技术栈与依赖

> 本章列出 VT 桌面端的全部技术选型、依赖清单与常用开发命令。
> 本章是 AGENTS.md 第 5 节"技术栈与依赖管理"的权威来源；依赖变更须先更新本章。

## 2.1 技术栈总览

| 层 | 技术 | 版本约束 |
| --- | --- | --- |
| 应用框架 | Tauri | 2.x |
| 前端框架 | Vue | 3.x（Composition API + `<script setup>`） |
| 前端语言 | TypeScript | 5.x，strict 模式 |
| 前端构建 | Vite | 6.x |
| UI 组件库 | PrimeVue | 4.x |
| 图标 | primeicons | 7.x |
| 前端状态管理 | Pinia | 3.x |
| 前端包管理 | pnpm | 9.x |
| Tauri 前端插件 | @tauri-apps/plugin-dialog、@tauri-apps/plugin-opener | 2.x |
| 后端语言 | Rust | stable（edition 2021，MSRV 随 Tauri 2 要求） |
| 异步运行时 | tokio | 1.x（multi-thread + full features） |
| WebSocket | tokio-tungstenite + futures-util | 0.24+ / 0.3 |
| 配对与发现 | qrcode、rand、mdns-sd | 最新稳定 |
| 序列化 | serde、serde_json | 1.x |
| 媒体与打包 | base64、image、zip | 最新稳定 |

## 2.2 前端依赖清单（package.json）

### 运行时依赖

| 依赖 | 用途 | 选型理由 |
| --- | --- | --- |
| `vue` | UI 框架 | 团队熟悉度、Composition API 与 TS 集成好 |
| `vue-router` | 设置窗口内多页路由 | 设置中心三栏导航需要 |
| `pinia` | 状态管理 | Vue 官方推荐，TS 推导好 |
| `primevue` | UI 组件库 | 组件覆盖全（表格、树、标签页、滑块等），主题系统满足明暗双主题（F-SET-01） |
| `primeicons` | 图标 | 与 PrimeVue 配套，避免额外图标库体积 |
| `@tauri-apps/api` | invoke / event / window API | Tauri 官方 |
| `@tauri-apps/plugin-dialog` | 原生文件 / 消息对话框 | 导入导出词典、备份等场景（用户指定） |
| `@tauri-apps/plugin-opener` | 打开链接 / 文件 / 目录 | 关于页打开官网、日志目录等（用户指定） |

### 开发依赖

| 依赖 | 用途 |
| --- | --- |
| `vite` | 构建与开发服务器（多窗口多页配置见第 3 章） |
| `typescript` | 类型系统 |
| `vue-tsc` | SFC 类型检查 |
| `@vitejs/plugin-vue` | Vue SFC 支持 |
| `@tauri-apps/cli` | `pnpm tauri dev / build` |
| `eslint` + `eslint-plugin-vue` | 静态检查 |
| `prettier` | 格式化 |
| `vitest` | 前端单元测试 |

## 2.3 Rust 依赖清单（Cargo.toml）

### 核心

| crate | 用途 | 选型理由 |
| --- | --- | --- |
| `tauri` | 应用框架 | 见第 1 章 ADR-1 |
| `tauri-plugin-dialog` | 对话框插件后端 | 与前端插件配套 |
| `tauri-plugin-opener` | opener 插件后端 | 与前端插件配套 |
| `tokio` | 异步运行时 | 音频流、WebSocket、HTTP 全部异步（用户指定） |
| `serde` / `serde_json` | 配置与协议序列化 | Rust 事实标准（用户指定） |
| `thiserror` | 错误类型定义 | 库级错误枚举 |
| `tracing` + `tracing-subscriber` | 结构化日志 | 日志滚动与脱敏的基础（NFR-S-02） |

### 实时通信与设备连接

| crate | 用途 | 选型理由 |
| --- | --- | --- |
| `tokio-tungstenite` | WebSocket 服务端 / 客户端 | 移动设备音频与控制键通道（用户指定，ADR-5） |
| `futures-util` | Stream / Sink 组合 | tokio-tungstenite 配套（用户指定） |
| `qrcode` | 配对二维码生成 | 扫码配对（F-MIC-01，用户指定） |
| `rand` | 一次性配对码 / 令牌生成 | 配对安全（NFR-S-08，用户指定） |
| `mdns-sd` | mDNS 广播与扫描 | 局域网设备发现（F-MIC-02，用户指定） |
| `base64` | 截图 / 二进制数据编码 | 移动端状态展示等（用户指定） |
| `image` | 图像处理 | 截图缩放、格式转换（用户指定） |
| `zip` | 压缩打包 | 图标缓存、备份包（用户指定） |

### 系统能力（平台相关，详见第 9 章）

| 能力 | 候选 crate | 说明 |
| --- | --- | --- |
| 全局快捷键 | Tauri 内置 `global-shortcut` | 按住 / 切换双模式在其事件上实现 |
| 音频采集 | `cpal` | 三平台麦克风采集，常驻预热 |
| 文本注入 | `enigo` + 平台 API | 模拟键入；剪贴板用 Tauri 内置 clipboard |
| 窗口焦点 / 场景识别 | 平台 API（`windows-sys` / Cocoa / X11） | F-CODE-01 场景规则依赖 |
| 密钥安全存储 | `keyring` | OS 凭据管理器（NFR-S-01） |
| HTTP 客户端 | `reqwest`（rustls） | AI 供应商调用、更新检查 |

> 上表"候选 crate"在对应任务 SPEC 中最终确认；引入新依赖须先在本章登记。

### 识别引擎（T-006 最终确认）

| 项目 | 结论 | 选型理由 |
| --- | --- | --- |
| 引擎 | `sherpa-rs` 0.6（`static` 特性） | 绑定 sherpa-onnx，静态编译 onnxruntime，纯本地推理断网可用（产品原则 1：本地优先）；C API 稳定，Paraformer 识别器支持 Send + Sync，可安全放入 `spawn_blocking` |
| 模型 | sherpa-onnx Paraformer 中文 2023-03-28（FunASR 训练），内置两档位：`paraformer-zh-int8`（速度优先，约 223 MB）与 `paraformer-zh`（准确率优先，约 864 MB） | 中文普通话识别效果好（集成测试对官方 test_wavs 实测通过）；同一份归档按量化拆档，兼顾低配置电脑与准确率诉求；模型不入安装包，首启下载（NFR-P-09） |
| 配套 | `sha2`（模型文件 SHA-256 校验）、`tar` + `bzip2`（归档解包） | 下载后哈希校验，损坏拒绝加载并引导重新下载（T-006 验收） |

集成测试（`src-tauri/tests/asr_recognition.rs`）加载 int8 档位对官方 test_wavs 实测识别，断言实际输出关键词；fp32 档位准确率基准需基准机实测。

## 2.4 仓库目录结构（实现后）

```
vt/
├── package.json              # pnpm 前端依赖与脚本
├── pnpm-lock.yaml
├── vite.config.ts            # 多窗口构建配置
├── tsconfig.json
├── index.html                # 设置主窗口入口
├── float.html                # 悬浮球窗口入口
├── mini.html                 # 语音输入小窗入口
├── src/                      # 前端源码（Vue）
│   ├── main.ts               # 设置窗口入口
│   ├── float/                # 悬浮球窗口
│   ├── mini/                 # 语音小窗
│   ├── views/                # 设置页各模块视图
│   ├── components/           # 共享组件
│   ├── stores/               # Pinia stores
│   ├── api/                  # invoke / event 封装
│   └── styles/               # 主题与全局样式
└── src-tauri/                # Rust 后端
    ├── Cargo.toml
    ├── tauri.conf.json
    ├── build.rs
    ├── icons/                # 应用图标（zip 打包缓存）
    └── src/
        ├── main.rs
        ├── commands/         # 命令层
        ├── services/         # 服务层
        ├── platform/         # 平台适配层
        └── error.rs          # AppError 统一定义
```

## 2.5 常用命令

### 前端 / 应用

```bash
pnpm install            # 安装前端依赖
pnpm dev                # 仅前端 Vite 开发服务器（调试用）
pnpm tauri dev          # 应用开发模式（前端热更新 + Rust 增量编译）
pnpm tauri build        # 构建安装包（三平台产物见第 9 章）
pnpm typecheck          # vue-tsc 类型检查
pnpm lint               # ESLint 静态检查
pnpm format             # Prettier 格式化
pnpm test               # Vitest 前端单元测试
```

### Rust 后端

```bash
cargo check --manifest-path src-tauri/Cargo.toml    # 快速检查
cargo clippy --manifest-path src-tauri/Cargo.toml   # 静态检查（CI 门禁 -D warnings）
cargo fmt --manifest-path src-tauri/Cargo.toml      # 格式化
cargo test --manifest-path src-tauri/Cargo.toml     # 后端单元测试
```

## 2.6 依赖管理约定

- 前端依赖一律通过 `pnpm add` 安装，禁止手工编辑 `package.json` 版本号；`pnpm-lock.yaml` 必须提交；
- Rust 依赖通过 `cargo add` 安装；`Cargo.lock` 必须提交（应用而非库）；
- 新增依赖前检查：许可证兼容（MIT / Apache-2.0 / BSD 可用，GPL 系禁止）、体积影响（NFR-P-09）、是否活跃维护；
- 涉及系统能力（键盘钩子、剪贴板、麦克风）的依赖，须在引入时于本章补充最小权限说明（AGENTS.md 3.5）；
- 版本升级：补丁版本可随时升级；次版本 / 大版本升级须在 PR 中说明动机与回归验证结果。
