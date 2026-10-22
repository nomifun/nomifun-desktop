//! Stop acknowledgement is not native result delivery. Original canonical
//! turns remain the source of workspace changes after domain cancellation.
use super::*;
use super::super::collaboration_port::native_delivery_observation;
use nomifun_agent_contracts::{EventId,EventProducerId,IdempotencyKey,SemanticSessionEventDraft,SessionEventAppend,SessionEventKind,SessionEventPayloadRef};
use nomifun_agent_session::{EffectEventRequest,EffectStrategy,EffectTerminalState};
use nomifun_agent_execution::AgentExecutionSessionPort;
use nomifun_db::{IAgentExecutionRepository,SqliteAgentExecutionRepository,NewAgentExecutionEvent};
use nomifun_common::{AgentExecutionActor,AgentExecutionCancellationScope,AgentExecutionEventKind};

async fn setup(fixture:&Fixture)->(String,String,String,String,nomifun_db::AgentExecutionStepDetailRow) {
    let (execution,step,running)=voice_running_decision_attempt_fixture(fixture).await;
    let attempt=&running.current_attempt.as_ref().unwrap().attempt;
    let key=nomifun_db::AttemptConversationEffects::default().current_turn_operation_key(&attempt.attempt_id).unwrap();
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let store=fixture.owner.canonical().store();
    store.cancel_active_turn(&session,"close-disposable-fixture-root".into(),"session_api".into()).await.unwrap();
    let operation=format!("turn:user:{}:{}:{key}",fixture.host.options.user_id,session.as_ref());
    store.start_turn(&session,"session_api".into(),"original-child-only".into(),operation.clone().into(),
        StrictJsonValue(json!({"content":"Canonical owner partial write followed by exact Stop","files":[],"inject_skills":[]}))).await.unwrap();
    (execution,step,key,operation,running)
}

async fn partial_write(fixture:&Fixture,operation:&str) {
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let store=fixture.owner.canonical().store();
    let started=store.read_turn_receipt(&session,&operation.into()).await.unwrap().started_event.unwrap();
    let call=uuid::Uuid::now_v7().to_string();let effect=uuid::Uuid::now_v7().to_string();let tool=uuid::Uuid::now_v7().to_string();
    let call_event=store.append_event(&SessionEventAppend {agent_session_id:session.clone(),event_id:call.clone().into(),
        producer_id:"runtime_supervisor".into(),idempotency_key:call.clone().into(),semantic_event:SemanticSessionEventDraft {
            kind:SessionEventKind("tool/call-started".into()),kind_version:1,correlation_id:call.clone().into(),
            causation_event_id:Some(started.event_id),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                "operation_id":tool,"call_id":call,"capability_id":"workspace.files","action_id":"workspace.files/write"})))},
    }).await.unwrap().record.unwrap();
    let start_id=uuid::Uuid::now_v7().to_string();
    let request=EffectEventRequest {agent_session_id:session.clone(),effect_id:effect.clone(),turn_id:operation.into(),operation_id:tool.into(),
        owner_domain:"workspace".into(),capability_module:"workspace.files".into(),action_id:"workspace.files/write".into(),
        resource_binding_id:None,resource_key:None,input_digest:"a".repeat(64).into(),recorded_at:1,
        event_id:EventId::from(start_id.clone()),producer_id:EventProducerId::from("workspace"),idempotency_key:IdempotencyKey::from(start_id),
        correlation_id:effect.clone().into(),strategy:EffectStrategy::ManagedEffect,causation_event_id:Some(call_event.event_id),
        payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
    };
    let effect_start=store.record_effect_started(request.clone()).await.unwrap().record.unwrap();
    std::fs::write(PathBuf::from(&fixture.host.options.workspace).join("cancelled-child-partial.txt"),"real fixture bytes before Stop").unwrap();
    let id=uuid::Uuid::now_v7().to_string();
    store.record_effect_terminal(EffectEventRequest {event_id:id.into(),producer_id:"runtime_supervisor".into(),
        causation_event_id:Some(effect_start.event_id),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"error":"owner reports failure after a partial write"}))),
        ..request},EffectTerminalState::Failed).await.unwrap();
}

