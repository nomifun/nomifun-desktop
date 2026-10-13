use super::*;
use crate::{AgentExecutionCheckpoint, AgentModelResponseRejection, AgentRejectedToolProposal, AgentTurnRecovery, EngineBinding};
use nomifun_agent_contracts::{ResolvedSnapshotRef, StrictJsonValue};
use nomifun_chat_model_broker::{ChatFinishReason, ToolArgumentDiagnostic, ToolArgumentDiagnosticCategory};

fn binding() -> EngineBinding {
    EngineBinding::new("session".into(), "binding".into(), "build".into(), "a".repeat(64).into(),
        ResolvedSnapshotRef { snapshot_id: "snapshot".into(), snapshot_digest: "b".repeat(64).into() }).unwrap()
}

fn start(step: u16, source: Option<u16>) -> AgentEngineEvent {
    AgentEngineEvent::ModelStepStarted { step, operation_id: format!("turn:model:{step}").into(), correction_source: source }
}

fn rejected() -> AgentEngineEvent {
    crate::protocol_recovery::ProtocolRecovery::default().prepare_rejection(1, vec!["bad-json".into()],
        AgentModelResponseRejection::ToolArguments { diagnostics: vec![AgentRejectedToolProposal {
            call_id: "bad-json".into(), name: "apply_patch".into(), diagnostic: ToolArgumentDiagnostic {
                category: ToolArgumentDiagnosticCategory::Syntax, line: Some(3), column: Some(11),
                byte_len: 41, digest: "d".repeat(64).into() } }] }, None, true).unwrap().event().clone()
}

fn requirement() -> ChatMessage { crate::context_lifecycle::text_message(ChatRole::User, "original-task".into()) }

fn rejection_prefix() -> Vec<AgentEngineEvent> {
    vec![AgentEngineEvent::TurnStarted { binding: binding(), turn_operation_id: "turn".into() }, start(1, None),
        AgentEngineEvent::ToolCallDelta { step: 1, call_id: "bad-json".into(), name: "apply_patch".into(), arguments_delta: String::new() }, rejected()]
}

fn completed_correction() -> Vec<AgentEngineEvent> {
    let mut events = rejection_prefix();
    let call = ChatToolCall { call_id: "corrected-write".into(), name: "write_file".into(),
        arguments: StrictJsonValue(serde_json::json!({"path":"answer.txt","content":"corrected"})), provider_metadata: None };
    events.extend([start(2, Some(1)), AgentEngineEvent::ToolCallCompleted { step: 2, call: call.clone() },
        AgentEngineEvent::ModelResponseValidated { step: 2 }, AgentEngineEvent::ToolStarted { step: 2, call_id: call.call_id.clone(),
            capability_id: "workspace.files".into(), action_id: "workspace.files/write".into() },
        AgentEngineEvent::ToolCompleted { step: 2, result: AgentToolResult::text(call.call_id.clone(), "REAL_OWNER_OUTPUT", false) },
        AgentEngineEvent::ToolResultsOrdered { step: 2, call_ids: vec![call.call_id] },
        AgentEngineEvent::TurnCompleted { model_steps: 2, finish_reason: ChatFinishReason::Completed }]);
    events
}

#[test]
fn corrected_native_call_preserves_exact_rejection_feedback_without_fabricated_pair() {
    let mut events = completed_correction();
    events.pop();
    events.extend([start(3, None), AgentEngineEvent::OutputTextDelta { step: 3, text: "final public answer".into() },
        AgentEngineEvent::TurnCompleted { model_steps: 3, finish_reason: ChatFinishReason::Completed }]);
    let mut history = vec![];
    replay_closed_turn(&mut history, requirement(), &events).unwrap();
    let exact_feedback = crate::protocol_recovery::rejection_feedback(&events[3]).unwrap();
    assert!(history.contains(&exact_feedback));
    let encoded = serde_json::to_string(&history).unwrap();
    assert!(encoded.contains("original-task") && encoded.contains("REAL_OWNER_OUTPUT"));
    assert!(!history.iter().flat_map(|message| &message.content).any(|part| matches!(part,
        ChatContentPart::ToolCall { call_id, .. } | ChatContentPart::ToolResult { call_id, .. } if call_id.as_ref() == "bad-json")));
}

#[test]
fn rejection_cannot_hide_admission_or_claim_validation_after_an_effect() {
    let mut events = rejection_prefix();
    events.insert(3, AgentEngineEvent::ToolStarted { step: 1, call_id: "bad-json".into(), capability_id: "workspace.files".into(), action_id: "workspace.files/patch".into() });
    events.push(AgentEngineEvent::TurnFailed { model_steps: 1, message: "failed".into(), failure: None });
    assert!(replay_closed_turn(&mut vec![], requirement(), &events).is_err());
    let mut events = completed_correction();
    let at = events.iter().position(|event| matches!(event, AgentEngineEvent::ModelResponseValidated { .. })).unwrap();
    let validated = events.remove(at); events.insert(at + 1, validated);
    assert!(replay_closed_turn(&mut vec![], requirement(), &events).is_err());
}

