//! Canonical AgentSession facts, projections, recovery, and deletion closure.
//!
//! The store attaches to the shared canonical Agent Store SQLite root and owns
//! Session, Turn, Event, Effect, Payload and Resource facts plus rebuildable projections.
//! It uses the frozen `nomifun-agent-contracts` vocabulary and the existing
//! database transaction guards for linked Execution admission. Neither side
//! copies the other's execution facts or creates another runtime loop.

#![forbid(unsafe_code)]

mod snapshot_compatibility;
mod context_snapshot;
mod error;
mod history_tool_projection;
mod projector;
mod registry;
mod store;
mod types;
mod turn_outcome;
pub use turn_outcome::{CanonicalTurnPublicResult, canonical_turn_outcome, canonical_turn_public_result};

pub use snapshot_compatibility::evaluate_snapshot_compatibility;
pub use context_snapshot::{CanonicalContextMessage, CanonicalContextRole, ForkContextSnapshot, canonical_context_messages};
pub use error::SessionStoreError;
pub use store::{
    ChannelInputDeliveryDisposition,RecoverableChannelInput,
    AgentSessionStore, PendingSessionInput, SessionInputClaim, SessionInputBlockedReason, canonical_steering_input, is_session_input_event, session_input_namespace, MAX_EVENT_PAGE_SIZE, MAX_INLINE_JSON_BYTES, MAX_SESSION_PAYLOAD_BYTES,
    MAX_SINGLE_PAYLOAD_BYTES,
    NativeCheckpoint, NativeCheckpointWrite, MAX_NATIVE_CHECKPOINT_BYTES,
    RuntimeStateObservation,
    ClearSessionContext, ContextClearBoundary, ContextClearPreparation,
    KnowledgeWritebackStartRequest, KnowledgeWritebackStartReceipt,
    NativeTurnMutationFence,NativeInputContextFence,
    NativeExecutionClaim, NativeExecutionLease, NATIVE_EXECUTION_LEASE_MS,
    CommittedNativeToolCallCausation, CommittedToolCallCausation, NativeExecutionScope,
    NativeExecutionInspection, NATIVE_RECOVERY_BLOCKED,
    NativePauseState, NativeResumeRequest, NativeOwnerEvidence, NativeResumeReceipt, NativeResumePreparation,
    NativeResumeRequestReceipt, PendingNativeResumeRequest, NativeResumeCommandReceipt,
    NativeVerifiedOutcome, NativeEffectReconciliationRequest, NativeEffectReconciliationCandidate, NativeEffectReconciliationCandidates,
};
pub use types::*;

#[cfg(test)]
mod tests;
