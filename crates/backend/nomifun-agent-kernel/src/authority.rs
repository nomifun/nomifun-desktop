use nomifun_agent_contracts::{
    CAPABILITY_NOT_ACTIVE, CAPABILITY_NOT_IN_PRESET, PRESET_RESOURCE_NOT_BOUND,
    RESOURCE_OWNER_MISMATCH, CanonicalErrorCode, RuntimeAuthorityCheckKind,
    RuntimeAuthorityDecision,
};

use crate::{
    ActiveCapabilitySetSnapshot, CapabilityInvocationRequest, CompiledSnapshot, KernelError,
};

pub struct ThinAuthority;

impl ThinAuthority {
    pub fn authorize(
        snapshot: &CompiledSnapshot,
        active: &ActiveCapabilitySetSnapshot,
        request: &CapabilityInvocationRequest,
    ) -> RuntimeAuthorityDecision {
        if request.principal != request.session_owner {
            return deny(
                RuntimeAuthorityCheckKind::PrincipalOwnership,
                RESOURCE_OWNER_MISMATCH,
            );
        }
        if &request.resolved_snapshot_ref != snapshot.snapshot_ref()
            || active.resolved_snapshot_ref != request.resolved_snapshot_ref
            || !snapshot
                .content()
                .capability_allowlist
                .contains(&request.capability_id)
        {
            return deny(
                RuntimeAuthorityCheckKind::SnapshotCapabilityAllowlist,
                CAPABILITY_NOT_IN_PRESET,
            );
        }
        if active.generation != request.active_set_generation
            || !active.active.contains(&request.capability_id)
        {
            return deny(
                RuntimeAuthorityCheckKind::SnapshotCapabilityAllowlist,
                CAPABILITY_NOT_ACTIVE,
            );
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
        if policy.resource_binding_ids != request.resource_binding_ids {
            return deny(
                RuntimeAuthorityCheckKind::TypedResourceBinding,
                PRESET_RESOURCE_NOT_BOUND,
            );
        }
        for binding_id in &request.resource_binding_ids {
            let Some(binding) = snapshot.binding(binding_id) else {
                return deny(
                    RuntimeAuthorityCheckKind::TypedResourceBinding,
                    PRESET_RESOURCE_NOT_BOUND,
                );
            };
            if binding.owner_id != request.principal.principal_id {
                return deny(
                    RuntimeAuthorityCheckKind::PrincipalOwnership,
                    RESOURCE_OWNER_MISMATCH,
                );
            }
        }
        RuntimeAuthorityDecision::Allow
    }

    pub fn enforce(
        snapshot: &CompiledSnapshot,
        active: &ActiveCapabilitySetSnapshot,
        request: &CapabilityInvocationRequest,
    ) -> Result<(), KernelError> {
        match Self::authorize(snapshot, active, request) {
            RuntimeAuthorityDecision::Allow => Ok(()),
            RuntimeAuthorityDecision::Deny { error_code, .. }
                if error_code.as_ref() == CAPABILITY_NOT_ACTIVE =>
            {
                Err(KernelError::CapabilityNotActive {
                    capability_id: request.capability_id.clone(),
                })
            }
            RuntimeAuthorityDecision::Deny { error_code, .. }
                if error_code.as_ref() == RESOURCE_OWNER_MISMATCH =>
            {
                let binding_id = request
                    .resource_binding_ids
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
                let binding_id = request
                    .resource_binding_ids
                    .iter()
                    .next()
                    .cloned()
                    .or_else(|| {
                        snapshot
                            .policy(&request.capability_id)
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
                    capability_id: request.capability_id.clone(),
                })
            }
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
