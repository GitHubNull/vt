//! 服务层：业务逻辑与 tokio 异步任务（tech/04 4.1、4.4）。
//!
//! 各服务（AudioService / AsrService / HotkeyService / InjectService / DictService /
//! CommandEngine / AiPipeline / McpServer / DeviceHub / ConfigStore / LogService /
//! UpdateService）随对应任务在本目录补充。

pub mod ai;
pub mod asr;
pub mod audio;
pub mod command_engine;
pub mod config_store;
pub mod dict;
pub mod esc_cancel;
pub mod hotkey;
pub mod inject;
pub mod lexicon;
pub mod log;
pub mod model_download;
pub mod pipeline;
pub mod post_process;
pub mod proper_nouns;
pub mod punct;
pub mod scene;
pub mod session;
pub mod stages;
pub mod update;
pub mod usage_stats;
