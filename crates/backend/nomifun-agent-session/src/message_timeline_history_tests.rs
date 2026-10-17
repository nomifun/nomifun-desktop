use super::*;
use serde_json::Value;
use nomifun_agent_contracts::{MessageTimeline, TimelineSegmentKind, TimelineSegmentState};

struct TimelineFixture { session: AgentSessionId, operation: OperationId, root: EventId, start: EventId }

async fn timeline_fixture(store: &AgentSessionStore, key: &str) -> TimelineFixture {
    let (session, _) = create_ready(store, key).await;
    let operation = OperationId::from(format!("{key}-operation"));
    let (input, start) = store.start_turn(&session.agent_session_id, "session_api".into(), format!("{key}-input").into(), operation.clone(),
        StrictJsonValue(json!({"content":"Inspect the workspace"}))).await.unwrap();
    TimelineFixture {session:session.agent_session_id,operation,root:input.ack.unwrap().event_id,start:start.ack.unwrap().event_id}
}

async fn timeline_progress(store: &AgentSessionStore, f: &TimelineFixture, key: &str, value: serde_json::Value) -> u64 {
    store.append_event(&append(&f.session,key,"runtime_supervisor",key,"runtime/progress-recorded",f.operation.as_ref(),
        Some(f.start.clone()),json!({"producer_seq":1,"event":value}))).await.unwrap().ack.unwrap().seq
}

async fn timeline_part(store: &AgentSessionStore, f: &TimelineFixture, key: &str, kind: &str, source: &str, text: &str) -> u64 {
    store.append_event(&append(&f.session,key,"runtime_supervisor",key,kind,source,Some(f.start.clone()),
        json!({"turn_id":f.root,"content":text}))).await.unwrap().ack.unwrap().seq
}

fn timeline(row: &crate::MessageProjection) -> MessageTimeline {
    serde_json::from_value(row.projection["timeline"].clone()).unwrap()
}

#[tokio::test]
async fn silent_finish_is_audit_only_in_live_timeline_and_cold_tool_history() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=timeline_fixture(&store,"silent-finish-history").await;
    for (call,name,failed) in [
        ("quiet-success","finish_without_reply",false),("quiet-failed","finish_without_reply",true),
        ("motion-success","robot_dog_action",false),("motion-failed","robot_dog_action",true),
        ("report","report_completion",false),
    ] {
        timeline_progress(&store,&f,&format!("{call}-delta"),json!({"event":"tool_call_delta","step":1,
            "call_id":call,"name":name,"arguments_delta":"{}"})).await;
        timeline_progress(&store,&f,&format!("{call}-proposal"),json!({"event":"tool_call_completed","step":1,
            "call":{"call_id":call,"name":name,"arguments":{}}})).await;
        timeline_progress(&store,&f,&format!("{call}-result"),json!({"event":"tool_completed","step":1,
            "result":{"call_id":call,"output":[],"is_error":failed}})).await;
    }
    let expected=BTreeSet::from(["motion-success","motion-failed","report"]);
    let live=store.message_timeline_updates_for_turn(&f.session,f.root.as_ref()).await.unwrap();
    assert_eq!(live.iter().filter_map(|update|update.call_id.as_deref()).collect::<BTreeSet<_>>(),expected);
    assert_eq!(live.iter().find(|update|update.call_id.as_deref()==Some("motion-failed")).unwrap().timeline.segments[0].state,TimelineSegmentState::Failed);
    store.cancel_active_turn(&f.session,"silent-finish-history-close".into(),"session_api".into()).await.unwrap();
    let before=store.read_events(&f.session,None,500).await.unwrap();
    for rebuild in [false,true] {
        if rebuild {store.rebuild_projections(&f.session).await.unwrap();}
        let history=store.message_history_before(&f.session,None,100).await.unwrap().0;
        let tools=history.iter().filter(|row|row.presentation_intent=="tool").collect::<Vec<_>>();
        assert_eq!(tools.iter().filter_map(|row|row.projection.pointer("/tool_summary/call_id").and_then(Value::as_str)).collect::<BTreeSet<_>>(),expected);
        assert!(tools.iter().all(|row|row.projection.pointer("/tool_summary/name").and_then(Value::as_str)!=Some("finish_without_reply")));
        for row in tools {
            let detail=store.message_by_id(&f.session,row.projection["correlation_id"].as_str().unwrap()).await.unwrap().unwrap();
            assert_eq!(timeline(&detail),timeline(row));
        }
    }
    assert_eq!(store.read_events(&f.session,None,500).await.unwrap(),before,"internal finish proposals and results remain canonical audit facts");
    assert_eq!(before.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded").count(),15);
}

