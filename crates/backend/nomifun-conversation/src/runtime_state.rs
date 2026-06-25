use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use nomifun_api_types::{ConversationRuntimeStateKind, ConversationRuntimeSummary};
use nomifun_common::{AppError, ConversationStatus, now_ms};
use tracing::{info, warn};

#[derive(Debug, Default)]
pub struct ConversationRuntimeStateService {
    /// Conversations with a live turn claim, mapped to the wall-clock time
    /// (epoch ms) the claim was taken. The timestamp is surfaced in
    /// `ConversationRuntimeSummary::processing_started_at` so the frontend's
    /// elapsed-time indicator can anchor to the real turn start and survive
    /// view unmount/remount instead of restarting from zero.
    active_turns: Mutex<HashMap<String, i64>>,
    /// Per-conversation signature of the knowledge mounts the live agent task
    /// was last built with. The agent bakes the knowledge retrieval-protocol
    /// section at build time and is cached per conversation, so a binding
    /// toggled mid-session does not reach the already-running agent.
    /// `apply_knowledge_mounts` compares the freshly-resolved signature against
    /// this map to decide whether to recycle the cached task. In-memory only
    /// (cleared on restart), which is intentional: after a restart the task map
    /// is empty too, so the first build naturally carries the current mounts.
    knowledge_signatures: Mutex<HashMap<String, String>>,
}

#[derive(Debug)]
pub struct TurnClaim {
    conversation_id: String,
    state: Weak<ConversationRuntimeStateService>,
    released: bool,
}

impl ConversationRuntimeStateService {
    pub fn try_claim_turn(self: &Arc<Self>, conversation_id: &str) -> Result<TurnClaim, AppError> {
        let mut active_turns = self.active_turns.lock().map_err(|_| {
            warn!(
                conversation_id,
                "conversation runtime state lock poisoned while claiming turn"
            );
            AppError::Internal("conversation runtime state lock poisoned".into())
        })?;

        if active_turns.contains_key(conversation_id) {
            info!(conversation_id, "conversation runtime turn claim rejected");
            return Err(AppError::Conflict(format!(
                "conversation {conversation_id} is already running"
            )));
        }

        active_turns.insert(conversation_id.to_owned(), now_ms());

        info!(conversation_id, "conversation runtime turn claimed");

        Ok(TurnClaim {
            conversation_id: conversation_id.to_owned(),
            state: Arc::downgrade(self),
            released: false,
        })
    }

    pub fn is_claimed(&self, conversation_id: &str) -> bool {
        self.active_turns
            .lock()
            .map(|active_turns| active_turns.contains_key(conversation_id))
            .unwrap_or(false)
    }

    /// Wall-clock time (epoch ms) the live turn for `conversation_id` was
    /// claimed, if one is active. `None` when no turn is in flight.
    pub fn claimed_at(&self, conversation_id: &str) -> Option<i64> {
        self.active_turns
            .lock()
            .ok()
            .and_then(|active_turns| active_turns.get(conversation_id).copied())
    }

    /// The knowledge-mount signature the live agent for `conversation_id` was
    /// last built with, if recorded. `None` means no build has been observed
    /// (e.g. after a restart, or a conversation never started) — callers treat
    /// that as "no live agent to reconcile against".
    pub fn knowledge_signature(&self, conversation_id: &str) -> Option<String> {
        self.knowledge_signatures
            .lock()
            .ok()
            .and_then(|sigs| sigs.get(conversation_id).cloned())
    }

    /// Record the knowledge-mount signature the agent for `conversation_id` was
    /// (re)built with. Called right after `apply_knowledge_mounts` resolves the
    /// mounts for an upcoming build so the NEXT binding change is detectable.
    pub fn set_knowledge_signature(&self, conversation_id: &str, signature: String) {
        if let Ok(mut sigs) = self.knowledge_signatures.lock() {
            sigs.insert(conversation_id.to_owned(), signature);
        }
    }

    /// Drop a conversation's recorded knowledge signature (on delete) so the
    /// map does not grow unbounded across a long-lived process.
    pub fn clear_knowledge_signature(&self, conversation_id: &str) {
        if let Ok(mut sigs) = self.knowledge_signatures.lock() {
            sigs.remove(conversation_id);
        }
    }

