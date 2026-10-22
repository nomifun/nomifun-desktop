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
        patch_recovery: Default::default(), segments: None, control_rejections: Default::default(), completion_review: Default::default(), pending_user_input: None, awaiting_children: None, collaboration: Default::default() }
}

fn proposal(id: &str) -> AgentEngineEvent {
    AgentEngineEvent::ToolCallCompleted { step: 1, call: ChatToolCall { call_id: id.into(), name: "write_file".into(),
        arguments: StrictJsonValue(json!({"path":"answer.txt","content":"saved"})), provider_metadata: None } }
}

fn admitted(id: &str) -> AgentEngineEvent {
    AgentEngineEvent::ToolStarted { step: 1, call_id: id.into(), capability_id: "workspace.files".into(), action_id: "workspace.files/write".into() }
}

fn started() -> AgentEngineEvent { AgentEngineEvent::ModelStepStarted { step: 1, operation_id: "turn:model:1".into(), correction_source: None } }

fn outcome(id: &str) -> AgentReconciledOutcome {
    AgentReconciledOutcome { result: AgentToolResult::text(id.into(), "owner-confirmed-write", false),
        source: AgentReconciliationSource::OwnerReceipt, evidence_event_id: Some("owner-receipt".into()), owner_operation_id: Some("write-operation".into()) }
}

#[test]
fn exact_action_reservation_and_owner_success_reconcile_without_replay() {
    let mut cp=checkpoint();
    let action=crate::exact_actions::ExactActionInput {id:"save".into(),source:crate::AgentInputCitation {input:0,quote:"save".into()},
        tool:"write_file".into(),expected_arguments:json!({"path":"answer.txt","content":"saved"}),receiver_ref:None}
        .compile(&[crate::context_lifecycle::text_message(ChatRole::User,"save".into())]).unwrap();
    cp.plan.revision=1;cp.plan.exact_actions=vec![action];
    let mut reserved=cp.plan.clone();reserved.exact_actions[0].attempted_call_id=Some("written".into());
    let mut tail=vec![started(),proposal("written"),AgentEngineEvent::PlanUpdated {plan:reserved}];
    let held=reconcile_execution_tail(&cp,1,&tail,&BTreeSet::new(),&BTreeMap::new(),&Default::default()).unwrap();
    assert!(held.checkpoint.plan.exact_actions[0].attempted_call_id.is_none());
    assert!(!held.checkpoint.plan.exact_actions[0].succeeded);
    tail.push(admitted("written"));
    assert!(reconcile_execution_tail(&cp,1,&tail,&BTreeSet::from(["written".into()]),&BTreeMap::new(),&Default::default()).is_err(),
        "admitted unknown effects cannot become pending/replayable");
    let receipt=AgentToolResult::text("written".into(),json!({"path":"answer.txt","written":true,"bytes":5,
        "sha256":nomifun_agent_contracts::digest_bytes(b"saved")}).to_string(),false);
    let success=AgentReconciledOutcome {result:receipt.clone(),source:AgentReconciliationSource::OwnerReceipt,
        evidence_event_id:Some("owner-receipt".into()),owner_operation_id:Some("write-operation".into())};
    let restored=reconcile_execution_tail(&cp,1,&tail,&BTreeSet::from(["written".into()]),
        &BTreeMap::from([("written".into(),success)]),&Default::default()).unwrap();
    assert!(restored.checkpoint.plan.exact_actions[0].succeeded);
    let AgentEngineEvent::ToolCallCompleted {call,..}=proposal("fresh-replay") else {unreachable!()};
    assert!(restored.checkpoint.plan.exact_action_gate(&call,true).is_some());
    tail.push(AgentEngineEvent::ToolCompleted { runtime_hold: None, step:1,result:receipt});
    let recorded=reconcile_execution_tail(&cp,1,&tail,&BTreeSet::from(["written".into()]),&BTreeMap::new(),&Default::default()).unwrap();
    assert!(recorded.checkpoint.plan.exact_actions[0].succeeded,"owner result before PlanUpdated settle must restore satisfied");
    assert!(!recorded.observations.iter().any(|event|matches!(event,AgentEngineEvent::ToolStarted{..})));
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
        AgentEngineEvent::ToolCompleted { runtime_hold: None, step:1,result:AgentToolResult::text("written".into(),"original-receipt",false) }];
    let result = reconcile_execution_tail(&checkpoint(),1,&tail,&BTreeSet::from(["written".into()]),
        &BTreeMap::from([("written".into(),outcome("written"))]),&Default::default()).unwrap();
    assert_eq!(result.checkpoint.tool_call_count, 1);
    assert!(!result.observations.iter().any(|event|matches!(event,AgentEngineEvent::ToolOutcomeReconciled{..})));
}

