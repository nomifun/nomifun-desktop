//! Production [`TurnTerminalProofProvider`]: the boot-scoped side of the
//! persisted exact terminal-proof protocol.
//!
//! Constructed once per boot, only after
//! `AppServices::has_valid_boot_reconciliation_authority` has revalidated the
//! exclusive data-dir server lock. Holding that lock proves the previous
//! backend process released it (it holds the lock handle for its entire
//! lifetime), so every turn generation frozen in the boot snapshot below was
//! admitted by a process that no longer exists. What the lock does NOT prove
//! is that the dead process's descendant tree is empty — that half of the
//! proof comes from [`nomifun_ai_agent::reap_orphan_agent_processes`], which
//! verifies every durable agent-process registry entry by exact identity and
//! kills-with-proof any survivor before this provider is built.
//!
//! The snapshot freeze is the anti-ABA fence: a generation is only provable
//! if its exact `(admission_epoch, active_operation_id)` was already
//! unsettled when this process booted. Anything newer was admitted by the
//! current process and has a live owner protocol of its own.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_ai_agent::AgentProcessReapReport;
use nomifun_conversation::terminal_proof::{
    OrphanProofRequirement, TerminalProofDecision, TurnTerminalProofProvider,
};

/// One frozen unsettled generation from the boot enumeration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrozenOrphanGeneration {
    pub user_id: String,
    pub admission_epoch: i64,
    pub active_operation_id: Option<String>,
}

pub(crate) struct BootTerminalProofProvider {
    /// conversation_id -> the exact generation observed before any work
    /// producer started. Only these generations may ever prove.
    frozen: HashMap<String, FrozenOrphanGeneration>,
    reap_report: AgentProcessReapReport,
}

impl BootTerminalProofProvider {
    pub(crate) fn new(
        frozen: HashMap<String, FrozenOrphanGeneration>,
        reap_report: AgentProcessReapReport,
    ) -> Arc<Self> {
        Arc::new(Self {
            frozen,
            reap_report,
        })
    }
}

