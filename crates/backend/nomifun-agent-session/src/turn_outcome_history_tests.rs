use super::*;
use nomifun_agent_contracts::{AgentTurnOutcome,canonical_assistant_step_message_id};

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
    assert!(store.hydrate_message_history_metadata(sid,std::slice::from_mut(&mut forged)).await.is_err());
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
