use super::*;
use nomifun_agent_contracts::{ToolDetailAvailability as Availability, ToolDetailQuery, ToolDetailSection as Section};

struct Fixture { session: AgentSessionId, operation: OperationId, root: EventId }

async fn fixture(store: &AgentSessionStore, key: &str) -> Fixture {
    let (session, _) = create_ready(store, key).await;
    new_turn(store, session.agent_session_id, key).await
}

async fn new_turn(store: &AgentSessionStore, session: AgentSessionId, key: &str) -> Fixture {
    let operation = OperationId::from(format!("{key}-operation"));
    let (input, _) = store.start_turn(&session, "session_api".into(), format!("{key}-input").into(), operation.clone(),
        StrictJsonValue(json!({"content":"Inspect this invocation"}))).await.unwrap();
    Fixture { session, operation, root: input.ack.unwrap().event_id }
}

async fn progress(store: &AgentSessionStore, f: &Fixture, key: &str, value: serde_json::Value) -> u64 {
    store.append_event(&append(&f.session, key, "runtime_supervisor", key, "runtime/progress-recorded", f.operation.as_ref(),
        Some(f.root.clone()), json!({"producer_seq":1,"event":value}))).await.unwrap().ack.unwrap().seq
}

async fn call(store: &AgentSessionStore, f: &Fixture, key: &str, name: &str, arguments: serde_json::Value) -> u64 {
    progress(store, f, key, json!({"event":"tool_call_completed","step":1,"call":{"call_id":"same-call","name":name,"arguments":arguments}})).await
}

async fn output(store: &AgentSessionStore, f: &Fixture, key: &str, text: &str) -> u64 {
    progress(store, f, key, json!({"event":"tool_completed","step":1,
        "result":{"call_id":"same-call","is_error":false,"output":[{"type":"text","text":text}]}})).await
}

fn query(f: &Fixture, section: Section) -> ToolDetailQuery {
    ToolDetailQuery { turn_id:f.root.as_ref().into(),call_id:"same-call".into(),section,
        through_seq:None,expected_result_seq:None,cursor:None,limit:None }
}

#[tokio::test]
async fn tool_detail_pages_hydrate_shared_bytes_without_mutating_facts_or_messages() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"detail-pages").await;
    let script="printf '真实😀输出'\n".repeat(100);
    let text="甲😀乙\n".repeat(300);
    call(&store,&f,"detail-pages-call","start_process",json!({"cmd":script,"cwd":"/tmp/example"})).await;
    let result=output(&store,&f,"detail-pages-result",&text).await;
    let before=store.read_events(&f.session,None,500).await.unwrap();
    let messages=store.messages_before(&f.session,None,100).await.unwrap();
    let mut request=query(&f,Section::Output);request.limit=Some(5);request.expected_result_seq=Some(result);
    let first=store.tool_detail_page(&owner(),&f.session,&request).await.unwrap();
    assert_eq!(first.availability,Availability::Available);assert_eq!(first.total_bytes,text.len() as u64);
    let mut reconstructed=first.content.clone();let mut next=first.next_cursor;
    // Later observations must never enter a cursor pinned to the original snapshot.
    progress(&store,&f,"detail-pages-later",json!({"event":"host_tool_settled","call_id":"same-call",
        "result":{"call_id":"same-call","output":[{"type":"text","text":"later correction"}],"is_error":true}})).await;
    while let Some(cursor)=next {
        request.cursor=Some(cursor);
        let page=store.tool_detail_page(&owner(),&f.session,&request).await.unwrap();
        assert_eq!(page.through_seq,first.through_seq);assert_eq!(page.result_seq,Some(result));
        reconstructed.push_str(&page.content);next=page.next_cursor;
    }
    assert_eq!(reconstructed,text);
    let arguments=store.tool_detail_page(&owner(),&f.session,&query(&f,Section::Arguments)).await.unwrap();
    let arguments:serde_json::Value=serde_json::from_str(&arguments.content).unwrap();assert_eq!(arguments["cmd"],script);
    assert_eq!(arguments["cwd"],"/tmp/example");
    let after=store.read_events(&f.session,None,500).await.unwrap();
    assert_eq!(&after.events[..before.events.len()],&before.events);
    assert_eq!(store.messages_before(&f.session,None,100).await.unwrap(),messages);
}

