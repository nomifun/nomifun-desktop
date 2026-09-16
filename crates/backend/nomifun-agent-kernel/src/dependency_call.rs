//! Managed dependency calls retain the existing Kernel admission and dispatch.
//! This module grants no capabilities and does not resolve a second plan.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, Weak};

use nomifun_agent_contracts::{
    ActionId, CapabilityId, IdempotencyKey, OperationId, StrictJsonValue, digest_payload,
};
use tokio::sync::watch;

use crate::{
    ActiveCapabilitySetSnapshot, CapabilityAccessRequest, CapabilityInvocationRequest,
    CompiledSnapshot, KernelError, KernelRegistry,
};

/// Only target, action, input and a local effect identity come from the caller.
/// All ownership, resource, Snapshot and correlation facts come from its parent.
#[derive(Clone, Debug, PartialEq)]
pub struct CapabilityDependencyCall {
    pub capability_id: CapabilityId,
    pub action_id: ActionId,
    /// Unique within one parent invocation, at most 128 UTF-8 bytes. On an
    /// explicit outer retry, reuse the same key for the same logical effect.
    /// This is namespaced by the parent's idempotency key, never a global key.
    pub call_key: String,
    pub input: StrictJsonValue,
}

/// A clone is a reference to the same invocation, not a transferable grant.
/// Retained clones reject work once the parent completes, fails or is dropped.
#[derive(Clone)]
pub struct CapabilityDependencyCaller {
    inner: Arc<DependencyInvocation>,
}

/// Authority evidence keeps the real parent kind; Context has no Tool action
/// or Tool idempotency key. No plugin can construct this ancestry.
#[derive(Clone)]
pub(crate) enum DependencyAncestor {
    Tool(CapabilityInvocationRequest),
    Context(CapabilityAccessRequest),
}

impl DependencyAncestor {
    pub(crate) fn access(&self) -> CapabilityAccessRequest {
        match self {
            Self::Context(request) => request.clone(),
            Self::Tool(request) => CapabilityAccessRequest {
                principal: request.principal.clone(),
                session_owner: request.session_owner.clone(),
                agent_session_id: request.agent_session_id.clone(),
                operation_id: request.operation_id.clone(),
                correlation_id: request.correlation_id.clone(),
                resolved_snapshot_ref: request.resolved_snapshot_ref.clone(),
                active_set_generation: request.active_set_generation,
                capability_id: request.capability_id.clone(),
                resource_binding_ids: request.resource_binding_ids.clone(),
                state_scope_key: request.state_scope_key.clone(),
            },
        }
    }
}

struct DependencyInvocation {
    kernel: Weak<KernelRegistry>,
    snapshot: Arc<CompiledSnapshot>,
    active: Arc<ActiveCapabilitySetSnapshot>,
    ancestry: Arc<Vec<DependencyAncestor>>,
    open: watch::Receiver<bool>,
    used_keys: Mutex<BTreeSet<String>>,
}

/// Owned by the actual dispatch future, not by any callback retained by a plugin.
pub(crate) struct DependencyInvocationGuard {
    open: watch::Sender<bool>,
    // Retained plugin callbacks must not retain the Registry which owns them.
    _kernel: Arc<KernelRegistry>,
}

impl Drop for DependencyInvocationGuard {
    fn drop(&mut self) {
        self.open.send_replace(false);
    }
}

fn denied(code: &str, message: &str) -> KernelError {
    KernelError::capability_execution_failed(code, message)
}

impl CapabilityDependencyCaller {
    pub(crate) fn scoped(
        kernel: KernelRegistry,
        snapshot: Arc<CompiledSnapshot>,
        active: Arc<ActiveCapabilitySetSnapshot>,
        ancestry: Arc<Vec<DependencyAncestor>>,
    ) -> (Self, DependencyInvocationGuard) {
        let (open, receiver) = watch::channel(true);
        let kernel = Arc::new(kernel);
        (
            Self {
                inner: Arc::new(DependencyInvocation {
                    kernel: Arc::downgrade(&kernel),
                    snapshot,
                    active,
                    ancestry,
                    open: receiver,
                    used_keys: Mutex::new(BTreeSet::new()),
                }),
            },
            DependencyInvocationGuard {
                open,
                _kernel: kernel,
            },
        )
    }

