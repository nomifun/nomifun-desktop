use super::*;
use serde_json::Value;
use crate::{SessionInputBlockedReason,canonical_steering_input,is_session_input_event,ClearSessionContext,ContextClearPreparation};

async fn accept(store:&AgentSessionStore,session:&AgentSessionId,key:&str,text:&str)->SessionEventRecord {
    store.accept_session_input(&owner(),session,"session_api".into(),key.into(),StrictJsonValue(json!({"content":text,"files":[],"inject_skills":[]})))
        .await.unwrap().record.unwrap()
}

#[tokio::test]
async fn initial_only_is_an_atomic_acceptance_constraint_and_replays_only_its_original_receipt() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"input-initial-only").await;
    let sid=&session.agent_session_id;let owner=owner();
    let body=StrictJsonValue(json!({"content":"First-message handoff","files":[],"inject_skills":[]}));
    let(first,other)=tokio::join!(
        store.accept_initial_session_input(&owner,sid,"session_api".into(),"initial-a".into(),body.clone()),
        store.accept_initial_session_input(&owner,sid,"session_api".into(),"initial-b".into(),body.clone()));
    let(key,accepted,rejected)=match(first,other) {
        (Ok(accepted),Err(error))=>("initial-a",accepted,error),
        (Err(error),Ok(accepted))=>("initial-b",accepted,error),
        other=>panic!("exactly one concurrent first input may be accepted: {other:?}"),
    };
    assert!(matches!(rejected,SessionStoreError::Conflict(_)));
    let id=accepted.record.unwrap().event_id;
    assert_eq!(store.pending_session_inputs(sid,None).await.unwrap().len(),1);
    store.claim_session_input_as_turn(&owner,sid,&id,&"first-claimed-turn".into(),StrictJsonValue(json!({}))).await.unwrap();
    let before=store.current_cursor(sid).await.unwrap();
    let replay=store.accept_initial_session_input(&owner,sid,"session_api".into(),key.into(),body).await.unwrap();
    assert!(replay.duplicate);assert_eq!(replay.record.unwrap().event_id,id);assert_eq!(store.current_cursor(sid).await.unwrap(),before);
    assert!(matches!(store.accept_initial_session_input(&owner,sid,"session_api".into(),"late-initial".into(),StrictJsonValue(json!({"content":"Must never become a late supplement"}))).await,
        Err(SessionStoreError::Conflict(_))));
    assert_eq!(store.head(sid).await.unwrap().active_turn_id.as_deref(),Some("first-claimed-turn"));
}

#[tokio::test]
async fn session_input_acceptance_is_durable_before_a_turn_and_replay_never_retargets_it() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"input-independent").await;
    let sid=&session.agent_session_id;
    store.start_turn(sid,"session_api".into(),"original".into(),"original-turn".into(),StrictJsonValue(json!({"content":"original work"}))).await.unwrap();
    let input=accept(&store,sid,"saved-input","A later user decision").await;
    assert_eq!(store.head(sid).await.unwrap().active_turn_id.as_deref(),Some("original-turn"));
    assert!(store.session_input_claim(sid,&input.event_id).await.unwrap().is_none());
    assert_eq!(store.pending_session_inputs(sid,None).await.unwrap()[0].event.event_id,input.event_id);
    let facts=store.chat_causality_facts(sid,&"original-turn".into()).await.unwrap();
    assert!(!facts.events.iter().any(|event|event.event_id==input.event_id),"unclaimed data cannot enter Runtime context");
    let before=store.current_cursor(sid).await.unwrap();
    let retry=store.accept_session_input(&owner(),sid,"session_api".into(),"saved-input".into(),StrictJsonValue(json!({"content":"A later user decision","files":[],"inject_skills":[]}))).await.unwrap();
    assert!(retry.duplicate);assert_eq!(retry.record.unwrap().event_id,input.event_id);assert_eq!(store.current_cursor(sid).await.unwrap(),before);
    assert!(matches!(store.accept_session_input(&owner(),sid,"session_api".into(),"saved-input".into(),StrictJsonValue(json!({"content":"different"}))).await,
        Err(SessionStoreError::IdempotencyConflict(_))));
    store.cancel_active_turn(sid,"end-original".into(),"session_api".into()).await.unwrap();
    store.claim_session_input_as_turn(&owner(),sid,&input.event_id,&"next-turn".into(),StrictJsonValue(json!({}))).await.unwrap();
    assert_eq!(store.session_input_claim(sid,&input.event_id).await.unwrap().unwrap().operation_id.as_ref(),"next-turn");
    assert!(store.pending_session_inputs(sid,None).await.unwrap().is_empty());
    assert!(store.claim_session_input_as_steering(&owner(),sid,&input.event_id,&"next-turn".into()).await.is_err());
}

