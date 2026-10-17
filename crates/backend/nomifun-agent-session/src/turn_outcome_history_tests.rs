use super::*;
use nomifun_agent_contracts::{AgentTurnOutcome,canonical_assistant_step_message_id};

async fn record_wait(store:&AgentSessionStore,sid:&AgentSessionId,operation:&OperationId,start:&EventId,key:&str,request_id:Option<&str>) -> u64 {
    if let Some(id)=request_id {
        store.append_event(&append(sid,&format!("{key}-question"),"runtime_supervisor",&format!("{key}-question"),
            "runtime/progress-recorded",operation.as_ref(),Some(start.clone()),json!({"producer_seq":1,"event":{
                "event":"user_input_requested","step":1,"request":{"request_id":id,"question":"Complete the pending action","remaining_work":"Verify and continue"}}}))).await.unwrap();
    }
    store.append_event(&append(sid,&format!("{key}-wait"),"runtime_supervisor",&format!("{key}-wait"),
        "runtime/progress-recorded",operation.as_ref(),Some(start.clone()),json!({"producer_seq":2,"event":{
            "event":"turn_waiting","model_steps":1,"request_id":request_id,"reason":if request_id.is_some(){"user_input"}else{"intervention_required"}}}))).await.unwrap().ack.unwrap().seq
}

#[tokio::test]
async fn waiting_reply_cas_preserves_exact_turn_question_and_original_idempotent_receipt() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"reply-cas").await;
    let sid=&session.agent_session_id;let operation=OperationId::from("reply-original");
    let(input,start)=store.start_turn(sid,"session_api".into(),"reply-start".into(),operation.clone(),StrictJsonValue(json!({"content":"Authorize"}))).await.unwrap();
    let root=input.ack.unwrap().event_id;let start=start.ack.unwrap().event_id;
    let first_wait_seq=record_wait(&store,sid,&operation,&start,"first",Some("authorize-a")).await;
    let input=StrictJsonValue(json!({"content":"Done"}));
    let before=store.current_cursor(sid).await.unwrap();
    for (target,wait_seq) in [(uuid::Uuid::now_v7().to_string(),first_wait_seq),(root.as_ref().to_owned(),first_wait_seq+100),(root.as_ref().to_owned(),0)] {
        assert!(store.steer_active_turn_for_wait(sid,"bad-reply".into(),"session_api".into(),input.clone(),&target,wait_seq).await.is_err());
    }
    assert_eq!(store.current_cursor(sid).await.unwrap(),before);
    let(target,accepted)=store.steer_active_turn_for_wait(sid,"reply-a".into(),"session_api".into(),input.clone(),root.as_ref(),first_wait_seq).await.unwrap();
    assert_eq!(target,operation);let accepted_id=accepted.record.unwrap().event_id;
    assert!(store.steer_active_turn_for_wait(sid,"second-answer".into(),"session_api".into(),input.clone(),root.as_ref(),first_wait_seq).await.is_err());
    store.append_event(&append(sid,"first-resolved","runtime_supervisor","first-resolved","runtime/progress-recorded",operation.as_ref(),Some(start.clone()),
        json!({"producer_seq":3,"event":{"event":"turn_wait_resolved","model_steps":1}}))).await.unwrap();
    let second_wait_seq=record_wait(&store,sid,&operation,&start,"second",Some("authorize-b")).await;
    assert!(store.steer_active_turn_for_wait(sid,"stale-a".into(),"session_api".into(),input.clone(),root.as_ref(),first_wait_seq).await.is_err());
    assert!(matches!(store.steer_active_turn_for_wait(sid,"reply-a".into(),"session_api".into(),input.clone(),root.as_ref(),second_wait_seq).await,Err(SessionStoreError::IdempotencyConflict(_))));
    assert!(matches!(store.steer_active_turn(sid,"reply-a".into(),"session_api".into(),input.clone()).await,Err(SessionStoreError::IdempotencyConflict(_))),"a retry cannot remove its exact target guard");
    store.cancel_active_turn(sid,"end-original".into(),"session_api".into()).await.unwrap();
    store.start_turn(sid,"session_api".into(),"reply-successor".into(),"reply-next".into(),StrictJsonValue(json!({"content":"Independent task"}))).await.unwrap();
    let before=store.current_cursor(sid).await.unwrap();
    assert!(store.steer_active_turn_for_wait(sid,"late-old".into(),"session_api".into(),input.clone(),root.as_ref(),second_wait_seq).await.is_err());
    let(target,replay)=store.steer_active_turn_for_wait(sid,"reply-a".into(),"session_api".into(),input,root.as_ref(),first_wait_seq).await.unwrap();
    assert_eq!(target,operation);assert!(replay.duplicate);assert_eq!(replay.record.unwrap().event_id,accepted_id);
    assert_eq!(store.current_cursor(sid).await.unwrap(),before);assert_eq!(store.head(sid).await.unwrap().active_turn_id.as_deref(),Some("reply-next"));
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn concurrent_requestless_wait_replies_are_consumed_once_by_the_canonical_writer() {
    let store=AgentSessionStore::open_in_memory_with_connections(2).await.unwrap();let(session,_)=create_ready(&store,"reply-race").await;
    let sid=&session.agent_session_id;let operation=OperationId::from("reply-requestless");
    let(input,start)=store.start_turn(sid,"session_api".into(),"requestless-start".into(),operation.clone(),StrictJsonValue(json!({"content":"Continue"}))).await.unwrap();
    let root=input.ack.unwrap().event_id;let wait_seq=record_wait(&store,sid,&operation,&start.ack.unwrap().event_id,"requestless",None).await;
    let input=StrictJsonValue(json!({"content":"Continue now"}));
    let(a,b)=tokio::join!(store.steer_active_turn_for_wait(sid,"reply-one".into(),"session_api".into(),input.clone(),root.as_ref(),wait_seq),
        store.steer_active_turn_for_wait(sid,"reply-two".into(),"session_api".into(),input,root.as_ref(),wait_seq));
    assert_ne!(a.is_ok(),b.is_ok(),"one exact wait can accept only one targeted reply");
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/steer-accepted'").bind(sid.as_ref()).fetch_one(store.test_pool()).await.unwrap();assert_eq!(count,1);
}

