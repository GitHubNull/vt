# T-001 Tauri 2 工程脚手架

| 项目 | 内容 |
| --- | --- |
| 任务编号 | T-001 |
| 所属阶段 | v0.1 MVP |
| 优先级 | P0 |
| 覆盖需求 | 全部任务的工程地基（无直接 PRD 功能编号） |
| 前置依赖 | 无 |

## 1. 目标与范围

### 1.1 目标

建立可编译、可运行、可打包的 Tauri 2 工程骨架，前后端技术栈全部就位，后续任务直接在骨架上开发。

### 1.2 范围内

- 初始化 pnpm 前端工程：Vue 3 + TypeScript（strict）+ Vite 6 + PrimeVue 4 + primeicons + Pinia + vue-router；
- 安装 Tauri 前端插件：`@tauri-apps/plugin-dialog`、`@tauri-apps/plugin-opener`；
- 初始化 `src-tauri`：Tauri 2 + tokio + serde / serde_json + thiserror + tracing + tracing-subscriber；并预置（登记但可不使用）tokio-tungstenite、futures-util、qrcode、rand、mdns-sd、base64、image、zip；
- Vite 多页构建配置（`index.html` / `float.html` / `mini.html` 三入口）；
- `tauri.conf.json`：三个窗口声明（main / mini / float），float 窗口参数符合不抢焦点约束；
- 目录结构按 [tech/02](../../tech/02-tech-stack.md) 第 2.4 节与 [tech/04](../../tech/04-backend.md) 第 4.1 节建立；
- 工程级工具：ESLint + Prettier + vue-tsc + cargo clippy / fmt 配置；
- `AppError` 统一错误类型骨架与 `AppState` 空壳；
- 日志初始化（tracing，按天滚动到日志目录）。

### 1.3 范围外

- 任何业务功能（快捷键、录音、识别、上屏等）；
- CI 流水线配置；
- 安装包签名。

## 2. 需求依据

- [tech/01 总体架构](../../tech/01-architecture.md)：1.2 窗口模型、1.3 后端分层；
- [tech/02 技术栈与依赖](../../tech/02-tech-stack.md)：2.1 ~ 2.5 全部；
- AGENTS.md 3.5：代码约定（密钥不入代码、文案集中、格式化与静态检查）。

## 3. 技术上下文

- 新建文件：`package.json`、`vite.config.ts`、`tsconfig.json`、三个 HTML 入口、`src/` 前端骨架、`src-tauri/` 后端骨架；
- 依赖清单以 [tech/02](../../tech/02-tech-stack.md) 2.2 / 2.3 节为准，不得擅自替换；
- 现有仓库 `src/` 为空目录，本任务填充它。

## 4. 任务步骤

1. `pnpm create vite` 初始化前端（Vue + TS 模板），按 tech/02 2.4 节目录结构调整；
2. `pnpm add` 安装运行时依赖（vue-router、pinia、primevue、primeicons、@tauri-apps/api、@tauri-apps/plugin-dialog、@tauri-apps/plugin-opener）与开发依赖（vite、typescript、vue-tsc、@vitejs/plugin-vue、@tauri-apps/cli、eslint、eslint-plugin-vue、prettier、vitest）；
3. 配置 `vite.config.ts` 多页输入（index / float / mini）与 Tauri 约定的 devServer 端口；
4. `pnpm tauri init` 初始化 src-tauri，包标识符 `com.vibetype.app`，窗口按 tech/01 1.2 节配置（float：`decorations:false, transparent:true, alwaysOnTop:true, focusable:false, skipTaskbar:true`）；
5. `cargo add` 安装 Rust 依赖（tech/02 2.3 节核心 + 实时通信两组）；
6. 建立 `src-tauri/src/` 分层目录（commands / services / platform / error.rs / state.rs），各目录放 `mod.rs` 空壳与模块注释；
7. 实现 `error.rs` 的 `AppError`（thiserror，含 Audio / Asr / Inject / Config / Network / Permission / Platform 变体骨架与错误编号前缀约定）；
8. `main.rs`：初始化 tracing（按天滚动、日志目录按 tech/08 8.1）、注册插件、注册空命令 `ping`（返回 `"pong"`，供前端联调）；
9. 前端三个入口各自渲染最小页面（设置窗口显示 PrimeVue 按钮组件、float / mini 显示占位文本），`src/api/client.ts` 封装 `ping()` 并在设置窗口调用验证；
10. 配置 ESLint / Prettier / `cargo clippy`（`-D warnings`）/ `cargo fmt`，`package.json` 脚本按 tech/02 2.5 节补齐；
11. 更新 `.gitignore`：`node_modules`、`dist`、`src-tauri/target`、`src-tauri/gen` 等（保留既有 tmp/ 等条目）。

## 5. 验收标准

- [ ] `pnpm install` 成功，lock 文件生成；
- [ ] `pnpm tauri dev` 启动应用：设置主窗口显示 PrimeVue 按钮，点击调用 `ping` 命令显示 `pong`；
- [ ] `pnpm typecheck`、`pnpm lint`、`cargo clippy -- -D warnings`、`cargo fmt --check` 全部通过；
- [ ] `pnpm tauri build` 在当前平台产出安装包；
- [ ] 窗口配置核查：`tauri.conf.json` 中 float 窗口 `focusable: false`；
- [ ] 依赖核查：`package.json` 与 `Cargo.toml` 中依赖与 tech/02 清单一致（不缺、不多）；
- [ ] 日志文件在平台标准日志目录按天生成；
- [ ] 代码与日志中无任何密钥 / 令牌 / 用户文本（AGENTS.md 3.5）。

## 6. 测试要求

- `pnpm test`（Vitest）跑通一个示例测试（如 `api/client.ts` 的封装）；
- `cargo test` 跑通一个示例测试（如 `AppError` 错误编号格式化）；
- 手动验证：当前平台 `pnpm tauri dev` 全流程。

## 7. 完成定义（DoD）

- 验收标准全部勾选；
- 目录结构与 tech/02 2.4 节一致；
- 提交信息：`feat: 初始化 Tauri 2 工程脚手架`（AGENTS.md 3.6）。
