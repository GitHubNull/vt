# T-018 日志中心（查看 / 搜索 / 双写配置）

| 项目 | 内容 |
| --- | --- |
| 任务编号 | T-018 |
| 所属阶段 | v0.1 MVP |
| 优先级 | P1 |
| 覆盖需求 | F-LOG-01 ~ F-LOG-03（PRD 3.13，v1.6 新增） |
| 前置依赖 | T-001（日志地基）、T-002（设置外壳）、T-003（配置存储） |

## 1. 目标与范围

### 1.1 目标

把日志从"只能导出后外部查看"升级为应用内可搜索、可过滤、可高亮定位的独立排障模块；日志记录支持文本文件 + 本地数据库双写（默认全开、异步、至少一项），数据库服务于应用内搜索与按条件导出。

### 1.2 范围内

- 独立一级导航"日志"（与"关于"平级，PRD 2.2 侧边栏位于"常见问题"与"关于"之间）；
- 日志查看：按时间倒序，级别 / 时间范围过滤，单页最多 200 条；
- 搜索：关键词匹配（默认）、正则表达式开关、大小写敏感开关；命中部分在列表中高亮；
- 语法高亮：时间 / 级别 / target / 正文分区着色（级别着色：DEBUG 灰、INFO 蓝绿、WARN 橙、ERROR 红）；
- 搜索结果导出：按当前查询条件导出（带系统信息头，自动脱敏，复用 NFR-S-02 脱敏层）；
- 记录目标配置：`log.targets`（`file` / `database`），默认两者全开，至少保留一项（后端校验拒绝全空）；
- 双写管道：tracing 事件 → 异步通道 → 文本按天滚动文件 + SQLite（写入前统一过脱敏层）；
- 数据库目标关闭时：停止写入并断开连接，查询 / 搜索导出命令返回带编号错误（E-LOG-001），前端搜索区禁用并展示说明；重新开启后热生效（重建连接）。

### 1.3 范围外

- 日志级别的运行时切换（诊断模式沿用"通用 › 系统与数据"，本任务不改）；
- 数据库日志的手动清理入口（保留策略 7 天自动执行，删除超龄记录）；
- 多文件合并时间线视图（查询范围为数据库记录，文本文件仅作导出与外部排查用途）。

## 2. 需求依据

- PRD 3.13 节（v1.6）业务规则与验收标准逐条；
- NFR 5.7 第 4、5 条：双写异步、至少一项、同 7 天保留、数据库不含敏感数据；
- NFR-S-02：界面与导出不得出现 API Key、令牌、识别文本、剪贴板内容（写入前脱敏）。

## 3. 技术上下文

- 修改：`src-tauri/src/services/log.rs`（双写管道 + 查询 + 导出）、`src-tauri/src/main.rs`（init_logging 接入）、`src-tauri/src/services/config_store.rs`（LogConfig + ConfigSection::Log + 全空校验）、`src-tauri/src/commands/log.rs`（新命令）、`src-tauri/src/state.rs`、`src-tauri/Cargo.toml`（`rusqlite` bundled、`regex`）；
- 新建：`src/views/log/LogView.vue`、`src/stores/log.ts`、`src/api/log.ts`；
- 修改前端：`src/router.ts`（navModule `log`，order 8，about 顺延 9）、`src/api/types.ts`、`src/stores/config.ts`（默认配置扩展）、i18n；
- 数据库：日志目录下 `vibetype-logs.sqlite3`，表 `logs(id INTEGER PK, ts_millis INTEGER, level TEXT, target TEXT, message TEXT)`，索引 `ts_millis` / `level`；启动时清理 `ts_millis < now-7d`；
- 查询实现：级别 / 时间范围走 SQL WHERE；关键词 / 正则匹配在消费侧逐行过滤（SQLite 无内置正则；关键词匹配注意 `%` / `_` 转义或使用 instr）；正则由 `regex` crate 编译，非法表达式返回 E-LOG-002；
- 文本滚动文件与旧 `export_logs`（F-ABOUT-02）行为保持不变。

