//! Exact source selections are prepared through real canonical Store input
//! facts. Accepted-but-unapplied and post-command inputs cannot leak into work.
use super::*;
use crate::router::nomi_core_session::{canonical_source_input,canonical_input_at_tool};
use nomifun_agent_contracts::{EventId,EventProducerId,IdempotencyKey,SemanticSessionEventDraft,SessionEventAppend,SessionEventKind,SessionEventPayloadRef};
use nomifun_agent_runtime::{AgentEngineEvent,AgentSteeringInput};
use nomifun_agent_session::AgentSessionStore;

async fn progress(store:&AgentSessionStore,session:&AgentSessionId,operation:&OperationId,event:AgentEngineEvent)->u64 {
    let id=uuid::Uuid::now_v7().to_string();
    let started=store.read_turn_receipt(session,operation).await.unwrap().started_event.unwrap();
    store.append_event(&SessionEventAppend {agent_session_id:session.clone(),event_id:EventId::from(id.clone()),
        producer_id:EventProducerId::from("runtime_supervisor"),idempotency_key:IdempotencyKey::from(id),
        semantic_event:SemanticSessionEventDraft {kind:SessionEventKind("runtime/progress-recorded".into()),kind_version:1,
            correlation_id:operation.as_ref().into(),causation_event_id:Some(started.event_id),
            payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"producer_seq":store.current_cursor(session).await.unwrap().seq,"event":event})))},
    }).await.unwrap().cursor.seq
}

async fn steering(store:&AgentSessionStore,session:&AgentSessionId,operation:&OperationId,label:&str,applied:bool)->u64 {
    let file=format!("{label}.txt");let skill=format!("skill-{label}");
    let (_,saved)=store.steer_exact_turn(session,operation,format!("source-steer-{label}").into(),"session_api".into(),
        StrictJsonValue(json!({"content":format!("Constraint {label}"),"files":[file],"inject_skills":[skill]}))).await.unwrap();
    let receipt=saved.record.unwrap();
    if applied {progress(store,session,operation,AgentEngineEvent::SteeringInputs {inputs:vec![AgentSteeringInput {
        receipt_operation_id:receipt.event_id.as_ref().into(),message_id:receipt.event_id.as_ref().into(),
        text:format!("Constraint {label}"),files:vec![file],inject_skills:vec![skill],image_count:0,
        prepared_images:vec![],prepared_skill_instructions:vec![],
    }]}).await} else {receipt.seq}
}

fn request()->nomifun_api_types::SendMessageRequest {
    nomifun_api_types::SendMessageRequest {content:"only this assigned child work".into(),files:vec![],inject_skills:vec![],
        hidden:false,origin:Some("agent_execution".into()),channel_platform:None}
}

