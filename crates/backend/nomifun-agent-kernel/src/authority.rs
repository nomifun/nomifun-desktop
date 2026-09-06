use nomifun_agent_contracts::{
    CAPABILITY_NOT_ACTIVE, CAPABILITY_NOT_IN_PRESET, PRESET_RESOURCE_NOT_BOUND,
    RESOURCE_OWNER_MISMATCH, CanonicalErrorCode, RuntimeAuthorityCheckKind,
    RuntimeAuthorityDecision,
};

use crate::{
    ActiveCapabilitySetSnapshot, CapabilityAccessRequest, CapabilityInvocationRequest,
    CompiledSnapshot, KernelError,
};

pub struct ThinAuthority;

impl ThinAuthority {
    pub fn authorize(
        snapshot: &CompiledSnapshot,
        active: &ActiveCapabilitySetSnapshot,
        request: &CapabilityInvocationRequest,
    ) -> RuntimeAuthorityDecision {
        let decision = authorize_capability_access(
            snapshot,
            active,
            &request.principal,
            &request.session_owner,
            &request.resolved_snapshot_ref,
            request.active_set_generation,
            &request.capability_id,
            &request.resource_binding_ids,
        );
        if !matches!(decision, RuntimeAuthorityDecision::Allow) {
            return decision;
        }
        let Some(policy) = snapshot.policy(&request.capability_id) else {
            return deny(
                RuntimeAuthorityCheckKind::SnapshotCapabilityAllowlist,
                CAPABILITY_NOT_IN_PRESET,
            );
        };
        if !policy.allowed_actions.contains(&request.action_id) {
            return deny(
                RuntimeAuthorityCheckKind::SnapshotCapabilityAllowlist,
                CAPABILITY_NOT_IN_PRESET,
            );
        }
        RuntimeAuthorityDecision::Allow
    }

    pub fn authorize_access(
        snapshot: &CompiledSnapshot,
        active: &ActiveCapabilitySetSnapshot,
        request: &CapabilityAccessRequest,
    ) -> RuntimeAuthorityDecision {
        authorize_capability_access(
            snapshot,
            active,
            &request.principal,
            &request.session_owner,
            &request.resolved_snapshot_ref,
            request.active_set_generation,
            &request.capability_id,
            &request.resource_binding_ids,
        )
    }

    pub fn enforce_access(
        snapshot: &CompiledSnapshot,
        active: &ActiveCapabilitySetSnapshot,
        request: &CapabilityAccessRequest,
    ) -> Result<(), KernelError> {
        enforce_decision(
            snapshot,
            &request.capability_id,
            &request.resource_binding_ids,
            Self::authorize_access(snapshot, active, request),
        )
    }

    pub fn enforce(
        snapshot: &CompiledSnapshot,
        active: &ActiveCapabilitySetSnapshot,
        request: &CapabilityInvocationRequest,
    ) -> Result<(), KernelError> {
        enforce_decision(
            snapshot,
            &request.capability_id,
            &request.resource_binding_ids,
            Self::authorize(snapshot, active, request),
        )
    }
}

