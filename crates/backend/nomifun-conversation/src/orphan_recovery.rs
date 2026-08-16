//! Restart-orphan policy for durable `running` Conversations.
//!
//! A missing process-local registry entry is only evidence that this process
//! no longer owns a runtime.  It is not evidence that work hosted by another
//! process or machine has stopped.  Keep this classification centralized so a
//! newly-added backend fails closed until its crash/parent-death contract has
//! been audited explicitly.
//!
//! Every variant still requires proof before a durable Running generation may
//! be finalized; a variant only names which persisted evidence source can
//! supply that proof.  Request-facing guards (send/stop/delete/warmup) treat
//! all variants identically — quarantined until the boot/background recovery
//! seam presents the proof — so adding a variant never opens a bypass.

use nomifun_common::{AgentType, AppError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunningOrphanDisposition {
    /// The turn owner is an in-process engine task that cannot outlive the
    /// application, and every effect-bearing child spawn path is contained by
    /// an audited parent-death authority (Windows kill-on-close Job armed
    /// before resume; per-child Unix watchdog sealing the process group).
    /// Terminal proof = exclusive boot-lock authority over the data dir plus
    /// a fully-reaped durable process registry with no surviving entry for
    /// this Conversation.
    LocalContainedAuthority,
    /// The backend's effect-bearing child process is durably registered at
    /// spawn and unregistered only on proven tree exit.  Terminal proof =
    /// verified boot reaping of every registry entry for this Conversation;
    /// absence of entries is itself proof (the child never spawned, exited
    /// with proof, or died under the same containment authorities as above).
    RegisteredLocalProcessTree,
    /// The backend registers a gateway process only when it self-spawned one;
    /// external or port-attached gateways host work this process never owned,
    /// so registry absence is ambiguous.  Terminal proof requires that entries
    /// existed for this Conversation and every one was reaped with
    /// exact-identity verification.
    RegisteredGatewayAuthorityRequired,
}

pub(crate) fn running_orphan_disposition(
    persisted_agent_type: &str,
) -> Result<RunningOrphanDisposition, AppError> {
    let disposition = match persisted_agent_type {
        // Crash contract audited 2026-07-31: the Nomi turn is an in-process
        // tokio task; Bash/exec children ride the ProcessSupervisor, MCP
        // stdio servers and browser roots ride ChildProcessBuilder — all
        // armed with kill-on-close Jobs (Windows) or forked parent-death
        // watchdogs (Unix) before their first instruction.  Long-lived
        // survivors are limited to read-only LSP servers; the browser
        // additionally has its own identity-verified boot recovery.
        value if value == AgentType::Nomi.serde_name() => {
            RunningOrphanDisposition::LocalContainedAuthority
        }
        value if value == AgentType::Acp.serde_name() => {
            RunningOrphanDisposition::RegisteredLocalProcessTree
        }
        value if value == AgentType::OpenclawGateway.serde_name() => {
            RunningOrphanDisposition::RegisteredGatewayAuthorityRequired
        }
        unknown => {
            return Err(AppError::Conflict(format!(
                "Conversation uses unknown Agent backend '{unknown}'; refusing to finalize an unproven running turn"
            )));
        }
    };
    Ok(disposition)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_current_backend_requires_proof_after_restart() {
        for (backend, disposition) in [
            (
                AgentType::Nomi.serde_name(),
                RunningOrphanDisposition::LocalContainedAuthority,
            ),
            (
                AgentType::Acp.serde_name(),
                RunningOrphanDisposition::RegisteredLocalProcessTree,
            ),
            (
                AgentType::OpenclawGateway.serde_name(),
                RunningOrphanDisposition::RegisteredGatewayAuthorityRequired,
            ),
        ] {
            assert_eq!(running_orphan_disposition(backend).unwrap(), disposition);
        }
    }

    #[test]
    fn unknown_backend_fails_closed() {
        assert!(matches!(
            running_orphan_disposition("future-backend"),
            Err(AppError::Conflict(_))
        ));
    }
}
