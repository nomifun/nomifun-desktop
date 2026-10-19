//! Real App Robot dispatch and formal question presentation callback. The test
//! actor accepts a presentation; it provides no audible-playback evidence.
use super::*;

async fn capture_recording(backend:&Arc<dyn nomifun_robot::wiring::RobotConversationBackend>,request:&mut nomifun_robot::services::RobotTurnRequest) {
    let context=backend.capture_input_context(&request.companion_id,&request.robot_id,
        &request.conversation_id,&request.connection_id,request.presented_question_id.clone()).await.unwrap()
        .expect("the real App backend captures recording-time canonical context");
    request.presented_question_id=context.presented_question_id.clone();
    request.captured_context=Some(context);
}

async fn dispatch_recording(flow:&PhysicalEntryFixture,backend:&Arc<dyn nomifun_robot::wiring::RobotConversationBackend>,
    request:&nomifun_robot::services::RobotTurnRequest)->(Vec<nomifun_robot::services::TurnEvent>,OperationId,nomifun_agent_contracts::EventId) {
    use nomifun_robot::services::TurnEvent;
    let mut events=backend.dispatch(request.clone()).await.unwrap();
    let events=tokio::time::timeout(Duration::from_secs(30),async {let mut output=Vec::new();loop {
        let event=events.recv().await.expect("the real Robot bridge must retain its terminal delivery");
        if let TurnEvent::Failed {message,..}=&event {panic!("direct Robot question dispatch failed: {message}");}
        let terminal=matches!(event,TurnEvent::Done);output.push(event);if terminal {break output;}
    }}).await.expect("the real Robot bridge must deliver its canonical public result");
    let key=crate::robot_wiring::robot_turn_idempotency_key(request);
    let principal=nomifun_agent_contracts::PrincipalRef {principal_kind:"user".into(),principal_id:flow.owner.clone()};
    let accepted=flow.fixture.owner.canonical().session_input_by_key(&principal,&flow.robot_id,&key).await.unwrap().unwrap();
    let root=accepted.event.event_id;
    let operation=flow.fixture.owner.canonical().store().session_input_claim(&flow.robot_id,&root).await.unwrap().unwrap().operation_id;
    wait_for_completed_turn(&flow.fixture,&flow.robot_id,&operation).await;
    (events,operation,root)
}

