use std::collections::BTreeSet;

use nomifun_agent_contracts::{CapabilityId, ResolvedSnapshotRef};
use crate::{CompiledSnapshot, KernelError};

/// The enabled ceiling is fixed for the lifetime of a saved session snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveCapabilitySetSnapshot {
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    /// Compatibility field for invocation ports; fixed at zero, not an
    /// engine-controlled activation counter.
    pub generation: u64,
    /// All capabilities enabled by the saved Snapshot, including its frozen
    /// dependency closure. This is not a mutable runtime selection.
    pub active: BTreeSet<CapabilityId>,
}

pub struct SessionCapabilityState {
    snapshot: ActiveCapabilitySetSnapshot,
}

impl SessionCapabilityState {
    pub fn new(snapshot: &CompiledSnapshot) -> Self {
        Self {
            snapshot: ActiveCapabilitySetSnapshot {
                resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                generation: 0,
                active: snapshot.content().capability_allowlist.clone(),
            },
        }
    }

    /// Rehydrate the one canonical capability generation committed by the
    /// AgentSession Store. Every ID must remain inside the newly compiled
    /// Snapshot ceiling; a stale or forged active set fails closed.
    pub fn from_committed(
        snapshot: &CompiledSnapshot,
        generation: u64,
        active: impl IntoIterator<Item = CapabilityId>,
    ) -> Result<Self, KernelError> {
        let active = active.into_iter().collect::<BTreeSet<_>>();
        if let Some(capability_id) = active
            .iter()
            .find(|id| !snapshot.content().capability_allowlist.contains(*id))
        {
            return Err(KernelError::CapabilityNotInPreset {
                capability_id: capability_id.clone(),
            });
        }
        Ok(Self {
            snapshot: ActiveCapabilitySetSnapshot {
                resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                generation,
                active,
            },
        })
    }

    pub fn snapshot(&self) -> Result<ActiveCapabilitySetSnapshot, KernelError> {
        Ok(self.snapshot.clone())
    }
}
