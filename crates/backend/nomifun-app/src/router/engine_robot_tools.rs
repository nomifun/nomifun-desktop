//! Frozen `robot` Module tools for source-integrated engines.
//!
//! Device discovery, permissions, resource identity, physical effects and
//! revocation remain application-owned. The engine sees one Module and exact
//! slash Action bindings; connection/audio lifecycle never enters its grant
//! set.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, AgentSessionId, CapabilityId, ContributionSourceKind, PrincipalRef,
};
use nomifun_agent_kernel::{CompiledSnapshot, SessionCapabilityState};
use nomifun_ai_agent::NomiHostDynamicToolInvocation;
use nomifun_common::AppError;
use nomifun_engine_core::{
    EngineToolError, EngineToolExposure, EngineToolInvocation, EngineToolInvoker, EngineToolPlan,
    EngineToolResult,
};
use nomifun_robot::capability::ROBOT_MODULE_ID;
use tokio_util::sync::CancellationToken;

use super::hosted_effect_receipts::{HostedEffectReceipts, RobotReceiptInvoker};
use super::nomi_core_robot::{
    ResolvedRobotSessionTools, RobotModuleOwner, action_ids, bound_robot_resource,
    module_capability_id,
};

pub(crate) fn supported_ids() -> BTreeSet<CapabilityId> {
    BTreeSet::from([module_capability_id()])
}

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Engine Robot tools: {message}"))
}

fn rejected(message: impl std::fmt::Display) -> EngineToolError {
    EngineToolError::ToolInvocation(format!("Robot authority: {message}"))
}

pub(crate) struct FrozenTools {
    pub plan: EngineToolPlan,
    resolved: ResolvedRobotSessionTools,
    delegate: Arc<dyn nomifun_ai_agent::NomiHostDynamicToolInvoker>,
    principal: PrincipalRef,
    session: AgentSessionId,
    provider_actions: Arc<BTreeMap<String, ActionId>>,
}

impl FrozenTools {
    /// Read-only snapshot/resource projection followed by exact live device
    /// discovery. No connection is created and no background service is
    /// activated while the Session is constructed.
    pub(crate) async fn resolve(
        owner: Arc<RobotModuleOwner>,
        receipts: HostedEffectReceipts,
        principal: PrincipalRef,
        session: AgentSessionId,
        snapshot: &CompiledSnapshot,
        compile: impl FnOnce(Vec<EngineToolExposure>) -> Result<EngineToolPlan, AppError>,
    ) -> Result<Self, AppError> {
        let resource = bound_robot_resource(
            snapshot.resource_bindings(),
            &principal.principal_id,
        )
        .map_err(failure)?;
        let module_id = module_capability_id();
        let selected = snapshot
            .resolved_capability(&module_id)
            .ok_or_else(|| failure("robot Module is not present in the compiled Snapshot"))?;
        let policy = snapshot
            .policy(&module_id)
            .ok_or_else(|| failure("robot Module policy is missing"))?;
        if selected.contribution_lock.source_kind != ContributionSourceKind::PlatformBuiltin
            || !policy.resource_binding_ids.contains(&resource.binding_id)
        {
            return Err(failure(
                "robot Module requires its bundled owner and exact resource policy",
            ));
        }
        let declared = action_ids();
        if policy.allowed_actions.is_empty()
            || policy
                .allowed_actions
                .iter()
                .any(|action| !declared.contains(action))
        {
            return Err(failure(
                "robot Module contains an empty or undeclared Action grant",
            ));
        }

        let resolved = owner
            .resolve_session_tools(
                &principal,
                &session,
                resource,
                &policy.allowed_actions,
            )
            .await
            .map_err(failure)?;
        let mut provider_actions = BTreeMap::new();
        let exposures = resolved
            .descriptors
            .iter()
            .cloned()
            .map(|descriptor| {
                if provider_actions
                    .insert(
                        descriptor.provider_name.clone(),
                        descriptor.action_id.clone(),
                    )
                    .is_some()
                {
                    return Err(failure(format!(
                        "duplicate Robot provider tool {}",
                        descriptor.provider_name
                    )));
                }
                Ok(EngineToolExposure {
                    definition: nomifun_chat_model_broker::ChatToolDefinition {
                        name: descriptor.provider_name,
                        description: format!(
                            "{}\nPhysical Robot action. Device permission and the exact live binding are rechecked before dispatch; a reply is not proof of reversibility or physical quiescence.",
                            descriptor.description
                        ),
                        input_schema: descriptor.input_schema,
                        deferred: false,
                    },
                    action_id: descriptor.action_id,
                    capability_id: module_id.clone(),
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        let provider_actions = Arc::new(provider_actions);
        let delegate: Arc<dyn nomifun_ai_agent::NomiHostDynamicToolInvoker> =
            Arc::new(RobotReceiptInvoker {
                receipts,
                user: principal.principal_id.clone(),
                session: session.as_ref().to_owned(),
                provider_actions: Arc::clone(&provider_actions),
                delegate: Arc::clone(&resolved.invoker),
            });
        Ok(Self {
            plan: compile(exposures)?,
            resolved,
            delegate,
            principal,
            session,
            provider_actions,
        })
    }

    /// Revoke this Session's local authority. The shared physical connection
    /// remains owned by the Robot domain and is not disconnected here.
    pub(crate) fn close(&self) -> Result<(), AppError> {
        self.resolved.revoke();
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
        if invocation.binding.capability_id.as_ref() != ROBOT_MODULE_ID {
            return self.inner.invoke(invocation, cancellation).await;
        }
        if cancellation.is_cancelled() {
            return Err(EngineToolError::Cancelled);
        }
        let active = self.active.snapshot().map_err(rejected)?;
        let expected_action = self
            .frozen
            .provider_actions
            .get(&invocation.call.name)
            .ok_or_else(|| rejected("tool was not frozen into this AgentSession"))?;
        if invocation.principal != self.frozen.principal
            || invocation.agent_session_id != self.frozen.session
            || invocation.resolved_snapshot_ref != *self.snapshot.snapshot_ref()
            || active.resolved_snapshot_ref != invocation.resolved_snapshot_ref
            || active.generation != invocation.active_set_generation
            || !active.active.contains(&module_capability_id())
            || invocation.binding.action_id != *expected_action
            || invocation.call.name != invocation.binding.model_name
            || self.frozen.plan.binding(&invocation.call.name) != Some(&invocation.binding)
        {
            return Err(rejected("invocation differs from frozen Session authority"));
        }
        invocation.binding.validate()?;
        nomifun_engine_core::parse_completed_arguments(&invocation.call)?;
        let result = self
            .frozen
            .delegate
            .invoke(NomiHostDynamicToolInvocation {
                capability_id: module_capability_id(),
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
            Err(error) if error.code.as_ref() == "ROBOT_OFFLINE" => Ok(EngineToolResult::text(
                invocation.call.call_id,
                "ROBOT_OFFLINE: no device call was dispatched. Connect the bound robot to this desktop before requesting a new operation.",
                true,
            )),
            Err(error) if error.code.as_ref() == "ROBOT_PERMISSION_DENIED" => {
                Ok(EngineToolResult::text(
                    invocation.call.call_id,
                    format!("ROBOT_PERMISSION_DENIED: {}", error.internal_message),
                    true,
                ))
            }
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
