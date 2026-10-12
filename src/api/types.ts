/**
 * 与 Rust 侧 serde 类型对应的 TS 类型（tech/03 3.5），手工保持同步。
 * 权威定义：src-tauri/src/services/config_store.rs（T-003）、error.rs。
 */

/** 后端统一错误（AppError）序列化形态：{ code, message }（src-tauri/src/error.rs）。 */
export interface AppErrorShape {
  code: string
  message: string
}

/* ---- 配置（与 config_store.rs 逐字段同步；serde 字段名即 JSON 键，snake_case） ---- */

export type LanguageOption = 'system' | 'zh_cn' | 'en_us' | 'zh_tw'
export type ThemeMode = 'system' | 'light' | 'dark'
export type HotkeyMode = 'hold' | 'toggle'
export type FloatBallPositionOption = 'cursor' | 'bottom_center' | 'fixed'

/** 界面设置（F-SET-01）。 */
export interface GeneralConfig {
  language: LanguageOption
  theme: ThemeMode
  /** 开机自启动（PRD 3.8.6，F-SET-05，默认关）。 */
  launch_at_login: boolean
  /** 保存语音数据集（PRD 3.8.6，默认关，隐私优先）。 */
  save_voice_dataset: boolean
  /** 诊断模式（debug 日志级别，T-013）。 */
  diagnostic_mode: boolean
  /** 悬浮球上次拖动位置 [x, y]（T-008 位置记忆；null = 未拖动过）。 */
  float_ball_xy: [number, number] | null
}

/**
 * 输入与录音设置（F-INPUT-01/02、PRD 3.8.2）。
 * 快捷键键名格式：`ControlLeft+ShiftLeft+Space`、`AltRight`、`Escape`（HotkeyService 解析，见 T-004）。
 */
export interface HotkeyConfig {
  /** 录音快捷键（默认 AltRight，即右 Alt / 右 Option）。 */
  trigger: string
  /** 触发模式：hold 按住说话（默认）/ toggle 切换。 */
  mode: HotkeyMode
  /** 取消录音快捷键（默认 Escape）。 */
  cancel_trigger: string
  /** 最长录音时长（分钟，1–10）。 */
  max_recording_minutes: number
  /** 录音时显示悬浮球。 */
  show_float_ball: boolean
  float_ball_position: FloatBallPositionOption
  /** 开始 / 结束录音提示音（默认关）。 */
  sound_hint: boolean
}

/** 录音环境设置（F-SET-03、PRD 3.8.4）。 */
export interface AudioConfig {
  /** 首选麦克风设备 ID（null = 系统默认；设备 ID 来自 list_audio_devices，T-005）。 */
  input_device_id: string | null
  /** 录音时静音系统扬声器（默认开）。 */
  mute_speaker_while_recording: boolean
  /** 麦克风常驻预热（默认开，NFR-P-01 / NFR-PR-07）。 */
  keep_warm: boolean
}

/** 文本输出设置（F-SET-02、PRD 3.8.3）。 */
export interface OutputConfig {
  /** 智能数字转换（"一百二十" → "120"）。 */
  smart_number: boolean
  /** 过滤语气词。 */
  filter_fillers: boolean
  /** 输出繁体中文。 */
  traditional_chinese: boolean
  /** 删除结尾标点。 */
  remove_trailing_punct: boolean
  /** 使用复制粘贴输入（默认开）。 */
  paste_input: boolean
  /** 保留识别文本到剪贴板（默认关）。 */
  keep_text_in_clipboard: boolean
}

/** 词典设置（F-DICT-01 总开关）。 */
/** 替换词典设置（F-DICT-01 总开关 + T-521/T-522 子词典）。 */
export interface DictConfig {
  enabled: boolean
  /** 技术词库（T-521）。 */
  tech_lexicon: TechLexiconConfig
  /** 专有名词（T-522）。 */
  proper_nouns: ProperNounsConfig
}

