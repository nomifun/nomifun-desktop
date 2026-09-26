use super::*;
use nomifun_agent_contracts::{NativeBudgetIncrease, StrictJsonValue};
use nomifun_chat_model_broker::{ChatFinishReason, ChatRole};
use serde_json::json;

fn checkpoint() -> AgentExecutionCheckpoint {
    AgentExecutionCheckpoint { version: 1,
        binding: crate::EngineBinding::new("session".into(), "binding".into(), "build".into(), "a".repeat(64).into(),
            nomifun_agent_contracts::ResolvedSnapshotRef { snapshot_id: "snapshot".into(), snapshot_digest: "b".repeat(64).into() }).unwrap(),
        turn_operation_id: "turn".into(), active_set_generation: 0, model_steps: 0, tool_call_count: 0,
        accepted_input_count: 1, applied_steering_receipts: vec![], plan: Default::default(), work: Default::default(),
        patch_recovery: Default::default(), segments: None, control_rejections: Default::default() }
}

fn proposal(id: &str) -> AgentEngineEvent {
    AgentEngineEvent::ToolCallCompleted { step: 1, call: ChatToolCall { call_id: id.into(), name: "write_file".into(),
        arguments: StrictJsonValue(json!({"path":"answer.txt","content":"saved"})), provider_metadata: None } }
}

fn admitted(id: &str) -> AgentEngineEvent {
    AgentEngineEvent::ToolStarted { step: 1, call_id: id.into(), capability_id: "workspace.files".into(), action_id: "workspace.files/write".into() }
}

fn started() -> AgentEngineEvent { AgentEngineEvent::ModelStepStarted { step: 1, operation_id: "turn:model:1".into() } }

fn outcome(id: &str) -> AgentReconciledOutcome {
    AgentReconciledOutcome { result: AgentToolResult::text(id.into(), "owner-confirmed-write", false),
        source: AgentReconciliationSource::OwnerReceipt, evidence_event_id: Some("owner-receipt".into()), owner_operation_id: Some("write-operation".into()) }
}

#[test]
fn partial_batch_retains_owner_receipt_and_marks_unexecuted_proposal_without_replay() {
    let cp = checkpoint();
    let tail = vec![started(), proposal("written"), proposal("not-dispatched"), admitted("written")];
    let result = reconcile_execution_tail(&cp, 1, &tail, &BTreeSet::from(["written".into()]),
        &BTreeMap::from([("written".into(), outcome("written"))]), &Default::default()).unwrap();
    assert_eq!(result.checkpoint.model_steps, 1);
    assert_eq!(result.checkpoint.tool_call_count, 2);
    assert!(result.checkpoint.plan.needs_replan);
    assert!(!result.observations.iter().any(|event| matches!(event, AgentEngineEvent::ToolStarted { .. })));
    assert!(result.observations.iter().any(|event| matches!(event,
        AgentEngineEvent::ToolOutcomeReconciled { result, source: AgentReconciliationSource::NotDispatched, .. } if result.call_id.as_ref() == "not-dispatched" && result.is_error)));
    let mut events = vec![AgentEngineEvent::TurnStarted { binding: cp.binding.clone(), turn_operation_id: "turn".into() },
        AgentEngineEvent::ExecutionCheckpointSaved { step: 0, revision: 1, digest: "c".repeat(64).into() }];
    events.extend(tail); events.extend(result.observations);
    events.push(AgentEngineEvent::TurnCompleted { model_steps: 1, finish_reason: ChatFinishReason::Completed });
    let mut history = vec![];
    crate::replay_closed_turn(&mut history, crate::context_lifecycle::text_message(ChatRole::User, "original-task".into()), &events).unwrap();
    let encoded = serde_json::to_string(&history).unwrap();
    assert!(encoded.contains("original-task") && encoded.contains("owner-confirmed-write") && encoded.contains("Not dispatched"));
}

#[test]
fn unknown_or_mismatched_owner_outcome_blocks_resume() {
    let tail = vec![started(), proposal("written"), admitted("written")];
    let dispatched = BTreeSet::from(["written".into()]);
    assert!(reconcile_execution_tail(&checkpoint(),1,&tail,&dispatched,&BTreeMap::new(),&Default::default()).is_err());
    assert!(reconcile_execution_tail(&checkpoint(),1,&tail,&dispatched,
        &BTreeMap::from([("written".into(),outcome("different-call"))]),&Default::default()).is_err());
}

