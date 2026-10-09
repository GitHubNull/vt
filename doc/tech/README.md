# VibeType（码语）技术文档

> 本目录是 VT 桌面端的技术方案文档，描述"用什么技术、怎么组织、关键机制如何设计"。
> 产品需求（做什么、为什么）以 [PRD](../prd/README.md) 为准；任务执行入口见 [任务路线图](../roadmap/README.md) 与 [任务 SPEC](../specs/README.md)。

| 项目 | 内容 |
| --- | --- |
| 文档版本 | v1.0 |
| 编写日期 | 2026-10-09 |
| 文档状态 | 初版 |
| 适用范围 | VT 桌面端（Windows / macOS / Linux），Tauri 2 + Rust + Vue 3 技术栈 |
| 文档性质 | 技术设计文档：规定技术选型、架构分层与关键机制设计，不重复 PRD 的产品需求描述 |

---

## 一、文档目录

| 章节 | 文件 | 主要内容 |
| --- | --- | --- |
| 第 1 章 | [总体架构](./01-architecture.md) | Tauri 2 分层架构、进程与窗口模型、模块划分、线程与通道模型 |
| 第 2 章 | [技术栈与依赖](./02-tech-stack.md) | 全部依赖清单与选型理由、pnpm / cargo / tauri 常用命令 |
| 第 3 章 | [前端架构](./03-frontend.md) | Vue 3 + Vite 6 + PrimeVue 4、多窗口构建、状态管理、前后端通信封装 |
| 第 4 章 | [Rust 后端架构](./04-backend.md) | tokio 运行时、命令层 / 服务层 / 平台适配层、核心服务设计 |
| 第 5 章 | [核心语音输入链路](./05-core-pipeline.md) | 快捷键 → 录音 → 本地识别 → 后处理 → 上屏 全管线时序与性能预算 |
| 第 6 章 | [AI 后处理与 MCP 服务](./06-ai-and-mcp.md) | 提示词模板引擎、AI 供应商抽象、超时降级、MCP 服务与访问控制 |
| 第 7 章 | [移动麦克风（桌面端配套）](./07-mobile-link.md) | 扫码配对、mDNS 发现、WebSocket 加密通道、设备管理与控制键路由 |
| 第 8 章 | [数据存储与安全](./08-data-and-security.md) | 数据目录布局、原子写入、密钥加密、日志脱敏、敏感信息检测 |
| 第 9 章 | [跨平台与构建打包](./09-cross-platform.md) | 三平台差异处理、Wayland 适配、安装包与更新签名校验 |

## 二、技术选型总览

| 层 | 选型 |
| --- | --- |
| 应用框架 | Tauri 2（Rust 后端 + 系统 Webview 前端） |
| 前端 | Vue 3 + TypeScript + Vite 6 + PrimeVue 4 + primeicons |
| 前端包管理 | pnpm |
| Tauri 官方插件 | @tauri-apps/plugin-dialog、@tauri-apps/plugin-opener |
| 后端异步运行时 | tokio |
| 实时通信 | tokio-tungstenite + futures-util（WebSocket，设备状态与音频通道） |
| 无线连接 | qrcode + rand（扫码配对）、mdns-sd（局域网 mDNS 设备发现） |
| 通用库 | serde / serde_json（序列化）、base64（截图编码）、image（截图处理）、zip（图标缓存打包） |

各依赖的版本约束与选型理由详见 [第 2 章](./02-tech-stack.md)。

## 三、阅读指引

| 角色 | 建议重点阅读 |
| --- | --- |
| AI 编程代理（执行任务） | 先读对应任务 SPEC（[doc/specs/](../specs/README.md)），SPEC 中会引用本目录的相关章节；需要理解全局时读第 1、4、5 章 |
| 架构评审 | 第 1、4、5、8 章 |
| 前端开发 | 第 1、2、3 章 |
| 后端开发 | 第 1、2、4、5、8 章 |
| 移动端协同（vt-mobile） | 第 7 章（协议定义），Android 端实现文档在 vt-mobile 仓库 |

## 四、文档约定

- 本文档与 [PRD](../prd/README.md) 分工：PRD 描述"做什么、为什么、做到什么程度"，本文档描述"用什么实现、怎么组织"。两者冲突时先修正文档使其一致，再改代码。
- 引用 PRD 需求时使用功能编号（如 `F-INPUT-01`）与非功能指标编号（如 `NFR-P-03`），保持可追溯。
- 本文档不估算任何任务的所需时间；任务粒度与执行顺序见 [任务路线图](../roadmap/README.md)。
- 文档使用简体中文，中英文之间加空格；专有名词保持标准写法（Tauri、tokio、WebSocket、MCP、API Key）。

## 五、修订记录

| 版本 | 日期 | 修订说明 |
| --- | --- | --- |
| v1.0 | 2026-10-09 | 初版：基于 PRD v1.3 与已确认技术选型完成全部 9 章 |
