//! Installed host + real local Knowledge tools; only the HTTP model is scripted.
use super::*;

#[tokio::test]
async fn ordinary_turn_preparation_does_not_search_or_inject_mounted_knowledge() {
    let fixture = Fixture::with_knowledge("knowledge-demand-ordinary", "http://127.0.0.1:9/v1").await;
    let (_, root) = fixture.knowledge_base.as_ref().unwrap();
    std::fs::write(root.join("reference.md"), "# Private reference\nASTER_KNOWLEDGE_DEMAND_MARKER").unwrap();
    let prepared = fixture.host.prepare_turn(&fixture.message, CancellationToken::new()).await.unwrap();
    let input = serde_json::to_string(&prepared.model_request.input).unwrap();
    assert!(!input.contains("ASTER_KNOWLEDGE_DEMAND_MARKER"));
    assert!(!input.contains("nomifun_knowledge_retrieval"));
    assert!(!input.contains("host performs one bounded search for every user turn"));
    let knowledge_calls: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='tool/call-started' AND json_extract(inline_json,'$.capability_id')='knowledge'")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(knowledge_calls, 0);
    assert!(prepared.tool_plan.model_name_for_action("knowledge", "knowledge/search").is_some());
    assert!(prepared.tool_plan.model_name_for_action("knowledge", "knowledge/read").is_some());
    fixture.finish().await;
}

#[tokio::test]
async fn explicit_knowledge_request_uses_real_search_and_read_results_in_the_original_session() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let router = axum::Router::new()
        .route("/v1/models", axum::routing::get(|| async {
            axum::Json(json!({"object":"list","data":[{"id":"cleanup-fixture","object":"model"}]}))
        }))
        .route("/v1/chat/completions", axum::routing::post(|
            axum::extract::State(requests): axum::extract::State<Arc<Mutex<Vec<Value>>>>,
            axum::Json(body): axum::Json<Value>,
        | async move {
            let call = { let mut recorded = requests.lock().unwrap(); recorded.push(body.clone()); recorded.len() };
            let name = |action| nomifun_agent_contracts::tool_presentation::platform_tool_name("knowledge", action);
            let (delta, reason) = match call {
                1 => (json!({"role":"assistant","tool_calls":[{"index":0,"id":"search-current-base","type":"function",
                    "function":{"name":name("knowledge/search"),"arguments":json!({"query":"aster","limit":5}).to_string()}}]}), "tool_calls"),
                2 => {
                    let result = body["messages"].as_array().unwrap().iter().rev()
                        .find(|message| message["role"] == "tool").unwrap();
                    let result: Value = serde_json::from_str(result["content"].as_str().unwrap()).unwrap();
                    let hit = &result["hits"][0];
                    assert_eq!(hit["relative_path"], "reference.md", "search must observe the actual mounted file");
                    let handle = hit["handle"].as_str().unwrap();
                    (json!({"role":"assistant","tool_calls":[{"index":0,"id":"read-found-reference","type":"function",
                        "function":{"name":name("knowledge/read"),"arguments":json!({"handle":handle}).to_string()}}]}), "tool_calls")
                }
                _ => {
                    if call == 3 {
                        let result = body["messages"].as_array().unwrap().iter().rev()
                            .find(|message| message["role"] == "tool").unwrap();
                        assert!(result["content"].as_str().unwrap().contains("ASTER_KNOWLEDGE_DEMAND_MARKER"),
                            "read must return the real document, not only an advertised tool");
                    }
                    (json!({"role":"assistant","content":"The mounted reference contains ASTER_KNOWLEDGE_DEMAND_MARKER (reference.md)."}), "stop")
                }
            };
            let chunk = json!({"id":format!("knowledge-demand-{call}"),"object":"chat.completion.chunk","created":1,
                "model":"cleanup-fixture","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
            let end = json!({"id":format!("knowledge-demand-{call}"),"object":"chat.completion.chunk","created":1,
                "model":"cleanup-fixture","choices":[{"index":0,"delta":{},"finish_reason":reason}]});
            ([(axum::http::header::CONTENT_TYPE,"text/event-stream")],
                format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
        })).with_state(requests.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
    let fixture = Fixture::with_knowledge("knowledge-demand-real-tools", &url).await;
    let (_, root) = fixture.knowledge_base.as_ref().unwrap();
    std::fs::write(root.join("reference.md"), "# Aster reference\nASTER_KNOWLEDGE_DEMAND_MARKER is the verified local value.").unwrap();
    fixture.host.prepare_turn(&fixture.message, CancellationToken::new()).await.unwrap();
    fixture.host.cleanup_turn(&fixture.message, &nomifun_ai_agent::engine_sdk::EngineTurnTerminal::Cancelled).await.unwrap();
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.unwrap();
    assert!(requests.lock().unwrap().is_empty(), "mounting and ordinary preparation must issue no model or knowledge request");
    let events = fixture.runtime.subscribe();
    let accepted = api(&fixture._router, &format!("/api/agent-sessions/{}/turns", fixture.host.options.conversation_id),
        json!({"idempotency_key":"knowledge-demand-explicit","input":{"content":"Search the mounted Knowledge for aster and read its reference file. Quote the marker and source."}})).await;
    receive_input_handoff_terminal(events).await;
    let operation = accepted["operation_id"].as_str().unwrap();
    let facts = fixture.owner.canonical().store().turn_output_facts(
        &fixture.host.options.conversation_id.clone().into(), &operation.into()).await.unwrap();
    let output = serde_json::to_string(&facts).unwrap();
    assert!(output.contains("ASTER_KNOWLEDGE_DEMAND_MARKER"));
    let tool_calls = facts.events.iter().filter(|event| event.kind.0 == "tool/call-started")
        .filter_map(|event| facts.event_payloads.get(event.event_id.as_ref()))
        .filter_map(|payload| payload.get("action_id").and_then(Value::as_str)).collect::<Vec<_>>();
    assert_eq!(tool_calls, ["knowledge/search", "knowledge/read"]);
    let work_model_calls = facts.events.iter().filter(|event| event.kind.0 == "context/model-visible-applied").count();
    assert_eq!(work_model_calls, 3, "search, read and final answer need no additional work model stage");
    let recorded = requests.lock().unwrap();
    assert!(recorded.len() >= 3);
    assert!(!serde_json::to_string(&recorded[0]).unwrap().contains("ASTER_KNOWLEDGE_DEMAND_MARKER"));
    println!("KNOWLEDGE_DEMAND_EVIDENCE {}", json!({"kind":"installed_host_local_knowledge_scripted_http_model",
        "work_model_calls":work_model_calls,"knowledge_actions":tool_calls,
        "first_request_bytes":serde_json::to_vec(&recorded[0]).unwrap().len(),"session_id":fixture.host.options.conversation_id}));
    drop(recorded);
    fixture.finish().await;
    server.abort(); let _ = server.await;
}
