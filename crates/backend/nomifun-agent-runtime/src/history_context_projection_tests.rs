use super::*;
use crate::{AgentEngine, AgentEngineBuild, EngineBuildId, AgentModelBudget, AgentContextBudget};
use nomifun_agent_contracts::{DigestHex, ResolvedSnapshotRef, StrictJsonValue};
use nomifun_chat_model_broker::{ChatToolResultPart, ChatModelRequest, ChatModelError, ChatFinishReason};
use serde_json::json;

fn binding() -> crate::EngineBinding {
    AgentEngine::new(AgentEngineBuild {
        build_id: EngineBuildId::from("context-projection-build"),
        build_digest: DigestHex::from("a".repeat(64)),
    }).unwrap().bind("session".into(), "runtime".into(), ResolvedSnapshotRef {
        snapshot_id: "snapshot".into(), snapshot_digest: DigestHex::from("b".repeat(64)),
    }).unwrap()
}

fn requirement() -> ChatMessage {
    crate::context_lifecycle::text_message(ChatRole::User, "Continue the accepted task.".into())
}

fn call(name: &str) -> ChatToolCall {
    ChatToolCall { call_id: "original-call".into(), name: name.into(),
        arguments: StrictJsonValue(json!({})), provider_metadata: None }
}

fn result_from(history: &[ChatMessage]) -> AgentToolResult {
    history.iter().flat_map(|message| &message.content).find_map(|part| match part {
        ChatContentPart::ToolResult { call_id, output, is_error } => Some(AgentToolResult {
            call_id: call_id.clone(), output: output.clone(), is_error: *is_error,
        }),
        _ => None,
    }).expect("original tool exchange")
}

fn exchange(call: &ChatToolCall, result: &AgentToolResult) -> Vec<ChatMessage> {
    vec![requirement(), ChatMessage { role: ChatRole::Assistant, provider_round_id: None,
        content: vec![ChatContentPart::ToolCall { call_id: call.call_id.clone(), name: call.name.clone(),
            arguments: call.arguments.clone(), provider_metadata: None }] },
        ChatMessage { role: ChatRole::Tool, provider_round_id: None,
            content: vec![ChatContentPart::ToolResult { call_id: result.call_id.clone(),
                output: result.output.clone(), is_error: result.is_error }] }]
}

#[test]
fn canonical_process_and_diff_results_use_the_same_live_closed_and_checkpoint_model_projection() {
    let source = "ORIGINAL_SOURCE😀\\\"\n".repeat(5000);
    for (name, capability, action, value, is_error) in [
        ("start_process", "workspace.process", "workspace.process/start", json!({
            "process_id":"owned-process", "state":"exited", "exit_code":7, "success":false,
            "output":{"text":source,"next_cursor":source.len(),"retained_bytes":source.len(),"dropped_bytes":0},
            "cleanup":{"reaped":true,"errors":[]}}), true),
        ("diff", "workspace.vcs", "workspace.vcs/diff", json!({
            "patch":source,"staged_patch":source,"unstaged_patch":source,"metadata":{"complete":true}}), false),
    ] {
        let call = call(name);
        let result = AgentToolResult::text(call.call_id.clone(), value.to_string(), is_error);
        let original = result.clone();
        assert!(serde_json::to_vec(&result).unwrap().len() > 32 * 1024);
        let kind = crate::tool_context::ToolContextKind::for_action(action).unwrap();
        let projected = crate::tool_context::project(kind, &result);
        assert!(serde_json::to_vec(&projected).unwrap().len() <= 24 * 1024);
        assert_ne!(projected.output, result.output);
        assert_eq!(result, original, "the context copy cannot rewrite the canonical result");
        let mut events = vec![
            AgentEngineEvent::TurnStarted { binding: binding(), turn_operation_id: "turn".into() },
            AgentEngineEvent::ModelStepStarted { step:1, operation_id:"turn:model:1".into(), correction_source:None },
            AgentEngineEvent::ToolCallCompleted { step:1, call:call.clone() },
            AgentEngineEvent::ToolStarted { step:1, call_id:call.call_id.clone(), capability_id:capability.into(), action_id:action.into() },
            AgentEngineEvent::ToolCompleted { step:1, result:result.clone(), runtime_hold:None },
        ];
        let canonical = events.clone();
        events.push(AgentEngineEvent::TurnCompleted { model_steps:1, finish_reason:ChatFinishReason::Completed });
        let mut closed = Vec::new();
        replay_closed_turn(&mut closed, requirement(), &events).unwrap();
        assert_eq!(result_from(&closed), projected);
        assert_eq!(&events[..canonical.len()], canonical.as_slice());

        events.pop();
        events.push(AgentEngineEvent::ExecutionCheckpointSaved { step:1, revision:1, digest:DigestHex::from("c".repeat(64)) });
        let mut restored = Vec::new();
        replay_into(&mut restored, requirement(), &events, false, Some(1), &BTreeMap::new()).unwrap();
        assert_eq!(result_from(&restored), projected, "cold prefix and live use the same model policy");

        let compacted = crate::compacted_history::prepare_event(500_000, 30_000, "Earlier work retained.".into(),
            &exchange(&call, &projected), &[requirement()], std::slice::from_ref(&call.call_id)).unwrap().unwrap();
        events.pop();
        events.push(compacted);
        events.push(AgentEngineEvent::TurnCompleted { model_steps:1, finish_reason:ChatFinishReason::Completed });
        let mut compacted_history = Vec::new();
        replay_closed_turn(&mut compacted_history, requirement(), &events).unwrap();
        assert_eq!(result_from(&compacted_history), projected);
        assert_eq!(&events[..canonical.len()], canonical.as_slice());
    }
}