#[tokio::test]
async fn steering_claim_keeps_one_original_body_even_when_input_precedes_turn_start() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"input-reference").await;
    let sid=&session.agent_session_id;
    let first=accept(&store,sid,"first","First task").await;let second=accept(&store,sid,"second","A safe-step supplement").await;
    let started=store.claim_session_input_as_turn(&owner(),sid,&first.event_id,&"active-turn".into(),StrictJsonValue(json!({}))).await.unwrap().ack.unwrap().event_id;
    let claimed=store.claim_session_input_as_steering(&owner(),sid,&second.event_id,&"active-turn".into()).await.unwrap().record.unwrap();
    let SessionEventPayloadRef::InlineJson(payload)=&claimed.payload else {panic!("bounded claim")};
    assert_eq!(claimed.kind_version,2);assert_eq!(claimed.causation_event_id.as_ref(),Some(&second.event_id));
    assert_eq!(payload.0["input_event_id"],second.event_id.as_ref());assert!(payload.0.get("input").is_none());
    let facts=store.turn_output_facts(sid,&"active-turn".into()).await.unwrap();
    let(original,input)=canonical_steering_input(&facts,&claimed).unwrap();assert_eq!(original,second.event_id);
    assert_eq!(input.0["content"],"A safe-step supplement");
    assert_eq!(store.steering_input_source(sid,&claimed.event_id).await.unwrap().0.event_id,second.event_id);
    assert_eq!(store.session_input_delivery(sid,&second.event_id).await.unwrap()["state"],json!("pending"),"the durable claim alone is not native delivery");
    let before_delivery=store.current_cursor(sid).await.unwrap().seq;
    let delivered=append(sid,"input-native-delivery","runtime_supervisor","input-native-delivery","runtime/progress-recorded","active-turn",Some(started),
        json!({"producer_seq":1,"event":{"event":"steering_inputs","inputs":[{"receipt_operation_id":claimed.event_id,"message_id":second.event_id,
            "text":"A safe-step supplement","files":[],"inject_skills":[],"image_count":0}]}}));
    let delivery_seq=store.append_event(&delivered).await.unwrap().cursor.seq;
    assert_eq!(store.session_input_delivery(sid,&second.event_id).await.unwrap()["state"],json!("claimed"));
    let page=store.messages_with_turns_after(sid,before_delivery).await.unwrap();
    let update=page.iter().find(|message|message.projection_id==format!("message:{}",second.event_id.as_ref())).unwrap();
    assert_eq!(update.last_seq,delivery_seq);assert_eq!(update.projection["input_delivery"]["state"],"claimed");
    let cursor=store.current_cursor(sid).await.unwrap();store.rebuild_projections(sid).await.unwrap();
    assert_eq!(store.current_cursor(sid).await.unwrap(),cursor);
    assert!(store.pending_session_inputs(sid,None).await.unwrap().is_empty());
    assert_eq!(store.messages_after(sid,0).await.unwrap().iter().filter(|row|row.projection.get("content").and_then(Value::as_str)==Some("A safe-step supplement")).count(),1);
}