/** 识别引擎设置（T-006 当前模型；T-017 下载体验；T-545 标点恢复）。 */
export interface AsrConfig {
  /** 当前激活模型 ID（null = 自动选择首个可用模型）。 */
  current_model_id: string | null
  /** 模型存储目录（null = 默认：安装目录旁 models/，不可写回退数据目录）。 */
  models_dir: string | null
  /** 模型下载代理（如 http://127.0.0.1:7890；null/空 = 直连；只作用于模型下载）。 */
  download_proxy: string | null
  /** 模型下载源（默认 mirror 镜像源）。 */
  model_source: 'mirror' | 'official'
  /** 识别标点恢复（T-545）。 */
  punct: PunctConfig
}

/** 关于页设置（F-ABOUT-01）。 */
export interface AboutConfig {
  /** 自动检查更新（默认开，NFR-PR-06）。 */
  auto_check_update: boolean
}

/** 日志记录目标（F-LOG-02）。 */
export type LogTargetOption = 'file' | 'database'

/** 日志设置（F-LOG-02：至少一项，默认文本 + 数据库全开）。 */
export interface LogConfig {
  targets: LogTargetOption[]
}

/** 应用场景（T-501，与 Rust SceneKind 逐字对应）。 */
export type SceneKind =
  | 'ide'
  | 'terminal'
  | 'ai_chat'
  | 'browser'
  | 'docs'
  | 'other'
  | 'unknown'

/** 用户自定义场景规则（T-501：应用标识 → 场景）。 */
export interface SceneRule {
  app: string
  scene: SceneKind
}

/** 标点风格（T-502，与 Rust PunctStyle 逐字对应）。 */
export type PunctStyleOption = 'follow' | 'half_width' | 'full_width'

/** 结尾标点策略（T-502）。 */
export type TrailingPunctOption = 'follow' | 'keep' | 'remove'

/** 中英文间空格策略（T-502）。 */
export type CjkSpaceOption = 'follow' | 'auto' | 'none'

/** 场景规则条目（T-502，PRD 3.2.1）。 */
export interface SceneEntry {
  id: string
  name: string
  icon_color: string
  scene: SceneKind
  apps: string[]
  title_keyword: string
  punct_style: PunctStyleOption
  trailing_punct: TrailingPunctOption
  cjk_space: CjkSpaceOption
  commands: boolean
  auto_enter: boolean
  ai_template: string | null
  builtin: boolean
}

/** 场景识别设置（T-501 自定义规则与 override；T-502 扩展场景规则条目）。 */
export interface SceneConfig {
  custom_rules: SceneRule[]
  override_scene: SceneKind | null
  /** 场景规则条目（空 = 使用内置默认）。 */
  entries: SceneEntry[]
}

/** AI 供应商条目（T-531；API Key 只存 OS 凭据，api_key_ref 为引用键）。 */
export interface AiProviderEntry {
  id: string
  name: string
  protocol: string
  base_url: string
  enabled: boolean
  model: string | null
  /** 已拉取 / 手动维护的可用模型清单（T-532）。 */
  available_models: string[]
  api_key_ref: string | null
  local: boolean
}

/** AI 设置（T-531）。 */
export interface AiConfig {
  providers: AiProviderEntry[]
}

/** 使用统计设置（T-541；匿名上报 v0.5 仅占位，默认关）。 */
export interface StatsConfig {
  anonymous_upload: boolean
}

/** 编程能力设置（T-511 起；T-517 设置页）。 */
export interface CodingConfig {
  commands_enabled: boolean
  mixed_text_rules: boolean
  terminal_guard: boolean
}

/** 技术词库设置（T-521）。 */
export interface TechLexiconConfig {
  enabled_stacks: string[]
}

/** 专有名词设置（T-522）。 */
export interface ProperNounsConfig {
  enabled: boolean
}

/** 识别标点恢复设置（T-545）。 */
export interface PunctConfig {
  enabled: boolean
}

