//! Live context and resource access under the admitted Conversation turn.
//! Kernel capabilities are fixed; legacy activation journals fail closed.
use std::sync::{Arc, Weak, atomic::Ordering};

use async_trait::async_trait;
use nomifun_agent_kernel::{CompiledSnapshot, SessionCapabilityState};
use nomifun_api_types::RuntimeEngineBinding;
use nomifun_chat_model_broker::ChatCausality;
use nomifun_coding_engine::CodingEngineError;
use nomifun_common::AppError;
use nomifun_db::{SqlitePool, sqlx};

use super::{ActiveTurn, ConversationCodingHost, error};

pub(super) struct HostPort(pub(super) Weak<ConversationCodingHost>);
impl std::fmt::Debug for HostPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConversationContextResourcePort")
    }
}

fn engine_error(value: impl std::fmt::Display) -> CodingEngineError {
    CodingEngineError::InvalidContract(format!("Coding context/resource: {value}"))
}

impl HostPort {
    fn host(&self) -> Result<Arc<ConversationCodingHost>, CodingEngineError> {
        self.0
            .upgrade()
            .ok_or_else(|| engine_error("Session host has shut down"))
    }
}

fn validate_turn(
    host: &ConversationCodingHost,
    turn: &ActiveTurn,
    causality: &ChatCausality,
    generation: u64,
) -> Result<(), CodingEngineError> {
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
    host: &ConversationCodingHost,
    turn: &ActiveTurn,
    causality: &ChatCausality,
) -> Result<(), CodingEngineError> {
    let admitted: (i64,) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM conversations c JOIN conversation_delivery_receipts r ON r.operation_id = c.active_turn_operation_id \
         WHERE c.conversation_id = ? AND c.user_id = ? AND c.status = 'running' AND c.admission_epoch = ? \
         AND c.active_turn_operation_id = ? AND r.conversation_id = c.conversation_id AND r.user_id = c.user_id \
         AND r.kind = 'turn' AND r.status = 'accepted' AND r.message_id = ? \
         AND EXISTS(SELECT 1 FROM conversation_runtime_events e WHERE e.conversation_id = c.conversation_id \
         AND e.turn_operation_id = r.operation_id AND e.model_operation_id = ? AND e.model_claimed = 1))")
        .bind(&host.options.conversation_id).bind(&host.options.user_id).bind(turn.epoch).bind(&turn.operation)
        .bind(&turn.root).bind(causality.operation_id.as_ref()).fetch_one(&host.pool).await.map_err(engine_error)?;
    if admitted.0 != 1 {
        return Err(engine_error(
            "Conversation generation/model operation is no longer admitted",
        ));
    }
    Ok(())
}


#[async_trait]
impl nomifun_coding_engine::CodingLiveContextPort for HostPort {
    async fn read(&self, causality: &ChatCausality, generation: u64)
        -> Result<Option<String>, CodingEngineError>
    {
        let host = self.host()?;
        let _transition = host.capability_transition.lock().await;
        let active = host.active.lock().await;
        let turn = active.as_ref().ok_or_else(|| engine_error("no active context turn"))?;
        validate_turn(&host, turn, causality, generation)?;
        // Context reads precede ModelStepStarted (including compaction), so
        // require the accepted turn, not an already-claimed model operation.
        let (admitted,): (i64,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM conversations c JOIN conversation_delivery_receipts r ON r.operation_id = c.active_turn_operation_id \
             WHERE c.conversation_id = ? AND c.user_id = ? AND c.status = 'running' AND c.admission_epoch = ? \
             AND c.active_turn_operation_id = ? AND r.conversation_id = c.conversation_id AND r.user_id = c.user_id \
             AND r.kind = 'turn' AND r.status = 'accepted' AND r.message_id = ?)")
            .bind(&host.options.conversation_id).bind(&host.options.user_id).bind(turn.epoch)
            .bind(&turn.operation).bind(&turn.root).fetch_one(&host.pool).await.map_err(engine_error)?;
        if admitted != 1 { return Err(engine_error("context turn is no longer admitted")); }
        host.resources.ensure_hosted_effects_settled().await.map_err(engine_error)?;
        host.resources.robot_vision_context(generation).await.map_err(engine_error)
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
            fence(&host, turn, causality).await.map_err(|error| fail(error.to_string()))?;
            if !turn.steering.permits_resource_dispatch() {
                return Err(fail("Resource dispatch paused for queued user input or closed turn".into()));
            }
            host.resources.start_mcp_resource_image(turn.journal.clone(), causality.clone(), generation,
                call_id.to_owned(), request).map_err(|error| fail(error.to_string()))?
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
            fence(&host, turn, causality).await.map_err(|error| fail(error.to_string()))?;
            if !turn.steering.permits_resource_dispatch() {
                return Err(fail("Resource dispatch paused for queued user input or closed turn".into()));
            }
            // Register synchronously under the steering/turn boundary,
            // then release it while the owned remote transaction runs.
            host.resources.start_mcp_resource(turn.journal.clone(), causality.clone(), generation,
                call_id.to_owned(), request).map_err(|error| fail(error.to_string()))?
        };
        task.result().await.map_err(|error| fail(error.to_string()))?
            .map_err(|error| fail(error.to_string()))
    }
}

pub(super) async fn restore(
    pool: &SqlitePool,
    options: &nomifun_ai_agent::types::AgentRuntimeBuildOptions,
    _binding: &RuntimeEngineBinding,
    _compiled: &CompiledSnapshot,
    _state: &SessionCapabilityState,
) -> Result<(), AppError> {
    // Fixed enabled capabilities have no activation history to replay. Keep
    // legacy Sessions unavailable rather than inventing generation zero for
    // their old transitions or migrating their exact Engine build identity.
    let (has_legacy_activation,): (i64,) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM conversation_runtime_events \
         WHERE conversation_id = ? AND json_extract(event_json, '$.event') = 'capabilities_activated')",
    )
    .bind(&options.conversation_id)
    .fetch_one(pool)
    .await
    .map_err(error)?;
    if has_legacy_activation != 0 {
        return Err(error(
            "Session unavailable: legacy capabilities_activated journal is incompatible with fixed enabled_capabilities; automatic replay and exact-build migration are not supported",
        ));
    }
    Ok(())
}
