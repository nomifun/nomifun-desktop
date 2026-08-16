//! Persisted exact terminal-proof protocol for restart-orphan Conversations.
//!
//! A durable `running` row whose owning process died must stay quarantined
//! until something proves the prior generation can no longer produce effects.
//! This module names the seam through which that proof arrives: the host
//! (nomifun-app) constructs a provider at boot from evidence it alone owns —
//! the exclusive data-dir server lock, the boot-frozen unsettled-admission
//! snapshot, and the verified reaping of the durable agent-process registry —
//! and installs it on `ConversationService`.  Without a provider every seam
//! keeps today's fail-closed behavior.

use async_trait::async_trait;

/// Which persisted evidence source must vouch for one orphaned generation.
///
/// Mirrors `RunningOrphanDisposition` for the provable backends; the
/// A disposition with no local evidence source never reaches a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrphanProofRequirement {
    /// In-process turn owner with audited parent-death containment for every
    /// effect-bearing child: proof = boot-lock authority + no surviving
    /// durable registry entry for the Conversation.
    LocalContainedAuthority,
    /// Effect-bearing child registered durably at spawn: proof = verified
    /// reaping of the Conversation's registry entries (absence is proof).
    RegisteredLocalProcessTree,
}

/// Outcome of one terminal-proof evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalProofDecision {
    /// The prior generation is provably terminal; `evidence` names the exact
    /// proof chain for the recovery log.
    Proven { evidence: String },
    /// No persisted proof covers this generation; the Conversation must stay
    /// quarantined. `reason` names the missing link for diagnostics.
    Unproven { reason: String },
}

/// Boot-scoped authority that can prove a prior process's turn generation
/// terminal.
///
/// Contract for implementations:
/// - Only generations frozen in the boot-time unsettled snapshot may prove:
///   the caller passes the admission it re-read under the per-Conversation
///   preparation gate, and the provider must reject any (epoch, operation)
///   that does not exactly match its snapshot — a mismatch means the
///   generation was admitted by the *current* process and has a live owner
///   protocol of its own.
/// - Proof must be persisted evidence (lock authority, reaped registry),
///   never a liveness heuristic.  When in doubt return `Unproven`.
#[async_trait]
pub trait TurnTerminalProofProvider: Send + Sync {
    async fn prove_orphan_generation_terminal(
        &self,
        user_id: &str,
        conversation_id: &str,
        requirement: OrphanProofRequirement,
        admission_epoch: i64,
        active_operation_id: Option<&str>,
    ) -> TerminalProofDecision;
}
