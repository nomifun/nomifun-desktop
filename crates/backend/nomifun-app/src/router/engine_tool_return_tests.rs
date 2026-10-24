use super::*;
use nomifun_agent_contracts::{PrincipalRef, ResolvedSnapshotRef, StrictJsonValue, canonical_json_bytes};
use nomifun_agent_runtime::{AgentEngineEvent, AgentReconciliationSource};
use nomifun_chat_model_broker::ChatToolCall;
use serde_json::{Value, json};

fn invocation() -> EngineToolInvocation {
    EngineToolInvocation {
        agent_session_id: "0190f5fe-7c00-7a00-8000-000000000002".into(),
        principal: PrincipalRef { principal_kind: "user".into(), principal_id: "owner".into() },
        resolved_snapshot_ref: ResolvedSnapshotRef { snapshot_id: "snapshot".into(), snapshot_digest: "b".repeat(64).into() },
        active_set_generation: 1,
        turn_operation_id: "t".repeat(1024).into(),
        operation_id: "o".repeat(1024).into(),
        idempotency_key: "original-call".into(),
        correlation_id: "original-call".into(),
        call: ChatToolCall { call_id: "c".repeat(256).into(), name: "read_file".into(),
            arguments: StrictJsonValue(json!({"path":"result.txt"})), provider_metadata: None },
        binding: serde_json::from_value(json!({
            "model_name":"read_file", "definition":{"name":"read_file","description":"fixture","input_schema":{}},
            "schema_digest":"a".repeat(64), "canonical_input_schema_ref":"fixture", "capability_contract_digest":"b".repeat(64),
            "capability_id":"workspace.files", "action_id":"workspace.files/read", "resource_binding_ids":[],
            "effect_class":"read_only", "parallel_safe":true
        })).unwrap(),
    }
}

fn native_envelope(event: Value) -> Value {
    json!({"producer_seq":u64::MAX,"event":event})
}

#[test]
fn canonical_result_admission_reserves_the_original_reconciliation_envelope() {
    let invocation = invocation();
    let dispatch = EngineToolDispatchRecord {
        operation_id: invocation.operation_id.as_ref().into(), call_id: invocation.call.call_id.as_ref().into(),
        capability_id: invocation.binding.capability_id.as_ref().into(), action_id: invocation.binding.action_id.as_ref().into(),
        model_name: invocation.call.name.clone(),
    };
    let host_event = |result: &EngineToolResult| json!({"event":"host_tool_settled","operation_id":dispatch.operation_id,
        "call_id":dispatch.call_id,"result":result,"error":null});
    let recovered_event = |result: &EngineToolResult| AgentEngineEvent::ToolOutcomeReconciled {
        step: u16::MAX, result: result.clone(), source: AgentReconciliationSource::OwnerReceipt,
        evidence_event_id: Some(format!("runtime-progress:{}:{}:{}", invocation.agent_session_id.as_ref(),
            invocation.turn_operation_id.as_ref(), u64::MAX)),
        owner_operation_id: Some(invocation.operation_id.clone()),
    };
    let limit = nomifun_agent_session::MAX_SINGLE_PAYLOAD_BYTES;
    let empty = EngineToolResult::text(invocation.call.call_id.clone(), "", false);
    let body_bytes = limit - canonical_json_bytes(&native_envelope(host_event(&empty))).unwrap().len();
    let original = EngineToolResult::text(invocation.call.call_id.clone(), "x".repeat(body_bytes), false);
    original.validate_for(&invocation.call.call_id).unwrap();
    assert_eq!(canonical_json_bytes(&native_envelope(host_event(&original))).unwrap().len(), limit);
    let recovery_bytes = canonical_json_bytes(&native_envelope(serde_json::to_value(recovered_event(&original)).unwrap())).unwrap().len();
    assert!(recovery_bytes > limit, "the owner return fits while its actual recovery envelope does not");

    let (payload, rejection) = settlement_payload(&dispatch, &invocation, &Ok(original.clone()), true);
    assert!(matches!(rejection, Some(ToolReturnRejection { kind: Violation::CanonicalResultTooLarge, .. })));
    let rejected: Value = serde_json::from_str(&payload).unwrap();
    assert!(rejected["result"].is_null());
    assert_eq!(rejected["contract_rejection"]["original_return"]["sha256"],
        serde_json::to_value(nomifun_agent_contracts::digest_bytes(&canonical_json_bytes(&original).unwrap())).unwrap());
    assert!(canonical_json_bytes(&native_envelope(rejected)).unwrap().len() < limit);

    // The last fully recoverable byte is accepted, with no independent excerpt.
    let accepted_bytes = body_bytes - (recovery_bytes - limit);
    let accepted = EngineToolResult::text(invocation.call.call_id.clone(), "x".repeat(accepted_bytes), false);
    let (payload, rejection) = settlement_payload(&dispatch, &invocation, &Ok(accepted.clone()), true);
    assert!(rejection.is_none());
    let recorded: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(serde_json::from_value::<EngineToolResult>(recorded["result"].clone()).unwrap(), accepted);
    assert!(canonical_json_bytes(&native_envelope(recorded)).unwrap().len() <= limit);
    let recovered = recovered_event(&accepted);
    let envelope = native_envelope(serde_json::to_value(&recovered).unwrap());
    let bytes = canonical_json_bytes(&envelope).unwrap();
    assert_eq!(bytes.len(), limit);
    let restored: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::from_value::<AgentEngineEvent>(restored["event"].clone()).unwrap(), recovered);
}
