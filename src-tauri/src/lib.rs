//! VibeType 后端库目标：供 `main.rs` 与集成测试（tests/）共享模块。
//!
//! 模块划分见 tech/04 4.1：commands（命令层）/ services（服务层）/ platform（平台适配层）/
//! events（事件广播）/ state（服务句柄聚合）/ tray / windows / error。

pub mod commands;
pub mod error;
pub mod events;
pub mod platform;
pub mod services;
pub mod state;
pub mod tray;
pub mod windows;
