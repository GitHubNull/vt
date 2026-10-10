# T-017 模型下载体验增强

| 项目 | 内容 |
| --- | --- |
| 任务编号 | T-017 |
| 所属阶段 | v0.1 MVP |
| 优先级 | P0 |
| 覆盖需求 | F-SET-06（模型管理，PRD 3.8.7 v1.5 增强） |
| 前置依赖 | T-006、T-014 |

## 1. 目标与范围

### 1.1 目标

解决模型下载"半天不动"的可用性问题：默认走国内镜像源、多线程分段加速、断点续传、实时速度显示，并提供下载目录、下载代理、下载源的可配置能力；模型默认存储到安装目录下的 `models/` 语义化子目录。

### 1.2 范围内

- 下载源：官方源（GitHub Release）/ 镜像源（hf-mirror.com 的 HuggingFace 同名仓库，作者 csukuangfj 官方维护），**默认镜像源**；两源下载产物一致，SHA-256 以内置元数据为准，镜像源校验不通过自动回退官方源重下；
- 下载目录可配置（模型存储位置，`asr.models_dir`；默认 = 安装目录旁 `models/`，不可写时回退数据目录 `models/` 并注明）；
- 下载代理可配置（`asr.download_proxy`，如 `http://127.0.0.1:7890`；留空直连；只作用于模型下载）；
- 下载器重写：多线程分段并发（Range 请求，默认 4 线程，服务端不支持 Range 时回退单流）、应用退出/失败后断点续传（`.part` 文件保留已下载字节）、实时速度（bytes/sec）进进度事件；
- 前端：下载进度条上方显示实时速度与预计剩余时间；模型页增加下载源、代理、存储位置设置项（目录经 plugin-dialog 选择）；
- 镜像直传文件模式：镜像源按文件直传（`model.onnx` / `model.int8.onnx` / `tokens.txt` 三个文件分别下载），官方源仍走归档解包，两条路径产物目录结构一致。

### 1.3 范围外

- 下载暂停（PRD 3.8.7 列了"暂停"，v0.1 保留取消/失败重试，暂停 v0.5 再议）；
- 模型目录"迁移已下载模型"确认流程（更改目录后旧目录模型留在原地，新目录重新下载即可，避免大文件搬运风险）；
- 更新检查等其他网络请求的代理（v0.1 更新服务器未就绪）。

## 2. 需求依据

- PRD 3.8.7 节（v1.5 修订）验收标准逐条；
- NFR-P-09：安装包不含模型；
- NFR-S-02：日志不含用户数据（代理地址属本机配置，可写日志；不得含密钥）；
- 内部试用反馈（2026-10-10，Win10）：GitHub 源国内直连不可用，需手动放置模型文件。

## 3. 技术上下文

- 新建：`src-tauri/src/services/model_download.rs`（下载器）、`.cargo/config.toml` 不涉及；
- 修改：`src-tauri/src/services/asr.rs`（下载路径接入、models_dir 动态化）、`src-tauri/src/services/config_store.rs`（AsrConfig 新字段）、`src-tauri/src/events.rs`（进度事件加 `bytes_per_sec`）、`src/views/general/ModelsView.vue`、`src/stores/asr.ts`、`src/api/types.ts`、i18n；
- 镜像直传 URL 模板：`https://hf-mirror.com/csukuangfj/sherpa-onnx-paraformer-zh-2023-03-28/resolve/main/{file}`（2026-10-10 实测：单连接约 12 MB/s，支持 Accept-Ranges，文件大小与内置元数据一致）；
- 官方归档源保留：`https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-paraformer-zh-2023-03-28.tar.bz2`。

## 4. 任务步骤

1. AsrConfig 增加 `models_dir: Option<String>`、`download_proxy: Option<String>`、`model_source`（mirror/official，默认 mirror）；
2. 默认模型目录解析：`exe_dir/models` 探测可写（创建+删除探针文件），不可写回退 `app_data_dir/models`；结果与回退原因记日志；
3. `model_download.rs`：分段计划（按大小与线程数切 Range）、每段独立请求+段内断点续传、聚合进度（AtomicU64）、速度滑动统计、代理 Client 构造、镜像/官方 URL 解析、单流回退；
4. asr.rs 接入：`download_inner` 改走新下载器（镜像直传三文件或官方归档单文件），`.part` 残留续传；models_dir 改读配置（动态解析），引擎加载/校验/删除路径全部走动态目录；
5. 事件：ModelDownloadProgress 增加 `bytes_per_sec`，前端显示速度与 ETA；
6. 前端：模型页设置区（下载源下拉、代理输入框、存储位置只读框+更改按钮经 plugin-dialog）；
7. 文案入 i18n；
8. 测试：分段计划、URL 解析、速度计算单测；asr store 测试更新。

## 5. 验收标准

- [ ] PRD 3.8.7（v1.5）验收标准逐条通过；
- [ ] 默认（镜像源、无代理）在国内网络下可完成下载，速度显著优于 GitHub 直连；
- [ ] 下载中显示实时速度与预计剩余时间；
- [ ] 下载中途退出应用，重启后点击下载从断点继续（`.part` 文件增长而非重建）；
- [ ] 配置代理后下载经代理（代理侧可见连接），清空恢复直连；
- [ ] 更改存储目录后新模型下载到该目录且可加载识别；
- [ ] 全新安装默认模型目录 = 安装目录 `models/`（不可写安装目录回退数据目录，界面注明）；
- [ ] 镜像源下载后 SHA-256 校验通过；篡改 `.part` 内容后续传/校验能发现损坏；
- [ ] 页面无硬编码文案。

## 6. 测试要求

- `cargo test`：分段计划（含小文件退化为单段）、镜像 URL 解析、速度计算、part 文件续传偏移计算；
- 手动验证：镜像/官方两源下载、断点续传、代理（如有代理环境）、目录更改。

## 7. 完成定义（DoD）

- 验收标准全部勾选；
- 提交信息：`feat: 模型下载体验增强（镜像源、多线程断点续传、速度显示、代理与目录配置）`。