#[tokio::test]
async fn message_timeline_paging_keeps_source_ranges_and_prefix_group_identity_in_the_same_snapshot() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=timeline_fixture(&store,"timeline-page").await;
    let answer=nomifun_agent_contracts::canonical_assistant_step_message_id(f.root.as_ref(),1).unwrap();
    timeline_progress(&store,&f,"page-output",json!({"event":"output_text_delta","step":1,"text":"I will inspect. "})).await;
    timeline_part(&store,&f,"page-answer-part","message/content-part",&answer,"I will inspect. ").await;
    timeline_progress(&store,&f,"page-reasoning",json!({"event":"reasoning_delta","step":1,"text":"看😀"})).await;
    let thought=Uuid::now_v7().to_string();
    timeline_part(&store,&f,"page-thought-part","thinking/content-part",&thought,"看😀").await;
    timeline_progress(&store,&f,"page-call",json!({"event":"tool_call_completed","step":1,
        "call":{"call_id":"read-one","name":"read_file","arguments":{"path":"README.md"}}})).await;
    let tool_id=Uuid::now_v7().to_string();
    let started=append(&f.session,"page-owner","runtime_supervisor","page-owner","tool/call-started",&tool_id,Some(f.start.clone()),
        json!({"operation_id":"read-operation","call_id":"read-one","name":"read_file","capability_id":"coding.workspace","action_id":"read"}));
    store.append_event(&started).await.unwrap();
    timeline_progress(&store,&f,"page-result",json!({"event":"tool_completed","step":1,"result":{"call_id":"read-one","output":[],"is_error":false}})).await;
    let before=store.read_events(&f.session,None,500).await.unwrap();
    let whole=store.message_history_before(&f.session,None,100).await.unwrap().0;
    let expected=timeline(whole.iter().find(|row|row.projection["correlation_id"]==tool_id).unwrap());
    assert_eq!(expected.segments[0].group_id,"process:page-answer-part");
    assert_eq!(expected.segments[0].state,TimelineSegmentState::Completed);
    let mut cursor=None;let mut saw_tool=false;let mut saw_thought=false;
    loop {
        let (page,more,_,next)=store.message_history_with_turns_before(&f.session,cursor,1).await.unwrap();
        for row in &page {
            if row.projection["correlation_id"]==tool_id {assert_eq!(timeline(row),expected);saw_tool=true;}
            if row.projection["correlation_id"]==thought {
                let slice=timeline(row);assert_eq!(slice.segments[0].end_utf16,Some(3));
                assert_eq!(slice.segments[0].group_id,"process:page-answer-part");
                assert_eq!(row.projection["state"],"recorded");saw_thought=true;
            }
        }
        if !more {break;}assert_ne!(cursor,next);cursor=next;
    }
    assert!(saw_tool&&saw_thought);
    let raw=store.messages_before(&f.session,None,2).await.unwrap();
    let rendered=store.renderer_messages_before(&f.session,None,2).await.unwrap();
    assert_eq!((raw.1,raw.2),(rendered.1,rendered.2));
    assert_eq!(raw.0.iter().map(|row|&row.projection_id).collect::<Vec<_>>(),rendered.0.iter().map(|row|&row.projection_id).collect::<Vec<_>>());
    for (source,display) in raw.0.iter().zip(&rendered.0) {
        assert!(source.projection.get("timeline").is_none());
        assert_eq!(source.semantic_digest,display.semantic_digest,"renderer metadata cannot change a canonical source fingerprint");
    }
    assert_eq!(store.read_events(&f.session,None,500).await.unwrap(),before);
    let persisted:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_messages WHERE json_type(projection_json,'$.timeline') IS NOT NULL")
        .fetch_one(store.test_pool()).await.unwrap();assert_eq!(persisted,0,"timeline reads never mutate the projection schema or stored documents");
    assert_eq!(timeline(&store.message_by_id(&f.session,&tool_id).await.unwrap().unwrap()),expected);
}