#[tokio::test]
async fn an_undelivered_claim_remains_visible_without_new_turn_replay_authority() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"input-undelivered").await;
    let sid=&session.agent_session_id;
    let first=accept(&store,sid,"first","Original task").await;let second=accept(&store,sid,"second","Saved despite delivery failure").await;
    let started=store.claim_session_input_as_turn(&owner(),sid,&first.event_id,&"failed-owner".into(),StrictJsonValue(json!({}))).await.unwrap().ack.unwrap().event_id;
    store.claim_session_input_as_steering(&owner(),sid,&second.event_id,&"failed-owner".into()).await.unwrap();
    let before=store.current_cursor(sid).await.unwrap().seq;
    assert_eq!(store.session_input_delivery(sid,&second.event_id).await.unwrap()["state"],json!("pending"));
    let terminal=append(sid,"failed-owner-terminal","runtime_supervisor","failed-owner-terminal","turn/failed","failed-owner",Some(started),json!({"error":"delivery unavailable"}));
    let seq=store.append_turn_terminal(&terminal,&"failed-owner".into()).await.unwrap().cursor.seq;
    let expected=json!({"state":"blocked","reason":"TURN_ENDED_BEFORE_INPUT_DELIVERY","event_seq":seq});
    assert_eq!(store.session_input_delivery(sid,&second.event_id).await.unwrap(),expected);
    let page=store.messages_with_turns_after(sid,before).await.unwrap();
    let message=page.iter().find(|message|message.projection_id==format!("message:{}",second.event_id.as_ref())).unwrap();
    assert_eq!(message.projection["input_delivery"],expected);assert_eq!(message.last_seq,seq);
    assert!(store.pending_session_inputs(sid,Some(&second.event_id)).await.unwrap().is_empty());
    assert!(store.claim_session_input_as_turn(&owner(),sid,&second.event_id,&"must-not-replay".into(),StrictJsonValue(json!({}))).await.is_err());
    let retry=accept(&store,sid,"second","Saved despite delivery failure").await;assert_eq!(retry.event_id,second.event_id);
    assert_eq!(store.turn_output_facts(sid,&"failed-owner".into()).await.unwrap().event_payloads[second.event_id.as_ref()]["content"],"Saved despite delivery failure");
    store.rebuild_projections(sid).await.unwrap();assert_eq!(store.session_input_delivery(sid,&second.event_id).await.unwrap(),expected);
}

#[tokio::test]
async fn input_preparation_block_is_visible_idempotent_and_does_not_block_later_inputs() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"input-blocked").await;
    let sid=&session.agent_session_id;let input=accept(&store,sid,"blocked","Use this selected Skill").await;
    let before=store.current_cursor(sid).await.unwrap().seq;
    store.mark_session_input_blocked(sid,&input.event_id,SessionInputBlockedReason::SkillUnavailable).await.unwrap();
    let blocked_cursor=store.current_cursor(sid).await.unwrap();
    store.mark_session_input_blocked(sid,&input.event_id,SessionInputBlockedReason::SkillUnavailable).await.unwrap();
    assert_eq!(store.current_cursor(sid).await.unwrap(),blocked_cursor);
    let page=store.messages_with_turns_after(sid,before).await.unwrap();
    let message=page.iter().find(|row|row.projection_id==format!("message:{}",input.event_id.as_ref())).unwrap();
    assert_eq!(message.projection["input_delivery"],json!({"state":"blocked","reason":"SKILL_UNAVAILABLE","event_seq":blocked_cursor.seq}));
    assert_eq!(message.last_seq,blocked_cursor.seq);
    assert!(store.pending_session_inputs(sid,None).await.unwrap().is_empty());
    assert_eq!(store.pending_session_inputs(sid,Some(&input.event_id)).await.unwrap().len(),1,"only explicit original-input retry includes the blocked item");
    let later=accept(&store,sid,"later","A different valid topic").await;
    assert_eq!(store.pending_session_inputs(sid,None).await.unwrap()[0].event.event_id,later.event_id);
    store.claim_session_input_as_turn(&owner(),sid,&input.event_id,&"repaired-turn".into(),StrictJsonValue(json!({}))).await.unwrap();
    assert_eq!(store.session_input_delivery(sid,&input.event_id).await.unwrap()["state"],json!("claimed"),"the unique actual claim supersedes a prior preparation observation");
}

#[tokio::test]
async fn context_clear_discards_unclaimed_input_without_a_new_failure_or_replay_authority() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"input-clear").await;
    let sid=&session.agent_session_id;let input=accept(&store,sid,"before-clear","Must not execute after clear").await;
    let request=ClearSessionContext {idempotency_key:"clear-input-context".into(),reason:"Discard the old context".into()};
    let ContextClearPreparation::Ready(boundary)=store.preflight_context_clear(&owner(),sid,&request).await.unwrap() else {panic!("first clear")};
    store.clear_context(&owner(),sid,&request,&boundary).await.unwrap();
    assert!(store.pending_session_inputs(sid,Some(&input.event_id)).await.unwrap().is_empty());
    assert_eq!(store.session_input_delivery(sid,&input.event_id).await.unwrap()["state"],json!("discarded"));
    assert!(matches!(store.claim_session_input_as_turn(&owner(),sid,&input.event_id,&"forbidden-replay".into(),StrictJsonValue(json!({}))).await,
        Err(SessionStoreError::ExecutionFenced)));
    let repeated=accept(&store,sid,"before-clear","Must not execute after clear").await;assert_eq!(repeated.event_id,input.event_id);
    assert_eq!(store.head(sid).await.unwrap().status,"ready");
}

