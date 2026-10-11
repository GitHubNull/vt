//! 平台适配层：操作系统差异收敛在此（tech/04 4.5）。
//!
//! 每个能力一个 trait（如 `trait TextInjector`），`platform/mod.rs` 提供工厂函数，
//! 三平台用 `#[cfg(target_os)]` 条件编译提供实现，并附 mock 实现供服务层单测。
//! 子目录（hotkey / inject / audio / window_info / credentials）随对应任务创建；
//! 调用 OS API 的位置必须附最小权限注释（AGENTS.md 3.5）。

pub mod audio;
pub mod hotkey;
pub mod inject;
pub mod notify;
pub mod volume;
pub mod window_info;