#[test]
fn large_child_status_is_reduced_from_original_facts_before_context_and_checkpoint_replay() {
    let snapshot = crate::collaboration::test_snapshot(json!({"executions":[{
        "execution_id":"execution", "sequence":7, "owned_step_ids":["child"], "owned_terminal_step_ids":["child"],
        "children":[{"step_id":"child","attempt_id":"attempt","status":"completed",
            "source":{"agent_session_id":"child-session","turn_operation_id":"child-turn","terminal_event_id":"child-terminal"},
            "terminal_receipt":{"execution_id":"execution","execution_event_sequence":7,"step_id":"child","status":"completed"},
            "workspace_change_sources":[],"result":"EXACT_CHILD_BODY😀\\\"\n".repeat(2400)}]
    }]}));
    let mut initial = crate::AgentCollaborationProgress::default();
    initial.pending_child_executions.insert("execution".into());
    let call = call(crate::collaboration::STATUS);
    let result = AgentToolResult::text(call.call_id.clone(), serde_json::to_string(&snapshot).unwrap(), false);
    assert!(serde_json::to_vec(&result).unwrap().len() > 32 * 1024);
    let mut live = initial.clone();
    live.observe_tool_result(&call, &result, false).unwrap();
    assert!(live.pending_child_executions.is_empty());
    assert!(live.results["execution"].source_digests.contains_key("child"));
    assert!(!serde_json::to_string(&live).unwrap().contains("EXACT_CHILD_BODY"));
    let mut events = vec![
        AgentEngineEvent::TurnStarted { binding:binding(), turn_operation_id:"turn".into() },
        AgentEngineEvent::CollaborationInherited { progress:initial },
        AgentEngineEvent::ModelStepStarted { step:1, operation_id:"turn:model:1".into(), correction_source:None },
        AgentEngineEvent::ToolCallCompleted { step:1, call:call.clone() },
        AgentEngineEvent::ToolCompleted { step:1, result:result.clone(), runtime_hold:None },
        AgentEngineEvent::ExecutionCheckpointSaved { step:1, revision:1, digest:DigestHex::from("c".repeat(64)) },
    ];
    assert_eq!(crate::collaboration::recorded_progress(&events).unwrap().0, live);
    let mut restored = Vec::new();
    replay_into(&mut restored, requirement(), &events, false, Some(1), &BTreeMap::new()).unwrap();
    assert_eq!(result_from(&restored), result);
    events.pop();
    events.push(AgentEngineEvent::TurnCompleted { model_steps:1, finish_reason:ChatFinishReason::Completed });
    let mut closed = Vec::new();
    replay_closed_turn(&mut closed, requirement(), &events).unwrap();
    assert_eq!(result_from(&closed), result);
}

