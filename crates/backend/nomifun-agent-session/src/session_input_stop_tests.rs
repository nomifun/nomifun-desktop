use super::*;
use crate::SessionInputStop;

async fn input(store:&AgentSessionStore,session:&AgentSessionId,key:&str)->SessionEventRecord {
    store.accept_session_input(&owner(),session,"session_api".into(),key.into(),StrictJsonValue(json!({
        "content":key,"delivery_intent":{"kind":"queue"},"files":[],"inject_skills":[],
    }))).await.unwrap().record.unwrap()
}

#[tokio::test]
async fn input_stop_cancels_preparation_and_holds_the_original_batch_without_a_fake_turn() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"preparation-stop").await;
    let sid=&session.agent_session_id;
    let original=input(&store,sid,"preparing").await;
    let queued=input(&store,sid,"queued-behind-preparation").await;
    let stopped=store.stop_session_input(&owner(),sid,&original.event_id,"stop-original").await.unwrap();
    let SessionInputStop::Input {event,duplicate,..}=stopped else {panic!("unclaimed source must not invent a Turn receipt")};
    assert!(!duplicate);assert_eq!(event.kind.0,"message/input-blocked");
    assert_eq!(event.causation_event_id.as_ref(),Some(&original.event_id));
    assert_eq!(store.session_input_delivery(sid,&original.event_id).await.unwrap()["state"],"cancelled");
    assert_eq!(store.session_input_delivery(sid,&original.event_id).await.unwrap()["placement"],"transcript",
        "Stop keeps the visible preparing input's cancellation record");
    assert!(store.session_input_claim(sid,&original.event_id).await.unwrap().is_none());
    let held=store.session_input_queue(&owner(),sid).await.unwrap();
    assert_eq!(held.items.len(),1);assert_eq!(held.items[0].message_id,queued.event_id);assert!(held.items[0].held);
    let fresh=input(&store,sid,"new-explicit-input").await;
    assert_eq!(store.pending_session_inputs(sid,None).await.unwrap().iter().map(|input|input.event.event_id.clone()).collect::<Vec<_>>(),vec![fresh.event_id.clone()]);
    let before=store.current_cursor(sid).await.unwrap();
    assert!(matches!(store.stop_session_input(&owner(),sid,&original.event_id,"stop-original").await.unwrap(),SessionInputStop::Input {duplicate:true,..}));
    assert_eq!(store.current_cursor(sid).await.unwrap(),before);
    assert!(store.stop_session_input(&owner(),sid,&fresh.event_id,"stop-original").await.is_err(),"the same Stop key must not acquire a later source");
    assert_eq!(store.pending_session_inputs(sid,None).await.unwrap()[0].event.event_id,fresh.event_id);
}

#[tokio::test]
async fn input_stop_and_initial_claim_have_one_writer_winner_and_never_target_the_latest_turn() {
    let store=AgentSessionStore::open_in_memory_with_connections(2).await.unwrap();let(session,_)=create_ready(&store,"stop-claim-race").await;
    let sid=&session.agent_session_id;let original=input(&store,sid,"race-original").await;
    let operation=nomifun_agent_contracts::OperationId::from("original-operation");
    let principal=owner();
    let (claimed,stopped)=tokio::join!(
        store.try_claim_session_input_as_turn(&principal,sid,&original.event_id,&operation,StrictJsonValue(json!({}))),
        store.stop_session_input(&principal,sid,&original.event_id,"stop-racing-source"),
    );
    match (claimed.unwrap(),stopped.unwrap()) {
        (None,SessionInputStop::Input {..})=>assert!(store.session_input_claim(sid,&original.event_id).await.unwrap().is_none()),
        (Some(_),SessionInputStop::Turn {operation_id})=>{
            assert_eq!(operation_id,operation);
            store.cancel_exact_turn_with_input_hold(sid,&operation,"finish-original-stop".into(),"session_api".into()).await.unwrap();
        },
        other=>panic!("claim and input cancellation cannot both win: {other:?}"),
    }
    // A later explicit root is outside the stopped batch. A delayed Stop for
    // the original input cannot resolve through the Session's current head.
    let newer=input(&store,sid,"later-root").await;
    store.claim_session_input_as_turn(&principal,sid,&newer.event_id,&"later-operation".into(),StrictJsonValue(json!({}))).await.unwrap();
    let before=store.head(sid).await.unwrap();
    match store.stop_session_input(&principal,sid,&original.event_id,"late-original-stop").await.unwrap() {
        SessionInputStop::Turn {operation_id}=>{assert_eq!(operation_id,operation);},
        SessionInputStop::Input {duplicate,..}=>assert!(duplicate),
    }
    assert_eq!(store.head(sid).await.unwrap(),before);
}

#[tokio::test]
async fn input_stop_losing_preparation_order_cannot_cancel_a_queued_source_or_hold_a_later_batch() {
    use nomifun_agent_contracts::SessionInputQueueAction;
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"stale-preparation-stop").await;
    let sid=&session.agent_session_id;
    let original=input(&store,sid,"formerly-starting-a").await;
    let selected=input(&store,sid,"new-starting-b").await;
    let queue=store.session_input_queue(&owner(),sid).await.unwrap();
    store.update_session_input_queue(&owner(),sid,&"select-b-first".into(),&SessionInputQueueAction::Reorder {
        message_ids:vec![selected.event_id.clone(),original.event_id.clone()],
    },Some(queue.version)).await.unwrap();
    let before=store.current_cursor(sid).await.unwrap();
    assert!(store.stop_session_input(&owner(),sid,&original.event_id,"late-before-b-claim").await.is_err());
    assert_eq!(store.current_cursor(sid).await.unwrap(),before);
    store.claim_session_input_as_turn(&owner(),sid,&selected.event_id,&"b-operation".into(),StrictJsonValue(json!({}))).await.unwrap();
    let later=input(&store,sid,"new-c-after-b-claimed").await;
    let before=store.current_cursor(sid).await.unwrap();
    assert!(store.stop_session_input(&owner(),sid,&original.event_id,"late-after-b-claim").await.is_err());
    assert_eq!(store.current_cursor(sid).await.unwrap(),before);
    let queue=store.session_input_queue(&owner(),sid).await.unwrap();
    assert!(queue.items.iter().all(|item|!item.held));
    assert!(queue.items.iter().any(|item|item.message_id==later.event_id));
    assert_eq!(store.session_input_delivery(sid,&original.event_id).await.unwrap()["state"],"pending");
    assert_eq!(store.head(sid).await.unwrap().active_turn_id.as_deref(),Some("b-operation"));
}
