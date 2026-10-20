//! Exercise the authenticated HTTP read against real canonical journal writes,
//! without dispatching a model or executing the recorded tool invocation.
use super::*;

#[tokio::test]
async fn tool_detail_http_reads_exact_invocation_without_mutating_history_or_starting_work() {
    let fixture=Fixture::new("tool-detail-http").await;
    fixture.host.prepare_turn(&fixture.message,CancellationToken::new()).await.unwrap();
    let call=nomifun_agent_contracts::chat_model::ChatToolCall {
        call_id:"inspect-only".into(),name:"read_file".into(),
        arguments:StrictJsonValue(json!({"path":"src/example.rs"})),provider_metadata:None,
    };
    fixture.host.record_event(&fixture.message,&AgentEngineEvent::ToolCallCompleted {step:1,call}).await.unwrap();
    fixture.host.record_event(&fixture.message,&AgentEngineEvent::ToolCompleted {
        step:1,runtime_hold:Some(nomifun_agent_contracts::RuntimeToolHoldReason::ArgumentsRejected),
        result:nomifun_agent_runtime::AgentToolResult::text("inspect-only".into(),"Already committed observation",true),
    }).await.unwrap();
    fixture.host.cleanup_turn(&fixture.message,&nomifun_ai_agent::engine_sdk::EngineTurnTerminal::Cancelled).await.unwrap();
    fixture.host.record_event(&fixture.message,&AgentEngineEvent::TurnCancelled {model_steps:1}).await.unwrap();
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let root=fixture.message.source_message_id.as_ref().unwrap();
    let path=format!("/api/agent-sessions/{}/tool-details?turn_id={root}&call_id=inspect-only&section=output&limit=8",session.as_ref());
    let store=fixture.owner.canonical().store();
    let before=store.read_events(&session,None,500).await.unwrap();
    let (status,first)=file_api_get(&fixture._router,&path,true).await;
    assert_eq!(status,axum::http::StatusCode::OK,"{first}");
    let mut page:nomifun_api_types::ToolDetailPage=serde_json::from_value(first["data"].clone()).unwrap();
    assert_eq!(page.availability,nomifun_api_types::ToolDetailAvailability::Available);
    let mut content=page.content.clone();
    while let Some(cursor)=page.next_cursor {
        let (status,response)=file_api_get(&fixture._router,&format!("{path}&cursor={cursor}"),true).await;
        assert_eq!(status,axum::http::StatusCode::OK,"{response}");
        page=serde_json::from_value(response["data"].clone()).unwrap();content.push_str(&page.content);
    }
    assert_eq!(content,"Already committed observation");
    let (status,arguments)=file_api_get(&fixture._router,&format!("/api/agent-sessions/{}/tool-details?turn_id={root}&call_id=inspect-only&section=arguments",session.as_ref()),true).await;
    assert_eq!(status,axum::http::StatusCode::OK);
    assert_eq!(serde_json::from_str::<Value>(arguments["data"]["content"].as_str().unwrap()).unwrap(),json!({"path":"src/example.rs"}));
    assert_eq!(store.read_events(&session,None,500).await.unwrap(),before);
    assert!(fixture.host.active.lock().await.is_none(),"detail query cannot reopen the closed Turn");
    let (status,_)=file_api_get(&fixture._router,&path,false).await;assert_eq!(status,axum::http::StatusCode::FORBIDDEN);
    let invalid=fixture._router.clone().oneshot(Request::builder().method("GET").uri(format!("{path}&unexpected=1"))
        .header("x-nomi-local-trust","cleanup-fixture-local-trust").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(invalid.status(),axum::http::StatusCode::BAD_REQUEST);
    let (status,missing)=file_api_get(&fixture._router,&path.replace("inspect-only","another-call"),true).await;
    assert_eq!(status,axum::http::StatusCode::NOT_FOUND,"{missing}");
    let (status,changed)=file_api_get(&fixture._router,&format!("{path}&expected_result_seq=0"),true).await;
    assert_eq!(status,axum::http::StatusCode::CONFLICT,"{changed}");
    assert_eq!(changed["code"],"TOOL_DETAIL_SOURCE_CHANGED");
}
