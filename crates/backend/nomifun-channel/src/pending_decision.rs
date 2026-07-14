//! Per-conversation store of the blocking decision currently awaiting a
//! numbered reply from a channel user.
//!
//! When a remote-channel-driven conversation hits a blocking decision (the
//! agent asks the user to choose / grant permission), the relay records the
//! pending decision here and forwards a numbered text list to the channel.
//! The message loop's inbound interception reads it back to map the user's
//! numeric reply onto an option, then clears it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::types::DecisionOption;

/// A blocking decision awaiting the channel user's numbered reply.
///
/// `prompt` is retained so a non-numeric reply can re-render the same
/// numbered list without re-deriving it from the agent stream.
#[derive(Debug, Clone)]
pub struct PendingDecision {
    pub conversation_id: String,
    pub call_id: String,
    pub prompt: String,
    pub options: Vec<DecisionOption>,
}

/// Concurrent store of pending decisions keyed by conversation id.
///
/// At most one decision is outstanding per conversation (a new decision for
/// the same conversation overwrites the previous one). Shared by the relay
/// (writer) and the message loop + message service (reader / clearer).
#[derive(Default)]
pub struct PendingDecisionStore {
    inner: Mutex<HashMap<String, PendingDecision>>,
}

impl PendingDecisionStore {
    /// Creates an empty store behind an `Arc` for sharing across the relay,
    /// message loop, and message service.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Records (or overwrites) the pending decision for its conversation.
    pub fn put(&self, decision: PendingDecision) {
        self.inner
            .lock()
            .unwrap()
            .insert(decision.conversation_id.clone(), decision);
    }

    /// Returns a clone of the pending decision for a conversation, if any,
    /// without removing it.
    pub fn peek(&self, conversation_id: &str) -> Option<PendingDecision> {
        self.inner.lock().unwrap().get(conversation_id).cloned()
    }

    /// Removes and returns the pending decision for a conversation, if any.
    pub fn take(&self, conversation_id: &str) -> Option<PendingDecision> {
        self.inner.lock().unwrap().remove(conversation_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(option_id: &str, label: &str) -> DecisionOption {
        DecisionOption {
            option_id: option_id.into(),
            label: label.into(),
        }
    }

    fn decision(conversation_id: &str, call_id: &str) -> PendingDecision {
        PendingDecision {
            conversation_id: conversation_id.into(),
            call_id: call_id.into(),
            prompt: "Proceed?".into(),
            options: vec![opt("a", "Allow"), opt("b", "Deny")],
        }
    }

    #[test]
    fn put_peek_take_round_trip() {
        let store = PendingDecisionStore::new();
        assert!(store.peek("conv-1").is_none());

        store.put(decision("conv-1", "call-1"));

        let peeked = store.peek("conv-1").expect("peek should see the put");
        assert_eq!(peeked.call_id, "call-1");
        assert_eq!(peeked.prompt, "Proceed?");
        assert_eq!(peeked.options.len(), 2);
        // peek does not consume.
        assert!(store.peek("conv-1").is_some());

        let taken = store.take("conv-1").expect("take should return the entry");
        assert_eq!(taken.call_id, "call-1");
        // take consumes.
        assert!(store.peek("conv-1").is_none());
        assert!(store.take("conv-1").is_none());
    }

    #[test]
    fn put_overwrites_by_conversation() {
        let store = PendingDecisionStore::new();
        store.put(decision("conv-1", "call-1"));
        store.put(decision("conv-1", "call-2"));

        let peeked = store.peek("conv-1").unwrap();
        assert_eq!(peeked.call_id, "call-2", "latest put wins per conversation");

        // Distinct conversations are independent.
        store.put(decision("conv-2", "call-x"));
        assert_eq!(store.peek("conv-1").unwrap().call_id, "call-2");
        assert_eq!(store.peek("conv-2").unwrap().call_id, "call-x");
    }
}
