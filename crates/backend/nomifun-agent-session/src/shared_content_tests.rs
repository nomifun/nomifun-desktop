use super::*;

async fn context(store: &AgentSessionStore, key: &str) -> (AgentSessionLiveRecord, EventId) {
    let (session, _) = create_ready(store, key).await;
    let (_, started) = store.start_turn(&session.agent_session_id, "session-api".into(), key.into(), key.into(),
        StrictJsonValue(json!({"content":"Inspect the saved source"}))).await.unwrap();
    (session, started.ack.unwrap().event_id)
}

fn progress(session: &AgentSessionId, started: &EventId, operation: &str, key: &str, seq: u64, event: serde_json::Value) -> SessionEventAppend {
    append(session, key, "runtime_supervisor", key, "runtime/progress-recorded", operation,
        Some(started.clone()), json!({"producer_seq":seq,"event":event}))
}

async fn leaves(store: &AgentSessionStore, session: &AgentSessionId) -> (i64, i64) {
    sqlx::query_as("SELECT COUNT(*),COALESCE(SUM(byte_len),0) FROM agent_payloads WHERE session_id=? AND payload_id LIKE 'shared-tool:%'")
        .bind(session.as_ref()).fetch_one(store.test_pool()).await.unwrap()
}

#[tokio::test]
async fn owner_result_runtime_observation_and_compaction_share_exact_bodies_and_arguments() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, started) = context(&store, "shared-bodies").await;
    let sid = &session.agent_session_id;
    let output = json!([{"type":"text","text":"文件正文😀".repeat(1000)}]);
    let arguments = json!({"path":"src/report.md","content":"写入原文😀".repeat(1000)});
    let result = json!({"call_id":"call-a","output":output,"is_error":false});
    let call = append(sid,"shared-call","runtime_supervisor","shared-call","tool/call-started","tool-message",Some(started.clone()),
        json!({"operation_id":"owner-a","call_id":"call-a","capability_id":"workspace.files","action_id":"workspace.files/write","name":"write_file"}));
    store.append_event(&call).await.unwrap();
    let events = vec![
        progress(sid,&started,"shared-bodies","shared-owner",1,json!({"event":"host_tool_settled","operation_id":"owner-a","call_id":"call-a","result":result,"error":null})),
        append(sid,"shared-display","runtime_supervisor","shared-display","tool/result-recorded","tool-message",Some(call.event_id),json!({"operation_id":"owner-a","call_id":"call-a","output":result,"error":null})),
        progress(sid,&started,"shared-bodies","shared-runtime",2,json!({"event":"tool_completed","step":1,"result":result})),
        progress(sid,&started,"shared-bodies","shared-arguments",3,json!({"event":"tool_call_completed","step":1,"call":{"call_id":"call-a","name":"write_file","arguments":arguments}})),
        progress(sid,&started,"shared-bodies","shared-compaction",4,json!({"event":"context_compacted","input_bytes_before":50000,"input_bytes_after":20000,
            "summary":"Read and wrote the file","retained_tool_call_ids":["call-a"],"retained_context":[
                {"kind":"message","message":{"role":"assistant","content":[{"type":"tool_call","call_id":"call-a","name":"write_file","arguments":arguments}]}},
                {"kind":"message","message":{"role":"tool","content":[{"type":"tool_result","call_id":"call-a","output":output,"is_error":false}]}}
            ]})),
    ];
    for event in &events { store.append_event(event).await.unwrap(); }
    let bytes = canonical_json_bytes(&output).unwrap().len() + canonical_json_bytes(&arguments).unwrap().len();
    assert_eq!(leaves(&store,sid).await,(2,bytes as i64));
    let original_bytes:usize=events.iter().map(|event|match &event.semantic_event.payload {
        SessionEventPayloadRef::InlineJson(value)=>canonical_json_bytes(&value.0).unwrap().len(),
        _=>unreachable!(),
    }).sum();
    let stored_event_bytes:i64=sqlx::query_scalar("SELECT COALESCE(SUM(length(CAST(inline_json AS BLOB))),0) FROM agent_events WHERE session_id=? AND event_id IN ('shared-owner','shared-display','shared-runtime','shared-arguments','shared-compaction')")
        .bind(sid.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    let stored_content_bytes:i64=sqlx::query_scalar("SELECT COALESCE(SUM(length(body)),0) FROM agent_payloads WHERE session_id=?")
        .bind(sid.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    let stored_bytes=(stored_event_bytes+stored_content_bytes) as usize;
    assert!(stored_bytes*3<original_bytes*2,"shared content must reduce real serialized storage, including reference metadata");
    eprintln!("Shared tool fixture: {original_bytes} original envelope bytes -> {stored_bytes} stored envelope/content bytes");
    let facts = store.chat_causality_facts(sid,&"shared-bodies".into()).await.unwrap();
    for event in &events {
        let SessionEventPayloadRef::InlineJson(original) = &event.semantic_event.payload else { unreachable!() };
        assert_eq!(facts.event_payloads[event.event_id.as_ref()],original.0);
    }
    let rows: Vec<String> = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND kind IN ('runtime/progress-recorded','tool/result-recorded')")
        .bind(sid.as_ref()).fetch_all(store.test_pool()).await.unwrap();
    assert!(rows.iter().all(|body| !body.contains("文件正文") && !body.contains("写入原文")));
    let before = store.message_history_before(sid,None,100).await.unwrap();
    store.rebuild_projections(sid).await.unwrap();
    assert_eq!(store.message_history_before(sid,None,100).await.unwrap(),before);
    for event in &events { assert!(store.append_event(event).await.unwrap().duplicate); }
    assert_eq!(leaves(&store,sid).await,(2,bytes as i64));
}

#[tokio::test]
async fn current_generation_unfactored_retry_keeps_original_fact_and_creates_no_orphan_leaves() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session,started)=context(&store,"shared-unfactored").await;
    let sid=&session.agent_session_id;
    let event=progress(sid,&started,"shared-unfactored","shared-old",1,json!({"event":"tool_completed","step":1,
        "result":{"call_id":"old-call","output":[{"type":"text","text":"old source".repeat(400)}],"is_error":false}}));
    store.append_event(&event).await.unwrap();
    let SessionEventPayloadRef::InlineJson(original)=&event.semantic_event.payload else {unreachable!()};
    sqlx::query("UPDATE agent_events SET inline_json=? WHERE event_id=?").bind(serde_json::to_string(&original.0).unwrap())
        .bind(event.event_id.as_ref()).execute(store.test_pool()).await.unwrap();
    sqlx::query("DELETE FROM agent_payloads WHERE session_id=?").bind(sid.as_ref()).execute(store.test_pool()).await.unwrap();
    let cursor=store.current_cursor(sid).await.unwrap();
    assert!(store.append_event(&event).await.unwrap().duplicate);
    assert_eq!(store.current_cursor(sid).await.unwrap(),cursor);
    assert_eq!(leaves(&store,sid).await,(0,0));
}