#[tokio::test]
async fn tool_detail_distinguishes_pending_empty_and_unconfirmed_without_executing() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"detail-pending").await;
    let proposed=call(&store,&f,"detail-pending-call","mcp_example",json!({"query":"x"})).await;
    let page=store.tool_detail_page(&owner(),&f.session,&query(&f,Section::Output)).await.unwrap();
    assert_eq!(page.availability,Availability::Pending);assert_eq!(page.result_seq,None);assert!(page.eof);
    output(&store,&f,"detail-empty-result","").await;
    let mut old=query(&f,Section::Output);old.through_seq=Some(proposed);
    assert_eq!(store.tool_detail_page(&owner(),&f.session,&old).await.unwrap().availability,Availability::Pending);
    let empty=store.tool_detail_page(&owner(),&f.session,&query(&f,Section::Output)).await.unwrap();
    assert_eq!(empty.availability,Availability::Available);assert_eq!(empty.total_bytes,0);assert!(empty.result_seq.is_some());
    store.cancel_active_turn(&f.session,"detail-empty-close".into(),"session_api".into()).await.unwrap();
    let later=new_turn(&store,f.session.clone(),"detail-no-result").await;
    call(&store,&later,"detail-no-result-call","mcp_example",json!({})).await;
    store.cancel_active_turn(&later.session,"detail-no-result-close".into(),"session_api".into()).await.unwrap();
    let missing=store.tool_detail_page(&owner(),&later.session,&query(&later,Section::Output)).await.unwrap();
    assert_eq!(missing.availability,Availability::Unconfirmed);assert_eq!(missing.result_seq,None);
}

#[tokio::test]
async fn tool_detail_is_scoped_to_owner_session_turn_result_and_cursor_section() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"detail-scope").await;
    call(&store,&f,"detail-scope-call","read_file",json!({"path":"first.rs"})).await;
    let first_result=output(&store,&f,"detail-scope-result","first").await;
    let mut foreign=owner();foreign.principal_id="another-user".into();
    assert!(store.tool_detail_page(&foreign,&f.session,&query(&f,Section::Output)).await.is_err());
    store.cancel_active_turn(&f.session,"detail-scope-close".into(),"session_api".into()).await.unwrap();
    let later=new_turn(&store,f.session.clone(),"detail-scope-next").await;
    call(&store,&later,"detail-scope-next-call","read_file",json!({"path":"second.rs"})).await;
    output(&store,&later,"detail-scope-next-result","second").await;
    assert_eq!(store.tool_detail_page(&owner(),&f.session,&query(&f,Section::Output)).await.unwrap().content,"first");
    let mut wrong=query(&later,Section::Output);wrong.expected_result_seq=Some(first_result);
    assert!(matches!(store.tool_detail_page(&owner(),&f.session,&wrong).await,Err(SessionStoreError::Conflict(_))));
    let other=fixture(&store,"detail-other-session").await;
    assert!(store.tool_detail_page(&owner(),&other.session,&query(&f,Section::Output)).await.is_err());
    let mut small=query(&later,Section::Arguments);small.limit=Some(4);
    let page=store.tool_detail_page(&owner(),&later.session,&small).await.unwrap();
    small.cursor=page.next_cursor;small.section=Section::Output;
    assert!(matches!(store.tool_detail_page(&owner(),&later.session,&small).await,Err(SessionStoreError::InvalidSession(_))));
    let mut future=query(&later,Section::Output);future.through_seq=Some(u64::MAX);
    assert!(store.tool_detail_page(&owner(),&later.session,&future).await.is_err());
}

#[tokio::test]
async fn tool_detail_reads_native_call_output_without_media_or_later_process_replay() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"detail-process").await;
    call(&store,&f,"detail-process-call","start_process",json!({"cmd":"printf example\nprintf '保留原文'","cwd":"/tmp/original"})).await;
    progress(&store,&f,"detail-process-started",json!({"event":"tool_started","step":1,"call_id":"same-call",
        "capability_id":"workspace.process","action_id":"workspace.process/start"})).await;
    let arguments=store.tool_detail_page(&owner(),&f.session,&query(&f,Section::Arguments)).await.unwrap();
    assert_eq!(arguments.format,nomifun_agent_contracts::ToolDetailFormat::Text);
    assert_eq!(arguments.content,"printf example\nprintf '保留原文'");assert_eq!(arguments.working_directory.as_deref(),Some("/tmp/original"));
    output(&store,&f,"detail-process-output",&json!({"state":"running","process_id":"original","output":{"text":"this observation",
        "next_cursor":47,"retained_bytes":16,"dropped_bytes":31,"source_encoding":"utf-8","decode_errors":2}}).to_string()).await;
    let page=store.tool_detail_page(&owner(),&f.session,&query(&f,Section::Output)).await.unwrap();
    assert_eq!(page.content,"this observation");assert_eq!(page.availability,Availability::Available);
    let retention=page.retention.unwrap();assert_eq!(retention.dropped_bytes,31);assert_eq!(retention.decode_errors,2);
    assert_eq!(retention.retained_bytes,16);assert_eq!(retention.next_cursor,47);
    let other=fixture(&store,"detail-media").await;
    call(&store,&other,"detail-media-call","external_image",json!({})).await;
    progress(&store,&other,"detail-media-output",json!({"event":"tool_completed","step":1,"result":{"call_id":"same-call","is_error":false,
        "output":[{"type":"text","text":"caption"},{"type":"image","media_type":"image/png","data_base64":"PRIVATE_BASE64"}]}})).await;
    let media=store.tool_detail_page(&owner(),&other.session,&query(&other,Section::Output)).await.unwrap();
    assert_eq!(media.content,"caption");assert_eq!(media.non_text_parts,1);assert!(!serde_json::to_string(&media).unwrap().contains("PRIVATE_BASE64"));
    assert!(media.retention.is_none());
}

