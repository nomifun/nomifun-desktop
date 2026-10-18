use super::*;
use nomifun_agent_contracts::{ChannelInputQuestionDeliveredV1,CHANNEL_INPUT_QUESTION_DELIVERED};

async fn question_turn(store:&AgentSessionStore,key:&str)->(AgentSessionLiveRecord,OperationId,EventId,u64) {
    let(session,_)=create_ready(store,key).await;let sid=&session.agent_session_id;
    let operation:OperationId=format!("turn-{key}").into();
    let(input,start)=store.start_turn(sid,"session_api".into(),format!("input-{key}").into(),operation.clone(),StrictJsonValue(json!({"content":"Authorize"}))).await.unwrap();
    let source=input.ack.unwrap().event_id;let start=start.ack.unwrap().event_id;
    let seq=question(store,sid,&operation,&start,"a").await;
    (session,operation,source,seq)
}
async fn question(store:&AgentSessionStore,session:&AgentSessionId,operation:&OperationId,start:&EventId,id:&str)->u64 {
    store.append_event(&append(session,&format!("question-{id}"),"runtime_supervisor",&format!("question-{id}"),"runtime/progress-recorded",operation.as_ref(),Some(start.clone()),
        json!({"producer_seq":1,"event":{"event":"user_input_requested","step":1,"request":{"request_id":id,
            "question":"Complete this original action","remaining_work":"Verify its result"}}}))).await.unwrap();
    store.current_cursor(session).await.unwrap().seq
}
fn delivery(session:&AgentSessionId,operation:&OperationId,source:&EventId,question_seq:u64)->ChannelInputQuestionDeliveredV1 {
    ChannelInputQuestionDeliveredV1 {agent_session_id:session.clone(),owner_ref:owner(),operation_id:operation.clone(),source_message_id:source.clone(),question_seq,
        request_id:"a".into(),platform:"lark".into(),channel_plugin_id:"plugin".into(),chat_id:"chat".into(),provider_message_id:"message-a".into()}
}
async fn close_question(store:&AgentSessionStore,sid:&AgentSessionId,operation:&OperationId,id:&str) {
    let start=store.read_turn_receipt(sid,operation).await.unwrap().started_event.unwrap().event_id;
    store.append_event(&append(sid,&format!("terminal-{id}"),"runtime_supervisor",&format!("terminal-{id}"),"turn/completed",operation.as_ref(),Some(start),
        json!({"model_steps":1,"user_input_request":{"request_id":id,"question":"Complete this original action","remaining_work":"Verify its result"}}))).await.unwrap();
}
#[tokio::test]
async fn finite_question_delivery_recovers_its_original_target_across_late_ack_and_successor() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,operation,source,seq)=question_turn(&store,"channel-proof").await;
    let sid=&session.agent_session_id;let proof=delivery(sid,&operation,&source,seq);
    let ack=store.record_channel_input_question_delivery(&owner(),proof.clone()).await.unwrap();
    assert_eq!(store.record_channel_input_question_delivery(&owner(),proof.clone()).await.unwrap(),ack);
    close_question(&store,sid,&operation,"a").await;
    let(next_input,next_start)=store.start_turn(sid,"session_api".into(),"input-b".into(),"turn-b".into(),StrictJsonValue(json!({"content":"Different work"}))).await.unwrap();
    let next_source=next_input.ack.unwrap().event_id;let next_start=next_start.ack.unwrap().event_id;
    let next=question(&store,sid,&"turn-b".into(),&next_start,"b").await;
    let before=store.head(sid).await.unwrap();
    let mut late=proof.clone();late.provider_message_id="late-a".into();
    store.record_channel_input_question_delivery(&owner(),late.clone()).await.unwrap();
    let after=store.head(sid).await.unwrap();assert_eq!(before.status,after.status);assert_eq!(before.active_turn_id,after.active_turn_id);
    let original=store.resolve_channel_input_question_delivery(&owner(),sid,"lark","plugin","chat","late-a").await.unwrap().unwrap();
    assert_eq!(original,late);assert_eq!(original.question_seq,seq);assert_ne!(original.source_message_id,next_source);
    let mut retargeted=proof.clone();retargeted.operation_id="turn-b".into();retargeted.source_message_id=next_source;retargeted.question_seq=next;retargeted.request_id="b".into();
    assert!(store.record_channel_input_question_delivery(&owner(),retargeted).await.is_err(),"provider identity cannot be rebound to B");
    let before_lookup=store.current_cursor(sid).await.unwrap();
    for address in [("telegram","plugin","chat","message-a"),("lark","other","chat","message-a"),("lark","plugin","other","message-a"),("lark","plugin","chat","missing")] {
        assert!(store.resolve_channel_input_question_delivery(&owner(),sid,address.0,address.1,address.2,address.3).await.unwrap().is_none());
    }
    assert_eq!(before_lookup,store.current_cursor(sid).await.unwrap(),"lookup is not execution or input authority");
    assert!(store.resolve_channel_input_question_delivery(&PrincipalRef {principal_kind:"user".into(),principal_id:"foreign".into()},sid,"lark","plugin","chat","message-a").await.is_err());
    store.rebuild_projections(sid).await.unwrap();
    assert_eq!(store.resolve_channel_input_question_delivery(&owner(),sid,"lark","plugin","chat","late-a").await.unwrap().unwrap(),late);
}
#[tokio::test]
async fn question_delivery_is_read_only_metadata_and_rejects_forged_source_or_request() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,operation,source,seq)=question_turn(&store,"channel-forge").await;
    let sid=&session.agent_session_id;let proof=delivery(sid,&operation,&source,seq);let before=store.head(sid).await.unwrap();
    let forged=append(sid,"forged","session_api","forged",CHANNEL_INPUT_QUESTION_DELIVERED,operation.as_ref(),None,serde_json::to_value(&proof).unwrap());
    assert!(store.append_event(&forged).await.is_err());
    let mut wrong=proof.clone();wrong.request_id="different".into();assert!(store.record_channel_input_question_delivery(&owner(),wrong).await.is_err());
    let mut wrong=proof.clone();wrong.source_message_id=event_id(&uuid::Uuid::now_v7().to_string());assert!(store.record_channel_input_question_delivery(&owner(),wrong).await.is_err());
    assert_eq!(store.head(sid).await.unwrap(),before);
    let principal=owner();let original=proof.clone();
    let(a,b)=tokio::join!(store.record_channel_input_question_delivery(&principal,original),store.record_channel_input_question_delivery(&principal,proof));
    assert_eq!(a.unwrap(),b.unwrap());
    let after=store.head(sid).await.unwrap();assert_eq!(before.status,after.status);assert_eq!(before.active_turn_id,after.active_turn_id);
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='channel/input-question-delivered'")
        .bind(sid.as_ref()).fetch_one(store.test_pool()).await.unwrap();assert_eq!(count,1);
}