## 4. 任务步骤

1. Cargo.toml 增加 `rusqlite = { version = "0.32", features = ["bundled"] }`、`regex = "1"`；
2. config_store：`LogTarget` 枚举（file/database）+ `LogConfig { targets }`（默认全开）；`ConfigSection::Log`（`"log"`）；apply 校验全空拒绝（E-CONFIG-004）；
3. log.rs 重构：
   - `LogRecord { ts_millis, level, target, message, fields }` + 自定义 `tracing_subscriber::Layer`（Visitor 收集 message 与结构化字段）；
   - `init(log_dir, targets_rx)`：创建 mpsc 无界通道 + 异步消费任务（`tokio::spawn`），按 watch 通道最新 targets 分发到文本 writer（tracing-appender 按天滚动）与 SQLite（单连接，启动建表 + 7 天清理）；
   - 脱敏在入队前完成（`sanitize_log_line` 复用到 message 与各字段值）；
   - `query(conn, LogQuery)`：`levels` / `from` / `to` SQL 过滤 + 消费侧关键词/正则匹配 + limit（≤200 默认 200）/ offset，返回条目与是否还有更多；
   - `export_by_query(...)`：同条件流式导出（系统信息头 + 脱敏行）；
4. main.rs：init_logging 改为注册自定义 Layer（EnvFilter 不变）；init_services 中创建 watch sender 并按配置初始化 log 双写，订阅 ConfigChanged(Log) 热更新 targets；AppState 增加 log 句柄（log_dir + db 连接管理器）；
5. commands/log.rs：`log_query`、`log_export`（log_export 带目标路径参数，前端经 plugin-dialog 选位置）；数据库未启用返回 E-LOG-001；正则非法返回 E-LOG-002；
6. 前端：
   - router：`log` 模块（icon `pi pi-list`，order 8）+ `/log/viewer` 子路由；about 顺延 9；路由测试与快照更新；
   - LogView.vue：记录目标卡片（两个 ToggleSwitch，全关时提示并拒绝保存）→ 搜索栏（关键词 InputText、正则 ToggleSwitch、大小写敏感 ToggleSwitch、级别 MultiSelect、起止时间）→ 结果列表（语法高亮 + 命中高亮）→ 导出按钮（plugin-dialog 选路径）；数据库未启用时搜索区禁用 + 说明；
   - stores/log.ts（query 状态 + 调用）+ 单测；
   - i18n zh-CN / en-US 全量文案；
7. 测试：后端匹配器单测（关键词大小写、正则合法性、级别过滤拼装）；config 全空拒绝单测；前端 store / 高亮工具单测。

## 5. 验收标准

- [ ] PRD 3.13（v1.6）验收标准逐条通过；
- [ ] 关键词过滤 + 命中高亮生效；
- [ ] 正则开关过滤生效；非法正则返回 E-LOG-002 且列表不变；
- [ ] 大小写敏感开关行为符合 PRD（默认不敏感）；
- [ ] 关闭数据库目标后搜索区禁用 + 说明；重新开启恢复；
- [ ] 全关两种记录目标无法保存并提示至少一项；
- [ ] 导出结果含系统信息头且搜索不到最近一次语音输入的文字；
- [ ] 数据库关闭期间产生的日志不入库（重新开启后对该时段搜索无结果）；
- [ ] 日志双写为异步：写库失败不影响语音输入主链路（降级记 stderr）；
- [ ] 页面无硬编码文案。

## 6. 测试要求

- `cargo test`：匹配器（关键词 / 大小写 / 正则 / 转义）、LogConfig 校验、查询分页；
- `pnpm test`：log store、日志高亮工具、router 导航快照；
- 手动验证：PRD 3.13 验收清单。

## 7. 完成定义（DoD）

- 验收标准全部勾选；
- 提交信息：`feat: 独立日志中心模块（双写配置、关键词/正则/大小写搜索、语法高亮、结果导出）`。
