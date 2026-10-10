# 第 4 章 Rust 后端架构

> 本章描述 Rust 后端的分层结构、tokio 运行时拓扑、核心服务设计与错误处理。
> 需求依据：PRD 第 5 章（NFR-R 可靠性、NFR-P 性能）。

## 4.1 分层结构与目录

```
src-tauri/src/
├── main.rs              # 入口：初始化日志、tokio 运行时、Tauri Builder、插件与命令注册
├── error.rs             # AppError 统一错误类型（thiserror）
├── state.rs             # AppState：各服务句柄的聚合，注入 Tauri State
├── commands/            # 命令层：#[tauri::command] 函数，按领域分文件
│   ├── mod.rs
│   ├── dict.rs          # 词典命令
│   ├── config.rs        # 配置命令
│   ├── ai.rs            # AI 命令
│   ├── device.rs        # 移动设备命令
│   └── ...
├── services/            # 服务层：业务逻辑，tokio 异步
│   ├── mod.rs
│   ├── audio.rs         # AudioService
│   ├── asr.rs           # AsrService
│   ├── hotkey.rs        # HotkeyService
│   ├── inject.rs        # InjectService
│   ├── dict.rs          # DictService
│   ├── command_engine.rs# CommandEngine（编程口令）
│   ├── ai_pipeline.rs   # AiPipeline
│   ├── mcp_server.rs    # McpServer（v1.0）
│   ├── device_hub.rs    # DeviceHub（v1.0）
│   ├── config_store.rs  # ConfigStore
│   ├── log.rs           # LogService
│   └── update.rs        # UpdateService
└── platform/            # 平台适配层：trait + 三平台实现
    ├── mod.rs           # trait 定义与工厂函数
    ├── hotkey/          # 全局快捷键
    ├── inject/          # 文本注入
    ├── audio/           # 音频采集
    ├── window_info/     # 前台窗口 / 场景识别
    └── credentials/     # OS 凭据存储
```

依赖规则（编译期靠模块可见性约束，评审期靠 code review）：

- `commands/` 只能调用 `services/` 与 `state.rs`；
- `services/` 只能调用 `platform/` 与同层服务的公开消息接口；
- `platform/` 不依赖上两层；
- 前端不感知 `services/` 与 `platform/` 的存在。

## 4.2 tokio 运行时拓扑

- 单一 `tokio::runtime::Runtime`（multi-thread，worker 数 = CPU 核数，上限 4，避免空闲 CPU 超标 NFR-P-07）；
- Tauri 主线程运行事件循环；所有命令处理函数为 `async`，在 tokio 运行时上调度；
- 长生命周期任务（音频采集、ASR 推理、WebSocket 服务、mDNS 守护）以 `tokio::spawn` 启动，句柄存于对应服务，服务 `shutdown()` 时统一 `abort` / 优雅停止；
- 阻塞型工作（ASR 推理、zip 打包、图像处理）放入 `tokio::task::spawn_blocking` 专用线程池，不占用异步 worker。

### 关键通道

| 通道 | 类型 | 容量 | 说明 |
| --- | --- | --- | --- |
| 音频帧流 | `mpsc::channel<AudioFrame>` | 64（约 2 秒缓冲） | AudioService → AsrService；满时丢最旧帧并记日志 |
| 录音控制 | `mpsc::channel<RecordCommand>` | 8 | 命令层 / 快捷键 → AudioService |
| 事件广播 | `broadcast::Sender<AppEvent>` | 256 | 服务 → 前端事件桥（recording:state 等） |
| 设备消息 | `mpsc::channel<DeviceMessage>` | 32 | DeviceHub 内部（v1.0） |

背压原则：音频通道满时宁可丢帧也不阻塞采集线程（丢帧记日志，NFR 可观测性）；控制类通道满时返回错误而非丢弃。

## 4.3 命令层约定

- 每个命令函数签名：`async fn cmd_x(state: State<'_, AppState>, args...) -> Result<T, AppError>`；
- 命令名 snake_case，与前端 `api/` 封装一一对应（见 [第 3 章](./03-frontend.md) 3.5）；
- 命令层做：参数校验（长度、格式、枚举合法性）、权限检查（如 MCP 是否启用）、调用服务、错误转换；
- 命令层**不做**业务逻辑，不写日志正文（日志在服务层），不包含用户文本内容（NFR-S-02）；
- 所有命令在 `main.rs` 的 `invoke_handler` 中集中注册，新增命令必须同步登记前端 `api/`。

## 4.4 服务层设计要点

### 通用模式

每个服务的标准形态：

- 持有状态：`Arc<RwLock<ServiceState>>`（读多写少）或 actor 模式（状态私有 + 消息驱动，音频 / 设备类服务优先 actor）；
- 生命周期：`start()`（注册进 AppState 时调用）/ `shutdown()`（应用退出时调用）；
- 状态变更通过 `broadcast::Sender<AppEvent>` 广播，由事件桥转为 Tauri Event 推给前端。

### 重点服务说明

