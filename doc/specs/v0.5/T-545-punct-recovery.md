# T-545 识别标点恢复（ct-transformer 标点模型）

| 项目 | 内容 |
| --- | --- |
| 任务编号 | T-545 |
| 所属阶段 | v0.5 Beta |
| 优先级 | P1 |
| 覆盖需求 | F-INPUT-03（识别链路）、F-SET-02（输出选项，删除结尾标点可测）、NFR-P-03（上屏延迟预算） |
| 前置依赖 | v0.1 T-006（模型下载体系）、T-011（后处理管线）；配置 `AsrConfig.punct.enabled` 已预注册（默认 false） |

## 1. 目标与范围

### 1.1 目标

为识别输出接入标点恢复：识别文本在送出 ASR 后、进入后处理管线前，经 sherpa-onnx OfflinePunctuation（ct-transformer 中文标点模型，int8 量化）恢复句读，使"删除结尾标点"等输出选项在有标点输出后可生效、可测；根治验收手册 §4.4.6"内置模型无标点输出，删除结尾标点暂记 N/A"已知限制。

### 1.2 范围内

- 标点模型注册与元数据（ID / 大小 / SHA-256 / 下载地址 / 归档内成员路径），走现有 model_download 下载体系（官方归档、多线程分段、断点续传、进度事件 `model:progress`、SHA-256 校验、删除）；
- PunctService：模型生命周期（下载 / 取消 / 删除 / 懒加载引擎 / 哈希校验）+ 标点恢复执行 + 优雅降级；
- AsrService 扩展：识别推理完成后（`Recognized` 事件前）应用标点恢复；`ConfigChanged(Asr)` 时重置标点引擎与校验缓存；
- 后端命令：`punct_model_status` / `download_punct_model` / `cancel_punct_model` / `delete_punct_model`（标点模型单一，命令不带 model_id 参数）；
- 前端：`src/api/asr.ts` 追加标点命令封装与 `PunctModelInfo` 类型（types.ts 为共享文件，本任务报告给出收编片段）；模型管理页（ModelsView）新增标点模型卡片（下载 / 进度 / 取消 / 删除 / 校验失败提示）；文本输出页（OutputView）新增"识别标点恢复"开关（绑定 `asr.punct.enabled`，默认关）。

### 1.3 范围外

- 后处理管线 Stage 改造（post_process.rs / stages.rs 不归本任务所有）：标点恢复在 AsrService 内、`Recognized` 事件前执行，管线内各 Stage 顺序不变；
- 场景规则中的标点风格（全角 / 半角，T-502 方向）；
- 英文专用标点模型与自定义标点模型（当前 zh-en 模型已覆盖中英混排）；
- 标点恢复对 `asr_recognize_wav` 验证命令的输出（保持原始识别文本，便于对照）。

## 2. 需求依据

- PRD 3.1.5 文本处理流程：识别结果上屏前依次处理（标点恢复位于语气词过滤之前，产出供后续 Stage 消费的带标点文本）；
- PRD 3.8.3 文本输出（F-SET-02）："删除结尾标点"默认开，开启后自动删除结尾标点（包括"。"和"？"）——需标点恢复后才有对象可删（验收手册 4.4.6 现记 N/A）；
- 验收手册 [v0.1-验收测试手册](../../test/v0.1-验收测试手册.md) 4.4.6 / §7：内置模型无标点输出使"删除结尾标点"开关暂无对象，本任务根治；
- NFR-P-03：≤10 s 语音上屏 P95 ≤800 ms（推理 ≤600 ms + 后处理 ≤50 ms + 上屏 ≤150 ms）；int8 标点模型单次推理须控制在预算内，未启用 / 未下载时零开销；
- 产品原则 1（本地优先）：标点推理纯本地，断网可用；原则 3（结果可预期）：开关默认关、可关闭；
- tech/02 2.3：sherpa-rs 0.6（static 特性）已含 `punctuate::{Punctuation, PunctuationConfig}` 绑定，C API `SherpaOnnxOfflinePunctuationConfig` 仅需 ct_transformer 模型路径单文件（词表内嵌 ONNX metadata，归档内 tokens.json 不参与加载）。

## 3. 技术上下文

- 新建实现：`src-tauri/src/services/punct.rs`（已有桩注释）；
- 扩展：`src-tauri/src/services/asr.rs`（AsrService 持有 PunctService；`AsrService::new` 公开签名不变，避免触碰 main.rs / state.rs）、`src-tauri/src/commands/asr.rs`（新命令）；
- 复用（均不改公开函数）：`services/model_download.rs` 的 `build_client` / `download_file` / `probe_size` / `speed_of` / `DEFAULT_THREADS`，asr.rs 的 `DownloadProgress` 与事件 `AppEvent::ModelDownloadProgress`（`model:progress`）；
- 模型（2026-10-12 实测登记）：
  - ID：`punct-ct-transformer-zh-en-int8`；
  - 官方归档：`https://github.com/k2-fsa/sherpa-onnx/releases/download/punctuation-models/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8.tar.bz2`（curl -IL 实测 302 → 200）；
  - 归档大小 64,717,756 字节，SHA-256 `c0d5aa5f8eeb686032345e180bedf39319dc2e0556781c6264bcadba8328a6e1`；
  - 归档成员 `sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8/model.int8.onnx`，解包后统一命名 `model.onnx`，大小 75,519,198 字节，SHA-256 `65a3fb9f5ad7bfb96bf69e0dc4481df97f6ee60513c1d94ce981ba6effd524b1`；
  - **下载仅走官方归档**：镜像源（hf-mirror.com）无对应仓库，`https://hf-mirror.com/csukuangfj/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12` 实测 404，不适用 `model_source` 镜像优先策略与 `mirror_files()`（其根地址为 Paraformer 专用仓库）；
  - sherpa-onnx ct-transformer 实现内部按 ~20 token 段循环推理（长文本无截断），长录音安全；