/** 应用配置根（get_config 返回完整镜像）。 */
export interface AppConfig {
  general: GeneralConfig
  hotkey: HotkeyConfig
  audio: AudioConfig
  output: OutputConfig
  dict: DictConfig
  asr: AsrConfig
  about: AboutConfig
  log: LogConfig
  scene: SceneConfig
  ai: AiConfig
  stats: StatsConfig
  coding: CodingConfig
}

export type ConfigSection = keyof AppConfig

/* ---- 使用统计（T-541 命令契约 stats_summary / stats_clear） ---- */

export type StatsRangeOption = 'today' | 'week' | 'month' | 'all'

/** 命名计数项（场景分布 / 口令 / 模板 / 词典命中）。 */
export interface NamedCount {
  name: string
  count: number
}

/** 单日趋势点。 */
export interface StatsDayPoint {
  date: string
  chars: number
  recordings: number
  recording_ms: number
}

/** 范围聚合总计。 */
export interface StatsTotals {
  recordings: number
  recording_ms: number
  chars: number
  degraded: number
  failed: number
  ai_uses: number
  command_uses: number
  template_uses: number
}

/** 统计查询结果（统计页整体数据源）。 */
export interface StatsSummary {
  totals: StatsTotals
  days: StatsDayPoint[]
  scenes: NamedCount[]
  commands: NamedCount[]
  templates: NamedCount[]
  dict_hits: NamedCount[]
}

/* ---- 音频设备（T-005 枚举命令 list_audio_devices 的返回元素，手工同步） ---- */

export interface AudioDeviceInfo {
  id: string
  name: string
  is_default: boolean
}

/* ---- 关于页（T-016 命令契约，后端实现见 T-016 SPEC） ---- */

export interface AppInfo {
  product_name: string
  version: string
  build_info: string
  config_dir: string
  data_dir: string
  log_dir: string
}

export type UpdateStatus = 'up_to_date' | 'available' | 'unavailable'

/** 更新检查结果（NFR-PR-06：请求只发送版本号 / OS / 架构，由后端保证）。 */
export interface UpdateCheckResult {
  status: UpdateStatus
  version: string | null
  notes: string | null
  error_code: string | null
}

export interface ExportLogsResult {
  path: string
  lines: number
}

export type LegalDocument = 'license' | 'disclaimer' | 'privacy' | 'third_party'

/* ---- 日志中心（T-018 命令契约，后端 services/log.rs） ---- */

/** log_query / log_export 查询条件（后端 LogQuery 序列化形态）。 */
export interface LogQuery {
  /** 关键词或正则（空 = 不过滤）。 */
  keyword: string | null
  /** 关键词按正则解释（默认 false = 关键词匹配）。 */
  regex: boolean
  /** 大小写敏感（默认 false）。 */
  case_sensitive: boolean
  /** 级别过滤（空数组 = 全部级别）。 */
  levels: string[]
  /** 起始时间（Unix 毫秒，闭区间）。 */
  from_millis: number | null
  /** 结束时间（Unix 毫秒，闭区间）。 */
  to_millis: number | null
  /** 单页条数（0 = 后端默认 200，最大 200）。 */
  limit: number
  /** 跳过前 offset 条匹配结果。 */
  offset: number
}

/** 日志条目（后端 LogEntry 序列化形态）。 */
export interface LogEntry {
  id: number
  ts_millis: number
  level: string
  target: string
  body: string
}

/** 查询结果页（后端 LogPage 序列化形态）。 */
export interface LogPage {
  entries: LogEntry[]
  has_more: boolean
}

/* ---- 事件载荷（tech/03 3.5；与 events.rs AppEvent 序列化形态同步） ---- */

/** config:changed 事件载荷：`{ type: "config_changed", section }`。 */
export interface ConfigChangedPayload {
  type: 'config_changed'
  section: ConfigSection
}

/** config:recovered 事件载荷：`{ type: "config_recovered", source }`。 */
export interface ConfigRecoveredPayload {
  type: 'config_recovered'
  source: 'backup' | 'default'
}