#[test]
fn applied_steering_keeps_receipt_order_and_deferred_inputs_remain_unapplied() {
    let input = |id:&str| crate::AgentSteeringInput { receipt_operation_id:id.into(),message_id:format!("message-{id}"),
        text:format!("instruction-{id}"),files:vec![format!("{id}.png")],inject_skills:vec!["selected-skill".into()],image_count:1,prepared_images:vec![],prepared_skill_instructions:vec![] };
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

fn interrupted_live_wait() -> Vec<AgentEngineEvent> {
    let question = nomifun_agent_contracts::AgentUserInputRequest { request_id: "authorize".into(),
        question: "Authorize the original live helper".into(), remaining_work: "Verify the original helper and finish".into() };
    vec![
        started(),
        AgentEngineEvent::ToolCallCompleted { step: 1, call: ChatToolCall { call_id: "launch".into(), name: "start_process".into(),
            arguments: StrictJsonValue(json!({"command":"helper"})), provider_metadata: None } },
        AgentEngineEvent::ToolStarted { step: 1, call_id: "launch".into(), capability_id: "workspace.process".into(), action_id: "workspace.process/start".into() },
        AgentEngineEvent::ToolCompleted { step: 1, runtime_hold: None, result: AgentToolResult::text("launch".into(), "original owner running", false) },
        AgentEngineEvent::WorkStatus { status: crate::AgentWorkStatus { running_processes: BTreeSet::from(["owned".into()]), ..Default::default() } },
        AgentEngineEvent::ModelStepStarted { step: 2, operation_id: "turn:model:2".into(), correction_source: None },
        AgentEngineEvent::ToolCallCompleted { step: 2, call: ChatToolCall { call_id: "authorize".into(), name: "request_user_input".into(),
            arguments: StrictJsonValue(json!({"question":question.question,"remaining_work":question.remaining_work})), provider_metadata: None } },
        AgentEngineEvent::UserInputRequested { step: 2, request: question },
        AgentEngineEvent::ToolCompleted { step: 2, runtime_hold: None, result: AgentToolResult::text("authorize".into(), crate::user_input_request::ACK, false) },
        AgentEngineEvent::TurnWaiting { model_steps: 2, request_id: Some("authorize".into()), reason: nomifun_agent_contracts::AgentWaitReason::UserInput },
    ]
}

#[test]
fn missing_original_live_owner_terminal_cannot_republish_old_authorization_question() {
    let result = reconcile_execution_tail(&checkpoint(), 1, &interrupted_live_wait(), &BTreeSet::from(["launch".into()]), &BTreeMap::new(), &Default::default());
    assert!(matches!(result, Err(AgentEngineError::ReplayContract(reason)) if reason.contains("original live wait owner has no reaped terminal")));
}

#[test]
fn original_owner_terminal_before_crash_resolves_resource_without_answering_question() {
    let mut tail = interrupted_live_wait();
    tail.push(AgentEngineEvent::OwnedProcessObserved { model_steps: 2, observation: crate::AgentOwnedProcessObservation {
        process_id: "owned".into(), poll: nomifun_engine_core::EngineProcessPoll::TimedOut {
            output: nomifun_engine_core::EngineProcessOutput { text: "expired".into(), next_cursor: 7, retained_bytes: 7,
                dropped_bytes: 0, source_encoding: "utf-8".into(), decode_errors: 0 },
            cleanup: nomifun_engine_core::EngineCleanupReport { interrupt_attempted: true, terminate_attempted: false,
                force_kill_attempted: false, reaped: true, elapsed_ms: 1, errors: Vec::new() },
        },
    } });
    let result = reconcile_execution_tail(&checkpoint(), 1, &tail, &BTreeSet::from(["launch".into()]), &BTreeMap::new(), &Default::default()).unwrap();
    assert!(result.checkpoint.pending_user_input.is_some());
    assert!(result.checkpoint.work.running_processes.is_empty());
    assert_eq!(result.checkpoint.work.failed_commands, 1);
    assert!(result.observations.iter().any(|event| matches!(event, AgentEngineEvent::TurnWaitResolved { model_steps: 2 })));
    assert!(!result.observations.iter().any(|event| matches!(event, AgentEngineEvent::SteeringInputs { .. } | AgentEngineEvent::TurnInputRequired { .. })));
    tail.extend(result.observations);
    assert!(crate::user_input_request::recorded_pending(&tail).unwrap().is_some());
}

#[test]
fn real_tool_progress_after_end_review_restores_the_same_review_phase_as_live_execution() {
    let mut cp = checkpoint();
    cp.completion_review = crate::AgentCompletionReviewState { phase: crate::AgentExecutionPhase::EndIntentReview,
        input_revision: 1, ..Default::default() };
    let tail = vec![started(), proposal("written"), admitted("written")];
    let result = reconcile_execution_tail(&cp, 1, &tail, &BTreeSet::from(["written".into()]),
        &BTreeMap::from([("written".into(), outcome("written"))]), &Default::default()).unwrap();
    assert_eq!(result.checkpoint.completion_review.phase, crate::AgentExecutionPhase::Execution,
        "an actual owner result ends declaration-only correction in both live and recovered execution");
}

#[test]
fn accepted_child_wait_recovers_its_exact_request_without_replaying_a_tool_or_resetting_budget() {
    let wait = crate::AgentChildrenWait { execution_ids: vec!["owned".into()], after_cursor:Some("canonical:4".into()),
        mode:crate::AgentChildrenWaitMode::All };
    let mut tail = vec![started(), AgentEngineEvent::ToolCallCompleted { step:1, call:ChatToolCall {
        call_id:"wait".into(),name:crate::collaboration::WAIT.into(),provider_metadata:None,
        arguments:StrictJsonValue(serde_json::to_value(&wait).unwrap()),
    } }, AgentEngineEvent::ToolCompleted { step:1, runtime_hold:None,
        result:AgentToolResult::text("wait".into(),"accepted",false) },
        AgentEngineEvent::AwaitingChildren { model_steps:1, request:wait.clone() }];
    let recovered = reconcile_execution_tail(&checkpoint(),1,&tail,&Default::default(),&Default::default(),&Default::default()).unwrap();
    assert_eq!(recovered.checkpoint.awaiting_children, Some(wait));
    assert_eq!(recovered.checkpoint.model_steps,1);
    assert_eq!(recovered.checkpoint.tool_call_count,1);
    assert!(!recovered.observations.iter().any(|event|matches!(event,AgentEngineEvent::ToolOutcomeReconciled {..})));
    tail.push(AgentEngineEvent::ChildrenWaitResolved { model_steps:1,cursor:"canonical:5".into() });
    let awakened = reconcile_execution_tail(&checkpoint(),1,&tail,&Default::default(),&Default::default(),&Default::default()).unwrap();
    assert!(awakened.checkpoint.awaiting_children.is_none());
    assert_eq!(awakened.checkpoint.model_steps,1);
    tail.pop();
    tail.push(AgentEngineEvent::ModelStepStarted { step:2, operation_id:"turn:model:2".into(),correction_source:None });
    assert!(reconcile_execution_tail(&checkpoint(),1,&tail,&Default::default(),&Default::default(),&Default::default()).is_err());
}

#[test]
fn child_message_needs_its_original_domain_receipt_even_without_kernel_admission() {
    let tail=vec![started(),AgentEngineEvent::ToolCallCompleted{step:1,call:ChatToolCall{
        call_id:"guide".into(),name:crate::collaboration::MESSAGE.into(),provider_metadata:None,
        arguments:StrictJsonValue(json!({"execution_id":"execution","step_id":"child","content":"Continue original work.","idempotency_key":"once"})),
    }}];
    assert!(reconcile_execution_tail(&checkpoint(),1,&tail,&Default::default(),&Default::default(),&Default::default()).is_err());
    let result=AgentReconciledOutcome{result:AgentToolResult::text("guide".into(),json!({"execution_id":"execution","sequence":7,"accepted":true}).to_string(),false),
        source:AgentReconciliationSource::NativeControlReceipt,evidence_event_id:Some("original-command-event".into()),owner_operation_id:Some("original-guidance-command".into())};
    let mut fake=result.clone();fake.source=AgentReconciliationSource::NotDispatched;
    assert!(reconcile_execution_tail(&checkpoint(),1,&tail,&Default::default(),&BTreeMap::from([("guide".into(),fake)]),&Default::default()).is_err(),
        "missing Kernel admission cannot turn a possibly committed guidance into non-dispatch");
    let mut fake=result.clone();fake.evidence_event_id=None;
    assert!(reconcile_execution_tail(&checkpoint(),1,&tail,&Default::default(),&BTreeMap::from([("guide".into(),fake)]),&Default::default()).is_err());
    let mut unknown=result.clone();unknown.result=AgentToolResult::text("guide".into(),json!({"state":"unknown","control_applied":null}).to_string(),true);
    assert!(reconcile_execution_tail(&checkpoint(),1,&tail,&Default::default(),&BTreeMap::from([("guide".into(),unknown)]),&Default::default()).is_err());
    let mut not_applied=result.clone();not_applied.result=AgentToolResult::text("guide".into(),json!({"state":"not_executed","control_applied":false,
        "code":"CHILD_MESSAGE_NOT_COMMITTED"}).to_string(),true);
    let closed=reconcile_execution_tail(&checkpoint(),1,&tail,&Default::default(),&BTreeMap::from([("guide".into(),not_applied)]),&Default::default()).unwrap();
    assert!(closed.checkpoint.collaboration.pending_child_executions.is_empty());
    assert_eq!(closed.checkpoint.work.failed_tools,1);
    let recovered=reconcile_execution_tail(&checkpoint(),1,&tail,&Default::default(),&BTreeMap::from([("guide".into(),result)]),&Default::default()).unwrap();
    assert!(recovered.checkpoint.collaboration.pending_child_executions.contains("execution"));
    assert_eq!(recovered.checkpoint.tool_call_count,1);
    assert!(recovered.observations.iter().any(|event|matches!(event,AgentEngineEvent::ToolOutcomeReconciled{source:AgentReconciliationSource::NativeControlReceipt,..})));
}