- 配置：`AsrConfig.punct: PunctConfig { enabled: bool }`（已预注册，默认 false，config_store.rs 不归本任务所有，禁止改动）；
- 前端：`src/api/asr.ts` 扩展（命令封装 + 类型）；`src/views/general/ModelsView.vue`、`src/views/general/OutputView.vue` 扩展；新文案经 `getMessage` 动态取词（i18n 条目由共享文件负责人补登，精确片段见任务报告）。

## 4. 任务步骤

1. ✅ 验证模型 URL（curl 200）并实测登记元数据：归档地址 / 大小 / SHA-256、归档内 `model.int8.onnx` 大小与 SHA-256（见 §3）；
2. 实现 `punct.rs`：模型元数据、PunctService（状态查询 / 下载（官方归档解包 + 校验 + 进度事件 + 取消）/ 删除 / 懒加载引擎（哈希校验 + spawn_blocking 语境执行）/ 降级语义 `punctuate_sync`）；
3. 扩展 `asr.rs`：AsrService 内部持有 PunctService；`handle_audio` 推理后在 `Recognized` 事件前应用标点恢复（仅当开关开启且引擎可用，空文本跳过）；`ConfigChanged(Asr)` 重置标点引擎与校验缓存；新增标点模型管理委托方法；
4. 扩展 `commands/asr.rs`：4 个新命令（契约与前端 `src/api/asr.ts` 逐字对应）；
5. 前端：`src/api/asr.ts` 封装；ModelsView 新增标点模型卡片（复用现有卡片样式与进度组件，监听 `model:progress` 按 ID 过滤）；OutputView 新增"识别标点恢复"开关（绑定 `asr.punct.enabled`，未下载时提示引导）；
6. 单元测试：元数据一致性、归档解包（构造微型 tar.bz2）、哈希校验通过 / 拒绝、未下载 / 已下载 / 损坏状态、模型不可用降级（原文直出）；
7. 更新验收手册说明由共享文件负责人执行（手册不归本任务所有）；本文档勾选状态随验收更新。

## 5. 验收标准

- [x] 标点模型元数据 URL 真实可验证（curl 200），归档与模型文件 SHA-256 为实测值；
- [ ] 模型管理页出现"标点恢复"模型卡片：未下载可下载（进度 / 速度 / 取消）、已下载可删除、删除后状态回退为未下载；
- [ ] 下载完成经 SHA-256 校验；损坏文件标记为无效并引导重新下载（拒绝加载，不误用）；
- [ ] 开启"识别标点恢复"且模型已下载：识别输出带中文句读（，。？！等）；
- [ ] 开启开关但模型未下载 / 未下载完成：识别原文直出，无报错、不阻塞核心链路（优雅降级）；
- [ ] 关闭开关：识别输出与 v0.1 一致（无标点，零行为变化）；默认关闭；
- [ ] 开启标点恢复后，"删除结尾标点"开关可测：开启时上屏文本无结尾标点，关闭时保留；
- [ ] 标点恢复在识别推理的 `spawn_blocking` 内执行，不阻塞事件循环；耗时记日志（不含文本，NFR-S-02）；
- [ ] 标点推理纯本地：断网后功能完整可用（产品原则 1）；
- [ ] Windows / macOS / Linux（X11）手动验证：下载 → 开启 → 口述"你好世界"类短句与长句，上屏带标点；关闭开关后无标点；
- [ ] 隐私检查（tech/08 8.9 对照）：标点模型文件与识别文本均不出本机；日志无识别文本（NFR-S-02）。

## 6. 测试要求

- `cargo test`：punct.rs 元数据 / 解包 / 校验 / 状态 / 降级单测全绿；asr.rs 既有测试不回归；
- `cargo clippy --all-targets -- -D warnings`、`cargo fmt --check` 通过（仅本任务文件零错误零警告；其他并行任务文件的问题不归本任务）；
- 前端 `pnpm lint && pnpm typecheck && pnpm test` 通过；
- 手动验证（三平台）：见验收标准第 10 条；断网验证见第 9 条。

## 7. 完成定义（DoD）

- 验收标准全部勾选（三平台手动验证项由用户在其测试机执行，后端 / 前端逻辑以单测与本机 Linux 验证兜底）；
- 未改动任何非本任务所有文件的公开函数签名与行为；共享文件（main.rs invoke_handler、i18n、types.ts）所需改动已在任务报告中给出精确片段；
- 提交信息（建议）：`feat: 识别标点恢复（ct-transformer int8 标点模型）`（文档提交单独：`docs: T-545 识别标点恢复 SPEC`）。