#[tokio::test]
async fn cancelled_domain_waits_for_exact_native_terminal_and_retains_partial_workspace_effects() {
    let fixture=Fixture::new("cancelled-native-result-delivery").await;
    let (execution,step,key,operation,running)=setup(&fixture).await;
    partial_write(&fixture,&operation).await;
    let repository=SqliteAgentExecutionRepository::new(fixture.pool().clone());let principal=&fixture.host.options.user_id;
    let version=repository.get_execution(principal,&execution).await.unwrap().unwrap().version;
    repository.cancel_step(principal,&execution,version,&step,running.step.version,AgentExecutionCancellationScope::Task,
        &NewAgentExecutionEvent {event_type:AgentExecutionEventKind::StepChanged,step_id:Some(step.clone()),attempt_id:None,
            actor:AgentExecutionActor::user(principal),payload:"{}".into()}).await.unwrap();
    let detail=fixture.voice_execution.get(principal,&execution).await.unwrap();let attempt=detail.attempts.last().unwrap();
    let pending=native_delivery_observation(fixture.owner.as_ref(),principal,&execution,&step,attempt,&key,true).await.unwrap();
    assert!(pending.pending);assert!(pending.native.is_none());assert_eq!(pending.state["state"],"running");
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    fixture.owner.canonical().store().cancel_active_turn(&session,"exact-original-child-stop".into(),"session_api".into()).await.unwrap();
    let delivered=native_delivery_observation(fixture.owner.as_ref(),principal,&execution,&step,attempt,&key,true).await.unwrap();
    assert!(!delivered.pending);assert_eq!(delivered.native.as_ref().unwrap().0,operation);
    let output=fixture.owner.read_turn_output(principal,session.as_ref(),Some(&key)).await.unwrap().unwrap();
    assert!(output.workspace_changed,"a cancelled task may already have changed shared workspace state");
    assert_eq!(output.canonical_operation_id,operation);
    assert_eq!(output.terminal_event_id.as_deref(),Some(delivered.native.as_ref().unwrap().1.as_str()));
    assert!(std::fs::read_to_string(PathBuf::from(&fixture.host.options.workspace).join("cancelled-child-partial.txt")).unwrap().contains("real fixture bytes"));
    fixture.finish().await;
}

