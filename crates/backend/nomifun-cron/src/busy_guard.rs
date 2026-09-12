use dashmap::{DashMap, mapref::entry::Entry};

use nomifun_common::ConversationId;

/// Presence-keyed busy set: a conversation id is in `busy` exactly while a cron
/// execution holds its permit. Dropping the permit removes the entry, so
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

    pub(crate) fn try_acquire(&self, conversation_id: &str) -> Option<CronBusyPermit<'_>> {
        ConversationId::try_from(conversation_id).ok()?;
        match self.busy.entry(conversation_id.to_owned()) {
            Entry::Occupied(_) => None,
            Entry::Vacant(entry) => {
                entry.insert(());
                Some(CronBusyPermit {
                    guard: self,
                    conversation_id: conversation_id.to_owned(),
                })
            }
        }
    }
}

/// Does not hold a DashMap lock across awaits; cancellation releases the entry.
pub(crate) struct CronBusyPermit<'a> {
    guard: &'a CronBusyGuard,
    conversation_id: String,
}

impl Drop for CronBusyPermit<'_> {
    fn drop(&mut self) {
        self.guard.busy.remove(&self.conversation_id);
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
    fn multiple_conversations_independent() {
        let guard = CronBusyGuard::new();
        let first = guard.try_acquire(CONVERSATION_1).unwrap();
        let second = guard.try_acquire(CONVERSATION_2).unwrap();
        assert!(guard.is_busy(CONVERSATION_1));
        assert!(guard.try_acquire(CONVERSATION_1).is_none());
        drop(first);
        assert!(guard.try_acquire(CONVERSATION_1).is_some());
        assert!(guard.is_busy(CONVERSATION_2));
        drop(second);
        assert!(guard.busy.is_empty());
        assert!(guard.try_acquire("invalid").is_none());
    }

    #[test]
    fn default_creates_empty_guard() {
        let guard = CronBusyGuard::default();
        assert!(!guard.is_busy(CONVERSATION_1));
    }
}
