//! Exercise real Session -> model tool call -> Kernel -> provider services ->
//! management HTTP routes, using an isolated database and scripted upstream.
use axum::{body::Body, http::Request};
use serde_json::{Value, json};
use tower::ServiceExt;

const TRUST: &str = "model-management-session-test";
const REPLY: &str = "Model import completed and conversation continued.";

async fn call(router: &axum::Router, method: &str, path: &str, body: Value) -> Value {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("x-nomi-local-trust", TRUST)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(status.is_success(), "{method} {path}: {status} {body}");
    body["data"].clone()
}

fn model(name: &str) -> Value {
    json!({"model":name,"capabilities":[{"task":"chat","protocol":"openai.chat_text","connection_role":"default"}]})
}

#[tokio::test]
async fn model_management_conversation_imports_and_continues_on_the_current_provider() {
    let upstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = request.body_json().unwrap();
            let results = body["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "tool").collect::<Vec<_>>();
            let (action, input) = match results.len() {
                0 => ("inspect", json!({"operation":"list"})),
                1 => ("create_provider", json!({"platform":"custom","name":"Imported via conversation","base_url":"https://imported.example/v1",
                    "auth_scheme":"bearer","credentials":{"api_keys":["import-secret-value"]},"initial_model":model("imported-one")})),
                2 => {
                    let listed: Value = serde_json::from_str(results[0]["content"].as_str().unwrap()).unwrap();
                    ("add_model", json!({"provider_id":listed["providers"][0]["provider_id"],"model":model("second-on-current-provider")}))
                }
                _ => ("", Value::Null),
            };
            let (delta, finish) = if action.is_empty() {
                assert!(results[1]["content"].as_str().unwrap().contains("created"), "{results:?}");
                assert!(results[2]["content"].as_str().unwrap().contains("created"), "{results:?}");
                assert!(!results.iter().any(|m| m.to_string().contains("import-secret-value")));
                (json!({"content":REPLY}), "stop")
            } else {
                let needle = format!("Action: model.management/{action}.");
                let tool = body["tools"].as_array().unwrap().iter().find(|t| t["function"]["description"].as_str().unwrap_or("").contains(&needle)).expect("model management tool must be in the frozen session plan");
                (json!({"role":"assistant","tool_calls":[{"index":0,"id":format!("import-{}",results.len()),"type":"function","function":{"name":tool["function"]["name"],"arguments":input.to_string()}}]}), "tool_calls")
            };
            let frame = json!({"id":"model-import","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
            let done = json!({"id":"model-import","choices":[{"index":0,"delta":{},"finish_reason":finish}]});
            wiremock::ResponseTemplate::new(200).insert_header("content-type","text/event-stream")
                .set_body_string(format!("data: {frame}\n\ndata: {done}\n\ndata: [DONE]\n\n"))
        }).mount(&upstream).await;

    let (router, services) = super::common::build_local_trust_app(TRUST).await;
    let mut changes = services.event_bus.subscribe_user();
    let provider = call(&router,"POST","/api/providers",json!({"platform":"stepfun-plan","name":"Current provider",
        "base_url":format!("{}/v1",upstream.uri()),"auth_scheme":"bearer","credentials":{"api_keys":["fixture"]},"initial_model":model("chat-fixture")})).await;
    let selection = json!({"provider_id":provider["provider_id"],"model":"chat-fixture"});
    let minimal = call(
        &router,
        "POST",
        "/api/agent-presets/from-template/chat.minimal",
        json!({"display_name":"Import source","reuse_existing":false,"model":selection}),
    )
    .await;
    let mut document = minimal["revision"]["document"].clone();
    document["enabled_capabilities"] = json!([{"capability":{"id":"model.management"},"action_allowlist":["model.management/inspect","model.management/create_provider","model.management/add_model"]}]);
    let preset = call(
        &router,
        "POST",
        "/api/agent-presets",
        json!({"display_name":"Model import Agent","document":document}),
    )
    .await;
    let session = call(
        &router,
        "POST",
        "/api/agent-sessions",
        json!({"preset_id":preset["preset"]["preset_id"],"model":selection}),
    )
    .await;
    let id = session["agent_session_id"].as_str().unwrap();
    call(&router,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"idempotency_key":uuid::Uuid::now_v7().to_string(),"input":{"content":"Add the supplied model configuration."}})).await;
    let mut completed = false;
    for _ in 0..600 {
        let history = call(
            &router,
            "GET",
            &format!("/api/agent-sessions/{id}/message-history?page_size=100"),
            json!({}),
        )
        .await;
        if history["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["content"]["content"] == REPLY)
        {
            completed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    if !completed {
        let events = call(
            &router,
            "GET",
            &format!("/api/agent-sessions/{id}/events?after_seq=0&limit=500"),
            json!({}),
        )
        .await;
        let tail = events["events"].as_array().unwrap().iter().rev().take(3).collect::<Vec<_>>();
        panic!("import did not finish: {tail:?}");
    }
    let providers = call(&router, "GET", "/api/providers", json!({})).await;
    assert_eq!(providers.as_array().unwrap().len(), 2);
    let current = providers
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["provider_id"] == provider["provider_id"])
        .unwrap();
    assert_eq!(current["models"].as_array().unwrap().len(), 2);
    let projection = call(
        &router,
        "GET",
        &format!("/api/agent-sessions/{id}/projection"),
        json!({}),
    )
    .await;
    assert_eq!(projection["model"], selection);
    let mut changed = Vec::new();
    while let Ok(envelope) = changes.try_recv() {
        if envelope.event.name == "providers.changed" {
            assert_eq!(envelope.user_id, services.authoritative_user_id.as_ref());
            assert!(
                !serde_json::to_string(&envelope.event)
                    .unwrap()
                    .contains("import-secret-value")
            );
            changed.push(envelope);
        }
    }
    assert_eq!(changed.len(), 2);
    services.shutdown_browser_platform().await.unwrap();
}
