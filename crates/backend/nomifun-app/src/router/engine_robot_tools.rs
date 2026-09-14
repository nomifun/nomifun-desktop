//! Frozen Robot device tools for source-integrated engines. Device discovery,
//! physical effects and lifecycle authority remain application-owned.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    AgentSessionId, CapabilityId, ContributionSourceKind, PrincipalRef, TypedResourceBinding,
};
use nomifun_agent_kernel::{CompiledSnapshot, SessionCapabilityState};
use nomifun_ai_agent::{
    ContextContributor, NomiHostDynamicToolInvocation, NomiHostDynamicToolInvoker,
};
use nomifun_common::AppError;
use nomifun_engine_core::{
    EngineToolError, EngineToolExposure, EngineToolInvocation, EngineToolInvoker, EngineToolPlan,
    EngineToolResult,
};
use tokio_util::sync::CancellationToken;

use super::hosted_effect_receipts::{HostedEffectReceipts, RobotReceiptInvoker};
use super::nomi_core_robot::{self, NomiCoreRobotWave4Owner};

pub(crate) fn supported_ids() -> BTreeSet<CapabilityId> {
    nomi_core_robot::tool_capability_ids()
        .into_iter()
        .chain(nomi_core_robot::lifecycle_capability_ids())
        .chain(nomi_core_robot::context_capability_ids())
        .collect()
}

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Engine Robot tools: {message}"))
}
fn rejected(message: impl std::fmt::Display) -> EngineToolError {
    EngineToolError::ToolInvocation(format!("Robot authority: {message}"))
}

#[derive(Default)]
struct Leases {
    closed: bool,
    retained: BTreeMap<CapabilityId, Arc<dyn ContextContributor>>,
}

pub(crate) struct FrozenTools {
    pub plan: EngineToolPlan,
    owner: Arc<NomiCoreRobotWave4Owner>,
    delegate: Arc<dyn NomiHostDynamicToolInvoker>,
    principal: PrincipalRef,
    session: AgentSessionId,
    resource: TypedResourceBinding,
    leases: Mutex<Leases>,
}

impl FrozenTools {
    /// Read-only catalog projection; no lease acquisition, device operation,
    /// or automatic activation of link/audio during Session construction.
    pub(crate) async fn resolve(
        owner: Arc<NomiCoreRobotWave4Owner>,
        receipts: HostedEffectReceipts,
        principal: PrincipalRef,
        session: AgentSessionId,
        snapshot: &CompiledSnapshot,
        compile: impl FnOnce(Vec<EngineToolExposure>) -> Result<EngineToolPlan, AppError>,
    ) -> Result<Self, AppError> {
        let resources = snapshot
            .target_resource_bindings
            .iter()
            .filter(|binding| binding.resource_kind.as_ref() == "robot")
            .collect::<Vec<_>>();
        let [resource] = resources.as_slice() else {
            return Err(failure(
                "one exact server-resolved Robot binding is required",
            ));
        };
        if principal.principal_kind != "user" || resource.owner_id != principal.principal_id {
            return Err(failure("Robot binding belongs to a different principal"));
        }
        let supported = supported_ids();
        let mut initial = BTreeSet::new();
        for (items, ids) in [
            (&snapshot.content().enabled_capabilities, &mut initial),
        ] {
            for selected in items {
                if !supported.contains(&selected.capability.id) {
                    continue;
                }
                let policy = snapshot
                    .policy(&selected.capability.id)
                    .ok_or_else(|| failure("Robot capability policy missing"))?;
                if selected.contribution_lock.source_kind != ContributionSourceKind::PlatformBuiltin
                    || !policy.resource_binding_ids.contains(&resource.binding_id)
                {
                    return Err(failure(
                        "Robot requires the bundled owner and exact resource policy",
                    ));
                }
                ids.insert(selected.capability.id.clone());
            }
        }
        let tools = nomi_core_robot::tool_capability_ids();
        if initial.iter().any(|id| tools.contains(id))
            && !initial.contains(&CapabilityId::from("robot.link"))
        {
            return Err(failure(
                "Robot tools require explicitly selected robot.link",
            ));
        }
        let (descriptors, delegate) = owner
            .resolve_session_tools(&principal, &session, resource, &initial)
            .await
            .map_err(failure)?;
        for selected in initial.iter().filter(|id| tools.contains(*id)) {
            if !descriptors
                .iter()
                .any(|descriptor| &descriptor.capability_id == selected)
            {
                return Err(failure(
                    "selected Robot capability has no frozen device tool",
                ));
            }
        }
        let exposures = descriptors.into_iter().map(|descriptor| EngineToolExposure {
            definition: nomifun_chat_model_broker::ChatToolDefinition {
                name: descriptor.provider_name,
                description: format!("{}\nPhysical Robot tool. Requires active robot.link; audio tools also require active robot.audio. A reply is not proof of physical quiescence or reversibility.", descriptor.description),
                input_schema: descriptor.input_schema,
                deferred: false,
            },
            action_id: format!("{}.invoke", descriptor.capability_id.as_ref()).into(),
            capability_id: descriptor.capability_id,
        }).collect();
        Ok(Self {
            plan: compile(exposures)?,
            owner,
            delegate: Arc::new(RobotReceiptInvoker {
                receipts,
                user: principal.principal_id.clone(),
                session: session.as_ref().to_owned(),
                delegate,
            }),
            principal,
            session,
            resource: (**resource).clone(),
            leases: Mutex::new(Leases::default()),
        })
    }