fn authorize_capability_access(
    snapshot: &CompiledSnapshot,
    active: &ActiveCapabilitySetSnapshot,
    principal: &nomifun_agent_contracts::PrincipalRef,
    session_owner: &nomifun_agent_contracts::PrincipalRef,
    resolved_snapshot_ref: &nomifun_agent_contracts::ResolvedSnapshotRef,
    active_set_generation: u64,
    capability_id: &nomifun_agent_contracts::CapabilityId,
    resource_binding_ids: &std::collections::BTreeSet<
        nomifun_agent_contracts::ResourceBindingId,
    >,
) -> RuntimeAuthorityDecision {
    if principal != session_owner {
        return deny(
            RuntimeAuthorityCheckKind::PrincipalOwnership,
            RESOURCE_OWNER_MISMATCH,
        );
    }
    if resolved_snapshot_ref != snapshot.snapshot_ref()
        || &active.resolved_snapshot_ref != resolved_snapshot_ref
        || !snapshot
            .content()
            .capability_allowlist
            .contains(capability_id)
    {
        return deny(
            RuntimeAuthorityCheckKind::SnapshotCapabilityAllowlist,
            CAPABILITY_NOT_IN_PRESET,
        );
    }
    if active.generation != active_set_generation || !active.active.contains(capability_id) {
        return deny(
            RuntimeAuthorityCheckKind::SnapshotCapabilityAllowlist,
            CAPABILITY_NOT_ACTIVE,
        );
    }
    let Some(policy) = snapshot.policy(capability_id) else {
        return deny(
            RuntimeAuthorityCheckKind::SnapshotCapabilityAllowlist,
            CAPABILITY_NOT_IN_PRESET,
        );
    };
    if &policy.resource_binding_ids != resource_binding_ids {
        return deny(
            RuntimeAuthorityCheckKind::TypedResourceBinding,
            PRESET_RESOURCE_NOT_BOUND,
        );
    }
    for binding_id in resource_binding_ids {
        let Some(binding) = snapshot.binding(binding_id) else {
            return deny(
                RuntimeAuthorityCheckKind::TypedResourceBinding,
                PRESET_RESOURCE_NOT_BOUND,
            );
        };
        if binding.owner_id != principal.principal_id {
            return deny(
                RuntimeAuthorityCheckKind::PrincipalOwnership,
                RESOURCE_OWNER_MISMATCH,
            );
        }
    }
    RuntimeAuthorityDecision::Allow
}

fn enforce_decision(
    snapshot: &CompiledSnapshot,
    capability_id: &nomifun_agent_contracts::CapabilityId,
    resource_binding_ids: &std::collections::BTreeSet<
        nomifun_agent_contracts::ResourceBindingId,
    >,
    decision: RuntimeAuthorityDecision,
) -> Result<(), KernelError> {
        match decision {
            RuntimeAuthorityDecision::Allow => Ok(()),
            RuntimeAuthorityDecision::Deny { error_code, .. }
                if error_code.as_ref() == CAPABILITY_NOT_ACTIVE =>
            {
                Err(KernelError::CapabilityNotActive {
                    capability_id: capability_id.clone(),
                })
            }
            RuntimeAuthorityDecision::Deny { error_code, .. }
                if error_code.as_ref() == RESOURCE_OWNER_MISMATCH =>
            {
                let binding_id = resource_binding_ids
                    .iter()
                    .next()
                    .cloned()
                    .unwrap_or_else(|| {
                        nomifun_agent_contracts::ResourceBindingId::from("session-owner")
                    });
                Err(KernelError::ResourceOwnerMismatch { binding_id })
            }
            RuntimeAuthorityDecision::Deny { error_code, .. }
                if error_code.as_ref() == PRESET_RESOURCE_NOT_BOUND =>
            {
                let binding_id = resource_binding_ids
                    .iter()
                    .next()
                    .cloned()
                    .or_else(|| {
                        snapshot
                            .policy(capability_id)
                            .and_then(|policy| {
                                policy.resource_binding_ids.iter().next().cloned()
                            })
                    })
                    .unwrap_or_else(|| {
                        nomifun_agent_contracts::ResourceBindingId::from("missing")
                    });
                Err(KernelError::ResourceBindingMissing { binding_id })
            }
            RuntimeAuthorityDecision::Deny { .. } => {
                Err(KernelError::CapabilityNotInPreset {
                    capability_id: capability_id.clone(),
                })
            }
        }
}

fn deny(
    failed_check: RuntimeAuthorityCheckKind,
    error_code: &'static str,
) -> RuntimeAuthorityDecision {
    RuntimeAuthorityDecision::Deny {
        failed_check,
        error_code: CanonicalErrorCode::from(error_code),
    }
}