    pub fn summary_from_parts(
        &self,
        conversation_id: &str,
        task_status: Option<ConversationStatus>,
        has_task: bool,
        pending_confirmations: usize,
    ) -> ConversationRuntimeSummary {
        let claimed_at = self.claimed_at(conversation_id);
        let claimed = claimed_at.is_some();

        let state = if pending_confirmations > 0 {
            ConversationRuntimeStateKind::WaitingConfirmation
        } else if claimed && task_status != Some(ConversationStatus::Running) {
            ConversationRuntimeStateKind::Starting
        } else if claimed || task_status == Some(ConversationStatus::Running) {
            ConversationRuntimeStateKind::Running
        } else {
            ConversationRuntimeStateKind::Idle
        };

        let is_processing = state != ConversationRuntimeStateKind::Idle;

        ConversationRuntimeSummary {
            state,
            can_send_message: !is_processing,
            has_task,
            task_status,
            is_processing,
            pending_confirmations,
            // Only surface a start time while actually processing. When the
            // turn is driven purely by a persisted Running status (no live
            // claim, e.g. an edge case after restart), `claimed_at` is None and
            // the frontend gracefully falls back to its local mount time.
            processing_started_at: if is_processing { claimed_at } else { None },
        }
    }

    fn release(&self, conversation_id: &str) {
        match self.active_turns.lock() {
            Ok(mut active_turns) => {
                active_turns.remove(conversation_id);
                info!(conversation_id, "conversation runtime turn claim released");
            }
            Err(_) => {
                warn!(
                    conversation_id,
                    "conversation runtime state lock poisoned while releasing turn"
                );
            }
        }
    }
}

impl TurnClaim {
    pub fn release(&mut self) {
        self.release_inner();
    }

    fn release_inner(&mut self) {
        if self.released {
            return;
        }

        if let Some(state) = self.state.upgrade() {
            state.release(&self.conversation_id);
        }
        self.released = true;
    }
}

impl Drop for TurnClaim {
    fn drop(&mut self) {
        self.release_inner();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn claim_rejects_second_active_turn() {
        let state = Arc::new(ConversationRuntimeStateService::default());
        let _claim = state.try_claim_turn("conv-1").expect("first claim should win");

        let err = state.try_claim_turn("conv-1").expect_err("second claim should fail");
        assert!(err.to_string().contains("already running"));
    }

    #[test]
    fn claim_releases_on_drop() {
        let state = Arc::new(ConversationRuntimeStateService::default());
        {
            let _claim = state.try_claim_turn("conv-1").expect("claim should be created");
            assert!(state.is_claimed("conv-1"));
        }

        assert!(!state.is_claimed("conv-1"));
        assert!(state.try_claim_turn("conv-1").is_ok());
    }

    #[test]
    fn summary_uses_claim_as_starting_state() {
        let state = Arc::new(ConversationRuntimeStateService::default());
        let _claim = state.try_claim_turn("conv-1").expect("claim should be created");

        let summary = state.summary_from_parts("conv-1", None, false, 0);

        assert_eq!(summary.state, ConversationRuntimeStateKind::Starting);
        assert!(summary.is_processing);
        assert!(!summary.can_send_message);
        assert!(
            summary.processing_started_at.is_some(),
            "a claimed turn must expose its start time"
        );
    }

    #[test]
    fn summary_exposes_claim_time_and_clears_when_idle() {
        let state = Arc::new(ConversationRuntimeStateService::default());

        // Idle: no claim, no start time.
        let idle = state.summary_from_parts("conv-1", None, false, 0);
        assert_eq!(idle.state, ConversationRuntimeStateKind::Idle);
        assert!(!idle.is_processing);
        assert_eq!(idle.processing_started_at, None);

        // Claimed: start time matches the recorded claim time.
        let claim = state.try_claim_turn("conv-1").expect("claim should be created");
        let expected = state.claimed_at("conv-1");
        assert!(expected.is_some());

        let running = state.summary_from_parts("conv-1", None, false, 0);
        assert!(running.is_processing);
        assert_eq!(running.processing_started_at, expected);

        // Released: back to idle, start time gone.
        drop(claim);
        let after = state.summary_from_parts("conv-1", None, false, 0);
        assert!(!after.is_processing);
        assert_eq!(after.processing_started_at, None);
    }
}