**AudioService**（actor 模式）：管理采集状态机 `Idle / Warming / Recording`；常驻预热模式下保持设备打开但**丢弃所有采样**（NFR-PR-07：预热不得采集或保存声音）；录音启动延迟预算见 [第 5 章](./05-core-pipeline.md)。

**AsrService**：模型加载在 `spawn_blocking`；推理结果通过回调送入后处理管线；模型文件完整性校验（下载后哈希比对）在加载前完成。

**HotkeyService**：封装 `global-shortcut`，在其 Pressed / Released 事件上实现按住说话（Press 开始 / Release 结束）与切换模式（Press 翻转状态）；冲突检测（F-SYS-03）在注册时进行。

**InjectService**：优先模拟键入上屏；目标应用不支持或权限不足时回退"剪贴板 + 模拟粘贴 + 恢复原剪贴板"；无焦点兜底（F-INPUT-09）走通知 + 复制到剪贴板。平台差异见 [第 9 章](./09-cross-platform.md)。

**ConfigStore**：单例；写入 = 序列化 → 写临时文件 → `fsync` → 原子 rename（NFR-R-05）；读取时校验 JSON Schema，损坏自动回退到 `.bak` 并提示。详见 [第 8 章](./08-data-and-security.md)。

**DeviceHub**（v1.0）：配对会话管理（`rand` 生成一次性码）、mDNS 广播（`mdns-sd`）、WebSocket 服务（`tokio-tungstenite`）、控制键事件路由到 InjectService。协议见 [第 7 章](./07-mobile-link.md)。

**McpServer**（v1.0）：默认关闭；仅绑定 `127.0.0.1`；每个请求校验令牌，失败一律拒绝并记录（NFR-S-04）。详见 [第 6 章](./06-ai-and-mcp.md)。

## 4.5 平台适配层

- 每个能力一个 trait（如 `trait TextInjector { fn inject(&self, text: &str) -> Result<()>; }`）；
- `platform/mod.rs` 提供工厂函数 `pub fn default_injector() -> Box<dyn TextInjector>`，内部按 `cfg!(target_os)` 选择实现；
- 三平台实现文件用 `#[cfg(target_os = "...")]` 条件编译；
- 每个 trait 提供 `mock` 实现（`#[cfg(test)]` 或 feature gate），服务层单测不依赖真实 OS 能力；
- 平台实现中调用 OS API 的位置必须附注释说明所需最小权限（AGENTS.md 3.5）。

## 4.6 错误处理

- 统一 `AppError`（thiserror 枚举）：`Audio`、`Asr`、`Inject`、`Config`、`Network`、`Permission`、`Platform` 等变体，每个变体带稳定错误编号（如 `E-AUDIO-001`）；
- 错误编号出现在用户可见提示中（NFR：错误提示带错误编号），编号与文案映射在 `src/i18n/`；
- 服务内部错误用 `thiserror` 定义本服务错误，命令层边界统一转换为 `AppError`；
- 不 `unwrap` / `expect`（测试与 `main.rs` 初始化除外），`cargo clippy` 配置 `unwrap_used` 警告；
- 崩溃兜底：panic hook 记录日志后退出；由系统级守护（安装包注册的开机项 / 手动重启）满足 NFR-R-02 崩溃自动重启，应用内不做看门狗进程。

## 4.7 日志

- `tracing` + `tracing-subscriber`，按天滚动、保留 7 天（NFR 可观测性）；
- 全局脱敏过滤器：写入前过滤 API Key、令牌、识别文本、剪贴板内容等模式（NFR-S-02），实现见 [第 8 章](./08-data-and-security.md)；
- 日志级别：默认 `info`；`debug`/`trace` 仅在用户开启"诊断模式"时生效。
- **双写管道（F-LOG-02，T-018）**：自定义 `tracing` Layer 把结构化记录（时间戳 / 级别 / target / 消息 / 字段）送入 `mpsc` 无界通道，独立异步消费任务按配置分发到两类目标——按天滚动文本文件（`tracing-appender`）与本地 SQLite 数据库（rusqlite，bundled 编译避免系统依赖）。记录目标可在设置中调整（`log.targets`，至少一项），变更经 `config:changed` 事件热生效：关闭数据库目标即断开连接（查询功能随之不可用），重新开启则重建连接；
- **日志查询（F-LOG-01）**：级别 / 时间范围在 SQL 侧过滤，关键词 / 正则 / 大小写匹配在消费侧按记录过滤（SQLite 无内置正则）；关键词匹配对 `%` / `_` 转义，正则由 `regex` crate 编译、非法表达式返回 E-LOG 编号错误；单页上限 200 条；
- **搜索导出（F-LOG-03）**：按当前查询条件流式导出匹配记录，复用导出脱敏头（system_info_header）。

## 4.8 后端测试

- 服务层单测：mock 平台适配层，覆盖状态机与业务规则（如词典替换优先级、口令解析）；
- 集成测试：核心链路用注入的假音频帧驱动（不依赖真实麦克风）；
- 平台层测试：三平台 CI 上跑冒烟测试（能创建、能调用、权限缺失时错误正确）；
- 性能测试：NFR-P 指标的基准测试脚本放在 `src-tauri/benches/`，发布前运行。