    /// Invoke a direct declared dependency through the ordinary Kernel path.
    /// No implicit retry is performed. Cancellation drops cooperative child
    /// work; it does not undo already committed effects or stop arbitrary OS IO.
    pub async fn invoke(
        &self,
        call: CapabilityDependencyCall,
    ) -> Result<StrictJsonValue, KernelError> {
        let inner = &self.inner;
        let mut open = inner.open.clone();
        if !*open.borrow() {
            return Err(denied(
                "DEPENDENCY_PARENT_CLOSED",
                "the parent invocation has ended",
            ));
        }
        let kernel = inner.kernel.upgrade().ok_or_else(|| {
            denied(
                "DEPENDENCY_PARENT_CLOSED",
                "the parent invocation has ended",
            )
        })?;
        let ancestor = inner.ancestry.last().ok_or(KernelError::RegistryPoisoned)?;
        let parent = ancestor.access();
        if call.call_key.trim().is_empty() || call.call_key.len() > 128 {
            return Err(denied(
                "DEPENDENCY_CALL_KEY_INVALID",
                "call_key must contain 1..128 bytes",
            ));
        }
        if inner
            .ancestry
            .iter()
            .any(|ancestor| ancestor.access().capability_id == call.capability_id)
        {
            return Err(denied(
                "DEPENDENCY_CALL_CYCLE",
                "a dependency call cannot re-enter an ancestor capability",
            ));
        }
        // A still-running descendant must not borrow the authority of a
        // withdrawn ancestor. Check exact parents as well as the child target.
        let mut dependencies = Vec::new();
        for ancestor in inner.ancestry.iter() {
            dependencies =
                kernel.invocation_dependencies(&inner.snapshot, &inner.active, ancestor)?;
        }
        let declared = dependencies
            .iter()
            .find(|dependency| dependency.id == call.capability_id)
            .ok_or_else(|| {
                denied(
                    "DEPENDENCY_NOT_DECLARED",
                    "the target is not a direct dependency of the selected implementation",
                )
            })?;
        // Frozen graph membership is necessary but is not a grant: the live,
        // exact selected implementation's declaration above must agree too.
        let frozen_parent = inner
            .snapshot
            .resolved_capability(&parent.capability_id)
            .ok_or(KernelError::RegistryPoisoned)?;
        if !frozen_parent.dependency_refs.contains(declared) {
            return Err(denied(
                "DEPENDENCY_TARGET_MISMATCH",
                "the dependency is not an edge of the frozen parent plan",
            ));
        }
        let resolved = inner
            .snapshot
            .resolved_capability(&call.capability_id)
            .ok_or_else(|| KernelError::CapabilityNotInPreset {
                capability_id: call.capability_id.clone(),
            })?;
        if resolved.capability != *declared {
            return Err(denied(
                "DEPENDENCY_TARGET_MISMATCH",
                "the dependency version differs from the frozen plan",
            ));
        }
        let policy = inner.snapshot.policy(&call.capability_id).ok_or_else(|| {
            KernelError::CapabilityNotInPreset {
                capability_id: call.capability_id.clone(),
            }
        })?;
        let identity = |kind: &str| {
            let digest = match ancestor {
                DependencyAncestor::Tool(tool) => digest_payload(&(
                    "kernel-dependency-v1",
                    kind,
                    if kind == "effect" {
                        tool.idempotency_key.as_ref()
                    } else {
                        tool.operation_id.as_ref()
                    },
                    &parent.principal,
                    &parent.agent_session_id,
                    &parent.resolved_snapshot_ref,
                    &parent.capability_id,
                    &tool.action_id,
                    &call.call_key,
                )),
                DependencyAncestor::Context(_) => digest_payload(&(
                    "kernel-context-dependency-v1",
                    kind,
                    &parent.operation_id,
                    &parent.principal,
                    &parent.agent_session_id,
                    &parent.resolved_snapshot_ref,
                    &parent.capability_id,
                    &call.call_key,
                )),
            };
            digest
                .map(|digest| format!("dependency-{}", digest.as_ref()))
                .map_err(|error| KernelError::Digest {
                    reason: error.to_string(),
                })
        };
        let request = CapabilityInvocationRequest {
            principal: parent.principal.clone(),
            session_owner: parent.session_owner.clone(),
            agent_session_id: parent.agent_session_id.clone(),
            operation_id: OperationId::from(identity("operation")?),
            idempotency_key: IdempotencyKey::from(identity("effect")?),
            correlation_id: parent.correlation_id.clone(),
            resolved_snapshot_ref: parent.resolved_snapshot_ref.clone(),
            active_set_generation: parent.active_set_generation,
            capability_id: call.capability_id,
            action_id: call.action_id,
            resource_binding_ids: policy.resource_binding_ids.clone(),
            state_scope_key: parent.state_scope_key.clone(),
            input: call.input,
        };
        crate::ThinAuthority::enforce(&inner.snapshot, &inner.active, &request)?;
        {
            let mut keys = inner
                .used_keys
                .lock()
                .map_err(|_| KernelError::RegistryPoisoned)?;
            // Bounded invocation-local bookkeeping, not a durable effect ledger.
            // Effect deduplication remains the owning handler's responsibility.
            if keys.len() >= 1024 {
                return Err(denied(
                    "DEPENDENCY_CALL_LIMIT",
                    "one parent admits at most 1024 dependency calls",
                ));
            }
            if !keys.insert(call.call_key) {
                return Err(denied(
                    "DEPENDENCY_CALL_KEY_REUSED",
                    "call_key has already been submitted by this parent; no automatic retry",
                ));
            }
        }
        tokio::select! {
            biased;
            _ = open.changed() => Err(denied("DEPENDENCY_PARENT_CLOSED", "the parent ended while its dependency was running")),
            result = kernel.invoke_scoped(
                Arc::clone(&inner.snapshot), Arc::clone(&inner.active), request, Arc::clone(&inner.ancestry),
            ) => result,
        }
    }
}
