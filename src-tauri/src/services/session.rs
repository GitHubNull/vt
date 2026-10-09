//! 录音会话追踪器（T-010）：会话代次 + 取消标记，贯穿 采集 → 识别 → 上屏链路。
//!
//! - `begin`：新录音会话开始（AudioService Start）时代次 +1，记为当前会话；
//! - `cancel`：会话被取消（AudioService Cancel，含识别在途的竞态取消），
//!   在途识别结果到达时按代次 / 取消标记丢弃；
//! - `is_valid`：上屏前最后检查（AsrService 结果回调、PostProcess 管线出口，
//!   T-012 上屏消费前再次确认）。
//!
//! 竞态场景（tech/05 5.2、T-010 SPEC）：
//! - Stop 与 Cancel 几乎同时到达：音频已交付识别，`cancel` 把会话标记为取消，
//!   结果到达时 `is_valid == false` → 丢弃不上屏；
//! - 会话 N 识别期间开始了会话 N+1：代次前进，N 的结果过期丢弃（单会话语义）。
//!
//! 隐私（NFR-S-02）：本结构只存会话编号，不含任何音频或文本内容。

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// 录音会话代次追踪器：识别结果过期丢弃与"上屏前最后检查"的权威状态。
pub struct SessionTracker {
    /// 会话代次：每次 begin 递增（识别结果回调按代次判过期）。
    generation: AtomicU64,
    /// 当前代次对应的会话 ID（u64::MAX = 尚无会话，真实会话号不会取该值）。
    current_session: AtomicU64,
    /// 已取消的会话集合（识别中取消的竞态窗口；u64 会话号，量级可忽略）。
    cancelled_sessions: Mutex<HashSet<u64>>,
}

impl Default for SessionTracker {
    fn default() -> Self {
        Self {
            generation: AtomicU64::new(0),
            current_session: AtomicU64::new(u64::MAX),
            cancelled_sessions: Mutex::new(HashSet::new()),
        }
    }
}

impl SessionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// 新会话开始：代次 +1 并记为当前会话；清除该 id 的历史取消标记（防御复用）。
    pub fn begin(&self, session_id: u64) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.current_session.store(session_id, Ordering::SeqCst);
        if let Ok(mut cancelled) = self.cancelled_sessions.lock() {
            cancelled.remove(&session_id);
        }
    }

    /// 标记会话取消；返回 false 表示会话已过期或已被取消（幂等）。
    ///
    /// 仅在会话仍为当前代次时生效——过期会话的取消没有意义（其结果本就会被丢弃）。
    pub fn cancel(&self, session_id: u64) -> bool {
        if !self.is_current(session_id) {
            return false;
        }
        match self.cancelled_sessions.lock() {
            Ok(mut cancelled) => cancelled.insert(session_id),
            Err(_) => false,
        }
    }

    /// 当前会话代次（begin 次数）。
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// 会话当前有效：是当前代次且未被取消（上屏前最后检查）。
    pub fn is_valid(&self, session_id: u64) -> bool {
        self.is_current(session_id)
            && !self
                .cancelled_sessions
                .lock()
                .map(|cancelled| cancelled.contains(&session_id))
                .unwrap_or(false)
    }

    fn is_current(&self, session_id: u64) -> bool {
        self.current_session.load(Ordering::SeqCst) == session_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_tracker_has_no_valid_session() {
        let tracker = SessionTracker::new();
        assert_eq!(tracker.generation(), 0);
        // 尚无会话：任何会话号都无效。
        assert!(!tracker.is_valid(0));
        assert!(!tracker.is_valid(1));
        assert!(!tracker.cancel(1));
    }

    #[test]
    fn begun_session_is_valid_and_generations_increment() {
        let tracker = SessionTracker::new();
        tracker.begin(1);
        assert_eq!(tracker.generation(), 1);
        assert!(tracker.is_valid(1));
        tracker.begin(2);
        assert_eq!(tracker.generation(), 2);
        assert!(tracker.is_valid(2));
    }

    #[test]
    fn cancel_invalidates_session_and_is_idempotent() {
        let tracker = SessionTracker::new();
        tracker.begin(1);
        assert!(tracker.cancel(1));
        assert!(!tracker.is_valid(1), "取消后会话应失效");
        assert!(!tracker.cancel(1), "重复取消返回 false（幂等）");
        assert!(!tracker.is_valid(1));
    }

    #[test]
    fn stale_session_becomes_invalid_when_new_session_begins() {
        let tracker = SessionTracker::new();
        tracker.begin(1);
        tracker.begin(2);
        // 代次过期丢弃：会话 1 的结果不得再上屏（单会话语义）。
        assert!(!tracker.is_valid(1));
        assert!(tracker.is_valid(2));
        // 过期会话的取消无意义（结果本就会被丢弃）。
        assert!(!tracker.cancel(1));
    }

    #[test]
    fn cancel_unknown_or_future_session_is_noop() {
        let tracker = SessionTracker::new();
        tracker.begin(1);
        assert!(!tracker.cancel(2));
        assert!(tracker.is_valid(1), "未受影响的会话保持有效");
    }

    #[test]
    fn begin_clears_stale_cancel_mark() {
        let tracker = SessionTracker::new();
        tracker.begin(1);
        tracker.cancel(1);
        // 防御：id 复用时（实际不会发生）取消标记不残留。
        tracker.begin(1);
        assert!(tracker.is_valid(1));
    }
}
