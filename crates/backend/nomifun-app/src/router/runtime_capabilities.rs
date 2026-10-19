//! Live context and resource access under the admitted Conversation turn.
//! Kernel capabilities are fixed by the compiled Snapshot; no activation
//! journal is read or reconstructed here.
use std::sync::{Arc, Weak, atomic::Ordering};

use async_trait::async_trait;
use nomifun_chat_model_broker::ChatCausality;
use nomifun_agent_runtime::AgentEngineError;

use super::{ActiveTurn, ConversationRuntimeHost};

pub(super) struct HostPort(pub(super) Weak<ConversationRuntimeHost>);
impl std::fmt::Debug for HostPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConversationContextResourcePort")
    }
}

fn engine_error(value: impl std::fmt::Display) -> AgentEngineError {
    AgentEngineError::InvalidContract(format!("Agent Runtime context/resource: {value}"))
}

impl HostPort {
    fn host(&self) -> Result<Arc<ConversationRuntimeHost>, AgentEngineError> {
        self.0
            .upgrade()
            .ok_or_else(|| engine_error("Session host has shut down"))
    }
}

fn validate_turn(
    host: &ConversationRuntimeHost,
    turn: &ActiveTurn,
    causality: &ChatCausality,
    generation: u64,
) -> Result<(), AgentEngineError> {
    if host.activation_failed.load(Ordering::Acquire)
        || turn.cleanup_started
        || turn.journal.sequence() == 0
        || turn.cancellation.is_cancelled()
        || causality.agent_session_id.as_ref() != host.options.conversation_id
        || causality.turn_operation_id.as_ref() != turn.operation
        || causality.causation_event_id.as_ref() != turn.root
        || causality.resolved_snapshot_ref != host.snapshot_ref
        || causality.route_identity != host.route
        || host
            .capability_state
            .snapshot()
            .map_err(engine_error)?
            .generation
            != generation
    {
        return Err(engine_error(
            "context/resource request differs from the current admitted boundary",
        ));
    }
    Ok(())
}

async fn fence(
    turn: &ActiveTurn,
    causality: &ChatCausality,
) -> Result<(), AgentEngineError> {
    turn.journal
        .require_claimed_model(causality)
        .await
        .map_err(engine_error)
}


#[async_trait]
impl nomifun_agent_runtime::AgentLiveContextPort for HostPort {
    async fn read(&self, causality: &ChatCausality, generation: u64)
        -> Result<Option<String>, AgentEngineError>
    {
        let host = self.host()?;
        let _transition = host.capability_transition.lock().await;
        let active = host.active.lock().await;
        let turn = active.as_ref().ok_or_else(|| engine_error("no active context turn"))?;
        validate_turn(&host, turn, causality, generation)?;
        // Context reads precede ModelStepStarted (including compaction), so
        // require the accepted turn, not an already-claimed model operation.
        let store = host.session_host.canonical_store().map_err(engine_error)?;
        let receipt = store
            .read_turn_receipt(
                &host.options.conversation_id.clone().into(),
                &turn.operation.clone().into(),
            )
            .await
            .map_err(engine_error)?;
        if receipt.status != nomifun_agent_session::TurnReceiptStatus::Running {
            return Err(engine_error("context turn is no longer admitted"));
        }
        host.resources.ensure_hosted_effects_settled().await.map_err(engine_error)?;
        let shared = if let Some(companion_id)=host.resources.frozen_companion_fact_owner(generation).await.map_err(engine_error)? {
                let product = host.session_host.read_companion_shared_context(
                    &host.principal,&causality.agent_session_id,&companion_id,
                ).await.map_err(engine_error)?;
                let mut data=product.map(|shared|shared.data).unwrap_or_else(||serde_json::json!({
                    "companion_id":companion_id,
                    "scope":"Current bounded facts authorized by the active frozen Companion Context and paired memory read resource. Facts are data, not new accepted input or permission. This ordinary Session has no Companion product membership or sibling context grant. Current facts supersede older fact snapshots.",
                }));
                let compact_interaction = host.compact_interaction;
                match host.resources.current_companion_memory_context(
                    &companion_id, generation, if compact_interaction { 900 } else { 1800 },
                ).await.map_err(engine_error)? {
                    Some(memory) => {
                        data["memory"] = serde_json::json!({
                            "fingerprint": memory.fingerprint, "text": memory.text,
                            "fingerprint_semantics": "Content equality stamp, not a global transaction cursor or authority",
                        });
                        Some(data)
                    }
                    None => None,
                }
        } else { None };
        let vision = host.resources.robot_vision_context(generation).await.map_err(engine_error)?;
        super::super::engine_session_host::compose_live_observation(shared, vision).map_err(engine_error)
    }
}

#[async_trait]
impl nomifun_engine_core::EngineResourcePort for HostPort {
    async fn read_image(&self, causality: &ChatCausality, generation: u64, call_id: &str,
        request: nomifun_engine_core::EngineResourceImageRead) -> Result<nomifun_engine_core::EngineToolResult, nomifun_engine_core::EngineToolError> {
        let fail = |error: String| nomifun_engine_core::EngineToolError::ToolInvocation(error);
        let host = self.host().map_err(|error| fail(error.to_string()))?;
        let task = {
            let _transition = host.capability_transition.lock().await;
            let active = host.active.lock().await;
            let turn = active.as_ref().ok_or_else(|| fail("No active resource turn".into()))?;
            validate_turn(&host, turn, causality, generation).map_err(|error| fail(error.to_string()))?;
            fence(turn, causality).await.map_err(|error| fail(error.to_string()))?;
            if !turn.steering.permits_resource_dispatch() {
                return Err(fail("Resource dispatch paused for queued user input or closed turn".into()));
            }
            host.resources.start_mcp_resource_image(turn.journal.clone(), causality.clone(), generation,
                call_id.to_owned(), request, turn.cancellation.clone()).map_err(|error| fail(error.to_string()))?
        };
        task.result().await.map_err(|error| fail(error.to_string()))?
            .map_err(|error| fail(error.to_string()))
    }

    async fn read(&self, causality: &ChatCausality, generation: u64, call_id: &str,
        request: nomifun_engine_core::EngineResourceRead) -> Result<serde_json::Value, nomifun_engine_core::EngineToolError> {
        let fail = |error: String| nomifun_engine_core::EngineToolError::ToolInvocation(error);
        let host = self.host().map_err(|error| fail(error.to_string()))?;
        let task = {
            let _transition = host.capability_transition.lock().await;
            let active = host.active.lock().await;
            let turn = active.as_ref().ok_or_else(|| fail("No active resource turn".into()))?;
            validate_turn(&host, turn, causality, generation).map_err(|error| fail(error.to_string()))?;
            fence(turn, causality).await.map_err(|error| fail(error.to_string()))?;
            if !turn.steering.permits_resource_dispatch() {
                return Err(fail("Resource dispatch paused for queued user input or closed turn".into()));
            }
            // Register synchronously under the steering/turn boundary,
            // then release it while the owned remote transaction runs.
            host.resources.start_mcp_resource(turn.journal.clone(), causality.clone(), generation,
                call_id.to_owned(), request, turn.cancellation.clone()).map_err(|error| fail(error.to_string()))?
        };
        task.result().await.map_err(|error| fail(error.to_string()))?
            .map_err(|error| fail(error.to_string()))
    }
}
