//! Dynamic Context consumption through the same frozen Kernel authority as
//! initial Context. No separate registry, resolver, or execution owner.
use super::*;
use nomifun_agent_contracts::{ContextContributionInput, ContextTurnInput};

pub(super) struct NomiTurnContextContributor {
    kernel: Arc<KernelRegistry>,
    compiled: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    owner: PrincipalRef,
    session_id: AgentSessionId,
    scope: ScopeKey,
    capabilities: Vec<CapabilityId>,
}

impl NomiTurnContextContributor {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        kernel: Arc<KernelRegistry>,
        compiled: Arc<CompiledSnapshot>,
        active: Arc<SessionCapabilityState>,
        owner: PrincipalRef,
        session_id: AgentSessionId,
        scope: ScopeKey,
        capabilities: Vec<CapabilityId>,
    ) -> Self {
        Self {
            kernel,
            compiled,
            active,
            owner,
            session_id,
            scope,
            capabilities,
        }
    }
}

#[async_trait]
impl ContextContributor for NomiTurnContextContributor {
    // There is no valid before-turn call without a host-owned source message.
    async fn pre_turn_context(&self) -> Option<String> {
        None
    }

    async fn pre_turn_context_for_turn_result(
        &self,
        turn: &TurnContext,
    ) -> Result<Option<String>, String> {
        let input = ContextContributionInput::BeforeTurn {
            turn: ContextTurnInput {
                source_message_id: turn.source_message_id.clone(),
                text: turn.text.clone(),
                image_media_types: turn.image_media_types.clone(),
            },
        };
        input.validate()?;
        let active = self.active.snapshot().map_err(|error| error.to_string())?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut contributions = Vec::new();
        for capability_id in &self.capabilities {
            let policy = self.compiled.policy(capability_id).ok_or_else(|| {
                format!("Context {} has no frozen policy", capability_id.as_ref())
            })?;
            let operation_id = OperationId::from(format!(
                "nomi-turn-context:{}:{}:{}",
                self.session_id.as_ref(),
                uuid::Uuid::now_v7(),
                capability_id.as_ref(),
            ));
            let result = tokio::time::timeout_at(
                deadline,
                self.kernel.contribute_context_with_input(
                    &self.compiled,
                    &active,
                    CapabilityAccessRequest {
                        principal: self.owner.clone(),
                        session_owner: self.owner.clone(),
                        agent_session_id: self.session_id.clone(),
                        correlation_id: CorrelationId::from(format!(
                            "{}:context",
                            operation_id.as_ref()
                        )),
                        operation_id,
                        capability_id: capability_id.clone(),
                        resource_binding_ids: policy.resource_binding_ids.clone(),
                        state_scope_key: self.scope.clone(),
                        resolved_snapshot_ref: self.compiled.snapshot_ref().clone(),
                        active_set_generation: active.generation,
                    },
                    input.clone(),
                ),
            )
            .await
            .map_err(|_| {
                format!(
                    "Context {} exceeded the shared 5 second deadline",
                    capability_id.as_ref()
                )
            })?
            .map_err(|error| format!("Context {} failed: {error}", capability_id.as_ref()))?;
            if let Some(value) = result.value {
                contributions.push(NomiInitialContextContribution {
                    capability_id: capability_id.clone(),
                    value,
                });
                if canonical_json_bytes(&contributions)
                    .map_err(|error| error.to_string())?
                    .len()
                    > MAX_INITIAL_CAPABILITY_CONTEXT_BYTES
                {
                    return Err("Dynamic capability context exceeds 64 KiB".into());
                }
            }
        }
        if contributions.is_empty() {
            return Ok(None);
        }
        let payload = String::from_utf8(
            canonical_json_bytes(&contributions).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        Ok(Some(format!(
            r#"<nomifun_turn_capability_context format="canonical-json">
{payload}
</nomifun_turn_capability_context>"#
        )))
    }

    fn label(&self) -> &str {
        "nomifun_turn_capability_context"
    }
}