#[tokio::test]
async fn a_later_requestless_wait_requires_its_own_event_sequence_and_replays_the_original_receipt() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"wait-epoch").await;
    let sid=&session.agent_session_id;let operation=OperationId::from("wait-epoch-turn");
    let(input,start)=store.start_turn(sid,"session_api".into(),"wait-epoch-input".into(),operation.clone(),StrictJsonValue(json!({"content":"Keep the helper"}))).await.unwrap();
    let root=input.ack.unwrap().event_id;let start=start.ack.unwrap().event_id;
    let first=record_wait(&store,sid,&operation,&start,"epoch-first",None).await;
    let answer=StrictJsonValue(json!({"content":"Continue the original wait"}));
    let(_,original)=store.steer_active_turn_for_wait(sid,"epoch-first-answer".into(),"session_api".into(),answer.clone(),root.as_ref(),first).await.unwrap();
    let original_id=original.record.unwrap().event_id;
    store.append_event(&append(sid,"epoch-first-resolved","runtime_supervisor","epoch-first-resolved","runtime/progress-recorded",operation.as_ref(),Some(start.clone()),
        json!({"producer_seq":3,"event":{"event":"turn_wait_resolved","model_steps":1}}))).await.unwrap();
    let second=record_wait(&store,sid,&operation,&start,"epoch-second",None).await;
    assert!(second>first);
    let before=store.current_cursor(sid).await.unwrap();
    assert!(store.steer_active_turn_for_wait(sid,"late-epoch-first".into(),"session_api".into(),answer.clone(),root.as_ref(),first).await.is_err());
    assert_eq!(store.current_cursor(sid).await.unwrap(),before,"an old requestless notice cannot answer the new wait");
    let(_,replay)=store.steer_active_turn_for_wait(sid,"epoch-first-answer".into(),"session_api".into(),answer.clone(),root.as_ref(),first).await.unwrap();
    assert!(replay.duplicate);assert_eq!(replay.record.unwrap().event_id,original_id);
    assert_eq!(store.current_cursor(sid).await.unwrap(),before);
    let(_,current)=store.steer_active_turn_for_wait(sid,"epoch-second-answer".into(),"session_api".into(),answer,root.as_ref(),second).await.unwrap();
    assert!(!current.duplicate);
    let SessionEventPayloadRef::InlineJson(payload)=current.record.unwrap().payload else {panic!("wait reply is inline")};
    assert_eq!(payload.0["expected_wait"],json!({"turn_id":root,"wait_seq":second}));
}

