use dashmap::DashMap;

use nomifun_common::ConversationId;

/// Presence-keyed busy set: a conversation id is in `busy` exactly while a cron
/// execution is processing it. `set_processing(false)` removes the entry, so
/// idle conversations never accumulate state.
pub struct CronBusyGuard {
    busy: DashMap<String, ()>,
}

impl CronBusyGuard {
    pub fn new() -> Self {
        Self { busy: DashMap::new() }
    }

    pub fn is_busy(&self, conversation_id: &str) -> bool {
        if ConversationId::try_from(conversation_id).is_err() {
            return false;
        }
        self.busy.contains_key(conversation_id)
    }

    pub fn set_processing(&self, conversation_id: &str, processing: bool) {
        if ConversationId::try_from(conversation_id).is_err() {
            return;
        }
        if processing {
            self.busy.insert(conversation_id.to_owned(), ());
        } else {
            self.busy.remove(conversation_id);
        }
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

    const CONVERSATION_1: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const CONVERSATION_2: &str = "0190f5fe-7c00-7a00-8000-000000000002";

    #[test]
    fn new_conversation_is_not_busy() {
        let guard = CronBusyGuard::new();
        assert!(!guard.is_busy(CONVERSATION_1));
    }

    #[test]
    fn set_processing_true_marks_busy() {
        let guard = CronBusyGuard::new();
        guard.set_processing(CONVERSATION_1, true);
        assert!(guard.is_busy(CONVERSATION_1));
    }

    #[test]
    fn set_processing_false_marks_not_busy() {
        let guard = CronBusyGuard::new();
        guard.set_processing(CONVERSATION_1, true);
        guard.set_processing(CONVERSATION_1, false);
        assert!(!guard.is_busy(CONVERSATION_1));
    }

    #[test]
    fn set_processing_false_releases_the_entry() {
        let guard = CronBusyGuard::new();
        guard.set_processing(CONVERSATION_1, true);
        guard.set_processing(CONVERSATION_1, false);
        assert!(guard.busy.is_empty(), "idle conversations must not retain state");
    }

    #[test]
    fn multiple_conversations_independent() {
        let guard = CronBusyGuard::new();
        guard.set_processing(CONVERSATION_1, true);
        guard.set_processing(CONVERSATION_2, false);
        assert!(guard.is_busy(CONVERSATION_1));
        assert!(!guard.is_busy(CONVERSATION_2));
    }

    #[test]
    fn default_creates_empty_guard() {
        let guard = CronBusyGuard::default();
        assert!(!guard.is_busy(CONVERSATION_1));
    }
}