#[test]
fn compaction_authoring_measures_the_complete_native_payload_without_clipping_results() {
    let call = call("read_file");
    let large = AgentToolResult::text(call.call_id.clone(), "完整原文".repeat(100_000), false);
    let messages = exchange(&call, &large);
    let original = messages.clone();
    assert!(crate::compacted_history::capture(&messages, &[requirement()], std::slice::from_ref(&call.call_id)).is_ok(),
        "the reader codec is distinct from the current authoring payload limit");
    assert!(crate::compacted_history::prepare_event(2_000_000, 1_300_000, "summary".into(),
        &messages, &[requirement()], std::slice::from_ref(&call.call_id)).unwrap().is_none());
    assert_eq!(messages, original);

    let fitting = AgentToolResult::text(call.call_id.clone(), "原文😀\n".repeat(5000), false);
    let event = crate::compacted_history::prepare_event(2_000_000, 100_000, "summary".into(),
        &exchange(&call, &fitting), &[requirement()], std::slice::from_ref(&call.call_id)).unwrap().unwrap();
    let native = json!({"producer_seq":u64::MAX,"event":event});
    assert!(serde_json::to_vec(&native).unwrap().len() <= nomifun_agent_contracts::CANONICAL_PAYLOAD_MAX_BYTES as usize);
    let AgentEngineEvent::ContextCompacted { retained_context:Some(items), retained_tool_call_ids, .. } =
        serde_json::from_value(native["event"].clone()).unwrap() else { panic!("compaction event") };
    let restored = crate::compacted_history::restore(&items, &[requirement()], &retained_tool_call_ids).unwrap();
    assert_eq!(result_from(&restored), fitting);
}

struct NeverSummarize;

#[async_trait::async_trait]
impl crate::AgentModelPort for NeverSummarize {
    async fn open_stream(&self, _:ChatModelRequest, _:tokio_util::sync::CancellationToken)
        -> Result<crate::AgentModelStream, ChatModelError> {
        panic!("an unstorable mandatory replacement must be rejected before a model request")
    }
}

#[tokio::test]
async fn mandatory_image_exchange_exceeding_storage_is_rejected_before_summarization() {
    let mut request = nomifun_chat_model_broker::recorded_conformance_fixtures().remove(0).request;
    let call = call("inspect");
    let result = AgentToolResult { call_id:call.call_id.clone(), is_error:false, output:vec![
        ChatToolResultPart::Text { text:"x".repeat(600_000) },
        ChatToolResultPart::Image { media_type:"image/png".into(), data_base64:"eA==".into() },
    ] };
    request.input.messages = exchange(&call, &result);
    let second_id = ToolCallId::from("second-call");
    request.input.messages[1].content.push(ChatContentPart::ToolCall {
        call_id:second_id.clone(), name:"inspect".into(), arguments:StrictJsonValue(json!({})), provider_metadata:None,
    });
    request.input.messages.push(ChatMessage { role:ChatRole::Tool, provider_round_id:None,
        content:vec![ChatContentPart::ToolResult { call_id:second_id, is_error:false,
            output:vec![ChatToolResultPart::Text { text:"y".repeat(600_000) }] }] });
    // Each individual result fits the owner return limit. Their complete
    // mandatory batch still has to fit one canonical compaction payload.
    request.input.messages.insert(1, crate::context_lifecycle::text_message(ChatRole::Assistant, "old context ".repeat(60_000)));
    request.input.max_output_tokens = Some(4096);
    let original = serde_json::to_value(&request.input).unwrap();
    let budget = AgentModelBudget::from_limits(Some(4_000_000), Some(4096)).unwrap();
    let mut lifecycle = crate::context_lifecycle::ContextLifecycle::new(budget, AgentContextBudget::default()).unwrap();
    let error = lifecycle.prepare(&mut request, &[requirement()], &binding(), std::sync::Arc::new(NeverSummarize),
        &crate::NoopAgentEventSink, tokio_util::sync::CancellationToken::new()).await.unwrap_err();
    assert!(matches!(error, AgentEngineError::Compaction(message) if message.contains("canonical payload budget")));
    assert_eq!(serde_json::to_value(&request.input).unwrap(), original);
}