#[tokio::test]
async fn active_waiting_is_atomic_outside_the_event_page_and_survives_projection_rebuild() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();
    let (session,_)=create_ready(&store,"waiting-page").await;
    let sid=&session.agent_session_id;let operation=OperationId::from("waiting-page-turn");
    let (_,start)=store.start_turn(sid,"session_api".into(),"waiting-input".into(),operation.clone(),
        StrictJsonValue(json!({"content":"Install and authorize"}))).await.unwrap();
    let start=start.ack.unwrap().event_id;
    for (index,event) in [json!({"event":"user_input_requested","step":1,"request":{
        "request_id":"authorize","question":"请完成浏览器授权","remaining_work":"检查授权结果"}}),
        json!({"event":"turn_waiting","model_steps":1,"request_id":"authorize","reason":"user_input"})].into_iter().enumerate() {
        let progress=append(sid,&format!("waiting-progress-{index}"),"runtime_supervisor",&format!("waiting-progress-{index}"),
            "runtime/progress-recorded",operation.as_ref(),Some(start.clone()),json!({"producer_seq":index+1,"event":event}));
        store.append_event(&progress).await.unwrap();
    }
    let expected=AgentTurnOutcome::Waiting {wait_seq:store.current_cursor(sid).await.unwrap().seq,request_id:Some("authorize".into()),reason:nomifun_agent_contracts::AgentWaitReason::UserInput};
    let page=store.observe(sid,None,1).await.unwrap();
    assert_eq!(page.events.len(),1);
    assert_eq!(page.head.status,"running");
    assert_eq!(page.active_turn_outcome,Some(expected.clone()));
    let plan=store.latest_runtime_state(sid,"plan_updated").await.unwrap();
    assert!(plan.event.is_none());assert_eq!(plan.turn_outcome,Some(expected.clone()));
    let before=store.read_events(sid,None,500).await.unwrap();
    store.rebuild_projections(sid).await.unwrap();
    let history=store.message_history_with_turns_before(sid,None,1).await.unwrap().0;
    let summary=history.iter().find(|row|row.presentation_intent=="turn_summary").unwrap();
    assert_eq!(summary.projection["turn_outcome"],serde_json::to_value(expected).unwrap());
    assert_eq!(summary.projection["user_input_request"]["question"],"请完成浏览器授权");
    assert_eq!(store.read_events(sid,None,500).await.unwrap(),before);
    store.cancel_active_turn(sid,"cancel-waiting".into(),"session_api".into()).await.unwrap();
    assert!(store.observe(sid,None,1).await.unwrap().active_turn_outcome.is_none());
    assert!(matches!(store.turn_outcome(sid,&operation).await.unwrap(),Some(AgentTurnOutcome::Cancelled {..})));
}