#[test]
fn incomplete_proposal_can_only_be_discarded_without_admitted_effects() {
    let incomplete = AgentEngineEvent::ToolCallDelta { step:1,call_id:"partial".into(),name:"write_file".into(),arguments_delta:"{".into() };
    let result = reconcile_execution_tail(&checkpoint(),1,&[started(),incomplete.clone()],&BTreeSet::new(),&BTreeMap::new(),&Default::default()).unwrap();
    assert_eq!(result.checkpoint.tool_call_count, 0);
    assert!(matches!(result.observations.last().unwrap(), AgentEngineEvent::ExecutionTailReconciled { discard_last_model_step:true,.. }));
    let mixed = vec![started(),proposal("written"),incomplete,admitted("written")];
    assert!(reconcile_execution_tail(&checkpoint(),1,&mixed,&BTreeSet::from(["written".into()]),
        &BTreeMap::from([("written".into(),outcome("written"))]),&Default::default()).is_err());
}

#[test]
fn completed_results_are_not_replaced_by_late_attestations_or_counted_twice() {
    let tail = vec![started(),proposal("written"),admitted("written"),
        AgentEngineEvent::ToolCompleted { step:1,result:AgentToolResult::text("written".into(),"original-receipt",false) }];
    let result = reconcile_execution_tail(&checkpoint(),1,&tail,&BTreeSet::from(["written".into()]),
        &BTreeMap::from([("written".into(),outcome("written"))]),&Default::default()).unwrap();
    assert_eq!(result.checkpoint.tool_call_count, 1);
    assert!(!result.observations.iter().any(|event|matches!(event,AgentEngineEvent::ToolOutcomeReconciled{..})));
}

#[test]
fn applied_steering_keeps_receipt_order_and_deferred_inputs_remain_unapplied() {
    let input = |id:&str| crate::AgentSteeringInput { receipt_operation_id:id.into(),message_id:format!("message-{id}"),
        text:format!("instruction-{id}"),files:vec![format!("{id}.png")],inject_skills:vec!["selected-skill".into()],image_count:1,prepared_images:vec![] };
    let inputs = vec![input("z-first"),input("a-second")];
    let tail = vec![AgentEngineEvent::SteeringInputs { inputs:inputs.clone() },
        AgentEngineEvent::SteeringDeferred { inputs:vec![input("pending")],reason:"paused".into() }];
    let result = reconcile_execution_tail(&checkpoint(),1,&tail,&BTreeSet::new(),&BTreeMap::new(),&Default::default()).unwrap();
    assert_eq!(result.checkpoint.applied_steering_receipts, vec!["z-first".into(),"a-second".into()]);
    assert_eq!(result.checkpoint.accepted_input_count,3);
    let repeated = vec![AgentEngineEvent::SteeringInputs { inputs }];
    assert!(reconcile_execution_tail(&result.checkpoint,2,&repeated,&BTreeSet::new(),&BTreeMap::new(),&Default::default()).is_err());
}

#[test]
fn explicit_segment_increase_preserves_consumed_steps_and_cumulative_limits() {
    let mut cp = checkpoint();
    cp.segments = Some(crate::AgentExecutionSegmentState::new(2,crate::AgentSegmentPolicy { max_segments:2,max_no_progress_segments:2 }).unwrap());
    let segments = cp.segments.as_mut().unwrap();
    segments.segment = 2; segments.segment_start_step = 2;
    cp.model_steps = 4;
    assert!(reconcile_execution_tail(&cp,1,&[],&BTreeSet::new(),&BTreeMap::new(),&Default::default()).is_err());
    let grant = NativeBudgetIncrease { additional_segments:1, ..Default::default() };
    let result = reconcile_execution_tail(&cp,1,&[],&BTreeSet::new(),&BTreeMap::new(),&grant).unwrap();
    assert_eq!(result.checkpoint.model_steps,4);
    assert_eq!(result.checkpoint.segments.unwrap().total_model_limit(),6);
    let over_limit = NativeBudgetIncrease { additional_segments:31, ..Default::default() };
    assert!(reconcile_execution_tail(&cp,1,&[],&BTreeSet::new(),&BTreeMap::new(),&over_limit).is_err());
}
