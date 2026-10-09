# 第 6 章 AI 后处理与 MCP 服务

> 本章描述 AI 提示词模板管线、AI 供应商抽象与 MCP 服务的技术设计。
> 需求依据：F-AI-01 ~ F-AI-11；NFR-S-01/03/04；NFR-PR-03。除模板引擎的基础结构外，大部分能力在 v0.5 / v1.0 落地。

## 6.1 AI 后处理管线（AiPipeline）

### 6.1.1 在后处理管线中的位置

AI 处理是 [第 5 章](./05-core-pipeline.md) 后处理管线的最后一个 Stage（可选）：仅当用户通过模板快捷键（F-AI-02）或小窗显式选择模板时触发，**默认不对每次语音输入做 AI 处理**。

### 6.1.2 模板引擎（F-AI-01/03/04）

- 模板 = `{ id, name, prompt, variables, hotkey?, builtin }`；
- 内置编程向模板（F-AI-03）：结构化 Prompt、Commit 信息、Code Review 评论、技术翻译等，内容以 PRD 3.5 节为准；
- 上下文变量（F-AI-04，v1.0）：`{{selected_text}}`、`{{clipboard}}`、`{{scene}}` 等，渲染时从平台层采集；采集前检查目标应用权限；
- 模板存储在配置目录 `templates.json`，导入导出走统一校验（NFR-S-07）。

### 6.1.3 供应商抽象（F-AI-06/07/08）

```
trait AiProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse>;
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;   // F-AI-08 拉取模型
    async fn test_connection(&self) -> Result<Latency>;      // F-AI-08 连通性测试
}
```

- 预置供应商（PRD 3.7 节名单：通义千问、DeepSeek、智谱 AI、豆包、硅基流动、Ollama、LM Studio、Kimi、Gemini、OpenAI 等）以适配器实现该 trait；OpenAI 兼容协议适配器覆盖多数供应商，Ollama / LM Studio 走本地 HTTP；
- 供应商配置（base_url、API Key、模型列表）存配置；**API Key 存 OS 凭据管理器**（`keyring`），配置文件只存凭据引用键（NFR-S-01），界面掩码显示；
- HTTP 客户端：`reqwest`（rustls），统一超时与重试策略；
- 供应商列表渲染前去重（AGENTS.md 原型问题修正清单）。

### 6.1.4 超时降级（F-AI-05）

- 单次 AI 调用超时默认 15 s（可配）；超时 / 失败时降级为**原始识别文本直接上屏**，并提示降级原因；
- 附加耗时预算：本地处理 + 供应商响应之外 ≤200 ms（NFR-P-05）；
- 断网时云端供应商自动标记不可用，本地输入链路不受影响（NFR-R-04）。

### 6.1.5 外发前隐私闸门

- 首次向某供应商发送文本前，弹窗告知并获得用户确认（NFR-PR-03），确认记录按供应商持久化；
- 发送前跑敏感信息检测（NFR-S-03）：云服务 / 大模型密钥、私钥、密码赋值、手机号、身份证、银行卡、邮箱、内网地址，命中项自动打码并提示用户；检测在本地完成，规则库内置；
- 任何情况下不向云端发送原始音频（本地供应商除外）（NFR-PR-03）。

## 6.2 MCP 服务（F-AI-09/10/11，v1.0）

### 6.2.1 定位与边界

MCP 服务让本机 AI 编程智能体可以：向用户发起语音提问（F-AI-10）、查询与建议专有名词。它是**服务端**（VT 暴露 MCP Server），不是客户端。

### 6.2.2 传输与安全（NFR-S-04）

- 默认**关闭**，设置中显式开启；
- 仅绑定 `127.0.0.1`，不监听任何外部接口；端口可配（默认在 ephemeral 段选一固定值，设置页显示）；
- 每个 MCP 客户端连接须出示令牌：令牌在开启服务时由 `rand` 生成，存 OS 凭据管理器，设置页可复制（掩码显示）；
- 校验失败一律拒绝并记录（来源、时间、原因），不区分"令牌错误"与"令牌缺失"的响应；
- 访问控制（F-AI-11）：按能力（提问 / 查词 / 建议词）逐项授权，调用记录（时间、客户端、能力、参数摘要）持久化 7 天，设置页可查看与清空。

### 6.2.3 语音提问链路（F-AI-10）

```
AI 智能体 ──MCP──► McpServer ──事件──► 前端弹提问卡片（不抢焦点）
                                            │ 用户语音回答
                                            ▼
                              核心链路（录音→识别，跳过 AI Stage）
                                            │
智能体 ◄──MCP 响应── McpServer ◄── 识别文本 ──┘
```

- 提问卡片不抢焦点（产品原则 2），用户可忽略；超时未答返回超时错误给智能体；
- 回答文本是否经过词典 Stage 由设置控制；不经过 AI Stage（避免循环）。

## 6.3 与其他模块的关系

| 依赖方 | 被依赖 | 关系 |
| --- | --- | --- |
| AiPipeline | ConfigStore | 读取模板与供应商配置 |
| AiPipeline | LogService | 调用记录（脱敏后） |
| AiPipeline | platform/credentials | API Key 存取 |
| McpServer | AiPipeline | 语音提问复用识别链路 |
| McpServer | DictService | 专有名词查询 / 建议 |
| CommandEngine | AiPipeline | 无直接依赖（口令是纯本地规则） |

## 6.4 落地阶段

| 能力 | 版本 | 说明 |
| --- | --- | --- |
| 模板管理 / 快捷键 / 内置模板 / 供应商管理 / 超时降级 | v0.5 | F-AI-01/02/03/05/06/07/08 |
| 模板上下文变量 | v1.0 | F-AI-04 |
| MCP 服务 / 语音提问 / 访问控制 | v1.0 | F-AI-09/10/11 |