#[tokio::test]
async fn turn_outcome_history_attaches_exact_results_without_consuming_raw_page_cursors_or_mutating_facts() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();
    let (session,_)=create_ready(&store,"outcome-page").await;
    let sid=&session.agent_session_id;
    let operation=OperationId::from("outcome-page-turn");
    let (input,start)=store.start_turn(sid,"session_api".into(),"outcome-input".into(),operation.clone(),
        StrictJsonValue(json!({"content":"Make a presentation"}))).await.unwrap();
    let root=input.ack.unwrap().event_id;
    let start=start.ack.unwrap().event_id;
    let mut ids=Vec::new();
    let mut cause=start.clone();
    for (step,text) in [(1,"I will start."),(2,"Here is the finished explanation.")] {
        let progress=append(sid,&format!("outcome-model-{step}"),"runtime_supervisor",&format!("outcome-model-{step}"),
            "runtime/progress-recorded",operation.as_ref(),Some(start.clone()),
            json!({"producer_seq":step,"event":{"event":"model_step_started","step":step,"operation_id":format!("{}:model:{step}",operation.as_ref())}}));
        store.append_event(&progress).await.unwrap();
        let id=canonical_assistant_step_message_id(root.as_ref(),step).unwrap();ids.push(id.clone());
        let part=append(sid,&format!("outcome-part-{step}"),"runtime_supervisor",&format!("outcome-part-{step}"),
            "message/content-part",&id,Some(cause),json!({"turn_id":root,"content":text}));
        store.append_event(&part).await.unwrap();
        let seal=append(sid,&format!("outcome-seal-{step}"),"runtime_supervisor",&format!("outcome-seal-{step}"),
            "message/completed",&id,Some(part.event_id),json!({"part_count":1,"content_digest":digest_bytes(text.as_bytes())}));
        store.append_event(&seal).await.unwrap();cause=seal.event_id;
    }
    let terminal=append(sid,"outcome-terminal","runtime_supervisor","outcome-terminal","turn/completed",
        operation.as_ref(),Some(cause),json!({"model_steps":2,"finished_at_ms":1_990_000_000_000_i64}));
    store.append_event(&terminal).await.unwrap();
    let expected=AgentTurnOutcome::Answered {message_ids:vec![ids[1].clone()]};
    let before=store.read_events(sid,None,500).await.unwrap();
    assert_eq!(store.turn_outcome(sid,&operation).await.unwrap(),Some(expected.clone()));
    let raw=store.message_history_before(sid,None,500).await.unwrap().0;
    let mut seen=BTreeSet::new();let mut cursor=None;
    loop {
        let (raw_page,raw_more,_)=store.message_history_before(sid,cursor,1).await.unwrap();
        assert_eq!(raw_page.len(),1);
        let raw_item=&raw_page[0];
        assert!(seen.insert(raw_item.projection_id.clone()),"raw pagination must not repeat or skip its window");
        let (page,more,_,next)=store.message_history_with_turns_before(sid,cursor,1).await.unwrap();
        assert_eq!(more,raw_more);
        assert_eq!(next,more.then_some(raw_item.first_seq),"associated summary cannot move the raw cursor");
        let summary=page.iter().find(|row|row.presentation_intent=="turn_summary").expect("every page in this Turn includes its exact result");
        assert_eq!(summary.projection["turn_outcome"],serde_json::to_value(&expected).unwrap());
        assert_eq!(summary.projection["source_message_id"],root.as_ref());
        if raw_item.projection_id==format!("message:{}",ids[0]) {
            assert!(!page.iter().any(|row|row.projection_id==format!("message:{}",ids[1])),
                "a partial page must name the exact final reference without relabeling its earlier progress");
        }
        if !more {break;}cursor=next;
    }
    assert_eq!(seen,raw.iter().map(|row|row.projection_id.clone()).collect());
    store.rebuild_projections(sid).await.unwrap();
    assert_eq!(store.turn_outcome(sid,&operation).await.unwrap(),Some(expected));
    assert_eq!(store.read_events(sid,None,500).await.unwrap(),before,"result reads and projection rebuild never author terminal or content facts");
    store.start_turn(sid,"session_api".into(),"newer-input".into(),"newer-turn".into(),
        StrictJsonValue(json!({"content":"another question"}))).await.unwrap();
    assert_eq!(store.turn_outcome(sid,&operation).await.unwrap(),Some(AgentTurnOutcome::Answered {message_ids:vec![ids[1].clone()]}));
}

#[tokio::test]
async fn standalone_history_requires_the_exact_projected_assistant_event_and_does_not_infer_from_missing_turn_id() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();
    let (session,ready)=create_ready(&store,"standalone-history").await;
    let sid=&session.agent_session_id;let id=Uuid::now_v7().to_string();
    let projected=append(sid,&id,"session_api","standalone-synthesis","message/assistant-projected",&id,Some(ready),
        json!({"content":"The collaboration result."}));
    store.append_event(&projected).await.unwrap();
    let raw=store.messages_after(sid,0).await.unwrap().into_iter().find(|row|row.projection_id==format!("message:{id}")).unwrap();
    assert!(raw.projection.get("standalone_projection").is_none());
    let page=store.message_history_before(sid,None,50).await.unwrap().0;
    let display=page.iter().find(|row|row.projection_id==raw.projection_id).unwrap();
    assert_eq!(display.projection["standalone_projection"],true);
    assert_eq!(display.projection["content"],"The collaboration result.");
    let mut forged=raw.clone();forged.projection["content"]=json!("Different content");
    assert!(store.test_hydrate_message_history_metadata(sid,std::slice::from_mut(&mut forged)).await.is_err());
    let (accepted,_)=store.start_turn(sid,"session_api".into(),"standalone-user".into(),"standalone-turn".into(),
        StrictJsonValue(json!({"content":"Next task"}))).await.unwrap();
    let accepted_id=accepted.ack.unwrap().event_id;
    let page=store.message_history_before(sid,None,50).await.unwrap().0;
    let user=page.iter().find(|row|row.projection_id==format!("message:{}", accepted_id.as_ref())).unwrap();
    assert!(user.projection.get("turn_id").is_none());
    assert!(user.projection.get("standalone_projection").is_none(),"absence of a Turn is never standalone provenance");
    let before=store.read_events(sid,None,500).await.unwrap();
    store.rebuild_projections(sid).await.unwrap();
    assert_eq!(store.read_events(sid,None,500).await.unwrap(),before);
    assert_eq!(store.message_history_before(sid,None,50).await.unwrap().0,page);
}
