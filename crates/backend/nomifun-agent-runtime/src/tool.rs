//! Compatibility names for the Runtime; canonical mapping and result contracts live
//! in engine-core and are shared with independently implemented engines.
use crate::error::AgentEngineError;
use crate::AgentExecutionPhase;
use async_trait::async_trait;
use nomifun_agent_contracts::{
    AgentSessionId, CorrelationId, DigestHex, IdempotencyKey, OperationId, PrincipalRef,
    ResolvedSnapshotRef, StrictJsonValue,
};
use nomifun_chat_model_broker::{ChatToolCall, ChatToolDefinition, ToolCallId};
use tokio_util::sync::CancellationToken;

pub use nomifun_engine_core::{
    EngineEffectClass as AgentEffectClass, EngineToolBinding as AgentToolBinding,
    EngineToolInvocation as AgentToolInvocation, EngineToolPlan as AgentToolPlan,
    EngineToolResult as AgentToolResult,
};

#[async_trait]
pub trait AgentToolInvoker: Send + Sync {
    async fn invoke(
        &self,
        invocation: AgentToolInvocation,
        cancellation: CancellationToken,
    ) -> Result<AgentToolResult, AgentEngineError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolBatchDisposition {
    /// Ordinary work/control accounting, including invalid model requests.
    /// This disposition alone never proves that an owner invocation occurred.
    WorkAccounting,
    PhaseCorrection,
}

pub(crate) struct ToolBatchRefusal {
    pub(crate) disposition: ToolBatchDisposition,
    pub(crate) results: Vec<(ToolCallId, Result<AgentToolResult, AgentEngineError>)>,
}

/// Exposure preflight runs before controls, discovery, admission or dispatch.
/// Its typed disposition owns whether the paired refusal enters work accounting.
pub(crate) fn reject_tool_surface_batch(
    calls: &[ChatToolCall],
    definitions: &[ChatToolDefinition],
    execution_surface: &std::collections::BTreeSet<String>,
    phase: AgentExecutionPhase,
) -> Option<ToolBatchRefusal> {
    if phase != AgentExecutionPhase::Execution
        && let Some(results) = report_only_refusal(calls, definitions, execution_surface, phase)
    {
        return Some(ToolBatchRefusal { disposition: ToolBatchDisposition::PhaseCorrection, results });
    }
    reject_unexposed_batch(calls, definitions).map(|results| ToolBatchRefusal {
        disposition: ToolBatchDisposition::WorkAccounting, results,
    })
}

/// A well-formed call to an unavailable name is a model-correctable request,
/// not a corrupt event stream. Check the exact surface sent for this step,
/// including optional engine controls, before entering ANY batch handler.
/// Reject the whole batch so correction cannot accidentally repeat a known
/// effect that ran alongside the misspelled/unavailable call.
pub(crate) fn reject_unexposed_batch(
    calls: &[ChatToolCall],
    definitions: &[ChatToolDefinition],
) -> Option<Vec<(ToolCallId, Result<AgentToolResult, AgentEngineError>)>> {
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
        (call.call_id.clone(), Ok(AgentToolResult::text(call.call_id.clone(), reason, true)))
    }).collect())
}

/// A report-only surface is a host phase restriction, not lost authority or a
/// failed execution. Recognize only exact names exposed before that restriction;
/// unknown names still take the ordinary unavailable-tool refusal above.
fn report_only_refusal(
    calls: &[ChatToolCall],
    definitions: &[ChatToolDefinition],
    execution_surface: &std::collections::BTreeSet<String>,
    phase: AgentExecutionPhase,
) -> Option<Vec<(ToolCallId, Result<AgentToolResult, AgentEngineError>)>> {
    if !calls.iter().all(|call| execution_surface.contains(&call.name))
        || calls.iter().all(|call| definitions.iter().any(|tool| tool.name == call.name))
    {
        return None;
    }
    Some(calls.iter().map(|call| {
        let reason = serde_json::json!({
            "status":"not_executed",
            "code":"REPORT_ONLY_ACTION_CLOSED",
            "phase":match phase { AgentExecutionPhase::DeliveryReview => "delivery_review", _ => "completion_review" },
            "tool":call.name,
            "message":"Only this newly proposed batch was held before dispatch because completion review is report-only. No call in this new batch ran. This is a phase restriction, not a missing capability or a command failure. Earlier calls in this same accepted turn retain their original receipts and invocation status; they were not erased, cancelled, or moved into an earlier turn. Determine whether original work ran from its original receipts, never from this refused new batch. Submit report_completion alone with a fresh call ID, using the original observations and unchanged runtime counts. Do not repeat settled actions, discover capabilities, or reopen the plan to repair this account. Missing or uncertain evidence and valid blockers remain unresolved."
        }).to_string();
        (call.call_id.clone(), Ok(AgentToolResult::text(call.call_id.clone(), reason, true)))
    }).collect())
}