#[tokio::test]
async fn message_timeline_metadata_never_attaches_future_content_or_future_terminal_to_an_old_body() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=timeline_fixture(&store,"timeline-watermark").await;
    let thought=Uuid::now_v7().to_string();
    timeline_progress(&store,&f,"watermark-phase",json!({"event":"reasoning_delta","step":1,"text":"😀"})).await;
    let first=timeline_part(&store,&f,"watermark-first","thinking/content-part",&thought,"😀").await;
    let mut old=store.message_by_id(&f.session,&thought).await.unwrap().unwrap();assert_eq!(timeline(&old).through_seq,first);
    timeline_part(&store,&f,"watermark-second","thinking/content-part",&thought,"later").await;
    store.cancel_active_turn(&f.session,"watermark-cancel".into(),"session_api".into()).await.unwrap();
    store.test_hydrate_message_history_metadata(&f.session,std::slice::from_mut(&mut old)).await.unwrap();
    let old_timeline=timeline(&old);assert_eq!(old_timeline.through_seq,first);
    assert_eq!(old_timeline.segments[0].end_utf16,Some(2));
    assert_eq!(old_timeline.segments[0].state,TimelineSegmentState::Running,"future terminal cannot close an earlier snapshot");
    let fresh=store.message_by_id(&f.session,&thought).await.unwrap().unwrap();
    assert_eq!(timeline(&fresh).segments[0].end_utf16,Some(7));assert_eq!(fresh.projection["state"],"recorded");
    old.projection["content"]=json!("");
    assert!(store.test_hydrate_message_history_metadata(&f.session,std::slice::from_mut(&mut old)).await.is_err());
}

#[tokio::test]
async fn message_timeline_scopes_reused_call_ids_to_the_exact_turn_and_retains_missing_owner_history() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=timeline_fixture(&store,"timeline-call-scope").await;
    timeline_progress(&store,&f,"scope-one-call",json!({"event":"tool_call_completed","step":1,
        "call":{"call_id":"same","name":"report_completion","arguments":{}}})).await;
    timeline_progress(&store,&f,"scope-one-result",json!({"event":"tool_completed","step":1,"result":{"call_id":"same","output":[],"is_error":false}})).await;
    let terminal=append(&f.session,"scope-one-terminal","runtime_supervisor","scope-one-terminal","turn/completed",f.operation.as_ref(),
        Some(f.start.clone()),json!({"model_steps":1}));store.append_event(&terminal).await.unwrap();
    let (input,start)=store.start_turn(&f.session,"session_api".into(),"scope-two-input".into(),"scope-two-operation".into(),
        StrictJsonValue(json!({"content":"Next"}))).await.unwrap();
    let second=TimelineFixture {session:f.session.clone(),operation:"scope-two-operation".into(),root:input.ack.unwrap().event_id,start:start.ack.unwrap().event_id};
    timeline_progress(&store,&second,"scope-two-call",json!({"event":"tool_call_completed","step":1,
        "call":{"call_id":"same","name":"report_completion","arguments":{}}})).await;
    timeline_progress(&store,&second,"scope-two-result",json!({"event":"tool_completed","step":1,"result":{"call_id":"same","output":[],"is_error":true}})).await;
    let first=store.message_timeline_updates_for_turn(&f.session,f.root.as_ref()).await.unwrap();
    let last=store.message_timeline_updates_for_turn(&f.session,second.root.as_ref()).await.unwrap();
    let one=&first.iter().find(|update|update.call_id.as_deref()==Some("same")).unwrap().timeline.segments[0];
    let two=&last.iter().find(|update|update.call_id.as_deref()==Some("same")).unwrap().timeline.segments[0];
    assert_eq!(one.state,TimelineSegmentState::Completed);assert_eq!(two.state,TimelineSegmentState::Failed);assert_ne!(one.group_id,two.group_id);
    let history=store.message_history_before(&f.session,None,100).await.unwrap().0;
    let tools=history.iter().filter(|row|row.presentation_intent=="tool").collect::<Vec<_>>();assert_eq!(tools.len(),2);
    for row in tools {let detail=store.message_by_id(&f.session,row.projection["correlation_id"].as_str().unwrap()).await.unwrap().unwrap();
        assert_eq!(timeline(&detail),timeline(row));}
}

