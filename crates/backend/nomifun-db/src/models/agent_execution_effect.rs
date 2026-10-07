//! Durable internal effects for one Agent attempt conversation.
//!
//! The attempt's `runtime_state` is the write-ahead intent.  External
//! Conversation delivery happens only after this value and its audit event
//! commit; successful settlement clears the state atomically with the attempt.

use nomifun_common::AppError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PendingConversationEffect {
    StopTurn {
        operation_id: String,
        target_operation_id: String,
    },
    DecisionInput {
        operation_id: String,
        content: String,
    },
    Steer {
        operation_id: String,
        target_operation_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptConversationEffects {
    pub pending_conversation_effects: Vec<PendingConversationEffect>,
    /// Fail-closed recovery marker written by the repository when a formerly
    /// Running invocation cannot prove that its external effect was untouched.
    /// There is intentionally no automatic resolution path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_blocked: Option<RecoveryReviewBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryReviewBlock {
    pub kind: String,
    pub operation_id: String,
    pub receipt_state: String,
    pub reason: String,
}

impl AttemptConversationEffects {
    /// The current invocation identity is either its initial key or the one
    /// durable decision continuation. Never select an unrelated latest Turn.
    pub fn current_turn_operation_key(&self, attempt_id: &str) -> Result<String, AppError> {
        let decisions = self.pending_conversation_effects.iter().filter_map(|effect| match effect {
            PendingConversationEffect::DecisionInput { operation_id, .. } => Some(operation_id),
            _ => None,
        }).collect::<Vec<_>>();
        match decisions.as_slice() {
            [] => Ok(format!("{attempt_id}:initial-turn")),
            [operation] if !operation.trim().is_empty() => Ok((*operation).clone()),
            _ => Err(AppError::Conflict("attempt has ambiguous decision continuation identity".to_owned())),
        }
    }
    pub fn push_stop_turn(&mut self, operation_id: String, target_operation_id: String) -> Result<(), AppError> {
        if !self.pending_conversation_effects.is_empty() {
            return Err(AppError::Conflict(
                "a conversation effect is already pending for this attempt".to_owned(),
            ));
        }
        self.pending_conversation_effects
            .push(PendingConversationEffect::StopTurn { operation_id, target_operation_id });
        Ok(())
    }

    pub fn push_steer(&mut self, operation_id: String, target_operation_id: String, content: String) -> Result<(), AppError> {
        if !self.pending_conversation_effects.is_empty() {
            return Err(AppError::Conflict(
                "a conversation effect is already pending for this attempt".to_owned(),
            ));
        }
        self.pending_conversation_effects
            .push(PendingConversationEffect::Steer {
                operation_id,
                target_operation_id,
                content,
            });
        Ok(())
    }

    pub fn push_decision(
        &mut self,
        operation_id: String,
        content: String,
    ) -> Result<(), AppError> {
        if self.review_blocked.is_some() {
            return Err(AppError::Conflict(
                "this interrupted Agent turn is blocked for manual review and cannot be resumed automatically"
                    .to_owned(),
            ));
        }
        if self
            .pending_conversation_effects
            .iter()
            .any(|effect| matches!(effect, PendingConversationEffect::DecisionInput { .. }))
        {
            return Err(AppError::Conflict(
                "a decision continuation is already pending for this attempt".to_owned(),
            ));
        }
        self.pending_conversation_effects
            .push(PendingConversationEffect::DecisionInput {
                operation_id,
                content,
            });
        Ok(())
    }

    pub fn decode(raw: Option<&str>) -> Result<Self, AppError> {
        match raw {
            Some(raw) => serde_json::from_str(raw).map_err(|error| {
                AppError::Internal(format!(
                    "invalid persisted attempt conversation effects: {error}"
                ))
            }),
            None => Ok(Self::default()),
        }
    }

    pub fn encode(&self) -> Result<String, AppError> {
        serde_json::to_string(self).map_err(|error| {
            AppError::Internal(format!("encode attempt conversation effects: {error}"))
        })
    }
}
