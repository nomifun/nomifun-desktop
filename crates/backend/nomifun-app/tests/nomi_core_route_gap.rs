//! Route-topology regression guard for the current Nomi-core composition.
//!
//! Route-topology and smoke guards for the current Nomi-core composition.
//!
//! The default router must expose the app-local Agent Settings/AgentSession/
//! Remote adapter while keeping the Fresh-v4/Codex router out of the product
//! path.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use serde_json::{Value, json};
use tower::ServiceExt;

fn repo_file(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(relative)
        .canonicalize()
        .unwrap_or_else(|error| panic!("cannot resolve {relative}: {error}"));
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

#[test]
fn default_nomi_core_router_exposes_only_canonical_agent_sessions_and_execution() {
    let routes = repo_file("src/router/routes.rs");

    for required_merge in [
        ".merge(agent_authenticated)",
        ".merge(agent_execution_authenticated)",
        ".merge(agent_execution_template_authenticated)",
    ] {
        assert!(
            routes.contains(required_merge),
            "Nomi-core router must retain {required_merge}"
        );
    }

    assert!(
        routes.contains("build_nomi_core_agent_router("),
        "default Nomi-core router must mount the app-local Agent adapter"
    );
    assert!(
        routes.contains(".nest(\"/mcp\", nomi_core_remote_mcp)"),
        "default Nomi-core router must mount the canonical Remote MCP transport"
    );
    assert!(
        !routes.contains("remote_rest::build("),
        "default Nomi-core router must not mount the Fresh-v4 Remote adapter"
    );
    for retired in [
        "conversation_routes(",
        "conversation_ops_routes(",
        "creative_studio_agent_session_routes(",
        "legacy_conversation_port",
    ] {
        assert!(
            !routes.contains(retired),
            "retired Conversation route authority must be unreachable: {retired}"
        );
    }
}

#[test]
fn canonical_surfaces_are_traceable_to_the_app_local_route_definitions() {
    let control_plane = repo_file("../nomifun-agent-control-plane/src/routes.rs");
    let session = repo_file("src/router/nomi_core_session.rs");

    // These are the exact route groups consumed by ui/src/common/adapter/
    // ipcBridge.ts and must remain in the current Nomi-core adapter.
    for path in [
        "/api/agent-preset-templates",
        "/api/capabilities",
    ] {
        assert!(control_plane.contains(path), "control-plane definition must remain discoverable for {path}");
        assert!(
            session.contains("build_nomi_core_agent_router"),
            "Nomi-core route adapter must be mounted for {path}"
        );
    }
    for path in [
        "/api/agent-sessions",
        "/api/agent-sessions/{agent_session_id}",
        "/api/agent-session-messages/search",
        "/api/agent-sessions/{agent_session_id}/creation-tasks",
        "/api/creative-studio/canvas-agent-sessions/resolve",
        "/api/remote/open",
        "/api/remote/turn",
        "/api/remote/observe",
        "/api/remote/cancel",
    ] {
        assert!(session.contains(path), "Nomi-core adapter must define {path}");
    }
}

#[test]
fn startup_reconciles_orphaned_running_turns_before_publishing_routes() {
    let state = repo_file("src/router/state.rs");
    let sessions = repo_file("src/router/nomi_core_session.rs");

    assert!(state.contains("reconcile_orphaned_active_turns().await?"));
    assert!(sessions.contains("Runtime owner was not recoverable after restart"));
    assert!(sessions.contains("head.status = 'running'"));
}

#[test]
fn current_nomi_core_projection_is_not_a_runtime_or_route_authority() {
    let projection = repo_file("src/router/agent_binding_projection.rs");

    assert!(
        projection.contains("pub fn project("),
        "the Nomi-core projection seam must remain available for the future adapter"
    );
    for forbidden in [
        "AgentPlatform::from_pool",
        "AgentSessionStore",
        "CodexRuntimeSupervisor",
        "RemoteRuntimeCoordinator",
        "Router::new()",
    ] {
        assert!(
            !projection.contains(forbidden),
            "projection seam must not become a second runtime/router authority: {forbidden}"
        );
    }
}

#[test]
fn nomi_core_projection_does_not_reintroduce_preset_resource_fields() {
    let projection = repo_file("src/router/agent_binding_projection.rs");
    assert!(
        !projection.contains("payload.resource_bindings"),
        "Preset Revision projection must not own concrete resource bindings"
    );
    assert!(
        !projection.contains("resource_binding_refs"),
        "Capability selections must not carry target resource references"
    );
}

#[tokio::test]
async fn canonical_session_turn_dispatches_and_projects_without_legacy_rows() {
    const TRUST: &str = "canonical-turn-dispatch";
    async fn call(
        router: axum::Router,
        method: &str,
        path: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        let response = router
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("x-nomi-local-trust", TRUST)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, body)
    }

    let upstream = wiremock::MockServer::start().await;
    const REPLY: &str = "CANONICAL_STORE_ONLY_OK";
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(format!(
                    "data: {{\"id\":\"canonical\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{REPLY}\"}},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"canonical\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
                )),
        )
        .mount(&upstream)
        .await;
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let (status, provider) = call(
        router.clone(),
        "POST",
        "/api/providers",
        json!({
            "platform": "stepfun-plan",
            "name": "Canonical dispatch fixture",
            "base_url": format!("{}/v1", upstream.uri()),
            "auth_scheme": "bearer",
            "credentials": { "api_keys": ["fixture-not-a-secret"] },
            "enabled": true,
            "initial_model": {
                "model": "step-3.7-flash",
                "enabled": true,
                "capabilities": [{
                    "task": "chat",
                    "traits": [],
                    "protocol": "openai.chat_text",
                    "connection_role": "default",
                    "provider_params": {}
                }]
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let model = json!({
        "provider_id": provider["data"]["provider_id"],
        "model": "step-3.7-flash"
    });
    let (status, preset) = call(
        router.clone(),
        "POST",
        "/api/agent-presets/from-template/chat.minimal",
        json!({
            "display_name": "Canonical dispatch",
            "reuse_existing": true,
            "model_route_refs": {},
            "chat_route_records": {},
            "model": model
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let preset_id = preset["data"]["preset"]["preset_id"].as_str().unwrap();
    let (status, session) = call(
        router.clone(),
        "POST",
        "/api/agent-sessions",
        json!({ "preset_id": preset_id, "model": model, "title": "Canonical" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let session_id = session["data"]["agent_session_id"].as_str().unwrap();
    let key = uuid::Uuid::now_v7().to_string();
    let (status, turn) = call(
        router.clone(),
        "POST",
        &format!("/api/agent-sessions/{session_id}/turns"),
        json!({ "idempotency_key": key, "input": { "content": "reply once" } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{turn}");
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let (status, history) = call(
                router.clone(),
                "GET",
                &format!("/api/agent-sessions/{session_id}/message-history?page_size=50"),
                json!({}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{history}");
            if history["data"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["content"]["content"] == REPLY)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("canonical assistant projection did not become durable");
    let canonical_store = nomifun_agent_session::AgentSessionStore::from_pool(
        services.database.pool().clone(),
    )
    .await
    .unwrap();
    let session_key = nomifun_agent_contracts::AgentSessionId::from(session_id.to_owned());
    let before_rebuild = canonical_store.head(&session_key).await.unwrap();
    let rebuilt = canonical_store.rebuild_projections(&session_key).await.unwrap();
    assert_eq!(rebuilt, before_rebuild, "projection rebuild must be deterministic");
    let (status, listed) = call(
        router.clone(),
        "GET",
        "/api/agent-sessions?limit=50",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(listed["data"]["items"].as_array().unwrap().iter()
        .any(|item| item["conversation_id"] == session_id));
    let (status, updated) = call(
        router.clone(),
        "PATCH",
        &format!("/api/agent-sessions/{session_id}"),
        json!({ "name": "Canonical renamed", "pinned": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["data"]["name"], "Canonical renamed");
    assert_eq!(updated["data"]["pinned"], true);
    let legacy_conversations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'conversations'")
    .fetch_one(services.database.pool())
    .await
    .unwrap();
    let legacy_messages: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'messages'")
            .fetch_one(services.database.pool())
            .await
            .unwrap();
    assert_eq!((legacy_conversations, legacy_messages), (0, 0));
    let (status, deleted) = call(
        router.clone(),
        "DELETE",
        &format!("/api/agent-sessions/{session_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    let (status, _) = call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for (method, path) in [
        ("GET", "/api/conversations"),
        ("POST", "/api/conversations"),
        ("GET", "/api/messages/search"),
    ] {
        let (status, _) = call(router.clone(), method, path, json!({})).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "retired route survived: {method} {path}");
    }
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn canonical_session_mounts_multiple_knowledge_bases_and_executes_search_before_answering() {
    const TRUST: &str = "canonical-knowledge-tool";
    const QUERY: &str = "NOMIFUN_KNOWLEDGE_QUERY_9233";
    const RESULT_MARKER: &str = "KNOWLEDGE_TOOL_RESULT_9233";
    const REPLY: &str = "KNOWLEDGE_SEARCH_WAS_USED";

    async fn call(
        router: axum::Router,
        method: &str,
        path: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        let response = router
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("x-nomi-local-trust", TRUST)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, body)
    }

    let upstream = wiremock::MockServer::start().await;
    let model_requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = Arc::clone(&model_requests);
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let body = request.body_json::<Value>().unwrap();
            captured.lock().unwrap().push(body.clone());
            let messages = body["messages"].as_array().unwrap();
            let tool_results = messages
                .iter()
                .filter(|message| message["role"] == "tool")
                .collect::<Vec<_>>();
            let (frame, finish_reason, response_id) = if tool_results.is_empty() {
                let search = body["tools"]
                    .as_array()
                    .and_then(|tools| {
                        tools.iter().find(|tool| {
                            tool["function"]["description"]
                                .as_str()
                                .is_some_and(|description| {
                                    description.contains("Action: knowledge/search")
                                })
                        })
                    })
                    .expect("the frozen Knowledge search Action must reach the model");
                (
                    json!({
                        "id": "knowledge-search-round",
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "role": "assistant",
                                "tool_calls": [{
                                    "index": 0,
                                    "id": "knowledge-search-call",
                                    "type": "function",
                                    "function": {
                                        "name": search["function"]["name"],
                                        "arguments": json!({"query": QUERY, "limit": 8}).to_string()
                                    }
                                }]
                            },
                            "finish_reason": null
                        }]
                    }),
                    "tool_calls",
                    "knowledge-search-round",
                )
            } else if tool_results.len() == 1 {
                let search_result: Value = serde_json::from_str(
                    tool_results[0]["content"]
                        .as_str()
                        .expect("Knowledge search result must be model-visible text"),
                )
                .expect("Knowledge search result must be canonical JSON");
                let handle = search_result["hits"][0]["handle"]
                    .as_str()
                    .expect("Knowledge search result must carry an opaque handle");
                let read = body["tools"]
                    .as_array()
                    .and_then(|tools| {
                        tools.iter().find(|tool| {
                            tool["function"]["description"]
                                .as_str()
                                .is_some_and(|description| {
                                    description.contains("Action: knowledge/read")
                                })
                        })
                    })
                    .expect("the frozen Knowledge read Action must remain available");
                (
                    json!({
                        "id": "knowledge-read-round",
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "role": "assistant",
                                "tool_calls": [{
                                    "index": 0,
                                    "id": "knowledge-read-call",
                                    "type": "function",
                                    "function": {
                                        "name": read["function"]["name"],
                                        "arguments": json!({"handle": handle}).to_string()
                                    }
                                }]
                            },
                            "finish_reason": null
                        }]
                    }),
                    "tool_calls",
                    "knowledge-read-round",
                )
            } else {
                assert!(
                    tool_results.last().is_some_and(|message| message.to_string().contains(RESULT_MARKER)),
                    "the full Knowledge document must reach the successor model round: {body}"
                );
                (
                    json!({
                        "id": "knowledge-final-round",
                        "choices": [{
                            "index": 0,
                            "delta": {"content": REPLY},
                            "finish_reason": null
                        }]
                    }),
                    "stop",
                    "knowledge-final-round",
                )
            };
            let done = json!({
                "id": response_id,
                "choices": [{"index": 0, "delta": {}, "finish_reason": finish_reason}]
            });
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(format!("data: {frame}\n\ndata: {done}\n\ndata: [DONE]\n\n"))
        })
        .mount(&upstream)
        .await;

    let (router, services) = common::build_local_trust_app(TRUST).await;
    let base_a = services
        .knowledge_service
        .create_base("Python handbook", "Python domain rules", None, None)
        .await
        .unwrap();
    let base_b = services
        .knowledge_service
        .create_base("Engineering notes", "Team-specific references", None, None)
        .await
        .unwrap();
    services
        .knowledge_service
        .write_file(
            base_a.knowledge_base_id.as_str(),
            "python-types.md",
            &format!("# Python types\n\n{QUERY} {RESULT_MARKER}"),
        )
        .await
        .unwrap();
    services
        .knowledge_service
        .write_file(
            base_b.knowledge_base_id.as_str(),
            "unrelated.md",
            "# Other notes\n\nNo matching marker here.",
        )
        .await
        .unwrap();

    let (status, provider) = call(
        router.clone(),
        "POST",
        "/api/providers",
        json!({
            "platform": "stepfun-plan",
            "name": "Knowledge tool fixture",
            "base_url": format!("{}/v1", upstream.uri()),
            "auth_scheme": "bearer",
            "credentials": {"api_keys": ["fixture-not-a-secret"]},
            "enabled": true,
            "initial_model": {
                "model": "step-3.7-flash",
                "enabled": true,
                "capabilities": [{
                    "task": "chat",
                    "traits": [],
                    "protocol": "openai.chat_text",
                    "connection_role": "default",
                    "provider_params": {}
                }]
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let model = json!({
        "provider_id": provider["data"]["provider_id"],
        "model": "step-3.7-flash"
    });
    let (status, minimal) = call(
        router.clone(),
        "POST",
        "/api/agent-presets/from-template/chat.minimal",
        json!({
            "display_name": "Knowledge route source",
            "reuse_existing": false,
            "model": model
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{minimal}");
    let mut document = minimal["data"]["revision"]["document"].clone();
    document["enabled_capabilities"] = json!([{
        "capability": {"id": "knowledge"},
        "action_allowlist": ["knowledge/read", "knowledge/search"]
    }]);
    document["instructions"] = Value::String(
        "Use the mounted Knowledge bases for covered questions.".to_owned(),
    );
    let (status, preset) = call(
        router.clone(),
        "POST",
        "/api/agent-presets",
        json!({
            "display_name": "Knowledge-only Agent",
            "description": "Canonical Knowledge regression fixture",
            "document": document
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let preset_id = preset["data"]["preset"]["preset_id"].as_str().unwrap();

    let (status, session) = call(
        router.clone(),
        "POST",
        "/api/agent-sessions",
        json!({
            "preset_id": preset_id,
            "model": model,
            "title": "Knowledge canonical Session",
            "resource_selections": [
                {"resource_kind": "knowledge_base", "resource_id": base_a.knowledge_base_id},
                {"resource_kind": "knowledge_base", "resource_id": base_b.knowledge_base_id}
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let resources = session["data"]["agent_binding"]["typed_resource_bindings"]
        .as_array()
        .unwrap();
    assert_eq!(resources.len(), 2, "{session}");
    assert!(resources.iter().all(|resource| resource["resource_kind"] == "knowledge_base"));
    assert!(resources.iter().any(|resource| {
        resource["typed_parameters"]["knowledge_name"] == "Python handbook"
    }));

    let session_id = session["data"]["agent_session_id"].as_str().unwrap();
    let (status, retired_binding) = call(
        router.clone(),
        "POST",
        &format!("/api/knowledge/binding/conversation/{session_id}"),
        json!({
            "enabled": true,
            "writeback": false,
            "writeback_eagerness": "manual",
            "channel_write_enabled": false,
            "kb_ids": [base_a.knowledge_base_id]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{retired_binding}");

    let (status, turn) = call(
        router.clone(),
        "POST",
        &format!("/api/agent-sessions/{session_id}/turns"),
        json!({
            "idempotency_key": uuid::Uuid::now_v7().to_string(),
            "input": {"content": format!("请根据挂载知识库回答 {QUERY}")}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{turn}");
    let completed = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let (status, history) = call(
                router.clone(),
                "GET",
                &format!("/api/agent-sessions/{session_id}/message-history?page_size=50"),
                json!({}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{history}");
            if history["data"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["content"]["content"] == REPLY)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await;
    if completed.is_err() {
        let (_, events) = call(
            router.clone(),
            "GET",
            &format!("/api/agent-sessions/{session_id}/events?after_seq=0&limit=200"),
            json!({}),
        )
        .await;
        let (_, history) = call(
            router.clone(),
            "GET",
            &format!("/api/agent-sessions/{session_id}/message-history?page_size=50"),
            json!({}),
        )
        .await;
        panic!(
            "Knowledge-backed assistant reply did not become durable; model requests: {}; events: {}; history: {}",
            serde_json::to_string_pretty(&*model_requests.lock().unwrap()).unwrap(),
            serde_json::to_string_pretty(&events).unwrap(),
            serde_json::to_string_pretty(&history).unwrap(),
        );
    }

    let requests = model_requests.lock().unwrap();
    assert_eq!(requests.len(), 3, "search, read, and final model rounds are required");
    let first = &requests[0];
    let prompt = first["messages"].to_string();
    assert!(prompt.contains("knowledge/search before answering from memory"), "{first}");
    assert!(prompt.contains("Python handbook"), "{first}");
    assert!(prompt.contains("Engineering notes"), "{first}");
    assert!(first["tools"].as_array().unwrap().iter().any(|tool| {
        tool["function"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("Action: knowledge/search"))
    }));
    assert!(requests[1].to_string().contains("knowledge-search-call"), "{}", requests[1]);
    assert!(requests[2].to_string().contains(RESULT_MARKER), "{}", requests[2]);
    drop(requests);

    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[path = "common/mod.rs"]
mod common;

#[allow(dead_code)]
#[path = "../src/router/agent_binding_projection.rs"]
mod projection;

#[tokio::test]
async fn default_nomi_core_router_answers_canonical_catalog_requests() {
    let (router, services) = common::build_local_trust_app("route-gap-local-trust").await;
    for path in [
        "/api/agent-preset-templates?source=official",
        "/api/agent-catalog",
        "/api/agent-role-defaults",
        "/api/capabilities",
        "/api/agent-catalog/skills",
        "/api/mcp-tool-mappings",
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("x-nomi-local-trust", "route-gap-local-trust")
                    .body(Body::empty())
                    .expect("build catalog request"),
            )
            .await
            .expect("dispatch catalog request");
        assert_eq!(response.status(), StatusCode::OK, "catalog route {path}");
        let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read catalog response");
        let value: Value = serde_json::from_slice(&body).expect("catalog response JSON");
        assert!(value.get("data").is_some(), "catalog response must be wrapped: {path}");
        if path == "/api/agent-catalog" {
            for field in ["capabilities", "skills", "mcp_tools", "roles"] {
                assert!(value["data"][field].is_array(), "complete Catalog must include {field}");
            }
        }
    }
    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn installation_token_is_limited_to_headless_product_control_planes() {
    let trust_secret = "headless-product-local-trust";
    let installation_token = "headless-product-installation-token";
    let (router, services) = common::build_local_trust_app(trust_secret).await;
    services
        .instance_token_validator
        .set_token(nomifun_auth::token_sha256_hex(installation_token));

    for path in [
        "/api/javascript-runtime/status",
        "/api/plugins",
        "/api/plugins/runtimes",
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header(
                        "authorization",
                        format!("Bearer {installation_token}"),
                    )
                    .body(Body::empty())
                    .expect("build installation-token request"),
            )
            .await
            .expect("dispatch installation-token request");
        assert_eq!(response.status(), StatusCode::OK, "product route {path}");
    }

    let mutation = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/plugins/projects")
                .header(
                    "authorization",
                    format!("Bearer {installation_token}"),
                )
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .expect("build installation-token mutation"),
        )
        .await
        .expect("dispatch installation-token mutation");
    assert_eq!(
        mutation.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "Bearer mutation must pass CSRF/auth and reach JSON validation"
    );

    let owner_jwt = services
        .jwt_service
        .sign(services.authoritative_user_id.as_ref(), "owner")
        .expect("sign owner JWT");
    // Read routes explicitly permit an authenticated installation owner JWT.
    for (path, token, expected) in [
        ("/api/plugins", "wrong-installation-token", StatusCode::FORBIDDEN),
        ("/api/plugins", owner_jwt.as_str(), StatusCode::OK),
        ("/api/capabilities", installation_token, StatusCode::FORBIDDEN),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("build rejected product request"),
            )
            .await
            .expect("dispatch rejected product request");
        assert_eq!(response.status(), expected, "route {path}");
    }

    services
        .shutdown_browser_platform()
        .await
        .expect("browser cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn nomi_core_remote_preserves_owner_jwt_and_rejects_selector_queries() {
    let trust_secret = "remote-jwt-query-local-trust";
    let (router, services) = common::build_local_trust_app(trust_secret).await;
    let jwt = services
        .jwt_service
        .sign(services.authoritative_user_id.as_ref(), "admin")
        .expect("sign installation owner JWT");

    let jwt_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/remote/observe?agent_session_id=0190f5fe-7c00-7a00-8000-000000000001")
                .header("authorization", format!("Bearer {jwt}"))
                .body(Body::empty())
                .expect("build JWT Remote request"),
        )
        .await
        .expect("dispatch JWT Remote request");
    assert_eq!(
        jwt_response.status(),
        StatusCode::NOT_FOUND,
        "the installation owner's JWT must still reach the owner-scoped Remote handler"
    );

    for uri in [
        "/api/remote/open?profile=agent",
        "/api/remote/turn?domains=agent",
        "/api/remote/cancel?selector=latest",
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("x-nomi-local-trust", trust_secret)
                    .body(Body::empty())
                    .expect("build selector query request"),
            )
            .await
            .expect("dispatch selector query request");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read selector query response");
        let value: Value = serde_json::from_slice(&body).expect("selector query JSON");
        assert_eq!(value["code"], "REMOTE_INVALID_REQUEST", "{uri}");
    }

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn nomi_core_catalog_exposes_native_nomi_capabilities() {
    let (router, services) = common::build_local_trust_app("catalog-placement-local-trust").await;
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/capabilities")
                .header("x-nomi-local-trust", "catalog-placement-local-trust")
                .body(Body::empty())
                .expect("build capability catalog request"),
        )
        .await
        .expect("dispatch capability catalog request");
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read capability catalog response");
    let value: Value = serde_json::from_slice(&body).expect("capability catalog JSON");
    let capabilities = value["data"]
        .as_array()
        .expect("capability catalog array");

    for capability_id in ["workspace.files", "workspace.vcs"] {
        let capability = capabilities
            .iter()
            .find(|item| item["capability"]["id"] == capability_id)
            .unwrap_or_else(|| panic!("missing capability {capability_id}"));
        assert_eq!(
            capability["materialization_state"],
            "materialized",
            "{capability_id} must remain selectable as a Module"
        );
        assert_eq!(
            capability["required_resource_kinds"].as_array().map(Vec::len),
            Some(1),
            "{capability_id} must retain its target resource-kind requirement"
        );
    }
    for capability_id in [
        "web.research",
        "agent.collaboration",
        "agent.tool-discovery",
        "automation.schedule",
    ] {
        let capability = capabilities.iter()
            .find(|item| item["capability"]["id"] == capability_id)
            .unwrap_or_else(|| panic!("missing repaired builtin {capability_id}"));
        assert_eq!(capability["materialization_state"], "materialized", "{capability_id}");
        assert!(capability["unavailable_code"].is_null(), "{capability_id}");
        assert_eq!(capability["source_kind"], "bundled");
    }
    let browser = capabilities
        .iter()
        .find(|item| item["capability"]["id"] == "browser")
        .expect("browser capability remains visible in the shared catalog");
    assert_eq!(
        browser["materialization_state"],
        "materialized",
        "Browser is a canonical Module; concrete Resource availability is resolved at binding time"
    );
    assert!(browser["unavailable_code"].is_null());

    let templates = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/agent-preset-templates?source=official")
                .header("x-nomi-local-trust", "catalog-placement-local-trust")
                .body(Body::empty())
                .expect("build official template request"),
        )
        .await
        .expect("dispatch official template request");
    assert_eq!(templates.status(), StatusCode::OK);
    let template_body = axum::body::to_bytes(templates.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read official template response");
    let template_value: Value =
        serde_json::from_slice(&template_body).expect("official template JSON");
    let general = template_value["data"]["official_templates"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["template_key"] == "assistant.general")
        })
        .expect("general template");
    let general_enabled = general["seed"]["enabled_capabilities"]
        .as_array()
        .expect("general enabled capabilities");
    for (module_id, action_id) in [
        ("workspace.artifacts", "workspace.artifacts/read"),
        ("workspace.files", "workspace.files/patch"),
        ("workspace.process", "workspace.process/exec"),
        ("workspace.vcs", "workspace.vcs/commit"),
        ("agent.collaboration", "agent/delegate"),
        ("requirements", "requirements/read"),
        ("agent.tool-discovery", "tool.discovery.rank"),
    ] {
        let selection = general_enabled
            .iter()
            .find(|item| item["capability"]["id"] == module_id)
            .unwrap_or_else(|| panic!("missing general Module {module_id}"));
        assert!(
            selection["action_allowlist"]
                .as_array()
                .is_some_and(|actions| actions.iter().any(|action| action == action_id)),
            "{module_id} must retain exact Action {action_id}"
        );
    }
    assert!(
        general_enabled
            .iter()
            .all(|item| item["capability"]["id"] != "plugin.development"),
        "general conversations must not require a selected Plugin project"
    );
    assert!(
        general["seed"]["required_resource_kinds"]
            .as_array()
            .is_some_and(|kinds| kinds.iter().all(|kind| kind != "plugin")),
        "general conversations must launch without a Plugin resource binding"
    );
    let coding = template_value["data"]["official_templates"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["template_key"] == "coding.codex")
        })
        .expect("coding template");
    let enabled = coding["seed"]["enabled_capabilities"]
        .as_array()
        .expect("coding enabled capabilities");
    for (module_id, action_id) in [
        ("workspace.files", "workspace.files/patch"),
        ("workspace.vcs", "workspace.vcs/commit"),
        ("workspace.process", "workspace.process/exec"),
        ("workspace.artifacts", "workspace.artifacts/publish"),
        ("project.memory", "project.memory/read"),
        ("agent.collaboration", "agent/delegate"),
        ("agent.tool-discovery", "tool.discovery.rank"),
        ("web.research", "web.research/search"),
    ] {
        let selection = enabled.iter()
            .find(|item| item["capability"]["id"] == module_id)
            .unwrap_or_else(|| panic!("missing coding Module {module_id}"));
        assert!(selection["action_allowlist"].as_array()
            .is_some_and(|actions| actions.iter().any(|action| action == action_id)),
            "{module_id} must retain exact Action {action_id}");
    }

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn nomi_core_accepts_exact_module_action_grants() {
    let trust_secret = "enabled-placement-local-trust";
    let (router, services) = common::build_local_trust_app(trust_secret).await;
    let created = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-presets/from-template/chat.minimal")
                .header("x-nomi-local-trust", trust_secret)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "reuse_existing": false,
                        "display_name": "On-demand placement smoke",
                        "model_route_refs": {},
                        "chat_route_records": {}
                    }))
                    .expect("serialize placement template request"),
                ))
                .expect("build placement template request"),
        )
        .await
        .expect("dispatch placement template request");
    assert_eq!(created.status(), StatusCode::OK);
    let created_body = axum::body::to_bytes(created.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read placement template response");
    let mut created_value: Value =
        serde_json::from_slice(&created_body).expect("placement template JSON");
    let preset_id = created_value["data"]["preset"]["preset_id"]
        .as_str()
        .expect("placement preset id")
        .to_owned();
    let revision = created_value["data"]["revision"]["reference"].clone();
    let enabled = [
        ("workspace.vcs", "workspace.vcs/status"),
        ("web.research", "web.research/search"),
        ("agent.collaboration", "agent/request_user_decision"),
        ("automation.schedule", "automation.schedule/list"),
    ];
    created_value["data"]["draft"]["document"]["enabled_capabilities"] = json!(
        enabled.iter().map(|(id, action)| json!({
            "capability": {"id": id},
            "action_allowlist": [action]
        })).collect::<Vec<_>>()
    );
    let saved = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent-presets/{preset_id}/revisions"))
                .header("x-nomi-local-trust", trust_secret)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "expected_current_revision": revision,
                        "draft": created_value["data"]["draft"],
                    }))
                    .expect("serialize enabled save request"),
                ))
                .expect("build enabled save request"),
        )
        .await
        .expect("dispatch enabled save request");
    assert_eq!(saved.status(), StatusCode::OK);
    let saved_body = axum::body::to_bytes(saved.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read enabled save response");
    let saved_value: Value = serde_json::from_slice(&saved_body).expect("save JSON");
    assert_eq!(
        saved_value["data"]["revision"]["document"]["enabled_capabilities"]
            .as_array()
            .map(Vec::len),
        Some(enabled.len()),
        "the saved revision must retain the exact Module grants"
    );
    for (id, action) in enabled {
        let selection = saved_value["data"]["revision"]["document"]["enabled_capabilities"]
            .as_array().and_then(|items| items.iter()
                .find(|item| item["capability"]["id"] == id))
            .unwrap_or_else(|| panic!("missing immutable Module {id}"));
        assert_eq!(selection["action_allowlist"], json!([action]));
    }

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn configured_agent_creation_persists_adjusted_capabilities_and_keeps_official_seeds() {
    const TRUST: &str = "configured-agent-workbench-test";
    async fn call(router: axum::Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let response = router.oneshot(Request::builder().method(method).uri(path)
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let (_, before) = call(router.clone(), "GET", "/api/agent-preset-templates?source=official", json!({})).await;
    let before_count = before["data"]["user_presets"].as_array().unwrap().len();
    let official_before = before["data"]["official_templates"].clone();
    let document = json!({
        "schema_version":"1.0.0", "model_route_refs":{}, "chat_route_records":{},
        "enabled_capabilities":[
            {"capability":{"id":"knowledge"},"action_allowlist":["knowledge/read","knowledge/search"]},
            {"capability":{"id":"web.research"},"action_allowlist":["web.research/fetch","web.research/search"]},
            {"capability":{"id":"automation.schedule"},"action_allowlist":["automation.schedule/list"]}
        ],
        "skill_bindings":[], "system_role_provider_overrides":{}, "persona":"Research helper",
        "instructions":"Use only the selected capabilities.", "starter_prompts":[]
    });
    let (status, created) = call(router.clone(), "POST", "/api/agent-presets", json!({
        "display_name":"My adjusted assistant", "description":"Custom capability scope", "document":document,
    })).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["data"]["preset"]["source"], "user");
    assert_eq!(created["data"]["revision"]["reference"]["revision"], 1);
    assert!(created["data"]["draft"]["source_template_key"].is_null());
    let id = created["data"]["preset"]["preset_id"].as_str().unwrap();
    let (status, reloaded) = call(router.clone(), "GET", &format!("/api/agent-presets/{id}/editor"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reloaded["data"]["draft"]["document"]["enabled_capabilities"], created["data"]["revision"]["document"]["enabled_capabilities"]);
    assert_eq!(reloaded["data"]["draft"]["document"]["enabled_capabilities"].as_array().unwrap().len(), 3);
    let mut invalid = document;
    invalid["enabled_capabilities"] = json!([{"capability":{"id":"missing.capability"}}]);
    let (status, _) = call(router.clone(), "POST", "/api/agent-presets", json!({"display_name":"Must not persist", "document":invalid})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, after) = call(router, "GET", "/api/agent-preset-templates?source=official", json!({})).await;
    assert_eq!(after["data"]["user_presets"].as_array().unwrap().len(), before_count + 1);
    assert_eq!(after["data"]["official_templates"], official_before);
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn official_template_creation_requires_explicit_persistence_intent() {
    const TRUST: &str = "official-agent-explicit-intent";
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let response = router.clone().oneshot(Request::builder()
        .method("POST")
        .uri("/api/agent-presets/from-template/chat.minimal")
        .header("x-nomi-local-trust", TRUST)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "display_name": "Must not become my Agent",
            "model_route_refs": {},
            "chat_route_records": {}
        })).unwrap())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY,
        "omitting launch/save intent must fail closed instead of creating a personal Agent");

    let library_response = router.oneshot(Request::builder()
        .uri("/api/agent-preset-templates")
        .header("x-nomi-local-trust", TRUST)
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(library_response.status(), StatusCode::OK);
    let library: Value = serde_json::from_slice(&axum::body::to_bytes(
        library_response.into_body(), 4 * 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(library["data"]["user_presets"], json!([]));
    let persisted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_presets")
        .fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(persisted, 0, "rejected ambiguous requests must not write hidden or visible configurations");
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn product_agent_selection_precedes_models_and_reports_host_capability_availability() {
    const TRUST: &str = "product-selection-preflight";
    async fn call(router: axum::Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let response = router.oneshot(Request::builder().method(method).uri(path)
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let (_, created) = call(router.clone(), "POST", "/api/companion/companions", json!({ "name": "Choose Agent first", "character": "ink" })).await;
    let companion_id = created["data"]["companion_id"].as_str().unwrap();
    let chosen = json!({ "kind": "template", "template_key": "chat.minimal" });
    let mut paths = Vec::new();
    for kind in ["customer", "creative_studio_canvas"] {
        let path = format!("/api/product-agent-bindings/{kind}/{companion_id}");
        let (status, saved) = call(router.clone(), "PUT", &path, json!({ "selection": chosen })).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["data"]["needs_model"], true);
        let (status, reloaded) = call(router.clone(), "GET", &path, json!({})).await;
        assert_eq!(status, StatusCode::OK, "{reloaded}");
        assert_eq!(reloaded["data"]["selection"], chosen);
        paths.push(path);
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_presets").fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(count, 0, "model-free selection must not allocate executable presets or select a default model");
    let upstream = wiremock::MockServer::start().await;
    let (status, provider) = call(router.clone(), "POST", "/api/providers", json!({
        "platform": "stepfun-plan", "name": "Preflight StepFun",
        "base_url": format!("{}/step_plan/v1", upstream.uri()),
        "auth_scheme": "bearer", "credentials": { "api_keys": ["test-only"] }, "enabled": true,
        "initial_model": { "model": "step-3.7-flash", "enabled": true,
            "capabilities": [{ "task": "chat", "traits": [],
                "protocol": "openai.chat_text", "connection_role": "default", "provider_params": {} }] }
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let model = json!({ "provider_id": provider["data"]["provider_id"], "model": "step-3.7-flash" });
    for path in &paths {
        let query = format!("{path}?provider_id={}&model=step-3.7-flash", model["provider_id"].as_str().unwrap());
        let (status, options) = call(router.clone(), "GET", &query, json!({})).await;
        assert_eq!(status, StatusCode::OK, "{options}");
        let general = options["data"]["options"].as_array().unwrap().iter().find(|item| item["selection"]["template_key"] == "assistant.general").unwrap();
        assert_eq!(general["available"], false, "{general}");
        assert_eq!(general["reason"], "capability", "the headless route fixture has no native Browser resource owner");
        let coding = options["data"]["options"].as_array().unwrap().iter().find(|item| item["selection"]["template_key"] == "coding.codex").unwrap();
        assert_eq!(coding["available"], true, "{coding}");
        assert!(coding["reason"].is_null(), "{coding}");
        let (_, after) = call(router.clone(), "GET", path, json!({})).await;
        assert_eq!(after["data"]["selection"], chosen, "availability checks must not change the saved Agent");
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_presets").fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(count, 0, "availability checks must not allocate executable presets");
    let coding = json!({ "kind": "template", "template_key": "coding.codex" });
    for path in &paths {
        let (status, saved) = call(router.clone(), "PUT", path, json!({
            "selection": coding, "model": model,
            "resource_selections": [
                {"resource_kind":"workspace", "resource_id":"default-workspace"},
                {"resource_kind":"process_session", "resource_id":"managed-process-session"},
                {"resource_kind":"project_memory", "resource_id":"default-project-memory"}
            ]
        })).await;
        assert_eq!(status, StatusCode::OK, "a chat-only model must support the headless-safe Coding Agent: {saved}");
        assert_eq!(saved["data"]["needs_model"], false);
        let (status, reloaded) = call(router.clone(), "GET", path, json!({})).await;
        assert_eq!(status, StatusCode::OK, "{reloaded}");
        assert_eq!(reloaded["data"]["selection"], coding);
    }
    assert!(upstream.received_requests().await.unwrap().is_empty());
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn canonical_coding_session_has_no_in_place_binding_override_routes() {
    const TRUST: &str = "next-turn-kernel-binding";
    async fn post(router: axum::Router, path: &str, body: Value) -> Value {
        let response = router.oneshot(Request::builder().method("POST").uri(path)
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
            .header("idempotency-key", uuid::Uuid::now_v7().to_string())
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(status.is_success(), "{path}: {status} {value}");
        value["data"].clone()
    }
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let upstream = wiremock::MockServer::start().await;
    let provider = post(router.clone(), "/api/providers", json!({
        "platform":"stepfun-plan", "name":"Kernel binding regression",
        "base_url":format!("{}/step_plan/v1", upstream.uri()),
        "auth_scheme":"bearer", "credentials":{"api_keys":["test-only"]}, "enabled":true,
        "initial_model":{"model":"step-3.7-flash", "enabled":true,
            "capabilities":[{"task":"chat", "traits":[],
                "protocol":"openai.chat_text", "connection_role":"default", "provider_params":{}}]}
    })).await;
    let model = json!({"provider_id":provider["provider_id"], "model":"step-3.7-flash"});
    let original = post(router.clone(), "/api/agent-presets/from-template/coding.codex",
        json!({"display_name":"Coding original", "reuse_existing":false, "model":model})).await;
    let target = post(router.clone(), "/api/agent-presets/from-template/coding.codex",
        json!({"display_name":"Coding selected", "reuse_existing":false, "model":model})).await;
    assert_ne!(original["preset"]["preset_id"], target["preset"]["preset_id"]);
    let resources = json!([
        {"resource_kind":"workspace", "resource_id":"default-workspace"},
        {"resource_kind":"process_session", "resource_id":"managed-process-session"},
        {"resource_kind":"project_memory", "resource_id":"default-project-memory"}
    ]);
    let session = post(router.clone(), "/api/agent-sessions", json!({
        "preset_id":original["preset"]["preset_id"], "model":model,
        "resource_selections":resources,
    })).await;
    let id = session["agent_session_id"].as_str().unwrap();
    for (suffix, body) in [
        ("preset", json!({
            "preset_id":target["preset"]["preset_id"], "resource_selections":resources
        })),
        ("capability-selection", json!({"capability_selection": {}})),
        ("mcp-selection", json!({"mcp_server_ids": []})),
    ] {
        let response = router.clone().oneshot(Request::builder().method("PUT")
            .uri(format!("/api/agent-sessions/{id}/{suffix}"))
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "retired route remained: {suffix}");
    }
    let observed = router.clone().oneshot(Request::builder()
        .uri(format!("/api/agent-sessions/{id}"))
        .header("x-nomi-local-trust", TRUST).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(observed.status(), StatusCode::OK);
    let observed: Value = serde_json::from_slice(&axum::body::to_bytes(
        observed.into_body(), 4 * 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(observed["data"]["session"]["agent_binding"], session["agent_binding"]);
    assert!(upstream.received_requests().await.unwrap().is_empty());
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn http_execution_freezes_the_lead_session_snapshot_and_projects_its_link() {
    const TRUST: &str = "execution-lead-session";
    async fn call(
        router: axum::Router,
        method: &str,
        path: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        let response = router
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("x-nomi-local-trust", TRUST)
                    .header("content-type", "application/json")
                    .header("idempotency-key", uuid::Uuid::now_v7().to_string())
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        let value = serde_json::from_slice(&bytes).unwrap();
        (status, value)
    }

    let (router, services) = common::build_local_trust_app(TRUST).await;
    let upstream = wiremock::MockServer::start().await;
    let (status, provider) = call(
        router.clone(),
        "POST",
        "/api/providers",
        json!({
            "platform":"stepfun-plan", "name":"Execution lead regression",
            "base_url":format!("{}/step_plan/v1", upstream.uri()),
            "auth_scheme":"bearer", "credentials":{"api_keys":["test-only"]}, "enabled":true,
            "initial_model":{"model":"step-3.7-flash", "enabled":true,
                "capabilities":[{"task":"chat", "traits":[],
                    "protocol":"openai.chat_text", "connection_role":"default", "provider_params":{}}]}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let provider_id = provider["data"]["provider_id"].as_str().unwrap();
    let model = json!({"provider_id":provider_id, "model":"step-3.7-flash"});
    let (status, preset) = call(
        router.clone(),
        "POST",
        "/api/agent-presets/from-template/chat.minimal",
        json!({"display_name":"Execution lead", "reuse_existing":false, "model":model}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let preset_id = preset["data"]["preset"]["preset_id"].as_str().unwrap();
    let (status, session) = call(
        router.clone(),
        "POST",
        "/api/agent-sessions",
        json!({"preset_id":preset_id, "model":model, "title":"Execution lead"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let session_id = session["data"]["agent_session_id"].as_str().unwrap();

    let (status, execution) = call(
        router.clone(),
        "POST",
        "/api/agent-executions",
        json!({
            "goal":"Review the release",
            "model_pool":{"mode":"single", "model":model},
            "lead_model":model,
            "lead_conversation_id":session_id,
            "delegation_policy":"prefer_parallel",
            "decision_policy":"ask_user",
            "steps":[{"title":"Review", "spec":"Review the release and report one result"}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{execution}");
    let execution_id = execution["data"]["execution_id"].as_str().unwrap();

    let (status, detail) = call(
        router.clone(),
        "GET",
        &format!("/api/agent-executions/{execution_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let participants = detail["data"]["participants"].as_array().unwrap();
    assert!(participants.iter().any(|participant| {
        participant["agent_snapshot"]["preset_id"].as_str() == Some(preset_id)
    }), "lead participant must retain the frozen Session snapshot: {detail}");

    let (status, projection) = call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/projection"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{projection}");
    assert_eq!(
        projection["data"]["linked_execution_id"].as_str(),
        Some(execution_id)
    );
    assert!(projection["data"]["execution_step_id"].is_null());
    assert!(projection["data"]["execution_attempt_id"].is_null());

    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn creative_studio_entry_uses_its_official_agent() {
    const TRUST: &str = "creative-product-agent";
    async fn call(router: axum::Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let response = router.oneshot(Request::builder().method(method).uri(path)
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let upstream = wiremock::MockServer::start().await;
    let (status, provider) = call(router.clone(), "POST", "/api/providers", json!({
        "platform": "stepfun-plan", "name": "Creative Agent regression",
        "base_url": format!("{}/step_plan/v1", upstream.uri()),
        "auth_scheme": "bearer", "credentials": { "api_keys": ["test-only"] },
        "enabled": true, "initial_model": {
            "model": "step-3.7-flash", "enabled": true,
            "capabilities": [{ "task": "chat", "traits": [],
                "protocol": "openai.chat_text", "connection_role": "default", "provider_params": {} }]
        }
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let provider_id = provider["data"]["provider_id"].as_str().unwrap();
    let (status, canvas) = call(router.clone(), "POST", "/api/creative-studio/canvases",
        json!({
            "title": "Agent-bound canvas",
            "agentKickoff": {
                "prompt": "plan",
                "model": { "providerId": provider_id, "model": "step-3.7-flash" }
            }
        })).await;
    assert_eq!(status, StatusCode::CREATED, "{canvas}");
    let canvas_id = canvas["data"]["canvas"]["canvasId"].as_str().unwrap();
    let document_json: String = sqlx::query_scalar(
        "SELECT document_json FROM creative_studio_projects WHERE project_id = ?")
        .bind(canvas_id).fetch_one(services.database.pool()).await.unwrap();
    let document: Value = serde_json::from_str(&document_json).unwrap();
    let session_id = document["chatSessions"][0]["id"].as_str().unwrap();
    let pending_key = document["chatSessions"][0]["pendingTurn"]["idempotencyKey"]
        .as_str().unwrap();
    let (status, session) = call(router.clone(), "POST",
        "/api/creative-studio/canvas-agent-sessions/resolve", json!({
            "canvas_id": canvas_id,
            "session_id": session_id,
            "model": { "provider_id": provider_id, "model": "step-3.7-flash" },
            "pending_turn_idempotency_key": pending_key
        })).await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    let conversation_id = session["data"]["binding"]["conversation_id"].as_str().unwrap();
    let (status, projected) = call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{conversation_id}/projection"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{projected}");
    let snapshot = &projected["data"]["agent_snapshot"];
    assert_eq!(snapshot["preset_name"], "creative-studio.default");
    assert!(snapshot["enabled_capabilities"].as_array().unwrap().iter()
        .any(|capability| capability == "creative.workshop"));
    assert!(snapshot["enabled_capability_actions"]["creative.workshop"].as_array().unwrap().iter()
        .any(|action| action == "creative.workshop/canvas.edit"));
    let target_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_bindings WHERE target_kind = 'creative_studio_canvas' AND target_id = ?")
        .bind(canvas_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(target_count, 1);
    let legacy_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'conversations'")
    .fetch_one(services.database.pool())
    .await
    .unwrap();
    assert_eq!(legacy_rows, 0, "Creative Studio must bind only a canonical AgentSession");
    assert!(upstream.received_requests().await.unwrap().is_empty());
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn companion_entry_is_fixed_to_its_official_agent() {
    const TRUST: &str = "companion-product-agent";
    async fn call(
        router: axum::Router,
        method: &str,
        path: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        let response = router.oneshot(Request::builder().method(method).uri(path)
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let upstream = wiremock::MockServer::start().await;
    const REPLY: &str = "COMPANION_CHAT_WITHOUT_DEVICE_OK";
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path_regex(".*/chat/completions$"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(format!(
                    "data: {{\"id\":\"companion\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{REPLY}\"}},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"companion\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
                )),
        )
        .mount(&upstream)
        .await;
    let (status, provider) = call(router.clone(), "POST", "/api/providers", json!({
        "platform": "stepfun-plan", "name": "Companion Agent regression",
        "base_url": format!("{}/step_plan/v1", upstream.uri()),
        "auth_scheme": "bearer", "credentials": { "api_keys": ["test-only"] },
        "enabled": true, "initial_model": {
            "model": "step-3.7-flash", "enabled": true,
            "capabilities": [{ "task": "chat", "traits": [],
                "protocol": "openai.chat_text", "connection_role": "default", "provider_params": {} }]
        }
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let provider_id = provider["data"]["provider_id"].as_str().unwrap();
    let (status, companion) = call(router.clone(), "POST", "/api/companion/companions",
        json!({ "name": "Agent-bound companion", "character": "ink" })).await;
    assert_eq!(status, StatusCode::CREATED, "{companion}");
    let companion_id = companion["data"]["companion_id"].as_str().unwrap();
    let (status, patched) = call(router.clone(), "PATCH",
        &format!("/api/companion/companions/{companion_id}"),
        json!({ "model": { "provider_id": provider_id, "model": "step-3.7-flash" } })).await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    let (status, selected_default) = call(router.clone(), "PUT",
        &format!("/api/product-agent-bindings/companion/{companion_id}"), json!({
            "selection": { "kind": "template", "template_key": "companion.default" },
            "model": { "provider_id": provider_id, "model": "step-3.7-flash" },
            "resource_selections": [
                { "resource_kind": "companion", "resource_id": companion_id },
                { "resource_kind": "companion_memory", "resource_id": companion_id },
                { "resource_kind": "scheduler", "resource_id": "installation-scheduler" }
            ]
        })).await;
    assert_eq!(status, StatusCode::OK, "{selected_default}");
    let frozen_resources = selected_default["data"]["agent_binding"]["typed_resource_bindings"]
        .as_array().expect("typed product resources");
    assert_eq!(frozen_resources.len(), 3);
    assert!(frozen_resources.iter().all(|resource|
        resource["resource_kind"] != "robot" && resource["resource_kind"] != "channel"));
    let path = format!("/api/companion/companions/{companion_id}/companion/threads");
    let ((status, thread), (other_status, other_thread)) = tokio::join!(
        call(router.clone(), "POST", &path, json!({})),
        call(router.clone(), "POST", &path, json!({})),
    );
    assert_eq!(status, StatusCode::OK, "{thread}");
    assert_eq!(other_status, StatusCode::OK, "{other_thread}");
    assert_eq!(thread["data"]["conversation_id"], other_thread["data"]["conversation_id"], "concurrent home/sidebar/device opens must share the first conversation");
    let conversation_id = thread["data"]["conversation_id"].as_str().unwrap();
    let (status, projected) = call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{conversation_id}/projection"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{projected}");
    let snapshot = &projected["data"]["agent_snapshot"];
    let preset_id = projected["data"]["preset_id"].as_str().unwrap().to_owned();
    let enabled = snapshot["enabled_capabilities"].as_array().unwrap().iter()
        .map(|capability| capability.as_str().unwrap()).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(enabled, std::collections::BTreeSet::from([
        "agent.tool-discovery", "automation.schedule", "channel.messaging", "companion",
        "companion.memory", "knowledge", "robot",
    ]));
    for (module, expected) in [
        ("companion", &["companion/evolve", "companion/learn"][..]),
        ("companion.memory", &["companion.memory/recall", "companion.memory/write"][..]),
        ("knowledge", &["knowledge/read", "knowledge/search"][..]),
        ("channel.messaging", &["channel.messaging/reply"][..]),
        ("robot", &["robot/vision"][..]),
        ("automation.schedule", &["automation.schedule/create", "automation.schedule/delete", "automation.schedule/list", "automation.schedule/update"][..]),
        ("agent.tool-discovery", &["tool.discovery.rank"][..]),
    ] {
        let actual = snapshot["enabled_capability_actions"][module].as_array().unwrap().iter()
            .map(|action| action.as_str().unwrap()).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, expected.iter().copied().collect(), "{module}");
    }
    assert_eq!(snapshot["preset_name"], "companion.default");
    assert_eq!(projected["data"]["extra"]["companion_session"], true);
    assert_eq!(projected["data"]["extra"]["companion_id"], companion_id);
    assert_eq!(projected["data"]["extra"]["product_agent_target_kind"], "companion");
    assert_eq!(projected["data"]["extra"]["product_agent_target_id"], companion_id);
    let (status, warmed) = call(
        router.clone(),
        "POST",
        &format!("/api/agent-sessions/{conversation_id}/warmup"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{warmed}");
    let failed_key = uuid::Uuid::now_v7().to_string();
    let (status, failed_turn) = call(
        router.clone(),
        "POST",
        &format!("/api/agent-sessions/{conversation_id}/turns"),
        json!({
            "idempotency_key": failed_key,
            "input": {
                "content": "exercise pre-model cleanup",
                "inject_skills": ["not-selected"]
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{failed_turn}");
    let failed_operation = failed_turn["data"]["operation_id"].as_str().unwrap().to_owned();
    let failed_error = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let row = sqlx::query_as::<_, (String, Option<String>)>(
                "SELECT state, error_json FROM agent_turns WHERE session_id = ? AND operation_id = ?",
            )
            .bind(conversation_id)
            .bind(&failed_operation)
            .fetch_one(services.database.pool())
            .await
            .unwrap();
            if row.0 == "failed" {
                break row.1.unwrap_or_default();
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("pre-model Companion failure did not reach a durable terminal");
    assert!(failed_error.contains("requested Skill is not in the Agent's immutable selected Skill locks"), "{failed_error}");
    assert!(!failed_error.contains("turn no longer admits progress"), "cleanup masked the preparation failure: {failed_error}");
    let (status, turn) = call(
        router.clone(),
        "POST",
        &format!("/api/agent-sessions/{conversation_id}/turns"),
        json!({
            "idempotency_key": uuid::Uuid::now_v7().to_string(),
            "input": { "content": "hello without a robot or IM channel" }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{turn}");
    let durable_history = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let (status, history) = call(
                router.clone(),
                "GET",
                &format!("/api/agent-sessions/{conversation_id}/message-history?page_size=50"),
                json!({}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{history}");
            let items = history["data"]["items"].as_array().unwrap();
            if let Some(reply) = items
                .iter()
                .find(|message| message["content"]["content"] == REPLY)
                && items.iter().any(|message| {
                    message["type"] == "agent_status"
                        && message["content"]["turn_summary"] == true
                        && message["content"]["status"] == "prepared"
                        && message["msg_id"] == reply["message_id"]
                })
            {
                break history;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("Companion reply without a Robot or IM binding did not become durable");
    let history_items = durable_history["data"]["items"].as_array().unwrap();
    let reply = history_items
        .iter()
        .find(|message| message["content"]["content"] == REPLY)
        .expect("durable Companion reply");
    let turn_summary = history_items
        .iter()
        .find(|message| {
            message["type"] == "agent_status"
                && message["content"]["turn_summary"] == true
                && message["content"]["status"] == "prepared"
                && message["msg_id"] == reply["message_id"]
        })
        .expect("completed turn must expose one durable renderer summary");
    assert_eq!(turn_summary["position"], "center");
    assert_ne!(turn_summary["message_id"], turn_summary["msg_id"]);
    assert!(turn_summary["content"]["turn_id"].as_str().is_some());
    let conversation_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'conversations'")
        .fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(conversation_count, 0, "canonical Companion must not write the retired Conversation Store");
    let session_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions WHERE state = 'live'")
        .fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(session_count, 1, "concurrent ensure must create one canonical AgentSession");
    let (status, options) = call(router.clone(), "GET",
        &format!("/api/product-agent-bindings/companion/{companion_id}"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{options}");
    assert_eq!(options["data"]["selection"], json!({ "kind": "template", "template_key": "companion.default" }),
        "the implicit official choice must remain a template selection rather than a personal Agent");
    assert_eq!(options["data"]["options"].as_array().unwrap().len(), 1,
        "Companion must not expose generic Agent alternatives");
    let target_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_bindings WHERE target_kind = 'companion' AND target_id = ?")
        .bind(companion_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(target_count, 1);

    let (status, minimal) = call(router.clone(), "POST",
        "/api/agent-presets/from-template/chat.minimal", json!({
            "display_name": "chat.minimal", "reuse_existing": true,
            "model": { "provider_id": provider_id, "model": "step-3.7-flash" }
        })).await;
    assert_eq!(status, StatusCode::OK, "{minimal}");
    let minimal_id = minimal["data"]["preset"]["preset_id"].as_str().unwrap();
    let (status, selected) = call(router.clone(), "PUT",
        &format!("/api/product-agent-bindings/companion/{companion_id}"), json!({
            "preset_id": minimal_id
        })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{selected}");
    assert_eq!(selected["code"], "CAPABILITY_UNAVAILABLE_ON_PLATFORM");
    let (status, unchanged) = call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{conversation_id}/projection"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{unchanged}");
    assert_eq!(unchanged["data"]["preset_id"], preset_id);
    assert_eq!(unchanged["data"]["agent_snapshot"]["enabled_capabilities"], snapshot["enabled_capabilities"]);
    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "the chat turn must use exactly one model request");
    let model_request = requests[0].body_json::<Value>().unwrap();
    assert!(
        model_request["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|tool| !tool["function"]["name"]
                .as_str()
                .unwrap_or_default()
                .contains("robot")),
        "an unbound Robot must not reach the model tool surface: {model_request}"
    );
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn creative_agent_launches_without_enabled_chat_generation_provider_or_canvas() {
    const TRUST: &str = "creative-agent-no-model";
    async fn call(router: axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
        let response = router.oneshot(Request::builder().method("POST").uri(path)
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    let (router, services) = common::build_local_trust_app(TRUST).await;
    // The managed Provider is a permanent application service and its refresh
    // task may recreate a deleted projection. Disable it through its owner so
    // this fixture proves the professional route does not depend on any usable
    // Chat or media model supply without racing that background task.
    services.managed_model_service.set_free_enabled(false).await.unwrap();
    let enabled_provider_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM providers WHERE enabled = 1",
    )
    .fetch_one(services.database.pool())
    .await
    .unwrap();
    assert_eq!(
        enabled_provider_count, 0,
        "the fixture must not supply an enabled Chat or media provider"
    );
    let (status, created) = call(router.clone(), "/api/agent-presets/from-template/creative-studio.default", json!({
        "display_name":"Creative", "reuse_existing":true,
    })).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let editor = &created["data"];
    let document: nomifun_api_types::AgentPresetDocumentDto =
        serde_json::from_value(editor["revision"]["document"].clone()).expect("saved Creative preset document");
    assert!(document.chat_route_records.is_empty(), "professional presets must not bind a Chat route");
    assert!(document.model_route_refs.is_empty(), "professional presets must not require a model route");
    let preset_id = editor["preset"]["preset_id"].as_str().unwrap();
    let (status, launched) = call(router.clone(), "/api/agent-sessions", json!({
        "preset_id": preset_id, "title": "Creative without providers",
        "resource_selections":[
            {"resource_kind":"asset_library", "resource_id":"creative-studio-assets"},
            {"resource_kind":"process_session", "resource_id":"managed-process-session"},
            {"resource_kind":"project_memory", "resource_id":"default-project-memory"},
            {"resource_kind":"workspace", "resource_id":"default-workspace"}
        ],
    })).await;
    assert_eq!(status, StatusCode::OK, "{launched}");
    let session_id = launched["data"]["agent_session_id"].as_str().unwrap();
    let observed = router.clone().oneshot(Request::builder()
        .uri(format!("/api/agent-sessions/{session_id}"))
        .header("x-nomi-local-trust", TRUST)
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(observed.status(), StatusCode::OK);
    let observed: Value = serde_json::from_slice(&axum::body::to_bytes(
        observed.into_body(), 4 * 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(observed["data"]["session"]["agent_binding"], launched["data"]["agent_binding"]);
    let resources = launched["data"]["agent_binding"]["typed_resource_bindings"].as_array().unwrap();
    assert!(resources.iter().all(|binding| binding["resource_kind"] != "canvas"));
    assert!(resources.iter().any(|binding| binding["resource_kind"] == "asset_library"));
    assert!(resources.iter().any(|binding| binding["resource_kind"] == "workspace"));
    assert!(resources.iter().any(|binding| binding["resource_kind"] == "process_session"));
    assert!(resources.iter().any(|binding| binding["resource_kind"] == "project_memory"));
    let enabled_provider_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM providers WHERE enabled = 1",
    )
    .fetch_one(services.database.pool())
    .await
    .unwrap();
    assert_eq!(
        enabled_provider_count, 0,
        "professional launch must not enable a hidden provider fallback"
    );
    let capabilities = editor["revision"]["document"]["enabled_capabilities"].as_array().unwrap();
    for capability in [
        "agent.tool-discovery", "creation.media", "creative.workshop", "office",
        "project.memory", "web.research", "workspace.artifacts", "workspace.files",
        "workspace.process",
    ] {
        assert!(capabilities.iter().any(|entry| entry["capability"]["id"] == capability), "{editor}");
    }
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn official_agent_direct_launch_reuses_configuration_and_creates_sessions() {
    const TRUST: &str = "official-agent-direct-launch";
    async fn call(router: axum::Router, path: &str, body: Value) -> Value {
        let response = router.oneshot(Request::builder().method("POST").uri(path)
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(status, StatusCode::OK, "{path}: {value}");
        value["data"].clone()
    }
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let create = json!({ "display_name": "Minimal", "model_route_refs": {}, "chat_route_records": {}, "reuse_existing": true });
    let first = call(router.clone(), "/api/agent-presets/from-template/chat.minimal", create.clone()).await;
    let second = call(router.clone(), "/api/agent-presets/from-template/chat.minimal", create).await;
    assert_eq!(first["preset"]["preset_id"], second["preset"]["preset_id"]);
    assert_eq!(first["revision"]["reference"], second["revision"]["reference"]);
    let preset_id = first["preset"]["preset_id"].as_str().unwrap();
    let session_a = call(router.clone(), "/api/agent-sessions", json!({ "preset_id": preset_id, "title": "First conversation" })).await;
    let session_b = call(router.clone(), "/api/agent-sessions", json!({ "preset_id": preset_id, "title": "Second conversation" })).await;
    assert_ne!(session_a["agent_session_id"], session_b["agent_session_id"]);
    assert_eq!(session_a["agent_binding"], session_b["agent_binding"]);
    assert_eq!(session_a["agent_binding"]["preset_revision_ref"], first["revision"]["reference"]);
    let persisted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_preset_revisions WHERE preset_id = ?")
        .bind(preset_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(persisted, 1);
    let library_response = router.oneshot(Request::builder()
        .uri("/api/agent-preset-templates")
        .header("x-nomi-local-trust", TRUST)
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(library_response.status(), StatusCode::OK);
    let library: Value = serde_json::from_slice(&axum::body::to_bytes(
        library_response.into_body(), 4 * 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(library["data"]["user_presets"], json!([]),
        "using an official Agent must not create personal Agents");
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn official_agent_launch_reuses_current_configuration_and_opens_canonical_session() {
    const TRUST: &str = "current-official-agent-reuse";
    async fn call(router: axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
        let response = router.oneshot(Request::builder().method("POST").uri(path)
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let upstream = wiremock::MockServer::start().await;
    let (status, provider) = call(router.clone(), "/api/providers", json!({
        "platform": "stepfun-plan", "name": "StepFun launch regression",
        "base_url": format!("{}/step_plan/v1", upstream.uri()),
        "auth_scheme": "bearer", "credentials": { "api_keys": ["test-only"] },
        "enabled": true, "initial_model": {
            "model": "step-3.7-flash", "enabled": true,
            "capabilities": [{ "task": "chat", "traits": [],
                "protocol": "openai.chat_text", "connection_role": "default", "provider_params": {} }]
        }
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let path = "/api/agent-presets/from-template/chat.minimal";
    let request = json!({ "display_name": "Minimal", "reuse_existing": true,
        "model": { "provider_id": provider["data"]["provider_id"], "model": "step-3.7-flash" } });
    let (status, original) = call(router.clone(), path, request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{original}");
    let original_id = original["data"]["preset"]["preset_id"].as_str().unwrap();
    assert_eq!(original["data"]["revision"]["document"]["chat_route_records"]["agent_chat"]["primary"]["model"], "step-3.7-flash");

    let (status, reused) = call(router.clone(), path, request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{reused}");
    assert_eq!(reused["data"]["preset"]["preset_id"], original_id);
    let (status, session) = call(router.clone(), "/api/agent-sessions",
        json!({ "preset_id": original_id, "title": "你好", "model": request["model"] })).await;
    assert_eq!(status, StatusCode::OK, "current configuration must create a session: {session}");
    assert!(session["data"]["agent_session_id"].is_string());
    let session_id = session["data"]["agent_session_id"].as_str().unwrap();
    let observed = router.clone().oneshot(Request::builder()
        .uri(format!("/api/agent-sessions/{session_id}"))
        .header("x-nomi-local-trust", TRUST)
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(observed.status(), StatusCode::OK);
    let observed: Value = serde_json::from_slice(&axum::body::to_bytes(
        observed.into_body(), 4 * 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(observed["data"]["session"]["agent_binding"], session["data"]["agent_binding"]);
    assert_eq!(observed["data"]["session"]["metadata"]["title"], "你好");
    assert_eq!(observed["data"]["session"]["agent_binding"]["preset_revision_ref"]["preset_id"], original_id);
    assert_eq!(original["data"]["preset"]["display_name"], "Minimal");

    assert!(upstream.received_requests().await.unwrap().is_empty(),
        "Agent preparation must not execute the selected commercial model");

    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn agent_session_model_selection_is_exact_persistent_and_keeps_the_agent_unchanged() {
    const TRUST: &str = "agent-model-selection-test";
    async fn call(router: axum::Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let response = router.oneshot(Request::builder().method(method).uri(path)
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let (status, original) = call(router.clone(), "POST", "/api/agent-presets", json!({
        "display_name": "Personal model test", "document": {
            "schema_version": "1.0.0", "model_route_refs": {}, "chat_route_records": {},
            "enabled_capabilities": [{
                "capability": { "id": "workspace.files" },
                "action_allowlist": ["workspace.files/read", "workspace.files/search"]
            }],
            "skill_bindings": [], "system_role_provider_overrides": {},
            "persona": "Research helper", "instructions": "Keep my working rules", "starter_prompts": []
        }
    })).await;
    assert_eq!(status, StatusCode::OK, "{original}");
    let original = original["data"].clone();
    let preset_id = original["preset"]["preset_id"].as_str().unwrap();
    let alternate = original["revision"]["document"]["chat_route_records"]["agent_chat"]["failovers"][0].clone();
    assert!(alternate["model"].is_string());
    let selection = json!({ "provider_id": alternate["provider_id"], "model": alternate["model"] });
    let mut sessions = Vec::new();
    for _ in 0..2 {
        let (status, result) = call(router.clone(), "POST", "/api/agent-sessions", json!({
            "preset_id": preset_id, "title": "Chosen model", "model": selection,
            "resource_selections": [{ "resource_kind": "workspace", "resource_id": "default-workspace" }]
        })).await;
        assert_eq!(status, StatusCode::OK, "{result}");
        let id = result["data"]["agent_session_id"].as_str().unwrap();
        let (status, observation) = call(router.clone(), "GET", &format!("/api/agent-sessions/{id}"), json!({})).await;
        assert_eq!(status, StatusCode::OK);
        let binding = result["data"]["agent_binding"].clone();
        assert_eq!(observation["data"]["session"]["agent_binding"], binding);
        let (status, projection) = call(
            router.clone(),
            "GET",
            &format!("/api/agent-sessions/{id}/projection"),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{projection}");
        assert_eq!(projection["data"]["extra"]["custom_workspace"], false);
        assert_eq!(
            projection["data"]["extra"]["is_temporary_workspace"],
            true,
            "the default-workspace resource must survive projection as a Nomi-managed workpath"
        );
        if sessions.is_empty() {
            let original_workspace = projection["data"]["extra"]["workspace"].clone();
            let (status, enabled) = call(
                router.clone(),
                "POST",
                "/api/requirements/autowork",
                json!({
                    "kind": "conversation",
                    "target_id": id,
                    "enabled": true,
                    "tag": "workspace-projection-regression"
                }),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{enabled}");

            let (status, refreshed) = call(
                router.clone(),
                "GET",
                &format!("/api/agent-sessions/{id}/projection"),
                json!({}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{refreshed}");
            assert_eq!(
                refreshed["data"]["extra"]["workspace"],
                original_workspace,
                "enabling AutoWork must not replace the frozen Session workspace"
            );
            assert_eq!(refreshed["data"]["extra"]["custom_workspace"], false);
            assert_eq!(
                refreshed["data"]["extra"]["is_temporary_workspace"],
                true
            );

            let (status, disabled) = call(
                router.clone(),
                "POST",
                "/api/requirements/autowork",
                json!({
                    "kind": "conversation",
                    "target_id": id,
                    "enabled": false
                }),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{disabled}");
        }
        let variant = binding["preset_revision_ref"]["preset_id"].as_str().unwrap();
        let (status, editor) = call(router.clone(), "GET", &format!("/api/agent-presets/{variant}/editor"), json!({})).await;
        assert_eq!(status, StatusCode::OK);
        let record = &editor["data"]["revision"]["document"]["chat_route_records"]["agent_chat"];
        assert_eq!(record["primary"]["provider_id"], selection["provider_id"]);
        assert_eq!(record["primary"]["model"], selection["model"]);
        assert_eq!(record["failovers"], json!([]));
        for field in ["enabled_capabilities", "skill_bindings", "system_role_provider_overrides", "persona", "instructions"] {
            assert_eq!(editor["data"]["revision"]["document"][field], original["revision"]["document"][field], "must retain {field}");
        }
        sessions.push(binding);
    }
    assert_eq!(
        sessions[0]["typed_resource_bindings"],
        sessions[1]["typed_resource_bindings"],
        "same model resolves the same owner-scoped resources"
    );
    let (_, reloaded) = call(router.clone(), "GET", &format!("/api/agent-presets/{preset_id}/editor"), json!({})).await;
    assert_eq!(reloaded["data"]["revision"], original["revision"]);
    let (_, library) = call(router.clone(), "GET", "/api/agent-preset-templates", json!({})).await;
    assert_eq!(library["data"]["user_presets"].as_array().unwrap().len(), 1, "internal model variants stay out of the Agent library");
    let (status, official) = call(router.clone(), "POST", "/api/agent-presets/from-template/chat.minimal", json!({
        "display_name": "Official chosen model", "model_route_refs": {}, "chat_route_records": {},
        "model": selection, "reuse_existing": true
    })).await;
    assert_eq!(status, StatusCode::OK, "{official}");
    assert_eq!(official["data"]["revision"]["document"]["chat_route_records"]["agent_chat"]["primary"]["model"], selection["model"]);
    let (status, _) = call(router, "POST", "/api/agent-sessions", json!({
        "preset_id": preset_id, "model": { "provider_id": selection["provider_id"], "model": "nonexistent-model" }
    })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "an unknown model cannot silently fall back");
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn nomi_core_agent_settings_template_and_binding_surface_is_persistent() {
    let (router, services) = common::build_local_trust_app("agent-settings-local-trust").await;
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-presets/from-template/chat.minimal")
                .header("x-nomi-local-trust", "agent-settings-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "reuse_existing": false,
                        "display_name": "Nomi-core smoke preset",
                        "model_route_refs": {},
                        "chat_route_records": {}
                    }))
                    .expect("serialize template request"),
                ))
                .expect("build template request"),
        )
        .await
        .expect("dispatch template request");
    let response_status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read template response");
    let value: Value = serde_json::from_slice(&body).expect("template response JSON");
    assert_eq!(
        response_status,
        StatusCode::OK,
        "template request failed: {value}"
    );
    let preset_id = value["data"]["preset"]["preset_id"]
        .as_str()
        .expect("template response preset id")
        .to_owned();
    let persisted_payload: String = sqlx::query_scalar(
        "SELECT payload_json FROM agent_preset_revisions \
         WHERE preset_id = ? AND revision_no = 1",
    )
    .bind(&preset_id)
    .fetch_one(services.database.pool())
    .await
    .expect("persisted AgentPreset revision payload");
    let persisted_document: nomifun_api_types::AgentPresetDocumentDto =
        serde_json::from_str(&persisted_payload).expect("persisted payload JSON");
    let response_document: nomifun_api_types::AgentPresetDocumentDto =
        serde_json::from_value(value["data"]["revision"]["document"].clone())
            .expect("response revision document");
    assert_eq!(
        persisted_document, response_document,
        "persisted and returned revision payloads must be semantically identical after defaults"
    );
    assert_eq!(
        value["data"]["revision"]["reference"]["revision"],
        1,
        "template creation must persist its first immutable revision"
    );
    let route = &value["data"]["revision"]["document"]["chat_route_records"]["agent_chat"];
    assert_eq!(
        route["schema"],
        "nomifun.chat-route-record.v1",
        "the host must materialize a canonical Chat route record"
    );
    assert_eq!(
        route["primary"]["model"],
        "big-pickle",
        "the managed Chat catalog's stable first candidate should be selected"
    );
    assert!(
        route["failovers"].as_array().is_some_and(|items| !items.is_empty()),
        "the host route should expose the remaining enabled Chat models as failovers"
    );
    assert!(
        route["primary"]["credential_ref"]
            .as_str()
            .is_some_and(|value| value.starts_with("nomi-core-chat-credential-")),
        "the route must carry an opaque credential reference, never credential material"
    );
    assert!(
        route["primary"]["credential_ref"]
            .as_str()
            .is_some_and(|value| !value.contains("test-only")),
        "credential material must not cross the route record boundary"
    );
    let editor = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/agent-presets/{preset_id}/editor"))
                .header("x-nomi-local-trust", "agent-settings-local-trust")
                .body(Body::empty())
                .expect("build editor request"),
        )
        .await
        .expect("dispatch editor request");
    assert_eq!(editor.status(), StatusCode::OK);
    let editor_body = axum::body::to_bytes(editor.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read editor response");
    let editor_value: Value = serde_json::from_slice(&editor_body).expect("editor JSON");
    assert_eq!(
        editor_value["data"]["preset"]["preset_id"],
        preset_id,
        "editor must reload the same persisted Preset"
    );
    let binding_json = json!({
        "preset_revision_ref": { "preset_id": preset_id }
    })
    .to_string();
    sqlx::query(
        "INSERT INTO agent_bindings \
         (target_kind, target_id, agent_binding_json) \
         VALUES ('conversation', 'retirement-target', ?)",
    )
    .bind(&binding_json)
    .execute(services.database.pool())
    .await
    .expect("insert active AgentBinding");
    sqlx::query(
        "INSERT INTO remote_bindings \
         (remote_binding_id, owner_user_id, name, agent_binding_json, nomi_snapshot_json, \
          provenance_json, agent_binding_digest, binding_version, created_at, updated_at) \
         VALUES ('0190f5fe-7c00-7a00-8000-000000000099', ?, 'Retirement Remote', ?, \
                 '{}', '{}', ?, 1, 1, 1)",
    )
    .bind(services.authoritative_user_id.as_ref())
    .bind(&binding_json)
    .bind("d".repeat(64))
    .execute(services.database.pool())
    .await
    .expect("insert active RemoteBinding");

    let retired = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/agent-presets/{preset_id}"))
                .header("x-nomi-local-trust", "agent-settings-local-trust")
                .body(Body::empty())
                .expect("build Preset retirement request"),
        )
        .await
        .expect("dispatch Preset retirement request");
    assert_eq!(retired.status(), StatusCode::OK);
    let retired_at_ms: Option<i64> = sqlx::query_scalar(
        "SELECT retired_at_ms FROM agent_presets WHERE preset_id = ?",
    )
    .bind(&preset_id)
    .fetch_one(services.database.pool())
    .await
    .expect("retired AgentPreset row");
    assert!(retired_at_ms.is_some());
    let revision_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_preset_revisions WHERE preset_id = ?",
    )
    .bind(&preset_id)
    .fetch_one(services.database.pool())
    .await
    .expect("retained AgentPreset revisions");
    assert_eq!(revision_count, 1);
    let agent_binding_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_bindings")
            .fetch_one(services.database.pool())
            .await
            .expect("count active AgentBindings");
    let remote_binding_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM remote_bindings")
            .fetch_one(services.database.pool())
            .await
            .expect("count active RemoteBindings");
    assert_eq!(agent_binding_count, 0);
    assert_eq!(remote_binding_count, 0);

    let library = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/agent-preset-templates?source=official")
                .header("x-nomi-local-trust", "agent-settings-local-trust")
                .body(Body::empty())
                .expect("build post-retirement library request"),
        )
        .await
        .expect("dispatch post-retirement library request");
    assert_eq!(library.status(), StatusCode::OK);
    let library_body = axum::body::to_bytes(library.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read post-retirement library");
    let library_value: Value =
        serde_json::from_slice(&library_body).expect("post-retirement library JSON");
    assert!(
        library_value["data"]["user_presets"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );

    let retired_editor = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/agent-presets/{preset_id}/editor"))
                .header("x-nomi-local-trust", "agent-settings-local-trust")
                .body(Body::empty())
                .expect("build retired editor request"),
        )
        .await
        .expect("dispatch retired editor request");
    assert_eq!(retired_editor.status(), StatusCode::NOT_FOUND);
    let retired_editor_body =
        axum::body::to_bytes(retired_editor.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read retired editor response");
    let retired_editor_error: Value =
        serde_json::from_slice(&retired_editor_body).expect("retired editor error JSON");
    assert_eq!(retired_editor_error["code"], "AGENT_PRESET_NOT_FOUND");

    let retired_session = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-sessions")
                .header("x-nomi-local-trust", "agent-settings-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "preset_id": preset_id,
                        "title": "Must not start"
                    }))
                    .expect("serialize retired Session request"),
                ))
                .expect("build retired Session request"),
        )
        .await
        .expect("dispatch retired Session request");
    assert_eq!(retired_session.status(), StatusCode::NOT_FOUND);
    let retired_session_body =
        axum::body::to_bytes(retired_session.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read retired Session response");
    let retired_session_error: Value =
        serde_json::from_slice(&retired_session_body).expect("retired Session error JSON");
    assert_eq!(retired_session_error["code"], "AGENT_PRESET_NOT_FOUND");

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn nomi_core_agent_session_projects_saved_chat_binding_without_internal_inputs() {
    let (router, services) = common::build_local_trust_app("agent-session-local-trust").await;
    let template_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-presets/from-template/chat.minimal")
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "reuse_existing": false,
                        "display_name": "Nomi-core session smoke",
                        "model_route_refs": {},
                        "chat_route_records": {}
                    }))
                    .expect("serialize session template request"),
                ))
                .expect("build session template request"),
        )
        .await
        .expect("dispatch session template request");
    assert_eq!(template_response.status(), StatusCode::OK);
    let template_body = axum::body::to_bytes(template_response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read session template response");
    let template: Value = serde_json::from_slice(&template_body).expect("session template JSON");
    let preset_id = template["data"]["preset"]["preset_id"]
        .as_str()
        .expect("session preset id");
    let revision = template["data"]["revision"]["reference"].clone();
    let draft = template["data"]["draft"].clone();

    let saved_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent-presets/{preset_id}/revisions"))
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "expected_current_revision": revision,
                        "draft": draft,
                    }))
                    .expect("serialize clean revision save request"),
                ))
                .expect("build clean revision save request"),
        )
        .await
        .expect("dispatch clean revision save request");
    assert_eq!(saved_response.status(), StatusCode::OK);
    let saved_body = axum::body::to_bytes(saved_response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read clean revision save response");
    let saved: Value = serde_json::from_slice(&saved_body).expect("clean revision save JSON");
    let expected_snapshot = saved["data"]["resolved_snapshot_ref"].clone();

    let rejected = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-sessions")
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "preset_id": preset_id,
                        "agent_binding": {
                            "preset_revision_ref": revision,
                            "resolved_snapshot_ref": expected_snapshot,
                            "typed_resource_bindings": [],
                            "binding_version": 1
                        }
                    }))
                    .expect("serialize rejected session request"),
                ))
                .expect("build rejected session request"),
        )
        .await
        .expect("dispatch rejected session request");
    assert_eq!(
        rejected.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "deny_unknown_fields must reject the removed agent_binding input"
    );

    let session_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-sessions")
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .header("content-type", "application/json")
                .header("idempotency-key", "agent-session-create-smoke")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "preset_id": preset_id,
                        "title": "Session smoke"
                    }))
                    .expect("serialize session create request"),
                ))
                .expect("build session create request"),
        )
        .await
        .expect("dispatch session create request");
    let session_status = session_response.status();
    let session_body = axum::body::to_bytes(session_response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read session response");
    let session: Value = serde_json::from_slice(&session_body).expect("session JSON");
    assert_eq!(
        session_status,
        StatusCode::OK,
        "saved Chat binding should project into Nomi-core Session: {session}"
    );
    let session_id = session["data"]["agent_session_id"]
        .as_str()
        .expect("session id");
    assert_eq!(session["data"]["state"], "ready");
    assert_eq!(session["data"]["agent_binding"]["binding_version"], 1);

    let get_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/agent-sessions/{session_id}"))
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .body(Body::empty())
                .expect("build session get request"),
        )
        .await
        .expect("dispatch session get request");
    assert_eq!(get_response.status(), StatusCode::OK);
    let get_body = axum::body::to_bytes(get_response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read session get response");
    let observation: Value = serde_json::from_slice(&get_body).expect("session observation JSON");
    assert_eq!(
        observation["data"]["session"]["owner_ref"]["principal_id"],
        services.authoritative_user_id.as_ref()
    );
    assert_eq!(
        observation["data"]["session"]["agent_binding"]["resolved_snapshot_ref"],
        expected_snapshot
    );

    let fork_body = json!({
        "target_agent_binding": session["data"]["agent_binding"],
        "parent_through_seq": 0,
        "title": "Session smoke child"
    });
    let fork_once = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent-sessions/{session_id}/forks"))
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .header("content-type", "application/json")
                .header("idempotency-key", "agent-session-fork-smoke")
                .body(Body::from(serde_json::to_vec(&fork_body).unwrap()))
                .expect("build Session fork request"),
        )
        .await
        .expect("dispatch Session fork request");
    assert_eq!(fork_once.status(), StatusCode::OK);
    let fork_once: Value = serde_json::from_slice(
        &axum::body::to_bytes(fork_once.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read Session fork response"),
    )
    .expect("Session fork JSON");
    let child_session_id = fork_once["data"]["child_agent_session_id"]
        .as_str()
        .expect("child Session id");
    assert_ne!(child_session_id, session_id);
    assert_eq!(fork_once["data"]["parent_agent_session_id"], session_id);
    assert_eq!(fork_once["data"]["child_base_is_self_contained"], true);
    assert_eq!(fork_once["data"]["migrates_runtime_private_handles"], false);
    assert_eq!(fork_once["data"]["replays_tool_or_effect"], false);

    let child_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/agent-sessions/{child_session_id}"))
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .body(Body::empty())
                .expect("build child Session observation request"),
        )
        .await
        .expect("dispatch child Session observation request");
    assert_eq!(child_response.status(), StatusCode::OK);
    let child: Value = serde_json::from_slice(
        &axum::body::to_bytes(child_response.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read child Session observation"),
    )
    .expect("child Session observation JSON");
    assert_eq!(
        child["data"]["session"]["parent_session_id"],
        session_id,
        "the child must record the exact host-owned parent"
    );
    assert!(
        child["data"]["session"]["fork_base_payload_id"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );

    let delete_child = || {
        Request::builder()
            .method("DELETE")
            .uri(format!("/api/agent-sessions/{child_session_id}"))
            .header("x-nomi-local-trust", "agent-session-local-trust")
            .header("idempotency-key", "agent-session-child-delete-smoke")
            .body(Body::empty())
            .expect("build child Session delete request")
    };
    let deleted_child = router
        .clone()
        .oneshot(delete_child())
        .await
        .expect("delete child Session");
    assert_eq!(deleted_child.status(), StatusCode::OK);
    let deleted_child: Value = serde_json::from_slice(
        &axum::body::to_bytes(deleted_child.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read child Session delete response"),
    )
    .expect("child Session delete JSON");
    assert_eq!(deleted_child["data"]["state"], "deleted");
    let replayed_child = router
        .clone()
        .oneshot(delete_child())
        .await
        .expect("replay child Session delete");
    assert_eq!(replayed_child.status(), StatusCode::OK);
    let replayed_child: Value = serde_json::from_slice(
        &axum::body::to_bytes(replayed_child.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read replayed child Session delete response"),
    )
    .expect("replayed child Session delete JSON");
    assert_eq!(
        replayed_child["data"]["deleted_at"],
        deleted_child["data"]["deleted_at"],
        "delete replay must return the original tombstone"
    );

    let retired = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/agent-presets/{preset_id}"))
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .body(Body::empty())
                .expect("build post-Session Preset retirement request"),
        )
        .await
        .expect("dispatch post-Session Preset retirement request");
    assert_eq!(retired.status(), StatusCode::OK);

    let capabilities = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/agent-sessions/{session_id}/capabilities"))
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .body(Body::empty())
                .expect("build retired-Preset Session capability request"),
        )
        .await
        .expect("dispatch retired-Preset Session capability request");
    assert_eq!(
        capabilities.status(),
        StatusCode::OK,
        "an existing Session must keep its frozen capability view after Preset retirement"
    );
    let capabilities_body =
        axum::body::to_bytes(capabilities.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read retired-Preset Session capabilities");
    let capabilities_value: Value =
        serde_json::from_slice(&capabilities_body).expect("Session capabilities JSON");
    assert_eq!(
        capabilities_value["data"]["resolved_snapshot_ref"],
        expected_snapshot
    );

    let historical = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/agent-sessions/{session_id}"))
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .body(Body::empty())
                .expect("build retired-Preset historical Session request"),
        )
        .await
        .expect("dispatch retired-Preset historical Session request");
    assert_eq!(historical.status(), StatusCode::OK);

    let new_session = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-sessions")
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "preset_id": preset_id,
                        "title": "Retired Preset must not launch"
                    }))
                    .expect("serialize retired Preset Session request"),
                ))
                .expect("build retired Preset Session request"),
        )
        .await
        .expect("dispatch retired Preset Session request");
    assert_eq!(new_session.status(), StatusCode::NOT_FOUND);

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn nomi_core_remote_replays_frozen_binding_and_persists_event_cursor() {
    let (router, services) = common::build_local_trust_app("remote-local-trust").await;
    let create_preset = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-presets/from-template/chat.minimal")
                .header("x-nomi-local-trust", "remote-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "reuse_existing": false,
                        "display_name": "Nomi-core remote smoke",
                        "model_route_refs": {},
                        "chat_route_records": {}
                    }))
                    .expect("serialize remote preset request"),
                ))
                .expect("build remote preset request"),
        )
        .await
        .expect("dispatch remote preset request");
    assert_eq!(create_preset.status(), StatusCode::OK);
    let preset_body = axum::body::to_bytes(create_preset.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read remote preset response");
    let preset: Value = serde_json::from_slice(&preset_body).expect("remote preset JSON");
    // A clean save reuses the immutable Revision and returns the exact
    // Snapshot reference used by RemoteBinding.
    let preset_id = preset["data"]["preset"]["preset_id"]
        .as_str()
        .expect("remote preset id");
    let revision = preset["data"]["revision"]["reference"].clone();
    let editor_draft = preset["data"]["draft"].clone();
    let saved = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent-presets/{preset_id}/revisions"))
                .header("x-nomi-local-trust", "remote-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "expected_current_revision": revision,
                        "draft": editor_draft,
                    }))
                    .expect("serialize clean remote revision save request"),
                ))
                .expect("build clean remote revision save request"),
        )
        .await
        .expect("dispatch clean remote revision save request");
    assert_eq!(saved.status(), StatusCode::OK);
    let saved_body = axum::body::to_bytes(saved.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read clean remote revision save response");
    let saved: Value = serde_json::from_slice(&saved_body).expect("clean remote revision save JSON");
    let binding = serde_json::json!({
        "preset_revision_ref": revision,
        "resolved_snapshot_ref": saved["data"]["resolved_snapshot_ref"],
        "typed_resource_bindings": [],
        "binding_version": 1
    });

    let create_binding = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/remote-bindings")
                .header("x-nomi-local-trust", "remote-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "name": "Nomi-core remote smoke binding",
                        "agent_binding": binding
                    }))
                    .expect("serialize remote binding request"),
                ))
                .expect("build remote binding request"),
        )
        .await
        .expect("dispatch remote binding request");
    assert_eq!(create_binding.status(), StatusCode::OK);
    let binding_body = axum::body::to_bytes(create_binding.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read remote binding response");
    let binding_response: Value =
        serde_json::from_slice(&binding_body).expect("remote binding JSON");
    let binding_id = binding_response["data"]["remote_binding_id"]
        .as_str()
        .expect("remote binding id")
        .to_owned();

    let open_request = || {
        Request::builder()
            .method("POST")
            .uri("/api/remote/open")
            .header("x-nomi-local-trust", "remote-local-trust")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "binding_id": binding_id,
                    "idempotency_key": "remote-open-smoke"
                }))
                .expect("serialize remote open request"),
            ))
            .expect("build remote open request")
    };
    let open = router.clone().oneshot(open_request()).await.expect("open remote");
    let open_status = open.status();
    let open_body = axum::body::to_bytes(open.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read remote open response");
    let opened: Value = serde_json::from_slice(&open_body).expect("remote open JSON");
    assert_eq!(open_status, StatusCode::OK, "{opened}");
    assert_eq!(opened["open_state"], json!({ "state": "ready" }));
    assert_eq!(
        opened["cursor"]["seq"],
        2,
        "Remote open cursor must be the Remote event cursor, not the Conversation message cursor"
    );
    let session_id = opened["agent_session_id"]
        .as_str()
        .expect("remote session id")
        .to_owned();

    let replay = router.clone().oneshot(open_request()).await.expect("replay remote open");
    assert_eq!(replay.status(), StatusCode::OK);
    let replay_body = axum::body::to_bytes(replay.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read replay response");
    let replayed: Value = serde_json::from_slice(&replay_body).expect("replay JSON");
    assert_eq!(replayed["agent_session_id"], session_id);
    assert_eq!(replayed["agent_binding"], opened["agent_binding"]);
    assert_eq!(
        replayed["cursor"]["seq"],
        opened["cursor"]["seq"],
        "idempotent Remote open must replay the same event cursor"
    );

    let observe = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/remote/observe?agent_session_id={session_id}&after_seq=0&limit=100"
                ))
                .header("x-nomi-local-trust", "remote-local-trust")
                .body(Body::empty())
                .expect("build remote observe request"),
        )
        .await
        .expect("observe remote");
    assert_eq!(observe.status(), StatusCode::OK);
    let observe_body = axum::body::to_bytes(observe.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read remote observe response");
    let observed: Value = serde_json::from_slice(&observe_body).expect("remote observe JSON");
    let events = observed["events"].as_array().expect("remote event array");
    assert!(events.iter().any(|event| event["kind"] == "session/opening"));
    assert!(events.iter().any(|event| event["kind"] == "session/ready"));
    let cursor = observed["next_cursor"]["seq"]
        .as_u64()
        .expect("remote next cursor");
    assert!(cursor >= 2);

    // Deleting the mutable binding does not invalidate the already-open
    // Session. Replaying the same open key must use the frozen projection.
    let delete_binding = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/remote-bindings/{binding_id}"))
                .header("x-nomi-local-trust", "remote-local-trust")
                .body(Body::empty())
                .expect("build remote binding delete request"),
        )
        .await
        .expect("delete remote binding");
    assert_eq!(delete_binding.status(), StatusCode::OK);
    let replay_after_delete = router
        .clone()
        .oneshot(open_request())
        .await
        .expect("replay remote open after binding delete");
    assert_eq!(replay_after_delete.status(), StatusCode::OK);
    let replay_after_delete_body =
        axum::body::to_bytes(replay_after_delete.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read replay after delete response");
    let replay_after_delete: Value =
        serde_json::from_slice(&replay_after_delete_body).expect("replay after delete JSON");
    assert_eq!(replay_after_delete["agent_session_id"], session_id);

    let cancel = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/remote/cancel")
                .header("x-nomi-local-trust", "remote-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "agent_session_id": session_id,
                        "idempotency_key": "remote-cancel-smoke"
                    }))
                    .expect("serialize remote cancel request"),
                ))
                .expect("build remote cancel request"),
        )
        .await
        .expect("cancel remote");
    assert_eq!(cancel.status(), StatusCode::OK);
    let cancel_body = axum::body::to_bytes(cancel.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read remote cancel response");
    let cancelled: Value = serde_json::from_slice(&cancel_body).expect("remote cancel JSON");
    assert_eq!(cancelled["session_status"], "cancelled");
    assert!(
        cancelled["cursor"]["seq"].as_u64().is_some_and(|seq| seq >= 4),
        "Remote cancel cursor must include both cancellation facts"
    );

    // Replaying the same cancel key is an absorbing operation. It must not
    // issue another runtime command or append a second terminal fact.
    let cancel_replay = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/remote/cancel")
                .header("x-nomi-local-trust", "remote-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "agent_session_id": session_id,
                        "idempotency_key": "remote-cancel-smoke"
                    }))
                    .expect("serialize cancel replay request"),
                ))
                .expect("build cancel replay request"),
        )
        .await
        .expect("replay remote cancel");
    assert_eq!(cancel_replay.status(), StatusCode::OK);
    let cancel_replay_body =
        axum::body::to_bytes(cancel_replay.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read cancel replay response");
    let cancel_replayed: Value =
        serde_json::from_slice(&cancel_replay_body).expect("cancel replay JSON");
    assert_eq!(cancel_replayed["session_status"], "cancelled");

    let turn_after_cancel = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/remote/turn")
                .header("x-nomi-local-trust", "remote-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "agent_session_id": session_id,
                        "input": {"content": "must not run"},
                        "idempotency_key": "remote-turn-after-cancel"
                    }))
                    .expect("serialize post-cancel turn request"),
                ))
                .expect("build post-cancel turn request"),
    )
    .await
    .expect("dispatch post-cancel turn");
    assert_eq!(turn_after_cancel.status(), StatusCode::CONFLICT);

    let deleted = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/agent-sessions/{session_id}"))
                .header("x-nomi-local-trust", "remote-local-trust")
                .header("idempotency-key", "remote-session-delete-smoke")
                .body(Body::empty())
                .expect("build canonical Session delete request"),
        )
        .await
        .expect("delete canonical Remote Session");
    assert_eq!(deleted.status(), StatusCode::OK);
    let post_delete = router
        .clone()
        .oneshot(open_request())
        .await
        .expect("replay Remote open after Session delete");
    assert_ne!(post_delete.status(), StatusCode::OK, "deleted Session must not resurrect");
    let live_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_sessions WHERE agent_session_id = ? AND state = 'live'",
    )
    .bind(&session_id)
    .fetch_one(services.database.pool())
    .await
    .unwrap();
    assert_eq!(live_rows, 0);

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn nomi_core_remote_accepts_installation_bearer_without_local_trust() {
    let trust_secret = "remote-installation-token-local-trust";
    let (router, services) = common::build_local_trust_app(trust_secret).await;

    let missing = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/remote/observe?agent_session_id=0190f5fe-7c00-7a00-8000-000000000001")
                .body(Body::empty())
                .expect("build unauthenticated Remote request"),
        )
        .await
        .expect("dispatch unauthenticated Remote request");
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
    let missing_body = axum::body::to_bytes(missing.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read unauthenticated Remote response");
    let missing_value: Value =
        serde_json::from_slice(&missing_body).expect("unauthenticated Remote JSON");
    assert_eq!(missing_value["code"], "REMOTE_AUTH_REQUIRED");

    let minted = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webui/access-token")
                .header("x-nomi-local-trust", trust_secret)
                .body(Body::empty())
                .expect("build installation token mint request"),
        )
        .await
        .expect("dispatch installation token mint request");
    assert_eq!(minted.status(), StatusCode::OK);
    let minted_body = axum::body::to_bytes(minted.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read installation token response");
    let minted_value: Value =
        serde_json::from_slice(&minted_body).expect("installation token JSON");
    let token = minted_value["data"]["token"]
        .as_str()
        .expect("installation token must be returned once")
        .to_owned();

    let authenticated = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/remote/observe?agent_session_id=0190f5fe-7c00-7a00-8000-000000000001")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .expect("build installation-token Remote request"),
        )
        .await
        .expect("dispatch installation-token Remote request");
    assert_eq!(
        authenticated.status(),
        StatusCode::NOT_FOUND,
        "valid installation token must reach the owner-scoped Remote handler"
    );
    let authenticated_body =
        axum::body::to_bytes(authenticated.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read authenticated Remote response");
    let authenticated_value: Value =
        serde_json::from_slice(&authenticated_body).expect("authenticated Remote JSON");
    assert_eq!(authenticated_value["code"], "REMOTE_SESSION_NOT_FOUND");

    let revoked = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/webui/access-token")
                .header("x-nomi-local-trust", trust_secret)
                .body(Body::empty())
                .expect("build installation token revoke request"),
        )
        .await
        .expect("dispatch installation token revoke request");
    assert_eq!(revoked.status(), StatusCode::OK);
    let revoked_request = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/remote/observe?agent_session_id=0190f5fe-7c00-7a00-8000-000000000001")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .expect("build revoked-token Remote request"),
        )
        .await
        .expect("dispatch revoked-token Remote request");
    assert_eq!(revoked_request.status(), StatusCode::UNAUTHORIZED);
    let revoked_body = axum::body::to_bytes(revoked_request.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read revoked-token Remote response");
    let revoked_value: Value =
        serde_json::from_slice(&revoked_body).expect("revoked-token Remote JSON");
    assert_eq!(revoked_value["code"], "REMOTE_AUTH_REQUIRED");

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

async fn mcp_response(
    router: &axum::Router,
    token: &str,
    session_id: Option<&str>,
    request: Value,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("host", "127.0.0.1")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    if let Some(session_id) = session_id {
        builder = builder
            .header("mcp-session-id", session_id)
            .header("mcp-protocol-version", "2025-06-18");
    }
    let response = router
        .clone()
        .oneshot(
            builder
                .body(Body::from(
                    serde_json::to_vec(&request).expect("serialize MCP request"),
                ))
                .expect("build MCP request"),
        )
        .await
        .expect("dispatch MCP request");
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read MCP response body");
    (status, headers, body.to_vec())
}

fn parse_mcp_response(body: &[u8]) -> Value {
    if let Ok(value) = serde_json::from_slice(body) {
        return value;
    }
    let text = String::from_utf8_lossy(body);
    let data = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim))
        .find(|line| !line.is_empty())
        .unwrap_or_else(|| panic!("MCP response must contain a JSON data event: {text:?}"));
    serde_json::from_str(data).expect("MCP data event must be JSON")
}

fn mcp_tool_error_code(response: &Value) -> String {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("MCP tool result must contain text");
    let text = text.strip_prefix("Error: ").unwrap_or(text);
    let payload: Value = serde_json::from_str(text)
        .unwrap_or_else(|error| panic!("MCP tool text must be JSON ({error}): {text:?}"));
    payload
        .pointer("/error/code")
        .or_else(|| payload.get("code"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("MCP tool error must contain a typed code: {response:?}"))
        .to_owned()
}

#[tokio::test]
async fn nomi_core_remote_mcp_uses_installation_auth_and_nomi_core_operations() {
    let trust_secret = "remote-mcp-local-trust";
    let (router, services) = common::build_local_trust_app(trust_secret).await;

    let minted = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webui/access-token")
                .header("x-nomi-local-trust", trust_secret)
                .body(Body::empty())
                .expect("build MCP token mint request"),
        )
        .await
        .expect("dispatch MCP token mint request");
    assert_eq!(minted.status(), StatusCode::OK);
    let minted_body = axum::body::to_bytes(minted.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read MCP token response");
    let token = serde_json::from_slice::<Value>(&minted_body)
        .expect("MCP token response JSON")["data"]["token"]
        .as_str()
        .expect("MCP token")
        .to_owned();

    let (status, _, unauthorized_body) = mcp_response(
        &router,
        "not-the-installation-token",
        None,
        json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "nomi-core-route-gap", "version": "1"}
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let unauthorized = serde_json::from_slice::<Value>(&unauthorized_body)
        .expect("unauthorized MCP JSON");
    assert_eq!(unauthorized["code"], "REMOTE_AUTH_REQUIRED");

    let (status, headers, body) = mcp_response(
        &router,
        &token,
        None,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "nomi-core-route-gap", "version": "1"}
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session_id = headers
        .get("mcp-session-id")
        .expect("MCP initialize must pin a transport session")
        .to_str()
        .expect("MCP session id must be valid UTF-8")
        .to_owned();
    assert!(parse_mcp_response(&body)["result"].is_object());

    let (status, _, _) = mcp_response(
        &router,
        &token,
        Some(&session_id),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (status, _, body) = mcp_response(
        &router,
        &token,
        Some(&session_id),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let list = parse_mcp_response(&body);
    let names = list["result"]["tools"]
        .as_array()
        .expect("MCP tools/list array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("MCP tool name"))
        .collect::<Vec<_>>();
    assert_eq!(names, ["open", "turn", "observe", "cancel"]);

    let session = "0190f5fe-7c00-7a00-8000-000000000001";
    let calls = [
        (
            "open",
            json!({
                "binding_id": "missing-binding",
                "idempotency_key": "mcp-open-gap"
            }),
            "REMOTE_BINDING_NOT_FOUND",
        ),
        (
            "turn",
            json!({
                "agent_session_id": session,
                "input": {"content": "unused"},
                "idempotency_key": "mcp-turn-gap"
            }),
            "REMOTE_SESSION_NOT_FOUND",
        ),
        (
            "observe",
            json!({
                "agent_session_id": session,
                "after_cursor": {"agent_session_id": session, "seq": 0},
                "limit": 1
            }),
            "REMOTE_SESSION_NOT_FOUND",
        ),
        (
            "cancel",
            json!({
                "agent_session_id": session,
                "idempotency_key": "mcp-cancel-gap"
            }),
            "REMOTE_SESSION_NOT_FOUND",
        ),
    ];
    for (index, (name, arguments, expected_code)) in calls.into_iter().enumerate() {
        let (status, _, body) = mcp_response(
            &router,
            &token,
            Some(&session_id),
            json!({
                "jsonrpc": "2.0",
                "id": index + 3,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments}
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "MCP tool call {name}");
        assert_eq!(mcp_tool_error_code(&parse_mcp_response(&body)), expected_code, "{name}");
    }

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}
