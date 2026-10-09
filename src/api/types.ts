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
export interface DictConfig {
  enabled: boolean
}

/** 识别引擎设置（T-006，F-SET-06 当前模型）。 */
export interface AsrConfig {
  /** 当前激活模型 ID（null = 自动选择首个可用模型）。 */
  current_model_id: string | null
}

/** 关于页设置（F-ABOUT-01）。 */
export interface AboutConfig {
  /** 自动检查更新（默认开，NFR-PR-06）。 */
  auto_check_update: boolean
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
}

export type ConfigSection = keyof AppConfig

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

export type LegalDocument = 'license' | 'disclaimer' | 'third_party'

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
}

/** model:progress 事件载荷：`{ type: "model_download_progress", model_id, status, ... }`。 */
export interface ModelDownloadProgressPayload {
  type: 'model_download_progress'
  model_id: string
  status: DownloadState
  downloaded_bytes: number
  total_bytes: number
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

/** dict:changed 事件载荷：`{ type: "dict_changed" }`。 */
export interface DictChangedPayload {
  type: 'dict_changed'
}

/** recording:result 事件载荷（tech/05 5.5，tag=status）。 */
export type RecordingResultPayload =
  | { status: 'ok'; text: string }
  | { status: 'degraded'; reason: string }
  | { status: 'failed'; code: string; message: string }
