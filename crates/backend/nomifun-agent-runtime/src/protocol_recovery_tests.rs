use super::*;
use crate::AgentRejectedToolProposal;
use nomifun_chat_model_broker::{ToolArgumentDiagnostic, ToolArgumentDiagnosticCategory};

fn begin(recovery: &mut ProtocolRecovery, step: u16, events: &mut Vec<AgentEngineEvent>) {
    let event = AgentEngineEvent::ModelStepStarted { step, operation_id: format!("turn:model:{step}").into(),
        correction_source: recovery.correction_source() };
    recovery.observe(&event).unwrap(); events.push(event);
}

fn reject(recovery: &mut ProtocolRecovery, step: u16, events: &mut Vec<AgentEngineEvent>) -> bool {
    let prepared = recovery.prepare_rejection(step, vec![], AgentModelResponseRejection::NonNativeMarkup, None, true).unwrap();
    let continuation = prepared.continuation(); events.push(prepared.event().clone());
    recovery.commit_rejection(prepared).unwrap(); continuation
}

pub(super) fn argument_rejection(step: u16) -> AgentEngineEvent {
    let diagnostic = ToolArgumentDiagnostic { category: ToolArgumentDiagnosticCategory::Syntax,
        line: Some(2), column: Some(9), byte_len: 40, digest: "a".repeat(64).into() };
    let prepared = ProtocolRecovery::default().prepare_rejection(step, vec!["bad-json".into()],
        AgentModelResponseRejection::ToolArguments { diagnostics: vec![AgentRejectedToolProposal {
            call_id: "bad-json".into(), name: "apply_patch".into(), diagnostic }] }, None, true).unwrap();
    prepared.event().clone()
}

#[test]
fn preparation_is_not_a_budget_commit_and_strategies_cannot_expand_it() {
    let mut recovery = ProtocolRecovery::default(); let mut events = vec![];
    begin(&mut recovery, 1, &mut events);
    let before = recovery.clone();
    let prepared = recovery.prepare_rejection(1, vec![], AgentModelResponseRejection::EmptyTaskTerminal, None, true).unwrap();
    assert!(prepared.continuation()); assert_eq!(before, recovery);
    drop(prepared); assert_eq!(before, recovery);
    struct Deny; impl ProtocolCorrectionStrategy for Deny { fn allow(&self, _: &AgentModelResponseRejection) -> bool { false } }
    assert!(!recovery.prepare_rejection_using(1, vec![], AgentModelResponseRejection::EmptyTaskTerminal, None, true, &Deny).unwrap().continuation());
    assert!(!recovery.prepare_rejection(1, vec![], AgentModelResponseRejection::EmptyTaskTerminal, None, false).unwrap().continuation());
    assert_eq!(before, recovery);
}

#[test]
fn live_and_replay_share_two_consecutive_eight_total_budget() {
    let mut live = ProtocolRecovery::default(); let mut events = vec![];
    for step in 1..=3 { begin(&mut live, step, &mut events); assert_eq!(reject(&mut live, step, &mut events), step <= 2); }
    let mut restored = ProtocolRecovery::default(); restored.restore(events.iter()).unwrap();
    assert_eq!(live, restored);
    // A validated response, not tool-result ordering or a fresh process,
    // clears the consecutive guard; it never replenishes the total guard.
    begin(&mut live, 4, &mut events);
    let validated = AgentEngineEvent::ModelResponseValidated { step: 4 };
    live.observe(&validated).unwrap(); events.push(validated);
    for offset in 0..6 {
        let step = 5 + offset * 2;
        begin(&mut live, step, &mut events); assert!(reject(&mut live, step, &mut events));
        begin(&mut live, step + 1, &mut events);
        let validated = AgentEngineEvent::ModelResponseValidated { step: step + 1 };
        live.observe(&validated).unwrap(); events.push(validated);
    }
    begin(&mut live, 17, &mut events); assert!(!reject(&mut live, 17, &mut events));
    let mut restored = ProtocolRecovery::default(); restored.restore(events.iter()).unwrap();
    assert_eq!(live, restored); assert_eq!(restored.total, 8);
}

#[test]
fn native_tool_results_are_not_model_format_validation() {
    let mut recovery = ProtocolRecovery::default(); let mut events = vec![];
    begin(&mut recovery, 1, &mut events); assert!(reject(&mut recovery, 1, &mut events));
    let before = recovery.clone();
    recovery.observe(&AgentEngineEvent::ToolResultsOrdered { step: 1, call_ids: vec!["call".into()] }).unwrap();
    assert_eq!(before, recovery);
    recovery.observe_new_input(); assert_eq!(recovery.total, 1); assert_eq!(recovery.consecutive, 1);
    assert!(recovery.correction_source().is_none());
}

#[test]
fn exact_safe_feedback_and_argument_diagnostics_survive_restore() {
    let mut event = argument_rejection(1);
    if let AgentEngineEvent::ModelResponseRejected { feedback, .. } = &mut event { *feedback = Some("EXACT_CANONICAL_FEEDBACK".into()); }
    let mut restored = ProtocolRecovery::default(); restored.restore([&event].into_iter()).unwrap();
    assert_eq!(restored.pending_feedback(), Some(rejection_feedback(&event).unwrap()));
    assert_eq!(restored.correction_source(), Some(1));
    assert_eq!(serde_json::to_value(&event).unwrap()["rejection"]["diagnostics"][0]["diagnostic"]["line"], 2);
}

