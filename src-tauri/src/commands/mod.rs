//! 命令层：前端唯一入口（tech/04 4.3）。
//!
//! 约定：每个 `#[tauri::command]` 函数做参数校验并调用服务层，返回 `Result<T, AppError>`；
//! 命令按领域分文件（dict.rs / config.rs / ai.rs / device.rs 等，随任务补充）。

pub mod about;
pub mod audio;
pub mod config;
pub mod hotkey;
pub mod inject;
pub mod system;
pub mod window;