#[tokio::test]
async fn direct_robot_question_dispatch_preserves_presented_source_natural_reply_and_absorbing_replay() {
    use nomifun_robot::services::TurnEvent;
    let server=CompanionFlowModelServer::with_question_mode().await;
    let flow=PhysicalEntryFixture::new(&server,"direct-robot-question-dispatch").await;
    let backend=flow.fixture.services.robot.as_ref().unwrap().conversation_backend_for_test();
    let store=flow.fixture.owner.canonical().store();
    let mut question=flow.request("direct-question-request",DIRECT_QUERY);
    capture_recording(&backend,&mut question).await;
    let (question_events,question_operation,question_root)=dispatch_recording(&flow,&backend,&question).await;
    let (source,presentation)=question_events.iter().find_map(|event|match event {
        TurnEvent::Question {source,text,presentation_id}=>{
            assert_eq!(text,DIRECT_QUESTION);Some((source.clone(),presentation_id.clone()))
        },_=>None,
    }).expect("production dispatch returns the typed direct-question event");
    assert_eq!(question_events.iter().filter(|event|matches!(event,TurnEvent::Question {..})).count(),1);
    assert_eq!(presentation,format!("robot-question:{}:{}",question_root.as_ref(),source.through_seq));
    let receipt=store.read_turn_receipt(&flow.robot_id,&question_operation).await.unwrap();
    let facts=store.turn_output_facts(&flow.robot_id,&question_operation).await.unwrap();
    let public=nomifun_agent_session::canonical_turn_public_result(&facts,&receipt).unwrap();
    assert_eq!(public.user_input_request.as_ref().unwrap().request_id.as_ref(),"direct-ask-name");
    assert_eq!(public.user_input_request.as_ref().unwrap().question,DIRECT_QUESTION);
    assert!(matches!(public.turn_outcome,Some(nomifun_agent_contracts::AgentTurnOutcome::InputRequired {..})));
    assert_eq!(facts.events.iter().filter(|event|event.kind.0=="robot/question-playback-accepted").count(),0);

    // This is the same callback the physical connection actor invokes after
    // accepting its exact source. It establishes no claim of audio consumption.
    backend.question_presented(&question,&source,&presentation).await.unwrap();
    let presented=store.chat_causality_facts(&flow.robot_id,&question_operation).await.unwrap();
    let proof=presented.events.iter().find(|event|event.kind.0=="robot/question-playback-accepted").unwrap();
    let proof_payload=presented.event_payloads.get(proof.event_id.as_ref()).unwrap();
    assert_eq!(proof_payload["notice_message_id"],source.message_id);assert_eq!(proof_payload["notice_seq"],source.through_seq);
    assert_eq!(proof_payload["presentation_id"],presentation);assert_eq!(proof_payload["connection_id"],flow.connection);
    assert!(proof.seq>source.through_seq);

    let mut answer=flow.request("direct-natural-answer",DIRECT_ANSWER);
    answer.presented_question_id=Some(presentation.clone());capture_recording(&backend,&mut answer).await;
    assert_eq!(answer.presented_question_id.as_deref(),Some(presentation.as_str()));
    let (answer_events,answer_operation,answer_root)=dispatch_recording(&flow,&backend,&answer).await;
    assert_ne!(answer_operation,question_operation);
    let spoken=answer_events.iter().filter_map(|event|match event {
        TurnEvent::Text(text)|TurnEvent::PublicSpeech {text,..}=>Some(text.as_str()),_=>None,
    }).collect::<String>();
    assert_eq!(spoken,DIRECT_RESULT);
    let (_,accepted)=store.turn_accepted_input(&flow.robot_id,&answer_operation).await.unwrap();
    assert_eq!(accepted.0["content"],DIRECT_ANSWER);
    assert_eq!(accepted.0["reply_to"],json!({"turn_id":question_root,"question_seq":source.through_seq}));
    assert_eq!(accepted.0["input_source"]["presented_question_id"],presentation);
    assert_eq!(accepted.0["input_source"]["robot_id"],flow.device_id);assert_eq!(accepted.0["input_source"]["connection_id"],flow.connection);
    let before_calls=server.state.requests.lock().unwrap().len();assert_eq!(before_calls,2,"the question and its direct answer each use one main-model request");
    let turn_count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(flow.robot_id.as_ref()).fetch_one(flow.fixture.pool()).await.unwrap();
    assert_eq!(turn_count,2,"direct clarification does not create hidden result-model Turns");
    assert!(flow.fixture.voice_execution.list(&flow.owner,Some(20),Some(0)).await.unwrap().is_empty());
    let key=crate::robot_wiring::robot_turn_idempotency_key(&answer);
    let replay=flow.fixture.owner.send_robot_session_message_idempotent(&flow.owner,&answer,&key,robot_message(DIRECT_ANSWER)).await.unwrap();
    assert!(replay.replayed);assert_eq!(replay.message_id,answer_root.as_ref());
    assert_eq!(server.state.requests.lock().unwrap().len(),before_calls);
    let replay_count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(flow.robot_id.as_ref()).fetch_one(flow.fixture.pool()).await.unwrap();
    assert_eq!(replay_count,turn_count,"the same answer key returns its original acknowledgement");

    let mut next=flow.request("direct-next-topic",FOLLOWUP_INPUT);
    next.presented_question_id=Some(presentation.clone());capture_recording(&backend,&mut next).await;
    assert!(next.presented_question_id.is_none(),"a newly recorded ordinary input drops the already answered question");
    let (_,next_operation,_)=dispatch_recording(&flow,&backend,&next).await;
    let (_,next_input)=store.turn_accepted_input(&flow.robot_id,&next_operation).await.unwrap();
    assert!(next_input.0.get("reply_to").is_none());assert!(next_input.0["input_source"]["presented_question_id"].is_null());
    let mut delayed=next.clone();delayed.request_id="direct-old-recording".into();
    api(&flow.fixture._router,&format!("/api/agent-sessions/{}/clear-context",flow.robot_id.as_ref()),json!({"idempotency_key":"direct-question-clear"})).await;
    let before_rejected=store.head(&flow.robot_id).await.unwrap().last_seq;
    let old_key=crate::robot_wiring::robot_turn_idempotency_key(&delayed);
    assert!(matches!(flow.fixture.owner.send_robot_session_message_idempotent(&flow.owner,&delayed,&old_key,robot_message(&delayed.text)).await,
        Err(nomifun_common::AppError::Conflict(_))));
    assert_eq!(store.head(&flow.robot_id).await.unwrap().last_seq,before_rejected);
    let mut fresh=flow.request("direct-fresh-after-clear",FOLLOWUP_INPUT);
    fresh.presented_question_id=Some(presentation);capture_recording(&backend,&mut fresh).await;
    assert!(fresh.presented_question_id.is_none());
    assert!(fresh.captured_context.as_ref().unwrap().context_floor>next.captured_context.as_ref().unwrap().context_floor);
    let (_,fresh_operation,_)=dispatch_recording(&flow,&backend,&fresh).await;
    assert!(store.turn_accepted_input(&flow.robot_id,&fresh_operation).await.unwrap().1.0.get("reply_to").is_none());
    assert!(flow.fixture.voice_execution.list(&flow.owner,Some(20),Some(0)).await.unwrap().is_empty());
    record_fixture_cost("physical_direct_question",&server.state.requests.lock().unwrap(),&flow);
    flow.finish().await;server.close().await;
}

