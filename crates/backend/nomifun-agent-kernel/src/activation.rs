use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use nomifun_agent_contracts::{
    CapabilityId, CompactOnDemandCapabilityEntry, OperationId, PrecomputedActivationPlan,
    ResolvedSnapshotRef,
};

use crate::{CompiledSnapshot, KernelError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletedTurnBoundary {
    pub completed_turn_operation_id: OperationId,
}

impl CompletedTurnBoundary {
    pub fn committed(completed_turn_operation_id: OperationId) -> Self {
        Self {
            completed_turn_operation_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveCapabilitySetSnapshot {
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub generation: u64,
    pub active: BTreeSet<CapabilityId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivationOutcome {
    Activated {
        generation: u64,
        activated_bundle: Vec<CapabilityId>,
    },
    AlreadyActive {
        generation: u64,
    },
}

#[derive(Clone, Debug)]
struct ActiveCapabilitySet {
    resolved_snapshot_ref: ResolvedSnapshotRef,
    generation: u64,
    active: BTreeSet<CapabilityId>,
    plans: BTreeMap<CapabilityId, PrecomputedActivationPlan>,
    compact_index: BTreeMap<CapabilityId, CompactOnDemandCapabilityEntry>,
}

impl ActiveCapabilitySet {
    fn from_compiled(snapshot: &CompiledSnapshot) -> Self {
        Self {
            resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
            generation: 0,
            active: snapshot
                .content()
                .initial_capabilities
                .iter()
                .map(|capability| capability.capability.id.clone())
                .collect(),
            plans: snapshot
                .content()
                .on_demand_activation_plans
                .clone(),
            compact_index: snapshot
                .content()
                .compact_on_demand_index
                .iter()
                .cloned()
                .map(|entry| (entry.capability_id.clone(), entry))
                .collect(),
        }
    }

    fn snapshot(&self) -> ActiveCapabilitySetSnapshot {
        ActiveCapabilitySetSnapshot {
            resolved_snapshot_ref: self.resolved_snapshot_ref.clone(),
            generation: self.generation,
            active: self.active.clone(),
        }
    }

    fn search(&self, query: &str, limit: usize) -> Vec<CompactOnDemandCapabilityEntry> {
        let query = query.trim().to_ascii_lowercase();
        self.compact_index
            .values()
            .filter(|entry| !self.active.contains(&entry.capability_id))
            .filter(|entry| {
                query.is_empty()
                    || entry
                        .capability_id
                        .as_ref()
                        .to_ascii_lowercase()
                        .contains(&query)
                    || entry.display_name.to_ascii_lowercase().contains(&query)
                    || entry
                        .short_description
                        .to_ascii_lowercase()
                        .contains(&query)
                    || entry.search_terms.iter().any(|term| term.contains(&query))
            })
            .take(limit)
            .cloned()
            .collect()
    }

    fn activate(
        &mut self,
        expected_generation: u64,
        capability_id: &CapabilityId,
        _boundary: &CompletedTurnBoundary,
    ) -> Result<ActivationOutcome, KernelError> {
        if self.active.contains(capability_id) {
            return Ok(ActivationOutcome::AlreadyActive {
                generation: self.generation,
            });
        }
        let Some(plan) = self.plans.get(capability_id) else {
            return Err(KernelError::CapabilityNotInPreset {
                capability_id: capability_id.clone(),
            });
        };
        if self.generation != expected_generation {
            return Err(KernelError::ActivationGenerationConflict {
                expected: expected_generation,
                current: self.generation,
            });
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(KernelError::ActivationGenerationExhausted)?;
        let activated_bundle = plan.capability_bundle.clone();
        self.active.extend(activated_bundle.iter().cloned());
        self.generation = generation;
        Ok(ActivationOutcome::Activated {
            generation,
            activated_bundle,
        })
    }
}

pub struct SessionCapabilityState {
    state: Mutex<ActiveCapabilitySet>,
}

impl SessionCapabilityState {
    pub fn new(snapshot: &CompiledSnapshot) -> Self {
        Self {
            state: Mutex::new(ActiveCapabilitySet::from_compiled(snapshot)),
        }
    }

    pub fn snapshot(&self) -> Result<ActiveCapabilitySetSnapshot, KernelError> {
        self.state
            .lock()
            .map(|state| state.snapshot())
            .map_err(|_| KernelError::RegistryPoisoned)
    }

    pub fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<CompactOnDemandCapabilityEntry>, KernelError> {
        self.state
            .lock()
            .map(|state| state.search(query, limit))
            .map_err(|_| KernelError::RegistryPoisoned)
    }

    pub fn activate_at_boundary(
        &self,
        expected_generation: u64,
        capability_id: &CapabilityId,
        boundary: CompletedTurnBoundary,
    ) -> Result<ActivationOutcome, KernelError> {
        self.state
            .lock()
            .map_err(|_| KernelError::RegistryPoisoned)?
            .activate(expected_generation, capability_id, &boundary)
    }
}

#[cfg(test)]
mod tests {
    use nomifun_agent_contracts::{
        AgentPresetId, DigestHex, PresetRevisionRef, ResolvedSnapshotContent,
        ResolvedSnapshotEnvelope, ResolvedSnapshotId, RuntimeProfileKind, VersionString,
    };

    use super::*;
    use crate::CompiledSnapshot;

    fn compiled_snapshot() -> CompiledSnapshot {
        let capability = CapabilityId::from("sample.echo");
        let snapshot_ref = ResolvedSnapshotRef {
            snapshot_id: ResolvedSnapshotId::from("snapshot"),
            snapshot_digest: DigestHex::from("digest"),
        };
        CompiledSnapshot {
            envelope: ResolvedSnapshotEnvelope {
                snapshot_ref,
                content: ResolvedSnapshotContent {
                    schema_version: VersionString::from("1.0.0"),
                    resolver_version: VersionString::from("1.0.0"),
                    preset_revision_ref: PresetRevisionRef {
                        preset_id: AgentPresetId::from("preset"),
                        revision: 1,
                        revision_digest: DigestHex::from("revision"),
                    },
                    required_runtime_protocol_version: VersionString::from("1.0.0"),
                    required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                    runtime_feature_inventory_digest: DigestHex::from("runtime"),
                    required_runtime_features: BTreeSet::new(),
                    compiled_runtime_profile_digest: DigestHex::from("profile"),
                    model_route_refs: BTreeMap::new(),
                    chat_route_identity: None,
                    initial_capabilities: Vec::new(),
                    on_demand_capabilities: Vec::new(),
                    on_demand_activation_plans: BTreeMap::from([(
                        capability.clone(),
                        PrecomputedActivationPlan {
                            root_capability_id: capability.clone(),
                            capability_bundle: vec![capability.clone()],
                            tool_schema_refs: Vec::new(),
                            context_schema_refs: Vec::new(),
                            resource_binding_refs: Vec::new(),
                            model_route_refs: Vec::new(),
                        },
                    )]),
                    compact_on_demand_index: vec![CompactOnDemandCapabilityEntry {
                        capability_id: capability.clone(),
                        display_name: "Echo".to_owned(),
                        short_description: "Echo a value".to_owned(),
                        search_terms: vec!["echo".to_owned()],
                        activation_plan_digest: DigestHex::from("plan"),
                    }],
                    capability_allowlist: BTreeSet::from([capability]),
                    skill_locks: Vec::new(),
                    mcp_tool_locks: Vec::new(),
                    typed_resource_bindings: Vec::new(),
                    canonical_schema_manifest_digest: DigestHex::from("schema"),
                    target_contribution_manifest_digest: DigestHex::from("target"),
                },
                actor: nomifun_agent_contracts::PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: "user-a".to_owned(),
                },
                scene: "test".to_owned(),
                surface: "test".to_owned(),
                audience: "test".to_owned(),
                created_at_ms: 0,
                resolver_run_id: OperationId::from("resolve"),
                availability_evidence_revision: "test".to_owned(),
            },
            authority_policies: BTreeMap::new(),
            registry_generation: 1,
            registry_digest: DigestHex::from("registry"),
        }
    }

    #[test]
    fn activation_is_boundary_only_monotonic_and_duplicate_idempotent() {
        let compiled = compiled_snapshot();
        let state = SessionCapabilityState::new(&compiled);
        let capability = CapabilityId::from("sample.echo");
        assert_eq!(state.search("echo", 10).unwrap().len(), 1);
        assert!(matches!(
            state.activate_at_boundary(
                1,
                &capability,
                CompletedTurnBoundary::committed(OperationId::from("wrong-generation")),
            ),
            Err(KernelError::ActivationGenerationConflict {
                expected: 1,
                current: 0,
            })
        ));
        assert!(matches!(
            state.activate_at_boundary(
                0,
                &CapabilityId::from("outside.snapshot"),
                CompletedTurnBoundary::committed(OperationId::from("outside")),
            ),
            Err(KernelError::CapabilityNotInPreset { .. })
        ));
        assert_eq!(
            state
                .activate_at_boundary(
                    0,
                    &capability,
                    CompletedTurnBoundary::committed(OperationId::from("turn-1")),
                )
                .unwrap(),
            ActivationOutcome::Activated {
                generation: 1,
                activated_bundle: vec![capability.clone()],
            }
        );
        assert_eq!(
            state
                .activate_at_boundary(
                    0,
                    &capability,
                    CompletedTurnBoundary::committed(OperationId::from("turn-1-retry")),
                )
                .unwrap(),
            ActivationOutcome::AlreadyActive { generation: 1 }
        );
        assert!(state.search("echo", 10).unwrap().is_empty());
    }
}
