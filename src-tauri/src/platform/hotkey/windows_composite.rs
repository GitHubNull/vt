//! Windows 组合后端：修饰键单独作触发键走低级钩子（RegisterHotKey 不覆盖修饰键），
//! 其余串（组合键 / 功能键）走 global-shortcut 插件；两路径互斥，切换时清理另一侧。

use tokio::sync::mpsc;

use super::modifier_key::modifier_trigger_vk;
use super::windows_llhook::LlHookBackend;
use super::{HotkeyBackend, HotkeyError, RawHotkeyEvent, TauriHotkeyBackend};

pub struct WindowsCompositeBackend {
    tauri: TauriHotkeyBackend,
    llhook: LlHookBackend,
    ll_events: mpsc::Sender<RawHotkeyEvent>,
}

impl WindowsCompositeBackend {
    /// `tauri` 为插件侧后端（组合键 / 功能键路径）；`ll_events` 为低级钩子的
    /// 原始按键事件出口（与主录音控制通道同一个 mpsc）。
    pub fn new(tauri: TauriHotkeyBackend, ll_events: mpsc::Sender<RawHotkeyEvent>) -> Self {
        Self {
            tauri,
            llhook: LlHookBackend::new(),
            ll_events,
        }
    }
}

impl HotkeyBackend for WindowsCompositeBackend {
    fn register(&self, accelerator: &str) -> Result<(), HotkeyError> {
        match modifier_trigger_vk(accelerator) {
            Some(vk) => {
                // 切到钩子路径前清掉插件侧可能残留的组合键注册（空注册时插件返回 Ok）。
                if let Err(err) = self.tauri.unregister_all() {
                    tracing::warn!(error = %err, "切换钩子路径前清理插件注册失败");
                }
                self.llhook.start(vk, self.ll_events.clone())
            }
            None => {
                self.llhook.stop();
                self.tauri.register(accelerator)
            }
        }
    }

    fn unregister(&self, accelerator: &str) -> Result<(), HotkeyError> {
        match modifier_trigger_vk(accelerator) {
            Some(_) => {
                self.llhook.stop();
                Ok(())
            }
            None => self.tauri.unregister(accelerator),
        }
    }

    fn unregister_all(&self) -> Result<(), HotkeyError> {
        self.llhook.stop();
        self.tauri.unregister_all()
    }
}
