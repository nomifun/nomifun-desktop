//! Per-conversation store of a channel-owned stop confirmation awaiting a
//! numbered reply from a channel user.
//!
//! When a remote stop is denied by the Channel surface, the relay records the
//! confirmation here and forwards a numbered text list to the channel. The
//! message loop maps the user's numeric reply onto the stop or cancel option.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::types::ChannelStopOption;

/// The only operation represented by a channel-owned stop confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelStopConfirmationKind {
    /// The companion's remote stop was denied on the Channel surface. The
    /// channel asks its user directly, then performs an owner-scoped stop or
    /// clears the request. This is ordinary channel product behavior, not an
    /// Agent approval/confirmation lifecycle.
    StopConversation {
        target_conversation_id: String,
    },
}

/// A channel-owned remote-stop confirmation awaiting a numbered reply.
///
/// `prompt` is retained so a non-numeric reply can re-render the same
/// numbered list without re-deriving it from the agent stream.
#[derive(Debug, Clone)]
pub struct ChannelStopConfirmation {
    pub conversation_id: String,
    pub kind: ChannelStopConfirmationKind,
    pub prompt: String,
    pub options: Vec<ChannelStopOption>,
}

impl ChannelStopConfirmation {
    pub fn new(
        conversation_id: impl Into<String>,
        target_conversation_id: impl Into<String>,
        prompt: impl Into<String>,
        options: Vec<ChannelStopOption>,
    ) -> Self {
        Self {
            conversation_id: conversation_id.into(),
            kind: ChannelStopConfirmationKind::StopConversation {
                target_conversation_id: target_conversation_id.into(),
            },
            prompt: prompt.into(),
            options,
        }
    }
}

/// Concurrent store of channel-owned stop confirmations keyed by conversation
/// id.
///
/// At most one decision is outstanding per conversation (a new decision for
/// the same conversation overwrites the previous one). Shared by the relay
/// (writer) and the message loop + message service (reader / clearer).
#[derive(Default)]
pub struct ChannelStopConfirmationStore {
    inner: Mutex<HashMap<String, ChannelStopConfirmation>>,
}

impl ChannelStopConfirmationStore {
    /// Creates an empty store behind an `Arc` for sharing across the relay,
    /// message loop, and message service.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Records (or overwrites) the stop confirmation for its conversation.
    pub fn put(&self, confirmation: ChannelStopConfirmation) {
        self.inner
            .lock()
            .unwrap()
            .insert(confirmation.conversation_id.clone(), confirmation);
    }

    /// Returns a clone of the stop confirmation for a conversation, if any,
    /// without removing it.
    pub fn peek(&self, conversation_id: &str) -> Option<ChannelStopConfirmation> {
        self.inner.lock().unwrap().get(conversation_id).cloned()
    }

    /// Removes and returns the stop confirmation for a conversation, if any.
    pub fn take(&self, conversation_id: &str) -> Option<ChannelStopConfirmation> {
        self.inner.lock().unwrap().remove(conversation_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(option_id: &str, label: &str) -> ChannelStopOption {
        ChannelStopOption {
            option_id: option_id.into(),
            label: label.into(),
        }
    }

    fn confirmation(
        conversation_id: &str,
        target_conversation_id: &str,
    ) -> ChannelStopConfirmation {
        ChannelStopConfirmation::new(
            conversation_id,
            target_conversation_id,
            "Proceed?",
            vec![opt("a", "Allow"), opt("b", "Deny")],
        )
    }

    #[test]
    fn put_peek_take_round_trip() {
        let store = ChannelStopConfirmationStore::new();
        assert!(store.peek("conv-1").is_none());

        store.put(confirmation("conv-1", "call-1"));

        let peeked = store.peek("conv-1").expect("peek should see the put");
        assert_eq!(
            peeked.kind,
            ChannelStopConfirmationKind::StopConversation {
                target_conversation_id: "call-1".into(),
            }
        );
        assert_eq!(peeked.prompt, "Proceed?");
        assert_eq!(peeked.options.len(), 2);
        // peek does not consume.
        assert!(store.peek("conv-1").is_some());

        let taken = store.take("conv-1").expect("take should return the entry");
        assert_eq!(
            taken.kind,
            ChannelStopConfirmationKind::StopConversation {
                target_conversation_id: "call-1".into(),
            }
        );
        // take consumes.
        assert!(store.peek("conv-1").is_none());
        assert!(store.take("conv-1").is_none());
    }

    #[test]
    fn put_overwrites_by_conversation() {
        let store = ChannelStopConfirmationStore::new();
        store.put(confirmation("conv-1", "call-1"));
        store.put(confirmation("conv-1", "call-2"));

        let peeked = store.peek("conv-1").unwrap();
        assert_eq!(
            peeked.kind,
            ChannelStopConfirmationKind::StopConversation {
                target_conversation_id: "call-2".into(),
            },
            "latest put wins per conversation"
        );

        // Distinct conversations are independent.
        store.put(confirmation("conv-2", "call-x"));
        assert_eq!(
            store.peek("conv-1").unwrap().kind,
            ChannelStopConfirmationKind::StopConversation {
                target_conversation_id: "call-2".into(),
            }
        );
        assert_eq!(
            store.peek("conv-2").unwrap().kind,
            ChannelStopConfirmationKind::StopConversation {
                target_conversation_id: "call-x".into(),
            }
        );
    }
}