#[tokio::test]
async fn robot_completed_delivery_refreshes_original_source_when_terminal_commits_between_snapshots() {
    use nomifun_robot::services::TurnEvent;
    let server=CompanionFlowModelServer::with_held_work().await;
    let flow=PhysicalEntryFixture::new(&server,"robot-terminal-between-snapshots").await;
    let backend=flow.fixture.services.robot.as_ref().unwrap().conversation_backend_for_test();
    let mut request=flow.request("original-two-snapshot-request",HELD_INPUT);capture_recording(&backend,&mut request).await;
    let key=crate::robot_wiring::robot_turn_idempotency_key(&request);
    let accepted=flow.fixture.owner.send_robot_session_message_idempotent(&flow.owner,&request,&key,robot_message(HELD_INPUT)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(20),server.state.held_entered.notified()).await.expect("the original real model request reaches its response barrier");
    let store=flow.fixture.owner.canonical().store();let root=nomifun_agent_contracts::EventId::from(accepted.message_id);
    let operation=store.session_input_claim(&flow.robot_id,&root).await.unwrap().unwrap().operation_id;
    let mut delivered=std::collections::BTreeMap::new();
    let before=flow.fixture.owner.robot_public_output(&flow.owner,flow.robot_id.as_ref(),&key,0,&mut delivered).await.unwrap();
    assert!(before.segments.is_empty()&&delivered.is_empty(),"snapshot A precedes the original final publication");
    assert!(matches!(flow.fixture.owner.session_turn_delivery_state(&flow.owner,flow.robot_id.as_ref(),&key).await.unwrap(),
        nomifun_conversation::PublicTurnDeliveryState::Accepted {..}));
    server.state.held_release.notify_one();
    let terminal=wait_for_completed_turn(&flow.fixture,&flow.robot_id,&operation).await;
    let nomifun_conversation::PublicTurnDeliveryState::Completed(receipt)=flow.fixture.owner
        .session_turn_delivery_state(&flow.owner,flow.robot_id.as_ref(),&key).await.unwrap() else {panic!("snapshot B must observe the actual committed terminal");};
    assert!(before.through_seq<terminal.terminal_event.as_ref().unwrap().seq);
    assert!(matches!(receipt.turn_outcome,Some(nomifun_agent_contracts::AgentTurnOutcome::Answered {..})));
    let cursor=store.current_cursor(&flow.robot_id).await.unwrap();let calls=server.state.requests.lock().unwrap().len();assert_eq!(calls,1);
    let events=crate::robot_wiring::completed_robot_public_delivery_events(&flow.fixture.owner,&flow.owner,&request,&key,
        &receipt,before.through_seq,&mut delivered).await.unwrap();
    let [TurnEvent::PublicSpeech {source,text},TurnEvent::Done]=events.as_slice() else {panic!("the completed refresh must deliver the original source before Done: {events:?}");};
    assert_eq!(text,"The independent message is answered.");assert_eq!(source.through_seq,terminal.terminal_event.as_ref().unwrap().seq);
    let facts=store.turn_output_facts(&flow.robot_id,&operation).await.unwrap();
    let source_event=facts.events.iter().find(|event|event.event_id.as_ref()==source.message_id).expect("the speech reference names the original committed canonical event");
    assert_eq!(source_event.kind.0,"message/completed");assert_eq!(source.revision,source_event.seq);
    let repeated=crate::robot_wiring::completed_robot_public_delivery_events(&flow.fixture.owner,&flow.owner,&request,&key,
        &receipt,before.through_seq,&mut delivered).await.unwrap();assert_eq!(repeated,vec![TurnEvent::Done],"the refresh cannot replay an already delivered prefix");
    assert_eq!(store.current_cursor(&flow.robot_id).await.unwrap(),cursor);assert_eq!(server.state.requests.lock().unwrap().len(),calls);
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(flow.robot_id.as_ref()).fetch_one(flow.fixture.pool()).await.unwrap();assert_eq!(count,1);
    flow.finish().await;server.close().await;
}