#[tokio::test]
async fn creation_message_data_is_neither_scheduled_nor_filtered_as_pending_intake() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,ready)=create_ready(&store,"creation-data").await;
    let sid=&session.agent_session_id;
    store.start_turn(sid,"session_api".into(),"owner-start".into(),"owner-turn".into(),StrictJsonValue(json!({"content":"Owner task"}))).await.unwrap();
    let id=Uuid::now_v7().to_string();let data=append(sid,&id,"session_api",&format!("creation-message:{}:media-data",sid.as_ref()),
        "message/user-accepted",&id,Some(ready),json!({"content":"Creation request history data"}));
    let data=store.append_event(&data).await.unwrap().record.unwrap();
    assert!(!is_session_input_event(&data,&owner()));
    assert!(store.pending_session_inputs(sid,None).await.unwrap().is_empty());
    assert!(store.chat_causality_facts(sid,&"owner-turn".into()).await.unwrap().events.iter().any(|event|event.event_id==data.event_id));
    let input=accept(&store,sid,"actual-intake","A real accepted supplement").await;
    assert!(is_session_input_event(&input,&owner()));
    assert_eq!(store.pending_session_inputs(sid,None).await.unwrap().iter().map(|input|input.event.event_id.clone()).collect::<Vec<_>>(),[input.event_id]);
}

#[tokio::test]
async fn quoted_closed_question_keeps_its_original_source_and_cannot_steer_a_later_turn() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"quoted-original").await;
    let sid=&session.agent_session_id;
    let(input,start)=store.start_turn(sid,"session_api".into(),"question-a".into(),"turn-a".into(),StrictJsonValue(json!({"content":"Authorization A"}))).await.unwrap();
    let source=input.ack.unwrap().event_id;let start=start.ack.unwrap().event_id;
    let question=nomifun_agent_contracts::AgentUserInputRequest {request_id:"question-a".into(),question:"Original question A".into(),remaining_work:"Verify A".into()};
    let event=append(sid,"question-a-source","runtime_supervisor","question-a-source","runtime/progress-recorded","turn-a",Some(start.clone()),
        json!({"producer_seq":1,"event":{"event":"user_input_requested","step":1,"request":question}}));
    let question_seq=store.append_event(&event).await.unwrap().cursor.seq;
    store.append_turn_terminal(&append(sid,"question-a-finish","runtime_supervisor","question-a-finish","turn/completed","turn-a",Some(start),
        json!({"user_input_request":question})),&"turn-a".into()).await.unwrap();
    store.start_turn(sid,"session_api".into(),"new-topic-b".into(),"turn-b".into(),StrictJsonValue(json!({"content":"Different topic B"}))).await.unwrap();
    let quoted=store.accept_session_input(&owner(),sid,"session_api".into(),"quoted-a-reply".into(),StrictJsonValue(json!({"content":"My reply to A",
        "files":[],"inject_skills":[],"reply_to":{"turn_id":source,"question_seq":question_seq}}))).await.unwrap().record.unwrap();
    assert!(matches!(store.claim_session_input_as_steering(&owner(),sid,&quoted.event_id,&"turn-b".into()).await,Err(SessionStoreError::ExecutionFenced)));
    assert!(store.session_input_claim(sid,&quoted.event_id).await.unwrap().is_none());
    assert_eq!(store.question_source(sid,source.as_ref(),question_seq).await.unwrap(),question);
    store.cancel_active_turn(sid,"close-b".into(),"session_api".into()).await.unwrap();
    store.claim_session_input_as_turn(&owner(),sid,&quoted.event_id,&"reply-turn-c".into(),StrictJsonValue(json!({}))).await.unwrap();
    assert_eq!(store.session_input(sid,&quoted.event_id).await.unwrap().input.0["reply_to"]["turn_id"],source.as_ref());
    assert_eq!(store.session_input_claim(sid,&quoted.event_id).await.unwrap().unwrap().operation_id.as_ref(),"reply-turn-c");
}