#[tokio::test]
async fn running_child_message_inherits_only_selections_visible_to_its_original_command() {
    use nomifun_db::{IAgentExecutionRepository,SqliteAgentExecutionRepository,NewAgentExecutionEvent,AgentExecutionMessageSource};
    use nomifun_common::{AgentExecutionActor,AgentExecutionEventKind};
    let fixture=Fixture::with_delivery("running-message-selections",vec!["A.txt".into()],vec!["skill-A".into()],None).await;
    let (execution,step,running)=voice_running_decision_attempt_fixture(&fixture).await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let turn=OperationId::from("cleanup-retry-turn");let store=fixture.owner.canonical().store();
    steering(store,&session,&turn,"B",true).await;
    steering(store,&session,&turn,"queued",false).await;
    let key="running-input-once";let content="Apply B while continuing the original child";
    progress(store,&session,&turn,AgentEngineEvent::ToolCallCompleted {step:1,call:nomifun_chat_model_broker::ChatToolCall {
        call_id:"running-child-message".into(),name:"agent_message".into(),provider_metadata:None,
        arguments:StrictJsonValue(json!({"execution_id":execution,"step_id":step,"content":content,"idempotency_key":key})),
    }}).await;
    let facts=store.turn_output_facts(&session,&turn).await.unwrap();
    let command=facts.events.iter().find(|event|facts.event_payloads.get(event.event_id.as_ref())
        .is_some_and(|value|value.pointer("/event/call/call_id")==Some(&json!("running-child-message")))).unwrap();
    let source=AgentExecutionMessageSource {agent_session_id:session.as_ref().into(),turn_operation_id:turn.as_ref().into(),
        native_fence:1,native_generation:1,command_event_id:command.event_id.as_ref().into()};
    let operation=format!("agent-message:{}",nomifun_agent_contracts::digest_bytes(&serde_json::to_vec(&(
        session.as_ref(),turn.as_ref(),&execution,&step,key)).unwrap()).as_ref());
    let repository=SqliteAgentExecutionRepository::new(fixture.pool().clone());let owner=&fixture.host.options.user_id;
    let version=repository.get_execution(owner,&execution).await.unwrap().unwrap().version;
    let attempt_id=running.current_attempt.unwrap().attempt.attempt_id;
    repository.append_event(owner,&execution,version,&NewAgentExecutionEvent {event_type:AgentExecutionEventKind::StepChanged,
        step_id:Some(step),attempt_id:Some(attempt_id.clone()),actor:AgentExecutionActor::agent(session.as_ref(),Some(attempt_id)),
        payload:json!({"change":"conversation_effect_requested","effect":"steer","operation_id":operation,
            "content_digest":nomifun_agent_contracts::digest_bytes(content.as_bytes()),"message_source":source}).to_string(),
    }).await.unwrap();
    steering(store,&session,&turn,"late",true).await;
    let mut original=request();original.content=content.into();
    let prepared=fixture.owner.prepare_execution_steering_input(owner,&session,&operation,original.clone()).await.unwrap();
    assert_eq!(prepared.files,["A.txt","B.txt"]);assert_eq!(prepared.inject_skills,["skill-A","skill-B"]);
    assert_eq!(prepared.content,content);
    let replay=fixture.owner.prepare_execution_steering_input(owner,&session,&operation,original.clone()).await.unwrap();
    assert_eq!(replay.files,prepared.files);assert_eq!(replay.inject_skills,prepared.inject_skills);
    let unchanged=fixture.owner.prepare_execution_steering_input(owner,&session,"ordinary-ui-steer",original.clone()).await.unwrap();
    assert!(unchanged.files.is_empty()&&unchanged.inject_skills.is_empty());
    original.content="changed command bytes".into();
    assert!(fixture.owner.prepare_execution_steering_input(owner,&session,&operation,original).await.is_err());
    fixture.finish().await;
}

async fn answer_tool(store:&AgentSessionStore,session:&AgentSessionId,turn:&OperationId,key:&str) {
    let started=store.read_turn_receipt(session,turn).await.unwrap().started_event.unwrap();
    let id=uuid::Uuid::now_v7().to_string();
    store.append_event(&SessionEventAppend {agent_session_id:session.clone(),event_id:id.clone().into(),producer_id:"runtime_supervisor".into(),
        idempotency_key:id.clone().into(),semantic_event:SemanticSessionEventDraft {
            kind:SessionEventKind("tool/call-started".into()),kind_version:1,correlation_id:id.clone().into(),causation_event_id:Some(started.event_id),
            payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"operation_id":key,"call_id":id,
                "capability_id":"session.management","action_id":"session.management/execution_answer_decision"})))},
    }).await.unwrap();
}

#[tokio::test]
async fn question_tool_reads_its_exact_applied_input_and_preserves_recording_authority_before_its_cause() {
    let fixture=Fixture::with_delivery("question-tool-exact-steering-input",vec!["A.txt".into()],vec![],None).await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());let turn=OperationId::from("cleanup-retry-turn");
    let store=fixture.owner.canonical().store();let capability="session.management".into();let action="session.management/execution_answer_decision".into();
    answer_tool(store,&session,&turn,"before-steering").await;
    let (_,root)=canonical_input_at_tool(store,&session,&turn,&"before-steering".into(),&capability,&action).await.unwrap();
    assert_eq!(root.0["files"],json!(["A.txt"]));
    let source=json!({"kind":"authenticated_robot","companion_id":uuid::Uuid::now_v7().to_string(),
        "robot_id":uuid::Uuid::now_v7().to_string(),"connection_id":"original-recording-socket","presented_question_id":"original-presented-question"});
    let (_,accepted)=store.steer_exact_turn(&session,&turn,"actual-answer-input".into(),"session_api".into(),
        StrictJsonValue(json!({"content":"小明","files":["B.txt"],"inject_skills":[],"origin":"robot","input_source":source}))).await.unwrap();
    let accepted=accepted.record.unwrap();
    progress(store,&session,&turn,AgentEngineEvent::SteeringInputs {inputs:vec![AgentSteeringInput {
        receipt_operation_id:accepted.event_id.as_ref().into(),message_id:accepted.event_id.as_ref().into(),text:"小明".into(),
        files:vec!["B.txt".into()],inject_skills:vec![],image_count:0,prepared_images:vec![],prepared_skill_instructions:vec![],
    }]}).await;
    steering(store,&session,&turn,"accepted-not-applied",false).await;
    answer_tool(store,&session,&turn,"original-answer-tool").await;
    steering(store,&session,&turn,"applied-after-answer-tool",true).await;
    let (original,input)=canonical_input_at_tool(store,&session,&turn,&"original-answer-tool".into(),&capability,&action).await.unwrap();
    assert_eq!(original.event_id,accepted.event_id);assert_eq!(input.0["content"],"小明");
    assert_eq!(input.0["files"],json!(["B.txt"]));assert_eq!(input.0["input_source"],source);
    assert!(canonical_input_at_tool(store,&session,&turn,&"missing-tool".into(),&capability,&action).await.is_err());
    assert!(canonical_input_at_tool(store,&session,&turn,&"original-answer-tool".into(),&capability,&"session.management/execution_cancel".into()).await.is_err());
    fixture.finish().await;
}

