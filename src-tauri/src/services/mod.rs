//! 服务层：业务逻辑与 tokio 异步任务（tech/04 4.1、4.4）。
//!
//! 各服务（AudioService / AsrService / HotkeyService / InjectService / DictService /
//! CommandEngine / AiPipeline / McpServer / DeviceHub / ConfigStore / LogService /
//! UpdateService）随对应任务在本目录补充。

pub mod asr;
pub mod audio;
pub mod config_store;
pub mod dict;
pub mod esc_cancel;
pub mod hotkey;
pub mod inject;
pub mod log;
pub mod pipeline;
pub mod post_process;
pub mod session;
pub mod update;