#[async_trait]
impl TurnTerminalProofProvider for BootTerminalProofProvider {
    async fn prove_orphan_generation_terminal(
        &self,
        user_id: &str,
        conversation_id: &str,
        requirement: OrphanProofRequirement,
        admission_epoch: i64,
        active_operation_id: Option<&str>,
    ) -> TerminalProofDecision {
        let Some(frozen) = self.frozen.get(conversation_id) else {
            return TerminalProofDecision::Unproven {
                reason: "generation was not unsettled at boot; it belongs to the current process"
                    .to_owned(),
            };
        };
        if frozen.user_id != user_id
            || frozen.admission_epoch != admission_epoch
            || frozen.active_operation_id.as_deref() != active_operation_id
        {
            return TerminalProofDecision::Unproven {
                reason: "generation no longer matches the boot-frozen unsettled snapshot"
                    .to_owned(),
            };
        }
        if !self.reap_report.registry_fully_processed() {
            return TerminalProofDecision::Unproven {
                reason: "durable agent-process registry could not be fully reaped".to_owned(),
            };
        }

        match requirement {
            // In-process owner died with the lock holder; contained children
            // died with their Job/watchdog authorities. A surviving durable
            // registry entry for this Conversation would contradict that
            // audit, so require none.
            OrphanProofRequirement::LocalContainedAuthority
            | OrphanProofRequirement::RegisteredLocalProcessTree => {
                if self
                    .reap_report
                    .conversation_tree_proven_empty(conversation_id)
                {
                    TerminalProofDecision::Proven {
                        evidence: "server-lock authority + boot-frozen generation + reaped durable process registry (no surviving entry)"
                            .to_owned(),
                    }
                } else {
                    TerminalProofDecision::Unproven {
                        reason: "durable process registry retains an unproven entry for this Conversation"
                            .to_owned(),
                    }
                }
            }
            // A gateway is registered only when self-spawned; absence could
            // mean an external/attached gateway still hosts the work.
            OrphanProofRequirement::RegisteredGatewayAuthority => {
                if self.reap_report.conversation_had_entries(conversation_id)
                    && self
                        .reap_report
                        .conversation_tree_proven_empty(conversation_id)
                {
                    TerminalProofDecision::Proven {
                        evidence: "server-lock authority + boot-frozen generation + identity-verified reaping of the self-spawned gateway"
                            .to_owned(),
                    }
                } else {
                    TerminalProofDecision::Unproven {
                        reason: "gateway process was not registry-owned by this data dir; external work cannot be proven terminal locally"
                            .to_owned(),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_ai_agent::reap_orphan_agent_processes;

    const USER: &str = "user-1";
    const CONV: &str = "conv-1";

    fn frozen_map(epoch: i64, operation: Option<&str>) -> HashMap<String, FrozenOrphanGeneration> {
        HashMap::from([(
            CONV.to_owned(),
            FrozenOrphanGeneration {
                user_id: USER.to_owned(),
                admission_epoch: epoch,
                active_operation_id: operation.map(str::to_owned),
            },
        )])
    }

    /// A fully-processed empty reap report (no registry file in the dir).
    async fn empty_reap_report() -> nomifun_ai_agent::AgentProcessReapReport {
        let dir = tempfile::tempdir().unwrap();
        reap_orphan_agent_processes(dir.path()).await
    }

    #[tokio::test]
    async fn frozen_generation_with_empty_registry_proves_local_backends() {
        let provider =
            BootTerminalProofProvider::new(frozen_map(3, Some("op-a")), empty_reap_report().await);
        for requirement in [
            OrphanProofRequirement::LocalContainedAuthority,
            OrphanProofRequirement::RegisteredLocalProcessTree,
        ] {
            let decision = provider
                .prove_orphan_generation_terminal(USER, CONV, requirement, 3, Some("op-a"))
                .await;
            assert!(
                matches!(decision, TerminalProofDecision::Proven { .. }),
                "{requirement:?}: {decision:?}"
            );
        }
    }

    #[tokio::test]
    async fn generation_outside_the_frozen_snapshot_never_proves() {
        let provider =
            BootTerminalProofProvider::new(frozen_map(3, Some("op-a")), empty_reap_report().await);
        for (epoch, operation) in [
            (4, Some("op-a")),
            (3, Some("op-b")),
            (3, None),
        ] {
            let decision = provider
                .prove_orphan_generation_terminal(
                    USER,
                    CONV,
                    OrphanProofRequirement::LocalContainedAuthority,
                    epoch,
                    operation,
                )
                .await;
            assert!(
                matches!(decision, TerminalProofDecision::Unproven { .. }),
                "epoch {epoch} op {operation:?}: {decision:?}"
            );
        }
        let unknown_conversation = provider
            .prove_orphan_generation_terminal(
                USER,
                "conv-other",
                OrphanProofRequirement::LocalContainedAuthority,
                0,
                None,
            )
            .await;
        assert!(matches!(
            unknown_conversation,
            TerminalProofDecision::Unproven { .. }
        ));
        let wrong_user = provider
            .prove_orphan_generation_terminal(
                "user-2",
                CONV,
                OrphanProofRequirement::LocalContainedAuthority,
                3,
                Some("op-a"),
            )
            .await;
        assert!(matches!(wrong_user, TerminalProofDecision::Unproven { .. }));
    }

    #[tokio::test]
    async fn gateway_requirement_treats_registry_absence_as_unproven() {
        let provider =
            BootTerminalProofProvider::new(frozen_map(1, Some("op-a")), empty_reap_report().await);
        let decision = provider
            .prove_orphan_generation_terminal(
                USER,
                CONV,
                OrphanProofRequirement::RegisteredGatewayAuthority,
                1,
                Some("op-a"),
            )
            .await;
        assert!(
            matches!(decision, TerminalProofDecision::Unproven { .. }),
            "a gateway with no registry entry may be external: {decision:?}"
        );
    }

    #[tokio::test]
    async fn unprocessed_registry_report_proves_nothing() {
        let provider = BootTerminalProofProvider::new(
            frozen_map(1, Some("op-a")),
            nomifun_ai_agent::AgentProcessReapReport::default(),
        );
        let decision = provider
            .prove_orphan_generation_terminal(
                USER,
                CONV,
                OrphanProofRequirement::LocalContainedAuthority,
                1,
                Some("op-a"),
            )
            .await;
        assert!(matches!(decision, TerminalProofDecision::Unproven { .. }));
    }
}
