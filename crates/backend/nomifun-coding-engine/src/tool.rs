//! Compatibility names for Coding; canonical mapping and result contracts live
//! in engine-core and are shared with independently implemented engines.
use crate::error::CodingEngineError;
use async_trait::async_trait;
use nomifun_agent_contracts::{
    AgentSessionId, CorrelationId, DigestHex, IdempotencyKey, OperationId, PrincipalRef,
    ResolvedSnapshotRef, StrictJsonValue,
};
use nomifun_chat_model_broker::{ChatToolCall, ChatToolDefinition, ToolCallId};
use tokio_util::sync::CancellationToken;

pub use nomifun_engine_core::{
    EngineEffectClass as CodingEffectClass, EngineToolBinding as CodingToolBinding,
    EngineToolInvocation as CodingToolInvocation, EngineToolPlan as CodingToolPlan,
    EngineToolResult as CodingToolResult,
};

#[async_trait]
pub trait CodingToolInvoker: Send + Sync {
    async fn invoke(
        &self,
        invocation: CodingToolInvocation,
        cancellation: CancellationToken,
    ) -> Result<CodingToolResult, CodingEngineError>;
}

/// A well-formed call to an unavailable name is a model-correctable request,
/// not a corrupt event stream. Check the exact surface sent for this step,
/// including optional engine controls, before entering ANY batch handler.
/// Reject the whole batch so correction cannot accidentally repeat a known
/// effect that ran alongside the misspelled/unavailable call.
pub(crate) fn reject_unexposed_batch(
    calls: &[ChatToolCall],
    definitions: &[ChatToolDefinition],
) -> Option<Vec<(ToolCallId, Result<CodingToolResult, CodingEngineError>)>> {
    let exposed: std::collections::BTreeSet<_> = definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .collect();
    if calls
        .iter()
        .all(|call| exposed.contains(call.name.as_str()))
    {
        return None;
    }
    Some(calls.iter().map(|call| {
        let reason = if exposed.contains(call.name.as_str()) {
            "Not executed: this batch contains a tool name absent from the current model tool definitions. No calls in this batch ran. Correct the batch before resubmitting with fresh call IDs; earlier batches and their effects are unchanged.".to_owned()
        } else {
            format!("Not executed: tool {:?} is not exposed in the current model tool definitions. No calls in this batch ran. Use an exact advertised tool name and its schema. The Agent's enabled capabilities are frozen for this Session; there is no implicit alias, fallback tool or permission grant. Reconsider the plan before effects, then submit a corrected batch with fresh call IDs.", call.name)
        };
        (call.call_id.clone(), Ok(CodingToolResult::text(call.call_id.clone(), reason, true)))
    }).collect())
}

pub(crate) fn validate_tool_argument_size(arguments: &str) -> Result<(), CodingEngineError> {
    nomifun_engine_core::validate_tool_argument_size(arguments).map_err(Into::into)
}

pub(crate) fn parse_completed_arguments(call: &ChatToolCall) -> Result<String, CodingEngineError> {
    nomifun_engine_core::parse_completed_arguments(call).map_err(Into::into)
}

pub fn input_schema_digest(schema: &StrictJsonValue) -> Result<DigestHex, CodingEngineError> {
    nomifun_engine_core::input_schema_digest(schema).map_err(Into::into)
}

pub(crate) fn invocation_for(
    agent_session_id: AgentSessionId,
    principal: PrincipalRef,
    resolved_snapshot_ref: ResolvedSnapshotRef,
    active_set_generation: u64,
    turn_operation_id: &OperationId,
    call: ChatToolCall,
    binding: &CodingToolBinding,
) -> CodingToolInvocation {
    let call_id = call.call_id.as_ref();
    let operation_id = OperationId::from(format!("{}:tool:{call_id}", turn_operation_id.as_ref()));
    CodingToolInvocation {
        agent_session_id,
        principal,
        resolved_snapshot_ref,
        active_set_generation,
        turn_operation_id: turn_operation_id.clone(),
        operation_id: operation_id.clone(),
        idempotency_key: tool_idempotency_key(operation_id.as_ref()),
        correlation_id: CorrelationId::from(turn_operation_id.as_ref()),
        call,
        binding: binding.clone(),
    }
}

fn tool_idempotency_key(operation_id: &str) -> IdempotencyKey {
    // Owner effect journals accept at most 128 visible ASCII bytes. Preserve
    // the full operation/call identity without forwarding its unbounded text.
    let digest = nomifun_agent_contracts::digest_bytes(operation_id.as_bytes());
    IdempotencyKey::from(format!("coding-tool:{}", digest.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_keys_fit_the_owner_contract_without_truncating_operation_identity() {
        let root = format!("nomi-core-turn:{}:{}:tool:{}", "s".repeat(64), "r".repeat(128), "调用".repeat(128));
        let key = tool_idempotency_key(&root);
        assert!(key.as_ref().len() <= 128);
        assert!(key.as_ref().bytes().all(|byte| byte.is_ascii_graphic()));
        assert_eq!(key, tool_idempotency_key(&root));
        assert_ne!(key, tool_idempotency_key(&format!("{root}-next-call")));
        assert_ne!(key, tool_idempotency_key(&format!("other-turn:{root}")));
    }
}