    /// Revoke local authority even if callers retain an Arc to the tool host.
    /// This does not disconnect shared hardware or claim physical shutdown.
    pub(crate) fn close(&self) -> Result<(), AppError> {
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| failure("lease state poisoned"))?;
        leases.closed = true;
        leases.retained.clear();
        Ok(())
    }

    async fn retain_active_leases(
        &self,
        active: &BTreeSet<CapabilityId>,
    ) -> Result<(), EngineToolError> {
        if !active.contains(&CapabilityId::from("robot.link")) {
            return Err(rejected(
                "robot.link must be explicitly activated before device use",
            ));
        }
        for id in nomi_core_robot::lifecycle_capability_ids() {
            if !active.contains(&id) {
                continue;
            }
            {
                let leases = self
                    .leases
                    .lock()
                    .map_err(|_| rejected("lease state poisoned"))?;
                if leases.closed {
                    return Err(rejected("Session resources closed"));
                }
                if leases.retained.contains_key(&id) {
                    continue;
                }
            }
            let lease = self
                .owner
                .lifecycle_context_contributor_for(
                    &id,
                    &self.principal,
                    &self.session,
                    std::slice::from_ref(&self.resource),
                )
                .await
                .map_err(rejected)?
                .ok_or_else(|| rejected("missing lifecycle owner"))?;
            let mut leases = self
                .leases
                .lock()
                .map_err(|_| rejected("lease state poisoned"))?;
            if leases.closed {
                return Err(rejected("Session resources closed"));
            }
            leases.retained.entry(id).or_insert(lease);
        }
        Ok(())
    }
}

/// Always nested inside EngineToolHost's retained, serialized effect lane.
pub(crate) struct SessionTools {
    pub frozen: Arc<FrozenTools>,
    pub inner: Arc<dyn EngineToolInvoker>,
    pub snapshot: Arc<CompiledSnapshot>,
    pub active: Arc<SessionCapabilityState>,
}

#[async_trait]
impl EngineToolInvoker for SessionTools {
    async fn invoke(
        &self,
        invocation: EngineToolInvocation,
        cancellation: CancellationToken,
    ) -> Result<EngineToolResult, EngineToolError> {
        if !nomi_core_robot::tool_capability_ids().contains(&invocation.binding.capability_id) {
            return self.inner.invoke(invocation, cancellation).await;
        }
        if cancellation.is_cancelled() {
            return Err(EngineToolError::Cancelled);
        }
        let active = self.active.snapshot().map_err(rejected)?;
        if invocation.principal != self.frozen.principal
            || invocation.agent_session_id != self.frozen.session
            || invocation.resolved_snapshot_ref != *self.snapshot.snapshot_ref()
            || active.resolved_snapshot_ref != invocation.resolved_snapshot_ref
            || active.generation != invocation.active_set_generation
            || !active.active.contains(&invocation.binding.capability_id)
            || invocation.call.name != invocation.binding.model_name
            || self.frozen.plan.binding(&invocation.call.name) != Some(&invocation.binding)
        {
            return Err(rejected("invocation differs from frozen Session authority"));
        }
        invocation.binding.validate()?;
        nomifun_engine_core::parse_completed_arguments(&invocation.call)?;
        if self
            .frozen
            .retain_active_leases(&active.active)
            .await
            .is_err()
        {
            return Ok(EngineToolResult::text(
                invocation.call.call_id,
                "ROBOT_LIFECYCLE_NOT_READY: no device call dispatched. Activate the selected robot.link (and robot.audio for audio tools), and check the exact device binding/connection before requesting a new operation.",
                true,
            ));
        }
        let result = self
            .frozen
            .delegate
            .invoke(NomiHostDynamicToolInvocation {
                capability_id: invocation.binding.capability_id,
                provider_name: invocation.call.name,
                operation_id: invocation.operation_id,
                idempotency_key: invocation.idempotency_key,
                correlation_id: invocation.correlation_id,
                arguments: invocation.call.arguments,
            })
            .await;
        match result {
            Ok(output) => Ok(EngineToolResult::text(
                invocation.call.call_id,
                serde_json::to_string(&output.0).map_err(rejected)?,
                false,
            )),
            Err(error) if error.code.as_ref() == "HOSTED_EFFECT_UNPROVEN" => Err(rejected(
                "HOSTED_EFFECT_UNPROVEN: inspect owner/device state; do not retry",
            )),
            Err(error) => Ok(EngineToolResult::text(
                invocation.call.call_id,
                format!(
                    "{}: Robot owner rejected or acknowledged a failed request. Do not infer that physical effects are reversible or the device is quiescent.",
                    error.code.as_ref()
                ),
                true,
            )),
        }
    }
}