#[tokio::test]
async fn a_new_queued_cancel_does_not_hide_an_earlier_failed_attempt_workspace_change() {
    use nomifun_db::{SettleAgentExecutionAttemptParams,RetryAgentExecutionStep,CreateAgentExecutionAttemptParams,AgentExecutionAttemptSessionKind};
    use nomifun_common::{ExecutionAttemptStatus,ExecutionStepStatus};
    let fixture=Fixture::new("cancelled-queued-after-native-writer").await;
    let (execution,step,key,operation,running)=setup(&fixture).await;partial_write(&fixture,&operation).await;
    let principal=&fixture.host.options.user_id;let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let store=fixture.owner.canonical().store();
    let started=store.read_turn_receipt(&session,&operation.clone().into()).await.unwrap().started_event.unwrap();
    let id=uuid::Uuid::now_v7().to_string();
    let terminal=store.append_event(&SessionEventAppend {agent_session_id:session.clone(),event_id:id.clone().into(),
        producer_id:"runtime_supervisor".into(),idempotency_key:id.into(),semantic_event:SemanticSessionEventDraft {
            kind:SessionEventKind("turn/failed".into()),kind_version:1,correlation_id:operation.clone().into(),causation_event_id:Some(started.event_id),
            payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"error":{"code":"fixture_partial_write","message":"The original owner failed after its partial write"}})))},
    }).await.unwrap().record.unwrap();
    let repository=SqliteAgentExecutionRepository::new(fixture.pool().clone());
    let old=running.current_attempt.as_ref().unwrap().attempt.clone();
    let failed=repository.settle_attempt(principal,&execution,&step,running.step.version,&old.attempt_id,old.version,None,
        &SettleAgentExecutionAttemptParams {expected_active_session_turn:None,expected_terminal_turn:None,
            attempt_status:ExecutionAttemptStatus::Failed,step_status:ExecutionStepStatus::Failed,execution_status:None,
            question:Some(None),error:Some(Some("original partial write failure".into())),output_summary:None,output_files:None,tokens:None,
            retry_after:Some(None),runtime_state:Some(None),started_at:None,finished_at:Some(Some(nomifun_common::now_ms())),loop_repeat_reset:None},
        &NewAgentExecutionEvent {event_type:AgentExecutionEventKind::AttemptChanged,step_id:Some(step.clone()),attempt_id:Some(old.attempt_id.clone()),
            actor:AgentExecutionActor::system(),payload:json!({"step_status":"failed","attempt_status":"failed", "canonical_operation_id":operation,"terminal_event_id":terminal.event_id}).to_string()}).await.unwrap();
    let version=repository.get_execution(principal,&execution).await.unwrap().unwrap().version;
    let reset=repository.reset_steps_for_retry(principal,&execution,version,&[RetryAgentExecutionStep {step_id:step.clone(),expected_step_version:failed.step.version}],
        &NewAgentExecutionEvent {event_type:AgentExecutionEventKind::StepChanged,step_id:Some(step.clone()),attempt_id:None,
            actor:AgentExecutionActor::user(principal),payload:"{}".into()}).await.unwrap();
    let reset_step=reset.steps.iter().find(|row|row.step_id==step).unwrap();
    let queued=repository.create_attempt(principal,&execution,&step,reset_step.version,None,
        &CreateAgentExecutionAttemptParams {participant_id:old.participant_id.clone(),start_immediately:false,trigger_reason:"retry".into(),
            effective_config:"{}".into(),retry_after:None,runtime_state:None},
        &NewAgentExecutionEvent {event_type:AgentExecutionEventKind::AttemptChanged,step_id:Some(step.clone()),attempt_id:None,
            actor:AgentExecutionActor::system(),payload:"{}".into()}).await.unwrap();
    let queued_attempt=queued.current_attempt.as_ref().unwrap().attempt.clone();
    let version=repository.get_execution(principal,&execution).await.unwrap().unwrap().version;
    repository.cancel_step(principal,&execution,version,&step,queued.step.version,AgentExecutionCancellationScope::Task,
        &NewAgentExecutionEvent {event_type:AgentExecutionEventKind::StepChanged,step_id:Some(step.clone()),attempt_id:None,
            actor:AgentExecutionActor::user(principal),payload:"{}".into()}).await.unwrap();
    let detail=fixture.voice_execution.get(principal,&execution).await.unwrap();
    let old_dto=detail.attempts.iter().find(|row|row.attempt_id==old.attempt_id).unwrap();
    let queued_dto=detail.attempts.iter().find(|row|row.attempt_id==queued_attempt.attempt_id).unwrap();
    assert!(queued_dto.conversation_id.is_none());
    let queued_key=nomifun_db::AttemptConversationEffects::default().current_turn_operation_key(&queued_attempt.attempt_id).unwrap();
    let zero=native_delivery_observation(fixture.owner.as_ref(),principal,&execution,&step,queued_dto,&queued_key,true).await.unwrap();
    assert!(!zero.pending);assert!(zero.native.is_none());assert_eq!(zero.state["state"],"not_started");
    let original=native_delivery_observation(fixture.owner.as_ref(),principal,&execution,&step,old_dto,&key,true).await.unwrap();
    assert!(!original.pending);assert!(original.native.is_some());
    let output=fixture.owner.read_turn_output(principal,session.as_ref(),Some(&key)).await.unwrap().unwrap();
    assert!(output.workspace_changed,"the latest zero-start cancellation cannot erase an earlier exact writer");
    assert_eq!(output.terminal_event_id.as_deref(),Some(original.native.as_ref().unwrap().1.as_str()));
    assert!(repository.start_attempt(principal,&execution,&step,queued.step.version,&queued_attempt.attempt_id,queued_attempt.version,
        session.as_ref(),AgentExecutionAttemptSessionKind::ChildAttempt,None,
        &NewAgentExecutionEvent {event_type:AgentExecutionEventKind::AttemptChanged,step_id:Some(step.clone()),attempt_id:Some(queued_attempt.attempt_id.clone()),
            actor:AgentExecutionActor::system(),payload:"{}".into()}).await.is_err(),"the durable Stop blocks a late queued startup");
    fixture.finish().await;
}