#[test]
fn existing_unclassified_facts_do_not_invent_a_reason() {
    let event: AgentEngineEvent = serde_json::from_value(serde_json::json!({"event":"model_response_rejected",
        "step":1,"discarded_tool_call_ids":[],"continuation":true})).unwrap();
    let encoded = serde_json::to_string(&rejection_feedback(&event).unwrap()).unwrap();
    assert!(encoded.contains("did not record a more precise reason"));
    assert!(!encoded.contains("emitted tool-call markup") && !encoded.contains("contained no public answer"));
    assert!(ProtocolRecovery::default().prepare_rejection(1, vec![], AgentModelResponseRejection::Unclassified, None, true).is_err());
}

#[test]
fn sources_require_a_real_pending_continuing_rejection() {
    let mut recovery = ProtocolRecovery::default(); let event = argument_rejection(1);
    recovery.observe(&event).unwrap();
    let step = |source| AgentEngineEvent::ModelStepStarted { step: 2, operation_id: "turn:model:2".into(), correction_source: source };
    assert!(recovery.clone().observe(&step(Some(99))).is_err());
    assert!(recovery.clone().observe(&step(None)).is_err());
    recovery.observe(&step(Some(1))).unwrap();
    recovery.observe(&AgentEngineEvent::ModelResponseValidated { step: 2 }).unwrap();
    assert!(recovery.observe(&AgentEngineEvent::ModelStepStarted { step: 3, operation_id: "turn:model:3".into(), correction_source: Some(1) }).is_err());
    assert!(ProtocolRecovery::default().observe(&step(Some(1))).is_err());
}

#[test]
fn malformed_canonical_rejection_and_duplicate_verdicts_fail_closed() {
    let mut event = argument_rejection(1);
    if let AgentEngineEvent::ModelResponseRejected { discarded_tool_call_ids, .. } = &mut event { discarded_tool_call_ids.clear(); }
    assert!(validate_rejection(&event).is_err());
    let mut event = argument_rejection(1);
    if let AgentEngineEvent::ModelResponseRejected { feedback, .. } = &mut event { *feedback = None; }
    assert!(validate_rejection(&event).is_err());
    let event = argument_rejection(1); let mut recovery = ProtocolRecovery::default(); recovery.observe(&event).unwrap();
    assert!(recovery.observe(&event).is_err());
    assert!(recovery.observe(&AgentEngineEvent::ModelResponseValidated { step: 1 }).is_err());
}

#[test]
fn full_diagnostic_batch_is_preserved_while_model_feedback_is_bounded() {
    let diagnostics = (0..64).map(|index| AgentRejectedToolProposal {
        call_id: format!("{index:02}{}", "\"".repeat(254)).into(),
        name: "\"".repeat(128), diagnostic: ToolArgumentDiagnostic {
            category: ToolArgumentDiagnosticCategory::Syntax, line: Some(1), column: Some(0),
            byte_len: 0, digest: "a".repeat(64).into(),
        },
    }).collect::<Vec<_>>();
    let ids = diagnostics.iter().map(|proposal| proposal.call_id.clone()).collect();
    let prepared = ProtocolRecovery::default().prepare_rejection(1, ids,
        AgentModelResponseRejection::ToolArguments { diagnostics }, None, true).unwrap();
    let AgentEngineEvent::ModelResponseRejected { rejection: Some(AgentModelResponseRejection::ToolArguments { diagnostics }), feedback: Some(feedback), .. } = prepared.event() else { panic!("typed rejection"); };
    assert_eq!(diagnostics.len(), 64);
    assert!(feedback.len() <= MAX_FEEDBACK_BYTES);
    assert!(feedback.contains("56 additional rejected proposals"));
}

#[test]
fn constraints_are_advisory_and_release_after_committed_validation() {
    let mut recovery = ProtocolRecovery::default();
    let prepared = recovery.prepare_rejection(1, vec![], AgentModelResponseRejection::NonNativeMarkup, Some("write_file".into()), true).unwrap();
    recovery.commit_rejection(prepared).unwrap();
    let tool = ChatToolDefinition { name: "write_file".into(), description: "write".into(),
        input_schema: nomifun_agent_contracts::StrictJsonValue(serde_json::json!({"type":"object"})), deferred: false };
    let mut choice = ChatToolChoice::Auto;
    recovery.constrain_tool_choice(&mut choice, true); assert_eq!(choice, ChatToolChoice::Required);
    recovery.constrain_exposed_tool(&mut choice, std::slice::from_ref(&tool));
    assert_eq!(choice, ChatToolChoice::Specific { name: "write_file".into() });
    let mut tools = vec![tool]; assert!(recovery.narrow_repair_surface(&choice, &mut tools));
    recovery.observe(&AgentEngineEvent::ModelStepStarted { step: 2, operation_id: "turn:model:2".into(), correction_source: Some(1) }).unwrap();
    recovery.observe(&AgentEngineEvent::ModelResponseValidated { step: 2 }).unwrap();
    let mut choice = ChatToolChoice::Auto; recovery.constrain_tool_choice(&mut choice, true); assert_eq!(choice, ChatToolChoice::Auto);
}