#[tokio::test]
async fn initial_source_inherits_applied_selections_and_excludes_unapplied_queue_input() {
    let fixture=Fixture::with_delivery("source-selection-initial",vec!["A.txt".into()],vec!["skill-A".into()],None).await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());let operation=OperationId::from("cleanup-retry-turn");
    let store=fixture.owner.canonical().store();
    steering(store,&session,&operation,"applied-A",true).await;
    steering(store,&session,&operation,"unapplied-A",false).await;
    let cutoff=store.current_cursor(&session).await.unwrap().seq+1;
    let original=store.turn_output_facts(&session,&operation).await.unwrap();
    let input=canonical_source_input(store,&session,&operation,cutoff).await.unwrap();
    let mut prepared=request();input.prepare_request(&mut prepared);
    assert_eq!(prepared.files,["A.txt","applied-A.txt"]);
    assert_eq!(prepared.inject_skills,["skill-A","skill-applied-A"]);
    assert_eq!(prepared.content,"only this assigned child work");
    let after=store.turn_output_facts(&session,&operation).await.unwrap();
    assert_eq!(after.events,original.events);assert_eq!(after.event_payloads,original.event_payloads);
    fixture.finish().await;
}

#[tokio::test]
async fn followup_selects_exact_b_command_boundary_without_reinjecting_a_or_changing_history() {
    let fixture=Fixture::with_delivery("source-selection-followup",vec!["A.txt".into()],vec!["skill-A".into()],None).await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());let original=OperationId::from("cleanup-retry-turn");
    let store=fixture.owner.canonical().store();
    steering(store,&session,&original,"applied-A",true).await;
    store.cancel_active_turn(&session,"end-A-for-exact-followup".into(),"session_api".into()).await.unwrap();
    let history=store.turn_output_facts(&session,&original).await.unwrap();
    let operation=OperationId::from("exact-B-followup");
    store.start_turn(&session,"session_api".into(),"accepted-B-once".into(),operation.clone(),
        StrictJsonValue(json!({"content":"Continue the original child with new B input","files":["B.txt"],"inject_skills":["skill-B"]}))).await.unwrap();
    steering(store,&session,&operation,"applied-B",true).await;
    steering(store,&session,&operation,"unapplied-B",false).await;
    let cutoff=progress(store,&session,&operation,AgentEngineEvent::ToolCallCompleted {step:1,call:nomifun_chat_model_broker::ChatToolCall {
        call_id:"exact-B-message".into(),name:"agent_message".into(),provider_metadata:None,
        arguments:StrictJsonValue(json!({"execution_id":"execution","step_id":"child","content":"B guidance","idempotency_key":"B-message-once"})),
    }}).await;
    steering(store,&session,&operation,"after-command-B",true).await;
    let input=canonical_source_input(store,&session,&operation,cutoff).await.unwrap();
    let mut prepared=request();input.prepare_request(&mut prepared);
    assert_eq!(prepared.files,["B.txt","applied-B.txt"]);
    assert_eq!(prepared.inject_skills,["skill-B","skill-applied-B"]);
    assert_eq!(prepared.content,"only this assigned child work");
    let after=store.turn_output_facts(&session,&original).await.unwrap();
    assert_eq!(after.events,history.events);assert_eq!(after.event_payloads,history.event_payloads);
    fixture.finish().await;
}