pub(crate) fn validate_tool_argument_size(arguments: &str) -> Result<(), AgentEngineError> {
    nomifun_engine_core::validate_tool_argument_size(arguments).map_err(Into::into)
}

pub(crate) fn parse_completed_arguments(call: &ChatToolCall) -> Result<String, AgentEngineError> {
    nomifun_engine_core::parse_completed_arguments(call).map_err(Into::into)
}

pub fn input_schema_digest(schema: &StrictJsonValue) -> Result<DigestHex, AgentEngineError> {
    nomifun_engine_core::input_schema_digest(schema).map_err(Into::into)
}

pub(crate) fn invocation_for(
    agent_session_id: AgentSessionId,
    principal: PrincipalRef,
    resolved_snapshot_ref: ResolvedSnapshotRef,
    active_set_generation: u64,
    turn_operation_id: &OperationId,
    call: ChatToolCall,
    binding: &AgentToolBinding,
) -> AgentToolInvocation {
    let call_id = call.call_id.as_ref();
    let operation_id = OperationId::from(format!("{}:tool:{call_id}", turn_operation_id.as_ref()));
    AgentToolInvocation {
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
    IdempotencyKey::from(format!("agent-tool:{}", digest.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_phase_refusal_is_atomic_and_does_not_classify_unknown_tools_as_authorized() {
        let definitions = vec![crate::completion::definition()];
        let surface = std::collections::BTreeSet::from([
            "ssh_exec".to_owned(), crate::completion::TOOL_NAME.to_owned(),
        ]);
        let call = |id: &str, name: &str| ChatToolCall {
            call_id: id.into(), name: name.into(), arguments: StrictJsonValue(serde_json::json!({})),
            provider_metadata: None,
        };
        let calls = [call("repeat", "ssh_exec"), call("account", crate::completion::TOOL_NAME)];
        let rejected = reject_tool_surface_batch(&calls, &definitions, &surface, AgentExecutionPhase::CompletionReview).unwrap();
        assert_eq!(rejected.disposition, ToolBatchDisposition::PhaseCorrection);
        assert_eq!(rejected.results.len(), 2, "the mixed report cannot be accepted alongside a held action");
        for (_, result) in rejected.results {
            let result = result.unwrap();
            assert!(result.is_error);
            let payload: serde_json::Value = serde_json::from_str(&result.output_text()).unwrap();
            assert_eq!(payload["code"], "REPORT_ONLY_ACTION_CLOSED");
            assert!(payload["message"].as_str().unwrap().contains("same accepted turn"));
        }
        let unknown = [call("unknown", "invented_ssh")];
        assert_eq!(reject_tool_surface_batch(&unknown, &definitions, &surface, AgentExecutionPhase::CompletionReview).unwrap().disposition,
            ToolBatchDisposition::WorkAccounting);
        assert_eq!(reject_tool_surface_batch(&calls, &definitions, &surface, AgentExecutionPhase::Execution).unwrap().disposition,
            ToolBatchDisposition::WorkAccounting, "the schema shape cannot choose the host phase");
        assert!(reject_tool_surface_batch(&calls[1..], &definitions, &surface, AgentExecutionPhase::CompletionReview).is_none());
        let delivery = reject_tool_surface_batch(&calls, &definitions, &surface, AgentExecutionPhase::DeliveryReview).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&delivery.results[0].1.as_ref().unwrap().output_text()).unwrap();
        assert_eq!(payload["phase"], "delivery_review");
    }

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