#[test]
fn forged_or_missing_correction_source_and_duplicate_verdicts_reject_history() {
    let mut events = completed_correction();
    if let AgentEngineEvent::ModelStepStarted { correction_source, .. } = &mut events[4] { *correction_source = Some(99); }
    assert!(replay_closed_turn(&mut vec![], requirement(), &events).is_err());
    let mut events = completed_correction();
    if let AgentEngineEvent::ModelStepStarted { correction_source, .. } = &mut events[4] { *correction_source = None; }
    assert!(replay_closed_turn(&mut vec![], requirement(), &events).is_err());
    let mut events = completed_correction(); events.insert(7, AgentEngineEvent::ModelResponseValidated { step: 2 });
    assert!(replay_closed_turn(&mut vec![], requirement(), &events).is_err());
}

#[test]
fn existing_current_generation_unclassified_rejection_remains_readable() {
    let mut events = rejection_prefix();
    events[3] = AgentEngineEvent::ModelResponseRejected { step: 1, discarded_tool_call_ids: vec!["bad-json".into()],
        continuation: false, tool_hint: None, rejection: None, feedback: None };
    events.push(AgentEngineEvent::TurnFailed { model_steps: 1, message: "existing-failure".into(), failure: None });
    let mut history = vec![]; replay_closed_turn(&mut history, requirement(), &events).unwrap();
    let encoded = serde_json::to_string(&history).unwrap();
    assert!(encoded.contains("did not record a more precise reason")); assert!(!encoded.contains("emitted tool-call markup"));
}

fn checkpoint() -> AgentExecutionCheckpoint {
    AgentExecutionCheckpoint { version: 1, binding: binding(), turn_operation_id: "turn".into(), active_set_generation: 0,
        model_steps: 0, tool_call_count: 0, accepted_input_count: 1, applied_steering_receipts: vec![], plan: Default::default(),
        work: Default::default(), patch_recovery: Default::default(), segments: None, control_rejections: Default::default(), completion_review: Default::default() }
}

#[test]
fn pending_canonical_tail_feedback_survives_recovery_without_retaining_carrier() {
    let checkpoint = checkpoint();
    let prefix = vec![AgentEngineEvent::TurnStarted { binding: binding(), turn_operation_id: "turn".into() },
        AgentEngineEvent::ExecutionCheckpointSaved { step: 0, revision: 1, digest: "c".repeat(64).into() }];
    let mut tail = rejection_prefix().into_iter().skip(1).collect::<Vec<_>>();
    tail.insert(1, AgentEngineEvent::OutputTextDelta { step: 1, text: "ABANDONED_CARRIER".into() });
    tail.push(start(2, Some(1)));
    let recovery = AgentTurnRecovery::new(checkpoint.clone(), 1, 2, prefix.clone(), tail.clone(), vec![]).unwrap();
    let mut state = crate::protocol_recovery::ProtocolRecovery::default();
    state.restore(recovery.prefix.iter().chain(&recovery.tail)).unwrap();
    assert_eq!(state.correction_source(), Some(1));
    assert_eq!(state.pending_feedback(), Some(crate::protocol_recovery::rejection_feedback(&rejected()).unwrap()));
    let mut history = vec![]; replay_checkpoint_prefix(&mut history, requirement(), &recovery).unwrap();
    assert!(!serde_json::to_string(&history).unwrap().contains("ABANDONED_CARRIER"));
    let mut forged = tail;
    if let AgentEngineEvent::ModelStepStarted { correction_source, .. } = forged.last_mut().unwrap() { *correction_source = Some(90); }
    assert!(AgentTurnRecovery::new(checkpoint, 1, 2, prefix, forged, vec![]).is_err());
}

#[test]
fn rejected_tail_reconciliation_never_fabricates_tool_outcomes() {
    let tail = rejection_prefix().into_iter().skip(1).collect::<Vec<_>>();
    let result = crate::reconcile_execution_tail(&checkpoint(), 1, &tail, &Default::default(), &Default::default(), &Default::default()).unwrap();
    assert_eq!(result.checkpoint.tool_call_count, 0);
    assert!(!result.observations.iter().any(|event| matches!(event, AgentEngineEvent::ToolOutcomeReconciled { .. } | AgentEngineEvent::ToolCompleted { .. } | AgentEngineEvent::ToolStarted { .. })));
    let mut forged = tail; forged.push(AgentEngineEvent::ModelResponseValidated { step: 1 });
    assert!(crate::reconcile_execution_tail(&checkpoint(), 1, &forged, &Default::default(), &Default::default(), &Default::default()).is_err());
}