#[tokio::test]
async fn tool_detail_rejects_wrong_steps_corrupt_shared_content_and_deleting_sessions() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"detail-corrupt").await;
    call(&store,&f,"detail-corrupt-call","read_file",json!({"path":"x"})).await;
    output(&store,&f,"detail-corrupt-result",&"retained".repeat(300)).await;
    let before=store.tool_detail_page(&owner(),&f.session,&query(&f,Section::Output)).await.unwrap();assert!(!before.content.is_empty());
    sqlx::query("UPDATE agent_payloads SET byte_len=byte_len+1 WHERE session_id=? AND media_type='application/vnd.nomifun.tool-output+json'")
        .bind(f.session.as_ref()).execute(store.test_pool()).await.unwrap();
    assert!(matches!(store.tool_detail_page(&owner(),&f.session,&query(&f,Section::Output)).await,Err(SessionStoreError::InvalidPayload(_))));
    let wrong=fixture(&store,"detail-step").await;
    call(&store,&wrong,"detail-step-call","read_file",json!({})).await;
    progress(&store,&wrong,"detail-step-result",json!({"event":"tool_completed","step":2,
        "result":{"call_id":"same-call","output":[],"is_error":false}})).await;
    assert!(matches!(store.tool_detail_page(&owner(),&wrong.session,&query(&wrong,Section::Output)).await,Err(SessionStoreError::InvalidEvent(_))));
    store.fence_delete(&DeleteAgentSessionCommand {agent_session_id:wrong.session.clone(),owner_ref:owner(),operation_id:"detail-delete".into(),requested_at:100}).await.unwrap();
    assert!(matches!(store.tool_detail_page(&owner(),&wrong.session,&query(&wrong,Section::Arguments)).await,Err(SessionStoreError::Deleted(_))));
}

#[tokio::test]
async fn tool_detail_preserves_exact_host_settlement_before_runtime_completion() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"detail-settlement").await;
    call(&store,&f,"detail-settlement-call","start_process",json!({"cmd":"sleep 3"})).await;
    store.cancel_active_turn(&f.session,"detail-settlement-close".into(),"session_api".into()).await.unwrap();
    let settled=progress(&store,&f,"detail-settlement-result",json!({"event":"host_tool_settled","call_id":"same-call",
        "result":{"call_id":"same-call","is_error":true,"output":[{"type":"text","text":"original cancellation receipt"}]}})).await;
    let mut request=query(&f,Section::Output);request.expected_result_seq=Some(settled);
    let page=store.tool_detail_page(&owner(),&f.session,&request).await.unwrap();
    assert_eq!(page.availability,Availability::Available);assert_eq!(page.content,"original cancellation receipt");
    assert_eq!(page.result_seq,Some(settled));
    let foreign=fixture(&store,"detail-foreign-cause").await;
    let unrelated=store.read_events(&foreign.session,None,1).await.unwrap().events[0].event_id.clone();
    store.append_event(&append(&foreign.session,"detail-foreign-cause-call","runtime_supervisor","detail-foreign-cause-call",
        "runtime/progress-recorded",foreign.operation.as_ref(),Some(unrelated),json!({"producer_seq":1,
            "event":{"event":"tool_call_completed","step":1,"call":{"call_id":"same-call","name":"read_file","arguments":{}}}}))).await.unwrap();
    assert!(matches!(store.tool_detail_page(&owner(),&foreign.session,&query(&foreign,Section::Arguments)).await,Err(SessionStoreError::InvalidEvent(_))));
}