/** hotkey:error 事件载荷：`{ type: "hotkey_error", code, message }`。 */
export interface HotkeyErrorPayload {
  type: 'hotkey_error'
  code: string
  message: string
}

/* ---- 模型管理（T-006 命令契约，后端 services/asr.rs） ---- */

export type ModelTier = 'accuracy' | 'speed'

export type DownloadState = 'downloading' | 'completed' | 'failed' | 'cancelled'

export interface DownloadProgress {
  downloaded_bytes: number
  total_bytes: number
  /** 瞬时下载速度 bytes/sec（T-017）。 */
  bytes_per_sec: number
}

/** list_models 返回元素（后端 ModelInfo 序列化形态）。 */
export interface ModelInfo {
  id: string
  name: string
  languages: string
  tier: ModelTier
  size_bytes: number
  downloaded: boolean
  active: boolean
  /** 模型文件存在但哈希校验失败（损坏，引导重新下载）。 */
  invalid: boolean
  progress: DownloadProgress | null
  /** 生效的模型存储目录（T-017）。 */
  models_dir: string
}

/** model:progress 事件载荷：`{ type: "model_download_progress", model_id, status, ... }`。 */
export interface ModelDownloadProgressPayload {
  type: 'model_download_progress'
  model_id: string
  status: DownloadState
  downloaded_bytes: number
  total_bytes: number
  bytes_per_sec: number
}

/** asr:notice 事件载荷：`{ type: "asr_notice", code, message }`。 */
export interface AsrNoticePayload {
  type: 'asr_notice'
  code: string
  message: string
}

/* ---- 替换词典（T-011 命令契约，后端 services/dict.rs） ---- */

/** 替换规则（后端 ReplaceRule 序列化形态）。 */
export interface ReplaceRule {
  id: number
  /** 错误词（识别结果中期望被替换的文本）。 */
  wrong: string
  /** 正确词；空字符串表示删除该词。 */
  right: string
  enabled: boolean
}

/* ---- 技术词库（T-521 命令契约，后端 services/lexicon.rs） ---- */

/** 术语条目：标准写法 + 读音别名 / 常见误识别形。 */
export interface TechTerm {
  term: string
  aliases: string[]
}

/** tech_lexicon_stacks 返回元素（单个技术栈词库，只读内置数据）。 */
export interface TechStackLexicon {
  /** 栈标识（前端经 i18n coding.glossary.stacks.<id> 映射显示名）。 */
  id: string
  version: number
  terms: TechTerm[]
}

/* ---- 专有名词（T-522 命令契约，后端 services/proper_nouns.rs） ---- */

/** 专有名词条目（后端 ProperNoun 序列化形态）。 */
export interface ProperNoun {
  id: number
  /** 标准写法。 */
  term: string
  /** 读音别名（可多个；空数组 = 按标准写法匹配）。 */
  aliases: string[]
  /** 备注（可选）。 */
  note: string
  enabled: boolean
}

/** dict:changed 事件载荷：`{ type: "dict_changed" }`。 */
export interface DictChangedPayload {
  type: 'dict_changed'
}

/** recording:result 事件载荷（tech/05 5.5，tag=status）。 */
export type RecordingResultPayload =
  | { status: 'ok'; text: string }
  | { status: 'degraded'; reason: string }
  | { status: 'failed'; code: string; message: string }

/* ---- 录音状态事件（T-008/T-009/T-010 消费，后端 events.rs AppEvent 序列化形态） ---- */

export type RecordingStateName = 'idle' | 'warming' | 'recording' | 'processing' | 'cancelled'

/** recording:state 事件载荷：`{ type: "recording_state", state, mode }`。 */
export interface RecordingStatePayload {
  type: 'recording_state'
  state: RecordingStateName
  /** 开始录音时的触发模式（仅 Recording 状态携带）。 */
  mode: HotkeyMode | null
}

/** recording:level 事件载荷：`{ type: "recording_level", level }`（RMS 0~1）。 */
export interface RecordingLevelPayload {
  type: 'recording_level'
  level: number
}