#[tokio::test]
async fn shared_content_corruption_missing_identity_media_and_pointer_fail_closed() {
    for damage in ["body","missing","session","media","pointer","digest","length"] {
        let store=AgentSessionStore::open_in_memory().await.unwrap();
        let key=format!("shared-damage-{damage}");
        let (session,started)=context(&store,&key).await;
        let sid=&session.agent_session_id;
        let event=progress(sid,&started,&key,&format!("shared-event-{damage}"),1,json!({"event":"tool_completed","step":1,
            "result":{"call_id":"read-a","output":[{"type":"text","text":"private bytes".repeat(400)}],"is_error":false}}));
        store.append_event(&event).await.unwrap();
        match damage {
            "body"=>{sqlx::query("UPDATE agent_payloads SET body=? WHERE session_id=?").bind(br#"{"kind":"json","value":[]}"#.as_slice()).bind(sid.as_ref()).execute(store.test_pool()).await.unwrap();},
            "missing"=>{sqlx::query("DELETE FROM agent_payloads WHERE session_id=?").bind(sid.as_ref()).execute(store.test_pool()).await.unwrap();},
            "session"=>{let(other,_)=create_ready(&store,"shared-other-owner").await;sqlx::query("UPDATE agent_payloads SET session_id=? WHERE session_id=?").bind(other.agent_session_id.as_ref()).bind(sid.as_ref()).execute(store.test_pool()).await.unwrap();},
            "media"=>{sqlx::query("UPDATE agent_payloads SET media_type='application/json' WHERE session_id=?").bind(sid.as_ref()).execute(store.test_pool()).await.unwrap();},
            other=>{
                let mut value:serde_json::Value=serde_json::from_str(&sqlx::query_scalar::<_,String>("SELECT inline_json FROM agent_events WHERE event_id=?").bind(event.event_id.as_ref()).fetch_one(store.test_pool()).await.unwrap()).unwrap();
                let reference=&mut value["_canonical_shared_content"]["references"][0];
                match other {"pointer"=>reference["pointer"]=json!("/event/result/call_id"),"digest"=>reference["digest"]=json!("0".repeat(64)),_=>reference["byte_len"]=json!(2048)}
                sqlx::query("UPDATE agent_events SET inline_json=? WHERE event_id=?").bind(value.to_string()).bind(event.event_id.as_ref()).execute(store.test_pool()).await.unwrap();
            }
        }
        assert!(store.chat_causality_facts(sid,&key.into()).await.is_err(),"{damage}");
    }
}

#[tokio::test]
async fn shared_content_drafts_cannot_supply_references_and_failed_event_rolls_back_leaves() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();
    let (session,started)=context(&store,"shared-atomic").await;
    let sid=&session.agent_session_id;
    let mut forged=progress(sid,&started,"shared-atomic","shared-forged",1,json!({"event":"tool_completed"}));
    let SessionEventPayloadRef::InlineJson(value)=&mut forged.semantic_event.payload else {unreachable!()};
    value.0["_canonical_shared_content"]=json!({"version":1,"references":[]});
    assert!(store.append_event(&forged).await.is_err());
    let event=progress(sid,&started,"shared-atomic","shared-rollback",1,json!({"event":"tool_completed","step":1,
        "result":{"call_id":"read-a","output":[{"type":"text","text":"rollback bytes".repeat(400)}],"is_error":false}}));
    sqlx::raw_sql("CREATE TRIGGER fail_shared_event BEFORE INSERT ON agent_events WHEN NEW.event_id='shared-rollback' BEGIN SELECT RAISE(ABORT,'injected'); END;")
        .execute(store.test_pool()).await.unwrap();
    let cursor=store.current_cursor(sid).await.unwrap();
    assert!(store.append_event(&event).await.is_err());
    assert_eq!(store.current_cursor(sid).await.unwrap(),cursor);
    assert_eq!(leaves(&store,sid).await,(0,0));
}

#[tokio::test]
async fn large_write_arguments_hydrate_after_reopen_without_a_second_envelope_body() {
    let directory=tempfile::tempdir().unwrap();
    let path=directory.path().join("shared-content.db");
    let database=nomifun_db::init_database(&path).await.unwrap();
    let store=AgentSessionStore::from_pool(database.pool().clone()).await.unwrap();
    let (session,started)=context(&store,"shared-large-write").await;
    let sid=&session.agent_session_id;
    let arguments=json!({"path":"report.md","content":"exact source😀".repeat(9000)});
    let mut event=progress(sid,&started,"shared-large-write","shared-large-event",1,json!({"event":"tool_call_completed","step":1,
        "call":{"call_id":"write-large","name":"write_file","arguments":arguments,"provider_metadata":{"provider":"unchanged"}}}));
    let SessionEventPayloadRef::InlineJson(value)=event.semantic_event.payload.clone() else {unreachable!()};
    let logical=canonical_json_bytes(&value.0).unwrap();
    assert!(logical.len()>crate::MAX_INLINE_JSON_BYTES);
    let payload=SessionPayloadRecord {payload_id:"original-large-envelope".into(),agent_session_id:sid.clone(),media_type:"application/json".into(),
        byte_len:logical.len() as u64,digest:digest_bytes(&logical),body:SessionPayloadBody::Json(value.clone())};
    event.semantic_event.payload=SessionEventPayloadRef::Stored(payload.payload_id.clone());
    store.append_event_with_payload(&event,Some(&payload)).await.unwrap();
    assert_eq!(leaves(&store,sid).await,(1,canonical_json_bytes(&arguments).unwrap().len() as i64));
    let original_count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_payloads WHERE payload_id='original-large-envelope'")
        .fetch_one(store.test_pool()).await.unwrap();
    assert_eq!(original_count,0,"the original large envelope must not remain a second copy");
    drop(store);
    database.close().await;
    let reopened=AgentSessionStore::connect_existing(&path).await.unwrap();
    assert_eq!(reopened.resolved_event_payloads(sid,std::slice::from_ref(&event.event_id)).await.unwrap()[event.event_id.as_ref()],value.0);
    let state=reopened.latest_runtime_state(sid,"tool_call_completed").await.unwrap();
    assert_eq!(state.event.unwrap(),value.0["event"]);
    let (foreign,_)=create_ready(&reopened,"shared-resolve-foreign").await;
    assert!(reopened.resolved_event_payloads(&foreign.agent_session_id,std::slice::from_ref(&event.event_id)).await.is_err());
    assert!(reopened.append_event_with_payload(&event,Some(&payload)).await.unwrap().duplicate);
}

#[tokio::test]
async fn shared_leaves_are_session_private_and_purged_only_with_their_session() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();
    let (session,started)=context(&store,"shared-delete").await;
    let (other,other_started)=context(&store,"shared-retained").await;
    let event=json!({"event":"tool_completed","step":1,"result":{"call_id":"read","output":[{"type":"text","text":"same source".repeat(400)}],"is_error":false}});
    for (session,started,operation,key) in [(&session,&started,"shared-delete","shared-delete-result"),(&other,&other_started,"shared-retained","shared-retained-result")] {
        store.append_event(&progress(&session.agent_session_id,started,operation,key,1,event.clone())).await.unwrap();
    }
    assert_eq!(leaves(&store,&session.agent_session_id).await.0,1);
    assert_eq!(leaves(&store,&other.agent_session_id).await.0,1);
    let terminal=append(&session.agent_session_id,"shared-finished","runtime_supervisor","shared-finished","turn/completed","shared-delete",Some(started),json!({}));
    store.append_turn_terminal(&terminal,&"shared-delete".into()).await.unwrap();
    let delete=DeleteAgentSessionCommand {operation_id:"shared-delete-command".into(),agent_session_id:session.agent_session_id.clone(),owner_ref:owner(),requested_at:3000};
    store.fence_delete(&delete).await.unwrap();
    store.complete_delete(&delete,3100).await.unwrap();
    assert_eq!(leaves(&store,&session.agent_session_id).await,(0,0));
    assert_eq!(leaves(&store,&other.agent_session_id).await.0,1);
    let facts=store.chat_causality_facts(&other.agent_session_id,&"shared-retained".into()).await.unwrap();
    assert_eq!(facts.event_payloads["shared-retained-result"]["event"],event);
}
