//! Unified Runtime adapter for the one frozen ToolSearch policy.

use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, AgentSessionId, CorrelationId, IdempotencyKey,
    OperationId, PrincipalRef, ResolvedCapability, ScopeKey, StrictJsonValue,
};
use nomifun_agent_kernel::{
    CapabilityInvocationRequest, CompiledSnapshot, KernelRegistry, SessionCapabilityState,
};
use nomifun_agent_runtime::{
    AgentEngineError, AgentToolDiscoveryCandidate, AgentToolDiscoveryPort,
};
use nomifun_chat_model_broker::ChatCausality;
use nomifun_common::AppError;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;


fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Engine ToolSearch: {message}"))
}

pub(crate) fn port(
    kernel: Arc<KernelRegistry>,
    snapshot: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    principal: PrincipalRef,
    session: AgentSessionId,
) -> Result<Option<Arc<dyn AgentToolDiscoveryPort>>, AppError> {
    let selected = snapshot
        .content()
        .contributions()
        .filter(|capability| {
            capability.actions.iter().any(|action| {
                action.action_id.as_ref() == nomifun_ai_agent::tool_discovery::ACTION_ID
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    let capability = match selected.as_slice() {
        [] => return Ok(None),
        [capability] => capability.clone(),
        _ => return Err(failure("multiple discovery policies are selected")),
    };
    capability
        .validate()
        .map_err(|error| failure(error.message))?;
    if capability.actions != [nomifun_ai_agent::tool_discovery::action()]
        || !capability.required_resource_kinds.is_empty()
        || (!capability.action_allowlist.is_empty()
            && !capability.action_allowlist.contains(&ActionId::from(
                nomifun_ai_agent::tool_discovery::ACTION_ID,
            )))
    {
        return Err(failure(
            "selected policy does not freeze the exact hidden discovery Action",
        ));
    }
    Ok(Some(Arc::new(Port {
        kernel,
        snapshot,
        active,
        principal,
        session,
        capability,
    })))
}

struct Port {
    kernel: Arc<KernelRegistry>,
    snapshot: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    principal: PrincipalRef,
    session: AgentSessionId,
    capability: ResolvedCapability,
}

impl std::fmt::Debug for Port {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EngineToolDiscoveryPort")
            .field("session", &self.session)
            .field("capability", &self.capability.capability.id)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    names: Vec<String>,
}

#[async_trait]
impl AgentToolDiscoveryPort for Port {
    async fn select(
        &self,
        causality: &ChatCausality,
        active_set_generation: u64,
        query: &str,
        candidates: &[AgentToolDiscoveryCandidate],
        limit: usize,
        cancellation: CancellationToken,
    ) -> Result<Vec<String>, AgentEngineError> {
        if cancellation.is_cancelled() {
            return Err(AgentEngineError::Cancelled);
        }
        let active = self
            .active
            .snapshot()
            .map_err(|error| AgentEngineError::ToolInvocation(error.to_string()))?;
        if self.principal.principal_kind != "user"
            || causality.agent_session_id != self.session
            || causality.resolved_snapshot_ref != *self.snapshot.snapshot_ref()
            || active.resolved_snapshot_ref != causality.resolved_snapshot_ref
            || active.generation != active_set_generation
            || !active.active.contains(&self.capability.capability.id)
            || !(1..=nomifun_agent_runtime::MAX_TOOL_DISCOVERY_MATCHES).contains(&limit)
        {
            return Err(AgentEngineError::ToolInvocation(
                "ToolSearch differs from the frozen AgentSession authority".into(),
            ));
        }
        let payload = StrictJsonValue(serde_json::json!({
            "query": query,
            "candidates": candidates,
            "limit": limit,
        }));
        if nomifun_agent_contracts::canonical_json_bytes(&payload.0)
            .map_err(|error| AgentEngineError::ToolInvocation(error.to_string()))?
            .len()
            > 256 * 1024
        {
            return Err(AgentEngineError::ToolInvocation(
                "ToolSearch candidate metadata exceeds 256 KiB".into(),
            ));
        }
        let action = ActionId::from(nomifun_ai_agent::tool_discovery::ACTION_ID);
        let operation = OperationId::from(format!(
            "{}:tool-search:{}",
            causality.operation_id.as_ref(),
            self.capability.capability.id.as_ref()
        ));
        let policy = self.snapshot.policy(&self.capability.capability.id).ok_or_else(|| {
            AgentEngineError::ToolInvocation("ToolSearch policy is unavailable".into())
        })?;
        let identity = operation.as_ref().to_owned();
        let request = CapabilityInvocationRequest {
            principal: self.principal.clone(),
            session_owner: self.principal.clone(),
            agent_session_id: self.session.clone(),
            turn_id: causality.turn_operation_id.clone(),
            operation_id: operation,
            idempotency_key: IdempotencyKey::from(identity.clone()),
            correlation_id: CorrelationId::from(identity),
            resolved_snapshot_ref: causality.resolved_snapshot_ref.clone(),
            active_set_generation,
            capability_id: self.capability.capability.id.clone(),
            action_id: action,
            resource_binding_ids: policy.resource_binding_ids.clone(),
            state_scope_key: ScopeKey::from(format!("session:{}", self.session.as_ref())),
            input: payload,
        };
        let output = tokio::select! {
            _ = cancellation.cancelled() => return Err(AgentEngineError::Cancelled),
            result = self.kernel.invoke_shared(self.snapshot.clone(), &active, request) => {
                result.map_err(|error| AgentEngineError::ToolInvocation(
                    format!("Selected discovery policy failed: {error}"),
                ))?
            }
        };
        let selection: Selection = serde_json::from_value(output.0).map_err(|_| {
            AgentEngineError::ToolInvocation(
                "Selected discovery policy returned an invalid response".into(),
            )
        })?;
        Ok(selection.names)
    }
}
