//! Canonical AgentSession facts, projections, recovery, and deletion closure.
//!
//! The store attaches to the shared canonical Agent Store SQLite root and owns
//! Session, Turn, Event, Effect, Payload and Resource facts plus rebuildable projections.
//! It depends only on the frozen `nomifun-agent-contracts` vocabulary and
//! general-purpose infrastructure crates.

#![forbid(unsafe_code)]

mod checkpoint;
mod error;
mod projector;
mod registry;
mod store;
mod types;

pub use checkpoint::{evaluate_snapshot_compatibility, validate_checkpoint};
pub use error::SessionStoreError;
pub use store::{
    AgentSessionStore, MAX_EVENT_PAGE_SIZE, MAX_INLINE_JSON_BYTES, MAX_SESSION_PAYLOAD_BYTES,
    MAX_SINGLE_PAYLOAD_BYTES,
    NativeCheckpoint, NativeCheckpointWrite, MAX_NATIVE_CHECKPOINT_BYTES,
    NativeExecutionClaim, NativeExecutionLease, NATIVE_EXECUTION_LEASE_MS,
    NativeExecutionInspection, NATIVE_RECOVERY_BLOCKED,
    NativePauseState, NativeResumeRequest, NativeOwnerEvidence, NativeResumeReceipt, NativeResumePreparation,
    NativeVerifiedOutcome, NativeEffectReconciliationRequest, NativeEffectReconciliationCandidate, NativeEffectReconciliationCandidates,
};
pub use types::*;

#[cfg(test)]
mod tests;
