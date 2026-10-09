# T-003 配置存储与数据目录

| 项目 | 内容 |
| --- | --- |
| 任务编号 | T-003 |
| 所属阶段 | v0.1 MVP |
| 优先级 | P0 |
| 覆盖需求 | NFR-R-05（配置原子性）、NFR-R-06（升级保留配置）的地基 |
| 前置依赖 | T-001 |

## 1. 目标与范围

### 1.1 目标

实现 ConfigStore 服务与数据目录初始化，为全部设置项、词典、模板等提供统一的原子化持久化能力。

### 1.2 范围内

- 平台标准目录解析与初始化（配置 / 数据 / 日志三个目录，tech/08 8.1）；
- ConfigStore：JSON 配置读写、原子写入（tmp + fsync + rename）、`.bak` 回退、变更事件广播（`config:changed`）；
- 配置 Schema 定义（v0.1 所需节：`general`、`hotkey`、`audio`、`output`、`dict`、`about`），serde 类型与前端 `api/types.ts` 同步；
- Tauri Commands：`get_config` / `set_config`（分节读写）；
- 前端 `useConfigStore`（Pinia）：读取镜像、修改 → invoke → 成功提交 / 失败回滚。

### 1.3 范围外

- 任何设置页 UI（T-013 等）；
- 密钥存储（OS 凭据，v0.5 T-531）；
- 备份与恢复（v1.0 T-1034）。

## 2. 需求依据

- NFR-R-05：配置保存原子性，损坏自动回退；
- [tech/04](../../tech/04-backend.md) 4.4 ConfigStore 设计；
- [tech/08](../../tech/08-data-and-security.md) 8.1 目录布局、8.2 配置持久化；
- [tech/03](../../tech/03-frontend.md) 3.4 useConfigStore 约定。

## 3. 技术上下文

- 新建：`src-tauri/src/services/config_store.rs`、`src-tauri/src/commands/config.rs`、`src/stores/config.ts`、`src/api/` 对应封装；
- 目录解析用 Tauri `path` API（`app_config_dir` / `app_data_dir` / `app_log_dir`）；
- 复用：T-001 的 AppError、AppState、tracing。

## 4. 任务步骤

1. 定义 Rust 配置结构体（serde，带 `Default`），覆盖 v0.1 各节字段（界面语言 / 主题、快捷键、录音环境、文本输出选项、词典开关、更新检查开关）；
2. 实现 ConfigStore：`load()`（解析失败 → `.bak` → 默认 + 提示事件）、`save()`（tmp → fsync → rename → 更新 `.bak`）、`update(section, value)`；
3. 目录初始化：首次启动创建三个目录；
4. 实现 `get_config` / `set_config` 命令，参数校验（节名枚举、值 Schema）；
5. 写盘成功后广播 `config:changed`（携带变更节名）；
6. 前端 `api/config.ts` 封装 + `api/types.ts` 类型（与 Rust 结构体手工同步）；
7. 前端 `useConfigStore`：`load()` 初始化、 optimistic 更新 + 失败回滚 + 错误提示；
8. 单元测试：原子写入中断模拟（tmp 残留不影响读取）、损坏回退、并发读写。

## 5. 验收标准

- [ ] 应用首次启动自动创建配置 / 数据 / 日志目录（平台标准位置）；
- [ ] 修改配置后 `config.json` 内容正确，磁盘上无 `.tmp` 残留；
- [ ] 手工把 `config.json` 写坏（非法 JSON），重启后自动回退 `.bak` 或默认配置，并弹出提示；
- [ ] 两个窗口同时打开时，一处修改配置，另一处通过 `config:changed` 事件同步（用 T-002 外壳的任意两页验证）；
- [ ] 单元测试覆盖：正常读写、损坏回退、并发写；
- [ ] 日志中不出现配置值正文（NFR-S-02），只出现节名与操作结果。

## 6. 测试要求

- `cargo test`：ConfigStore 全部单测通过；
- 手动验证：损坏回退流程在当前平台实测。

## 7. 完成定义（DoD）

- 验收标准全部勾选；
- Rust 结构体与 `api/types.ts` 类型一致（PR 中自查清单勾选）；
- 提交信息：`feat: 配置存储与数据目录（原子写入与损坏回退）`。