#[tokio::test]
async fn message_timeline_resolves_external_payloads_and_fails_closed_for_orphaned_reasoning() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=timeline_fixture(&store,"timeline-stored").await;
    let thought=Uuid::now_v7().to_string();
    let value=json!({"producer_seq":1,"event":{"event":"reasoning_delta","step":1,"text":"界😀"}});
    let bytes=canonical_json_bytes(&value).unwrap();let payload=SessionPayloadRecord {payload_id:ArtifactId::from("timeline-payload"),
        agent_session_id:f.session.clone(),media_type:"application/json".into(),byte_len:bytes.len() as u64,digest:digest_bytes(&bytes),
        body:SessionPayloadBody::Json(StrictJsonValue(value))};
    let mut progress=append(&f.session,"stored-phase","runtime_supervisor","stored-phase","runtime/progress-recorded",f.operation.as_ref(),Some(f.start.clone()),json!({}));
    progress.semantic_event.payload=SessionEventPayloadRef::Stored(payload.payload_id.clone());store.append_event_with_payload(&progress,Some(&payload)).await.unwrap();
    timeline_part(&store,&f,"stored-part","thinking/content-part",&thought,"界😀").await;
    let mut row=store.message_by_id(&f.session,&thought).await.unwrap().unwrap();
    assert_eq!(row.projection["state"],"streaming");assert_eq!(timeline(&row).segments[0].kind,TimelineSegmentKind::Reasoning);
    assert_eq!(timeline(&row).segments[0].end_utf16,Some(3));
    row.projection["turn_id"]=json!(Uuid::now_v7().to_string());
    assert!(store.test_hydrate_message_history_metadata(&f.session,std::slice::from_mut(&mut row)).await.is_err());
}

#[tokio::test]
async fn message_timeline_owner_only_settlement_uses_formal_output_and_error_fields() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=timeline_fixture(&store,"timeline-owner-only").await;
    for (name,output,error,state) in [
        ("success",json!(null),json!(null),TimelineSegmentState::Completed),
        ("failure",json!(null),json!("SETTLEMENT_ERROR"),TimelineSegmentState::Failed),
        ("typed-failure",json!({"call_id":"typed-failure","is_error":true,"output":[]}),json!(null),TimelineSegmentState::Failed),
    ] {
        let owner=Uuid::now_v7().to_string();
        let started=append(&f.session,&format!("owner-only-{name}-start"),"runtime_supervisor",&format!("owner-only-{name}-start"),
            "tool/call-started",&owner,Some(f.start.clone()),json!({"operation_id":format!("owner-{name}"),"call_id":name,"name":"read_file"}));
        store.append_event(&started).await.unwrap();
        let settled=append(&f.session,&format!("owner-only-{name}-result"),"runtime_supervisor",&format!("owner-only-{name}-result"),
            "tool/result-recorded",&owner,Some(started.event_id),json!({"operation_id":format!("owner-{name}"),"call_id":name,"output":output,"error":error}));
        store.append_event(&settled).await.unwrap();
        let row=store.message_by_id(&f.session,&owner).await.unwrap().unwrap();
        assert_eq!(timeline(&row).segments[0].state,state,"formal owner settlement does not need a later Runtime result");
        let live=store.message_timeline_updates_for_turn(&f.session,f.root.as_ref()).await.unwrap();
        assert_eq!(live.iter().find(|update|update.call_id.as_deref()==Some(name)).unwrap().timeline,timeline(&row));
    }
}
