# T-531 AI 供应商抽象与预置适配器

| 项目 | 内容 |
| --- | --- |
| 任务编号 | T-531 |
| 所属阶段 | v0.5 Beta |
| 优先级 | P0 |
| 覆盖需求 | F-AI-06（AI 供应商管理的服务端）、NFR-S-01（密钥存储） |
| 前置依赖 | v0.1 T-003 |

## 1. 目标与范围

### 1.1 目标

实现 AI 供应商抽象层与预置供应商适配器，API Key 走 OS 凭据管理器加密保存。

### 1.2 范围内

- `AiProvider` trait：`complete` / `list_models` / `test_connection`（tech/06 6.1.3）；
- OpenAI 兼容协议适配器（覆盖多数云端供应商）+ Ollama / LM Studio 本地适配器；
- 预置供应商清单：PRD 3.7 节名单（通义千问、DeepSeek、智谱 AI、豆包、硅基流动、Ollama、LM Studio、Kimi、Gemini、OpenAI 等），含默认 base_url 与模型获取方式；
- 供应商配置 CRUD 命令（base_url、启用状态、模型选择）；
- API Key 保管：`keyring` 存 OS 凭据管理器，配置文件只存引用键；界面层掩码约定；Linux 无 Secret Service 时的降级加密（tech/08 8.3）；
- HTTP 客户端：reqwest（rustls），统一超时 / 重试。

### 1.3 范围外

- 供应商设置页 UI（T-536）；
- 模板引擎（T-533）与 AI Stage（T-534）；
- 连通性测试 UI（T-536，本任务只实现服务端命令）。

## 2. 需求依据

- F-AI-06：AI 供应商管理（PRD 3.7 节）；
- NFR-S-01：API Key 加密保存（优先 OS 凭据），界面掩码显示；
- NFR-PR-03：任何情况下不向云端发送原始音频；
- [tech/06](../../tech/06-ai-and-mcp.md) 6.1.3；
- [tech/08](../../tech/08-data-and-security.md) 8.3。

## 3. 技术上下文

- 新建：`src-tauri/src/services/ai_pipeline.rs`（供应商部分）、`src-tauri/src/platform/credentials/`、`src-tauri/src/commands/ai.rs`；
- 引入 `keyring`、`reqwest`（tech/02 已登记）；
- 复用：T-003 ConfigStore。

## 4. 任务步骤

1. 定义 `AiProvider` trait 与请求 / 响应模型；
2. 实现 OpenAI 兼容适配器与 Ollama / LM Studio 适配器；
3. 预置供应商清单数据化（PRD 3.7 名单）；
4. 实现 `platform/credentials/`：keyring 封装 + Linux 降级加密；
5. 供应商配置 CRUD 命令 + Key 存取命令（Key 永不回传明文给前端，只回传"已设置"状态）；
6. 单元测试：适配器请求构造（mock HTTP）、凭据存取、降级加密；
7. 联调：用真实或本地 Ollama 验证 complete 调用。

## 5. 验收标准

- [ ] 预置供应商清单与 PRD 3.7 节一致；
- [ ] 配置 API Key 后配置文件中无明文 Key（只有引用键），OS 凭据管理器中有对应条目；
- [ ] OpenAI 兼容供应商与 Ollama 本地供应商均可完成一次文本补全（联调验证）；
- [ ] 断网时云端供应商调用返回明确错误（错误编号），本地供应商不受影响；
- [ ] 日志与错误信息中不出现 API Key（构造测试验证，NFR-S-01/02）；
- [ ] Linux 无 Secret Service 环境降级加密生效并明示保护级别。

## 6. 测试要求

- `cargo test`：适配器、凭据层；
- 手动验证：至少一个云端供应商（有 Key 时）与本地 Ollama 联调。

## 7. 完成定义（DoD）

- 验收标准全部勾选；
- 提交信息：`feat: AI 供应商抽象与密钥安全存储`。
