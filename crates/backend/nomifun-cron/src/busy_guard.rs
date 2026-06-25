use dashmap::DashMap;

use nomifun_common::{TimestampMs, now_ms};

const IDLE_CLEANUP_THRESHOLD_MS: i64 = 3_600_000; // 1 hour

#[derive(Debug, Clone)]
struct ConversationState {
    is_processing: bool,
    last_active_at: TimestampMs,
}

pub struct CronBusyGuard {
    states: DashMap<String, ConversationState>,
}

impl CronBusyGuard {
    pub fn new() -> Self {
        Self { states: DashMap::new() }
    }

    pub fn is_busy(&self, conversation_id: &str) -> bool {
        self.states
            .get(conversation_id)
            .map(|s| s.is_processing)
            .unwrap_or(false)
    }

    pub fn set_processing(&self, conversation_id: &str, processing: bool) {
        let now = now_ms();
        self.states
            .entry(conversation_id.to_owned())
            .and_modify(|s| {
                s.is_processing = processing;
                s.last_active_at = now;
            })
            .or_insert(ConversationState {
                is_processing: processing,
                last_active_at: now,
            });
    }

    pub fn cleanup(&self) {
        let cutoff = now_ms() - IDLE_CLEANUP_THRESHOLD_MS;
        self.states
            .retain(|_, state| state.is_processing || state.last_active_at > cutoff);
    }

    pub fn active_count(&self) -> usize {
        self.states.iter().filter(|entry| entry.is_processing).count()
    }
}

impl Default for CronBusyGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_conversation_is_not_busy() {
        let guard = CronBusyGuard::new();
        assert!(!guard.is_busy("conv_1"));
    }

    #[test]
    fn set_processing_true_marks_busy() {
        let guard = CronBusyGuard::new();
        guard.set_processing("conv_1", true);
        assert!(guard.is_busy("conv_1"));
    }

    #[test]
    fn set_processing_false_marks_not_busy() {
        let guard = CronBusyGuard::new();
        guard.set_processing("conv_1", true);
        guard.set_processing("conv_1", false);
        assert!(!guard.is_busy("conv_1"));
    }

    #[test]
    fn multiple_conversations_independent() {
        let guard = CronBusyGuard::new();
        guard.set_processing("conv_1", true);
        guard.set_processing("conv_2", false);
        assert!(guard.is_busy("conv_1"));
        assert!(!guard.is_busy("conv_2"));
    }

    #[test]
    fn active_count_reflects_processing() {
        let guard = CronBusyGuard::new();
        assert_eq!(guard.active_count(), 0);
        guard.set_processing("conv_1", true);
        guard.set_processing("conv_2", true);
        assert_eq!(guard.active_count(), 2);
        guard.set_processing("conv_1", false);
        assert_eq!(guard.active_count(), 1);
    }

    #[test]
    fn cleanup_removes_idle_entries() {
        let guard = CronBusyGuard::new();
        // Insert a state with old timestamp
        guard.states.insert(
            "conv_old".to_owned(),
            ConversationState {
                is_processing: false,
                last_active_at: now_ms() - IDLE_CLEANUP_THRESHOLD_MS - 1000,
            },
        );
        // Insert a recent idle state
        guard.set_processing("conv_recent", false);

        guard.cleanup();

        assert!(guard.states.get("conv_old").is_none());
        assert!(guard.states.get("conv_recent").is_some());
    }

    #[test]
    fn cleanup_keeps_processing_entries_even_if_old() {
        let guard = CronBusyGuard::new();
        guard.states.insert(
            "conv_busy".to_owned(),
            ConversationState {
                is_processing: true,
                last_active_at: now_ms() - IDLE_CLEANUP_THRESHOLD_MS - 1000,
            },
        );

        guard.cleanup();

        assert!(guard.states.get("conv_busy").is_some());
    }

    #[test]
    fn default_creates_empty_guard() {
        let guard = CronBusyGuard::default();
        assert_eq!(guard.active_count(), 0);
    }
}
