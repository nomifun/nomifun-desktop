//! Route-topology regression guard for the current Nomi-core composition.
//!
//! Route-topology and smoke guards for the current Nomi-core composition.
//!
//! The default router must expose the app-local Agent Settings/AgentSession/
//! Remote adapter while keeping the Fresh-v4/Codex router out of the product
//! path.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use nomifun_app::{AppConfig, compatibility::{AppServices, create_router}};
use nomifun_auth::AuthPolicy;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use serde_json::{Value, json};
use tower::ServiceExt;

/// Keep authentication and mutation idempotency explicit at each call site.
struct LocalJsonClient<'a>(&'a str);

impl LocalJsonClient<'_> {
    async fn call(
        &self,
        router: axum::Router,
        method: &str,
        path: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        self.request(router, method, path, body, None).await
    }

    async fn call_with_idempotency(
        &self,
        router: axum::Router,
        method: &str,
        path: &str,
        body: Value,
        key: &str,
    ) -> (StatusCode, Value) {
        self.request(router, method, path, body, Some(key)).await
    }

    async fn request(
        &self,
        router: axum::Router,
        method: &str,
        path: &str,
        body: Value,
        key: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(path)
            .header("x-nomi-local-trust", self.0)
            .header("content-type", "application/json");
        if let Some(key) = key {
            request = request.header("idempotency-key", key);
        }
        let response = router.oneshot(request
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        let value = if bytes.is_empty() { Value::Null } else {
            serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                if status.is_client_error() {
                    // Axum rejects malformed query extractors before the JSON
                    // application error layer; cursor fences assert this text.
                    Value::String(String::from_utf8_lossy(&bytes).into_owned())
                } else {
                    panic!("{method} {path} returned {status}: {error}; {}", String::from_utf8_lossy(&bytes))
                }
            })
        };
        (status, value)
    }
}

fn repo_file(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(relative)
        .canonicalize()
        .unwrap_or_else(|error| panic!("cannot resolve {relative}: {error}"));
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

async fn create_chat_provider(
    router: &axum::Router,
    trust_secret: &str,
    name: &str,
    model: &str,
    sort_order: i64,
) -> Value {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/providers")
                .header("x-nomi-local-trust", trust_secret)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "platform": "openai",
                        "name": name,
                        "base_url": "https://example.invalid/v1",
                        "auth_scheme": "bearer",
                        "credentials": { "api_keys": ["test-only"] },
                        "enabled": true,
                        "sort_order": sort_order,
                        "initial_model": {
                            "model": model,
                            "enabled": true,
                            "capabilities": [{
                                "task": "chat",
                                "traits": [],
                                "protocol": "openai.chat_text",
                                "connection_role": "default",
                                "provider_params": {}
                            }]
                        }
                    }))
                    .expect("serialize provider fixture"),
                ))
                .expect("build provider fixture request"),
        )
        .await
        .expect("dispatch provider fixture request");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read provider fixture response");
    let value: Value = serde_json::from_slice(&bytes).expect("provider fixture JSON");
    assert_eq!(status, StatusCode::CREATED, "{value}");
    value["data"].clone()
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
        "/api/agent-sessions/{agent_session_id}/model",
        "/api/agent-sessions/{agent_session_id}/agent-switch/preview",
        "/api/agent-sessions/{agent_session_id}/agent",
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
    let recovery = repo_file("src/router/native_turn_recovery.rs");

    // V2 schedules fenced recovery instead of converting every orphan into
    // failed. Behavioral crash/pause/quarantine acceptance lives in the
    // native_execution_recovery product fixture.
    let installed = state.find("conversation_owner.install_official_runtime(").unwrap();
    let scheduled = state.find("reconcile_orphaned_active_turns(engine_sessions).await?").unwrap();
    assert!(installed < scheduled, "the actual Runtime must be installed before recovery is scheduled");
    assert!(sessions.contains("self.schedule_native_recovery(engine_sessions).await"));
    assert!(recovery.contains("h.status IN ('running','reconciliation') AND t.state='running'"));
    assert!(recovery.contains("quarantine_native_recovery"));
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
    let client = LocalJsonClient(TRUST);
    const TRUST: &str = "canonical-turn-dispatch";

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
    let (status, provider) = client.call(
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
    let (status, preset) = client.call(
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
    let (status, session) = client.call(
        router.clone(),
        "POST",
        "/api/agent-sessions",
        json!({ "preset_id": preset_id, "model": model, "title": "Canonical" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let session_id = session["data"]["agent_session_id"].as_str().unwrap();
    let key = uuid::Uuid::now_v7().to_string();
    let (status, turn) = client.call(
        router.clone(),
        "POST",
        &format!("/api/agent-sessions/{session_id}/turns"),
        json!({ "idempotency_key": key, "input": { "content": "reply once" } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{turn}");
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let (status, history) = client.call(
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
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while canonical_store.head(&session_key).await.unwrap().status == "running" {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("Turn terminal did not settle");
    let checkpoints: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_events WHERE session_id = ? AND kind = 'runtime/progress-recorded' \
         AND json_extract(inline_json, '$.event.event') = 'execution_checkpoint_saved'",
    ).bind(session_id).fetch_one(services.database.pool()).await.unwrap();
    assert!(checkpoints >= 1, "the real Runtime/Host path must commit a checkpoint before model execution");
    let retained: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_turns WHERE session_id = ? AND native_checkpoint_json IS NOT NULL",
    ).bind(session_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(retained, 0, "completed turns must release their active checkpoint payload");
    let before_rebuild = canonical_store.head(&session_key).await.unwrap();
    let rebuilt = canonical_store.rebuild_projections(&session_key).await.unwrap();
    assert_eq!(rebuilt, before_rebuild, "projection rebuild must be deterministic");
    let (status, listed) = client.call(
        router.clone(),
        "GET",
        "/api/agent-sessions?limit=50",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(listed["data"]["items"].as_array().unwrap().iter()
        .any(|item| item["conversation_id"] == session_id));
    let (status, updated) = client.call(
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
    let (status, deleted) = client.call(
        router.clone(),
        "DELETE",
        &format!("/api/agent-sessions/{session_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    let (status, _) = client.call(
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
        let (status, _) = client.call(router.clone(), method, path, json!({})).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "retired route survived: {method} {path}");
    }
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn message_history_http_cursor_preserves_tool_details_and_crosses_filtered_pages() {
    let client = LocalJsonClient(TRUST);
    const TRUST:&str = "settlement-history-cursor";
    let (router,services)=common::build_local_trust_app(TRUST).await;
    let provider=create_chat_provider(&router,TRUST,"Settlement history","step-3.7-flash",1).await;
    let model=json!({"provider_id":provider["provider_id"],"model":"step-3.7-flash"});
    let (status,preset)=client.call(router.clone(),"POST","/api/agent-presets/from-template/chat.minimal",json!({
        "display_name":"Settlement history","reuse_existing":true,"model_route_refs":{},"chat_route_records":{},"model":model
    })).await;
    assert_eq!(status,StatusCode::OK,"{preset}");
    let preset_id=preset["data"]["preset"]["preset_id"].as_str().unwrap();
    let (status,session)=client.call(router.clone(),"POST","/api/agent-sessions",json!({
        "preset_id":preset_id,"model":model,"title":"Settlement history"
    })).await;
    assert_eq!(status,StatusCode::OK,"{session}");
    let session_id=session["data"]["agent_session_id"].as_str().unwrap().to_owned();
    let typed_session=nomifun_agent_contracts::AgentSessionId::from(session_id.clone());
    let store=nomifun_agent_session::AgentSessionStore::from_pool(
        services.database.pool().clone(),
    ).await.unwrap();
    let (input,turn)=store.start_turn(
        &typed_session,
        nomifun_agent_contracts::EventProducerId::from("session_api"),
        nomifun_agent_contracts::IdempotencyKey::from("settlement-http-turn"),
        nomifun_agent_contracts::OperationId::from("settlement-http-turn"),
        nomifun_agent_contracts::StrictJsonValue(json!({"content":"exercise settlement projection"})),
    ).await.unwrap();
    let root_message_id=input.record.unwrap().event_id.as_ref().to_owned();
    let turn_started=turn.record.unwrap().event_id;
    let projection_id=uuid::Uuid::now_v7().to_string();
    let tool=nomifun_agent_contracts::SessionEventAppend {
        agent_session_id:typed_session.clone(),
        event_id:nomifun_agent_contracts::EventId::from("settlement-http-tool-call"),
        producer_id:nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
        idempotency_key:nomifun_agent_contracts::IdempotencyKey::from("settlement-http-tool-call"),
        semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {
            kind:nomifun_agent_contracts::SessionEventKind("tool/call-started".to_owned()),
            kind_version:1,
            correlation_id:nomifun_agent_contracts::CorrelationId::from(projection_id.clone()),
            causation_event_id:Some(turn_started.clone()),
            payload:nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                nomifun_agent_contracts::StrictJsonValue(json!({
                    "operation_id":"settlement-http-operation","call_id":"settlement-http-call",
                    "capability_id":"workspace.files","action_id":"workspace.files/write","name":"write_file"
                })),
            ),
        },
    };
    let tool_ack=store.append_event(&tool).await.unwrap().ack.unwrap();
    let result=nomifun_agent_contracts::SessionEventAppend {
        agent_session_id:typed_session.clone(),
        event_id:nomifun_agent_contracts::EventId::from("settlement-http-tool-result"),
        producer_id:nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
        idempotency_key:nomifun_agent_contracts::IdempotencyKey::from("settlement-http-tool-result"),
        semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {
            kind:nomifun_agent_contracts::SessionEventKind("tool/result-recorded".to_owned()),
            kind_version:1,
            correlation_id:nomifun_agent_contracts::CorrelationId::from(projection_id),
            causation_event_id:Some(tool_ack.event_id),
            payload:nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                nomifun_agent_contracts::StrictJsonValue(json!({
                    "operation_id":"settlement-http-operation","call_id":"settlement-http-call","output":null,
                    "error":"Capability Kernel rejected Agent Runtime Tool (CAPABILITY_UNAVAILABLE): The workspace owner reported success. Do not retry automatically. Re-read the affected owner state."
                })),
            ),
        },
    };
    store.append_event(&result).await.unwrap();

    // The canonical projection window can contain presentation-only rows.
    // An empty reasoning body is filtered by the renderer-facing mapper; it
    // still consumes a raw pagination slot and must carry a usable boundary.
    for (suffix,content) in [("visible","Inspect the workspace"),("empty"," ")] {
        let identity=format!("settlement-http-thinking-{suffix}");
        store.append_event(&nomifun_agent_contracts::SessionEventAppend {
            agent_session_id:typed_session.clone(),
            event_id:nomifun_agent_contracts::EventId::from(identity.clone()),
            producer_id:nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
            idempotency_key:nomifun_agent_contracts::IdempotencyKey::from(identity),
            semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {
                kind:nomifun_agent_contracts::SessionEventKind("thinking/content-part".to_owned()),
                kind_version:1,
                correlation_id:nomifun_agent_contracts::CorrelationId::from(uuid::Uuid::now_v7().to_string()),
                causation_event_id:Some(turn_started.clone()),
                payload:nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                    nomifun_agent_contracts::StrictJsonValue(json!({
                        "content":content,"turn_id":root_message_id,
                    })),
                ),
            },
        }).await.unwrap();
    }

    let mut cursor:Option<String>=None;
    let mut items=Vec::new();
    let mut total=None;
    let mut filtered_pages=0;
    let mut page_count=0;
    loop {
        let path=cursor.as_ref().map_or_else(
            || format!("/api/agent-sessions/{session_id}/message-history?page_size=1"),
            |cursor| format!("/api/agent-sessions/{session_id}/message-history?page_size=1&cursor={cursor}"),
        );
        let (status,page)=client.call(router.clone(),"GET",&path,json!({})).await;
        assert_eq!(status,StatusCode::OK,"{page}");
        let data=&page["data"];
        let page_items=data["items"].as_array().unwrap();
        assert!(page_items.len()<=1,"{page}");
        page_count+=1;
        if page_items.is_empty() {
            filtered_pages+=1;
            assert!(data["has_more"].as_bool().unwrap(),"{page}");
        }
        let page_total=data["total"].as_u64().unwrap();
        assert_eq!(*total.get_or_insert(page_total),page_total);
        items.extend(page_items.iter().cloned());
        if !data["has_more"].as_bool().unwrap() {
            assert!(data.get("next_cursor").is_none(),"terminal page must not advertise another window");
            break;
        }
        let next_cursor=data["next_cursor"].as_str().expect("every nonterminal raw window has a cursor");
        if let Some(previous)=&cursor {
            let timestamp=|value:&str| value.split_once(':').unwrap().0.parse::<i64>().unwrap();
            assert!(timestamp(next_cursor)<timestamp(previous),"history cursor must advance across filtered pages");
        }
        cursor=Some(next_cursor.to_owned());
        assert!(page_count<10,"history cursor did not converge");
    }
    assert_eq!(filtered_pages,1);
    assert_eq!(items.len() as u64+filtered_pages,total.unwrap());
    let unique=items.iter().map(|item| item["message_id"].as_str().unwrap()).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(),items.len());
    let tools=items.iter().filter(|item| item["type"]=="tool_call").collect::<Vec<_>>();
    assert_eq!(tools.len(),1,"{}",serde_json::to_string_pretty(&items).unwrap());
    assert_eq!(tools[0]["status"],"error");
    assert_eq!(tools[0]["content"]["status"],"error");
    assert!(tools[0]["content"]["output"].as_str().unwrap().contains("Do not retry"));
    let thinking=items.iter().find(|item| item["type"]=="thinking").unwrap();
    assert_eq!(thinking["content"]["content"],"Inspect the workspace");
    assert_eq!(thinking["content"]["status"],"done");
    assert!(items.iter().any(|item| item["message_id"]==root_message_id),"the oldest accepted input must survive the filtered head window");
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn message_history_http_cursor_reads_persisted_relocation_projections_from_canonical_facts() {
    const TRUST: &str = "relocation-history-projection";
    async fn call(router: &axum::Router, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let response = router.clone().oneshot(Request::builder()
            .method(if body.is_some() { "POST" } else { "GET" }).uri(path)
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
            .body(Body::from(body.unwrap_or_else(|| json!({})).to_string())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let provider = create_chat_provider(&router, TRUST, "Relocated history", "relocation-history-model", 1).await;
    let model = json!({"provider_id":provider["provider_id"],"model":"relocation-history-model"});
    let (status, preset) = call(&router, "/api/agent-presets", Some(json!({
        "display_name":"Relocated history","document":{
            "schema_version":"1.0.0","model_route_refs":{},"chat_route_records":{},
            "enabled_capabilities":[{
                "capability":{"id":"workspace.files"},"action_allowlist":["workspace.files/read","workspace.files/search"],
            }],
            "skill_bindings":[],"system_role_provider_overrides":{},
            "persona":"History export fixture","instructions":"Preserve the managed workspace history","starter_prompts":[],
        },
    }))).await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let (status, session) = call(&router, "/api/agent-sessions", Some(json!({
        "preset_id":preset["data"]["preset"]["preset_id"],"model":model,"title":"Relocated history",
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"}],
    }))).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let session_id = session["data"]["agent_session_id"].as_str().unwrap();
    let workspace = session["data"]["agent_binding"]["typed_resource_bindings"].as_array().unwrap()
        .iter().find(|resource| resource["resource_kind"] == "workspace").unwrap();
    assert_eq!(workspace["resource_id"], "default-workspace");
    assert_eq!(workspace["typed_parameters"], serde_json::to_value(
        nomifun_agent_contracts::managed_workspace_parameters()).unwrap());
    assert!(services.work_dir.join("conversations").join(session_id).is_dir(),
        "API Session creation must materialize its managed relative workspace");
    let typed_session = nomifun_agent_contracts::AgentSessionId::from(session_id);
    let store = nomifun_agent_session::AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    let operation = nomifun_agent_contracts::OperationId::from(uuid::Uuid::now_v7().to_string());
    let (input, turn) = store.start_turn(&typed_session,
        nomifun_agent_contracts::EventProducerId::from("session_api"),
        nomifun_agent_contracts::IdempotencyKey::from("relocation-history-input"),
        operation.clone(), nomifun_agent_contracts::StrictJsonValue(json!({"content":"Keep this history after moving storage"})),
    ).await.unwrap();
    let root_message_id = input.record.unwrap().event_id.as_ref().to_owned();
    let terminal_id = uuid::Uuid::now_v7().to_string();
    store.append_turn_terminal(&nomifun_agent_contracts::SessionEventAppend {
        agent_session_id:typed_session.clone(),
        event_id:nomifun_agent_contracts::EventId::from(terminal_id.clone()),
        producer_id:nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
        idempotency_key:nomifun_agent_contracts::IdempotencyKey::from(terminal_id),
        semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {
            kind:nomifun_agent_contracts::SessionEventKind("turn/completed".into()),kind_version:1,
            correlation_id:nomifun_agent_contracts::CorrelationId::from(operation.as_ref()),
            causation_event_id:Some(turn.record.unwrap().event_id),
            payload:nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(nomifun_agent_contracts::StrictJsonValue(
                json!({"model_steps":0,"finish_reason":"completed","finished_at_ms":0}),
            )),
        },
    }, &operation).await.unwrap();

    // Move the isolated data root to the existing work root, so the running
    // fixture's managed Workspace location already equals the proven target.
    let relocation_id = nomifun_agent_contracts::OperationId::from(uuid::Uuid::now_v7().to_string());
    let moved = store.relocate_managed_workspaces(&services.data_dir, &services.work_dir,
        &services.work_dir, &relocation_id).await.unwrap();
    assert_eq!(moved.changed_sessions, 1);
    let event_id = format!("storage-relocation:{}:{session_id}", relocation_id.as_ref());
    let event_before: String = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE event_id=?")
        .bind(&event_id).fetch_one(services.database.pool()).await.unwrap();
    let mut canonical_payload: Value = serde_json::from_str(&event_before).unwrap();
    assert!(canonical_payload.get("last_event_id").is_none());
    let transition = serde_json::from_value::<nomifun_agent_contracts::AgentBindingChangedPayloadV1>(canonical_payload.clone())
        .unwrap();
    transition.validate_storage_relocation().unwrap();
    assert_eq!(transition.workspace_relocation.as_ref().unwrap()
        .managed_session_namespace.as_ref().unwrap().agent_session_id, typed_session);

    // Reproduce the current-generation persisted projection pollution, even
    // after the writer is fixed. A consumer must reconstruct from the event.
    let projection_id = format!("agent-transition:{}", relocation_id.as_ref());
    let raw: String = sqlx::query_scalar("SELECT projection_json FROM agent_messages WHERE session_id=? AND projection_id=?")
        .bind(session_id).bind(&projection_id).fetch_one(services.database.pool()).await.unwrap();
    let mut projection: Value = serde_json::from_str(&raw).unwrap();
    projection["reference"]["last_event_id"] = json!(event_id);
    sqlx::query("UPDATE agent_messages SET projection_json=?,semantic_digest=? WHERE session_id=? AND projection_id=?")
        .bind(projection.to_string()).bind(nomifun_agent_contracts::digest_payload(&projection).unwrap().0)
        .bind(session_id).bind(&projection_id).execute(services.database.pool()).await.unwrap();

    let mut cursor = String::new();
    let mut messages = Vec::new();
    let mut empty_pages = 0;
    for page_number in 0..4 {
        let (status, page) = call(&router,
            &format!("/api/agent-sessions/{session_id}/message-history?page_size=1&cursor={cursor}"), None).await;
        assert_eq!(status, StatusCode::OK, "persisted diagnostics must not corrupt canonical history: {page}");
        let items = page["data"]["items"].as_array().unwrap();
        empty_pages += usize::from(items.is_empty());
        messages.extend(items.iter().cloned());
        if page["data"]["has_more"] == false { break; }
        let next = page["data"]["next_cursor"].as_str().unwrap();
        assert_ne!(next, cursor);
        cursor = next.to_owned();
        assert!(page_number < 3, "relocation history cursor did not converge");
    }
    assert_eq!(empty_pages, 1, "storage relocation is hidden, while its raw cursor advances");
    assert_eq!(messages.len(), 2);
    assert!(messages.iter().any(|message| message["message_id"] == root_message_id
        && message["content"]["content"] == "Keep this history after moving storage"));
    assert!(messages.iter().all(|message| message["content"].get("agent_transition").is_none()));
    let (status, detail) = call(&router,
        &format!("/api/agent-sessions/{session_id}/message-history/{root_message_id}"), None).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["data"]["content"]["content"], "Keep this history after moving storage");
    let (status, detail) = call(&router,
        &format!("/api/agent-sessions/{session_id}/message-history/{}", relocation_id.as_ref()), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "hidden storage metadata is not an Agent switch message: {detail}");
    let event_after: String = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE event_id=?")
        .bind(&event_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(event_after, event_before, "projection recovery must not rewrite canonical facts");

    // Exercise the actual export endpoint with the complete hydrated history.
    // Both destinations belong to this fixture's isolated temporary directory.
    messages.sort_by_key(|message| message["created_at"].as_i64().unwrap());
    let conversation = json!({"id":session_id,"name":"Relocated history","type":"nomi"});
    let exported = json!({"version":1,"conversation":conversation,"messages":messages});
    let selected_directory = services.work_dir.parent().unwrap().join("selected-export");
    for directory in [&services.work_dir, &selected_directory] {
        fs::create_dir_all(directory).unwrap();
        let output = directory.join("relocated-conversation.zip");
        let (status, result) = call(&router, "/api/fs/zip", Some(json!({
            "path":output,"files":[{"name":"conversation.json","content":exported.to_string()}],
        }))).await;
        assert_eq!(status, StatusCode::OK, "{}: {result}", output.display());
        assert_eq!(result["data"], true, "{}: {result}", output.display());
        let mut archive = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
        assert_eq!(archive.len(), 1);
        let restored: Value = serde_json::from_reader(archive.by_name("conversation.json").unwrap()).unwrap();
        assert_eq!(restored, exported, "ZIP must preserve the complete history at {}", output.display());
        assert_eq!(restored["messages"].as_array().unwrap().len(), 2);
        assert!(restored["messages"].as_array().unwrap().iter().any(|message|
            message["message_id"] == root_message_id
                && message["content"]["content"] == "Keep this history after moving storage"));
    }

    // A clean cached projection cannot authorize reading a damaged canonical
    // relocation proof. Both export history and single-message hydration fail.
    canonical_payload["request_digest"] = json!("0".repeat(64));
    sqlx::query("UPDATE agent_events SET inline_json=? WHERE event_id=?")
        .bind(canonical_payload.to_string()).bind(&event_id).execute(services.database.pool()).await.unwrap();
    for path in [format!("/api/agent-sessions/{session_id}/message-history?page_size=500"),
        format!("/api/agent-sessions/{session_id}/message-history/{root_message_id}")] {
        let (status, failure) = call(&router, &path, None).await;
        assert!(!status.is_success(), "damaged canonical proof must fail closed: {failure}");
    }
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn settlement_error_survives_event_cursor_reconnect_after_backend_restart() {
    let client = LocalJsonClient(TRUST);
    const TRUST: &str = "settlement-events-reconnect";

    let root = tempfile::tempdir().unwrap();
    let config = AppConfig {
        data_dir: root.path().join("data"),
        work_dir: root.path().join("work"),
        auth_policy: AuthPolicy::TrustLocalToken,
        local_trust_secret: Some(TRUST.into()),
        ..Default::default()
    };
    fs::create_dir_all(&config.data_dir).unwrap();
    let database = nomifun_db::init_database(&config.database_path())
        .await
        .unwrap();
    let first = AppServices::from_config(database, &config).await.unwrap();
    let router = create_router(&first).await;
    let provider =
        create_chat_provider(&router, TRUST, "Settlement events", "step-3.7-flash", 1).await;
    let model = json!({"provider_id":provider["provider_id"],"model":"step-3.7-flash"});
    let (status, preset) = client.call(
        router.clone(),
        "POST",
        "/api/agent-presets/from-template/chat.minimal",
        json!({
            "display_name":"Settlement events","reuse_existing":true,
            "model_route_refs":{},"chat_route_records":{},"model":model
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let preset_id = preset["data"]["preset"]["preset_id"].as_str().unwrap();
    let (status, session) = client.call(
        router.clone(),
        "POST",
        "/api/agent-sessions",
        json!({"preset_id":preset_id,"model":model,"title":"Settlement events"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let session_id = session["data"]["agent_session_id"].as_str().unwrap().to_owned();
    let typed_session = nomifun_agent_contracts::AgentSessionId::from(session_id.clone());
    let store =
        nomifun_agent_session::AgentSessionStore::from_pool(first.database.pool().clone())
            .await
            .unwrap();
    let (_, turn) = store
        .start_turn(
            &typed_session,
            nomifun_agent_contracts::EventProducerId::from("session_api"),
            nomifun_agent_contracts::IdempotencyKey::from("settlement-events-turn"),
            nomifun_agent_contracts::OperationId::from("settlement-events-turn"),
            nomifun_agent_contracts::StrictJsonValue(
                json!({"content":"exercise event reconnect"}),
            ),
        )
        .await
        .unwrap();
    let turn_started = turn.record.unwrap().event_id;
    let projection_id = uuid::Uuid::now_v7().to_string();
    let call_append = nomifun_agent_contracts::SessionEventAppend {
        agent_session_id: typed_session.clone(),
        event_id: nomifun_agent_contracts::EventId::from("settlement-events-tool-call"),
        producer_id: nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
        idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(
            "settlement-events-tool-call",
        ),
        semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
            kind: nomifun_agent_contracts::SessionEventKind("tool/call-started".to_owned()),
            kind_version: 1,
            correlation_id: nomifun_agent_contracts::CorrelationId::from(projection_id.clone()),
            causation_event_id: Some(turn_started),
            payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                nomifun_agent_contracts::StrictJsonValue(json!({
                    "operation_id":"settlement-events-operation","call_id":"settlement-events-call",
                    "capability_id":"workspace.files","action_id":"workspace.files/write","name":"write_file"
                })),
            ),
        },
    };
    let call_ack = store.append_event(&call_append).await.unwrap().ack.unwrap();
    let result_append = nomifun_agent_contracts::SessionEventAppend {
        agent_session_id: typed_session.clone(),
        event_id: nomifun_agent_contracts::EventId::from("settlement-events-tool-result"),
        producer_id: nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
        idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(
            "settlement-events-tool-result",
        ),
        semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
            kind: nomifun_agent_contracts::SessionEventKind("tool/result-recorded".to_owned()),
            kind_version: 1,
            correlation_id: nomifun_agent_contracts::CorrelationId::from(projection_id),
            causation_event_id: Some(call_ack.event_id.clone()),
            payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                nomifun_agent_contracts::StrictJsonValue(json!({
                    "operation_id":"settlement-events-operation","call_id":"settlement-events-call","output":null,
                    "error":"Capability Kernel rejected Agent Runtime Tool (CAPABILITY_UNAVAILABLE): The workspace owner reported success. Do not retry automatically. Re-read the affected owner state."
                })),
            ),
        },
    };
    let result_ack = store.append_event(&result_append).await.unwrap().ack.unwrap();
    let turn_terminal = nomifun_agent_contracts::SessionEventAppend {
        agent_session_id: typed_session.clone(),
        event_id: nomifun_agent_contracts::EventId::from("settlement-events-turn-failed"),
        producer_id: nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
        idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(
            "settlement-events-turn-failed",
        ),
        semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
            kind: nomifun_agent_contracts::SessionEventKind("turn/failed".to_owned()),
            kind_version: 1,
            correlation_id: nomifun_agent_contracts::CorrelationId::from("settlement-events-turn"),
            causation_event_id: Some(result_ack.event_id.clone()),
            payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                nomifun_agent_contracts::StrictJsonValue(json!({
                    "model_steps":0,
                    "message":"tool settlement could not be persisted",
                    "error":{"code":"CAPABILITY_UNAVAILABLE","retryable":false},
                    "finished_at_ms":0
                })),
            ),
        },
    };
    store
        .append_turn_terminal(
            &turn_terminal,
            &nomifun_agent_contracts::OperationId::from("settlement-events-turn"),
        )
        .await
        .unwrap();

    // The consumer drops after one page; the event cursor is all it keeps.
    let (status, page) = client.call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/events?limit=1"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(
        page["data"]["next_cursor"]["agent_session_id"].as_str().unwrap(),
        session_id
    );
    let mut after_seq = page["data"]["next_cursor"]["seq"].as_u64().unwrap();
    let mut seen = page["data"]["events"].as_array().unwrap().clone();
    assert_eq!(seen.len(), 1, "{page}");
    drop(router);
    first.shutdown_browser_platform().await.unwrap();
    first.database.close().await;
    drop(first);

    let reopened = nomifun_db::init_database(&config.database_path())
        .await
        .unwrap();
    let second = AppServices::from_config(reopened, &config).await.unwrap();
    let restored = create_router(&second).await;

    loop {
        let (status, page) = client.call(
            restored.clone(),
            "GET",
            &format!("/api/agent-sessions/{session_id}/events?after_seq={after_seq}&limit=1"),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let data = &page["data"];
        assert_eq!(data["agent_session_id"].as_str().unwrap(), session_id);
        assert_eq!(
            data["next_cursor"]["agent_session_id"].as_str().unwrap(),
            session_id
        );
        match data["events"].as_array().unwrap().as_slice() {
            [] => {
                assert_eq!(data["next_cursor"]["seq"].as_u64().unwrap(), after_seq);
                break;
            }
            [event] => {
                let seq = event["seq"].as_u64().unwrap();
                assert_eq!(seq, after_seq + 1, "reconnect must not skip or repeat seq");
                assert_eq!(data["next_cursor"]["seq"].as_u64().unwrap(), seq);
                after_seq = seq;
                seen.push(event.clone());
            }
            _ => panic!("limit=1 returned more than one event"),
        }
        assert!(seen.len() < 32, "event cursor did not converge");
    }
    let unique_ids = seen
        .iter()
        .map(|event| event["event_id"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique_ids.len(), seen.len(), "event replay must be exactly once");
    let committed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_events WHERE session_id = ?",
    )
    .bind(&session_id)
    .fetch_one(second.database.pool())
    .await
    .unwrap();
    assert_eq!(
        seen.len() as i64,
        committed,
        "cursor reconnect must deliver every committed event exactly once"
    );
    let count_kind = |kind: &str| seen.iter().filter(|event| event["kind"] == kind).count();
    assert_eq!(count_kind("tool/call-started"), 1);
    assert_eq!(count_kind("tool/result-recorded"), 1);
    assert_eq!(count_kind("turn/failed"), 1, "terminal event must not be lost");
    let result = seen
        .iter()
        .find(|event| event["kind"] == "tool/result-recorded")
        .unwrap();
    let error = result["payload"]["value"]["error"].as_str().unwrap();
    assert!(error.contains("CAPABILITY_UNAVAILABLE"), "{error}");
    assert!(error.contains("Do not retry"), "{error}");
    assert_eq!(
        result["payload"]["value"]["call_id"].as_str().unwrap(),
        "settlement-events-call"
    );

    // A write-path replay of the same settlement facts stays idempotent and
    // cannot create a second canonical event after reconnect.
    let reopened_store =
        nomifun_agent_session::AgentSessionStore::from_pool(second.database.pool().clone())
            .await
            .unwrap();
    for append in [&call_append, &result_append, &turn_terminal] {
        let replay = reopened_store.append_event(append).await.unwrap();
        assert!(replay.duplicate, "{replay:?}");
        assert!(replay.record.is_some(), "{replay:?}");
    }
    let replay_result = reopened_store.append_event(&result_append).await.unwrap();
    assert_eq!(replay_result.record.unwrap().seq, result_ack.seq);
    let still: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_events WHERE session_id = ?",
    )
    .bind(&session_id)
    .fetch_one(second.database.pool())
    .await
    .unwrap();
    assert_eq!(still, committed);

    // Event and message-history cursors stay on independent axes: each rejects
    // the other's wire form, and a forward cursor fails closed instead of
    // silently returning an empty page.
    let (status, mixed) = client.call(
        restored.clone(),
        "GET",
        &format!(
            "/api/agent-sessions/{session_id}/events?after_seq=1700000000000:0190f5fe-7c00-7a00-8abc-0123456789ab"
        ),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{mixed}");
    assert!(
        mixed.as_str().is_some_and(|body| body.contains("after_seq")),
        "events endpoint must reject a message-history cursor, got {mixed}"
    );
    let (status, mixed) = client.call(
        restored.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/message-history?cursor={after_seq}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{mixed}");
    assert!(
        mixed.as_str().is_some_and(|body| body.contains("cursor")),
        "history endpoint must reject an event-seq cursor, got {mixed}"
    );
    let (status, ahead) = client.call(
        restored.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/events?after_seq={}", after_seq + 1000),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{ahead}");

    // The projection rebuilt from the canonical stream still owns exactly one
    // error tool row after reconnect; history pages stay strictly monotone.
    let mut cursor: Option<String> = None;
    let mut items = Vec::new();
    let mut total = None;
    loop {
        let path = cursor.as_ref().map_or_else(
            || format!("/api/agent-sessions/{session_id}/message-history?page_size=1"),
            |cursor| {
                format!("/api/agent-sessions/{session_id}/message-history?page_size=1&cursor={cursor}")
            },
        );
        let (status, page) = client.call(restored.clone(), "GET", &path, json!({})).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let data = &page["data"];
        let page_items = data["items"].as_array().unwrap();
        assert_eq!(page_items.len(), 1, "{page}");
        let page_total = data["total"].as_u64().unwrap();
        assert_eq!(*total.get_or_insert(page_total), page_total);
        let item = page_items[0].clone();
        cursor = Some(format!(
            "{}:{}",
            item["created_at"].as_i64().unwrap(),
            item["message_id"].as_str().unwrap()
        ));
        items.push(item);
        if !data["has_more"].as_bool().unwrap() {
            break;
        }
        assert!(items.len() < 16, "history cursor did not converge");
    }
    assert_eq!(items.len() as u64, total.unwrap());
    let created = items
        .iter()
        .map(|item| item["created_at"].as_i64().unwrap())
        .collect::<Vec<_>>();
    assert!(
        created.windows(2).all(|pair| pair[0] > pair[1]),
        "message-history cursor must move strictly backwards: {created:?}"
    );
    let unique = items
        .iter()
        .map(|item| item["message_id"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(), items.len());
    let tools = items
        .iter()
        .filter(|item| item["type"] == "tool_call")
        .collect::<Vec<_>>();
    assert_eq!(tools.len(), 1, "{}", serde_json::to_string_pretty(&items).unwrap());
    assert_eq!(tools[0]["status"], "error");
    assert_eq!(tools[0]["content"]["status"], "error");
    let output = tools[0]["content"]["output"].as_str().unwrap();
    assert!(output.contains("Do not retry"), "{output}");
    assert!(output.contains("CAPABILITY_UNAVAILABLE"), "{output}");

    drop(restored);
    second.shutdown_browser_platform().await.unwrap();
    second.database.close().await;
}

async fn append_settlement_events(
    store: &nomifun_agent_session::AgentSessionStore,
    typed_session: &nomifun_agent_contracts::AgentSessionId,
    suffix: &str,
) -> nomifun_agent_contracts::SessionEventAck {
    let (_, turn) = store
        .start_turn(
            typed_session,
            nomifun_agent_contracts::EventProducerId::from("session_api"),
            nomifun_agent_contracts::IdempotencyKey::from(format!("settlement-turn-{suffix}")),
            nomifun_agent_contracts::OperationId::from(format!("settlement-turn-{suffix}")),
            nomifun_agent_contracts::StrictJsonValue(
                json!({"content":format!("exercise cursor stability {suffix}")}),
            ),
        )
        .await
        .unwrap();
    let turn_started = turn.record.unwrap().event_id;
    let projection_id = uuid::Uuid::now_v7().to_string();
    let call_append = nomifun_agent_contracts::SessionEventAppend {
        agent_session_id: typed_session.clone(),
        event_id: nomifun_agent_contracts::EventId::from(format!("tool-call-{suffix}")),
        producer_id: nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
        idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(format!("tool-call-{suffix}")),
        semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
            kind: nomifun_agent_contracts::SessionEventKind("tool/call-started".to_owned()),
            kind_version: 1,
            correlation_id: nomifun_agent_contracts::CorrelationId::from(projection_id.clone()),
            causation_event_id: Some(turn_started),
            payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                nomifun_agent_contracts::StrictJsonValue(json!({
                    "operation_id":format!("settlement-operation-{suffix}"),
                    "call_id":format!("settlement-call-{suffix}"),
                    "capability_id":"workspace.files","action_id":"workspace.files/write","name":"write_file"
                })),
            ),
        },
    };
    let call_ack = store.append_event(&call_append).await.unwrap().ack.unwrap();
    let result_append = nomifun_agent_contracts::SessionEventAppend {
        agent_session_id: typed_session.clone(),
        event_id: nomifun_agent_contracts::EventId::from(format!("tool-result-{suffix}")),
        producer_id: nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
        idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(format!(
            "tool-result-{suffix}"
        )),
        semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
            kind: nomifun_agent_contracts::SessionEventKind("tool/result-recorded".to_owned()),
            kind_version: 1,
            correlation_id: nomifun_agent_contracts::CorrelationId::from(projection_id),
            causation_event_id: Some(call_ack.event_id.clone()),
            payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                nomifun_agent_contracts::StrictJsonValue(json!({
                    "operation_id":format!("settlement-operation-{suffix}"),
                    "call_id":format!("settlement-call-{suffix}"),"output":null,
                    "error":"Capability Kernel rejected Agent Runtime Tool (CAPABILITY_UNAVAILABLE): The workspace owner reported success. Do not retry automatically. Re-read the affected owner state."
                })),
            ),
        },
    };
    let result_ack = store.append_event(&result_append).await.unwrap().ack.unwrap();
    let turn_terminal = nomifun_agent_contracts::SessionEventAppend {
        agent_session_id: typed_session.clone(),
        event_id: nomifun_agent_contracts::EventId::from(format!("turn-failed-{suffix}")),
        producer_id: nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
        idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(format!(
            "turn-failed-{suffix}"
        )),
        semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
            kind: nomifun_agent_contracts::SessionEventKind("turn/failed".to_owned()),
            kind_version: 1,
            correlation_id: nomifun_agent_contracts::CorrelationId::from(format!(
                "settlement-turn-{suffix}"
            )),
            causation_event_id: Some(result_ack.event_id.clone()),
            payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                nomifun_agent_contracts::StrictJsonValue(json!({
                    "model_steps":0,
                    "message":"tool settlement could not be persisted",
                    "error":{"code":"CAPABILITY_UNAVAILABLE","retryable":false},
                    "finished_at_ms":0
                })),
            ),
        },
    };
    store
        .append_turn_terminal(
            &turn_terminal,
            &nomifun_agent_contracts::OperationId::from(format!("settlement-turn-{suffix}")),
        )
        .await
        .unwrap();
    result_ack
}

#[tokio::test]
async fn messages_forward_cursor_keeps_truncated_and_appended_projections() {
    let client = LocalJsonClient(TRUST);
    const TRUST: &str = "settlement-messages-forward-cursor";

    let (router, services) = common::build_local_trust_app(TRUST).await;
    let provider =
        create_chat_provider(&router, TRUST, "Forward cursor", "step-3.7-flash", 1).await;
    let model = json!({"provider_id":provider["provider_id"],"model":"step-3.7-flash"});
    let (status, preset) = client.call(
        router.clone(),
        "POST",
        "/api/agent-presets/from-template/chat.minimal",
        json!({
            "display_name":"Forward cursor","reuse_existing":true,
            "model_route_refs":{},"chat_route_records":{},"model":model
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let preset_id = preset["data"]["preset"]["preset_id"].as_str().unwrap();
    let (status, session) = client.call(
        router.clone(),
        "POST",
        "/api/agent-sessions",
        json!({"preset_id":preset_id,"model":model,"title":"Forward cursor"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let session_id = session["data"]["agent_session_id"].as_str().unwrap().to_owned();
    let typed_session = nomifun_agent_contracts::AgentSessionId::from(session_id.clone());
    let store = nomifun_agent_session::AgentSessionStore::from_pool(
        services.database.pool().clone(),
    )
    .await
    .unwrap();
    let (_, turn) = store
        .start_turn(
            &typed_session,
            nomifun_agent_contracts::EventProducerId::from("session_api"),
            nomifun_agent_contracts::IdempotencyKey::from("settlement-turn-one"),
            nomifun_agent_contracts::OperationId::from("settlement-turn-one"),
            nomifun_agent_contracts::StrictJsonValue(
                json!({"content":"exercise forward cursor"}),
            ),
        )
        .await
        .unwrap();
    let turn_started = turn.record.unwrap().event_id;
    let call_one_projection = uuid::Uuid::now_v7().to_string();
    let call_two_projection = uuid::Uuid::now_v7().to_string();
    let tool_event = |suffix: &str,
                      kind: &str,
                      projection_id: &str,
                      causation: nomifun_agent_contracts::EventId| {
        nomifun_agent_contracts::SessionEventAppend {
            agent_session_id: typed_session.clone(),
            event_id: nomifun_agent_contracts::EventId::from(format!("{kind}-{suffix}")),
            producer_id: nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
            idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(format!(
                "{kind}-{suffix}"
            )),
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: nomifun_agent_contracts::SessionEventKind(kind.to_owned()),
                kind_version: 1,
                correlation_id: nomifun_agent_contracts::CorrelationId::from(
                    projection_id.to_owned(),
                ),
                causation_event_id: Some(causation),
                payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                    nomifun_agent_contracts::StrictJsonValue(json!({
                        "operation_id":format!("settlement-operation-{suffix}"),
                        "call_id":format!("settlement-call-{suffix}"),
                        "capability_id":"workspace.files","action_id":"workspace.files/write",
                        "name":"write_file"
                    })),
                ),
            },
        }
    };
    let call_one = tool_event(
        "01",
        "tool/call-started",
        &call_one_projection,
        turn_started.clone(),
    );
    let call_one_ack = store.append_event(&call_one).await.unwrap().ack.unwrap();
    let call_two = tool_event(
        "02",
        "tool/call-started",
        &call_two_projection,
        turn_started,
    );
    let call_two_ack = store.append_event(&call_two).await.unwrap().ack.unwrap();
    // Results settle out of order: call two finishes first, so the earlier
    // projection keeps moving after the later one has already stopped.
    let mut result_two = tool_event(
        "02",
        "tool/result-recorded",
        &call_two_projection,
        call_two_ack.event_id.clone(),
    );
    result_two.semantic_event.payload =
        nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
            nomifun_agent_contracts::StrictJsonValue(json!({
                "operation_id":"settlement-operation-02","call_id":"settlement-call-02",
                "output":null,
                "error":"Capability Kernel rejected Agent Runtime Tool (CAPABILITY_UNAVAILABLE): The workspace owner reported success. Do not retry automatically. Re-read the affected owner state."
            })),
        );
    store.append_event(&result_two).await.unwrap();
    let mut result_one = tool_event(
        "01",
        "tool/result-recorded",
        &call_one_projection,
        call_one_ack.event_id.clone(),
    );
    result_one.semantic_event.payload =
        nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
            nomifun_agent_contracts::StrictJsonValue(json!({
                "operation_id":"settlement-operation-01","call_id":"settlement-call-01",
                "output":null,
                "error":"Capability Kernel rejected Agent Runtime Tool (CAPABILITY_UNAVAILABLE): The workspace owner reported success. Do not retry automatically. Re-read the affected owner state."
            })),
        );
    store.append_event(&result_one).await.unwrap();
    let turn_terminal = nomifun_agent_contracts::SessionEventAppend {
        agent_session_id: typed_session.clone(),
        event_id: nomifun_agent_contracts::EventId::from("turn-failed-one"),
        producer_id: nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
        idempotency_key: nomifun_agent_contracts::IdempotencyKey::from("turn-failed-one"),
        semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
            kind: nomifun_agent_contracts::SessionEventKind("turn/failed".to_owned()),
            kind_version: 1,
            correlation_id: nomifun_agent_contracts::CorrelationId::from("settlement-turn-one"),
            causation_event_id: Some(call_two_ack.event_id.clone()),
            payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                nomifun_agent_contracts::StrictJsonValue(json!({
                    "model_steps":0,"message":"tool settlement could not be persisted",
                    "error":{"code":"CAPABILITY_UNAVAILABLE","retryable":false},"finished_at_ms":0
                })),
            ),
        },
    };
    store
        .append_turn_terminal(
            &turn_terminal,
            &nomifun_agent_contracts::OperationId::from("settlement-turn-one"),
        )
        .await
        .unwrap();

    let mut after_seq = 0_u64;
    let mut seen_ids = Vec::<String>::new();
    let mut appended = false;
    loop {
        let (status, page) = client.call(
            router.clone(),
            "GET",
            &format!("/api/agent-sessions/{session_id}/messages?after_seq={after_seq}&limit=1"),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let data = &page["data"];
        assert_eq!(data["agent_session_id"].as_str().unwrap(), session_id);
        let messages = data["messages"].as_array().unwrap();
        assert!(messages.len() <= 1, "{page}");
        let next_seq = data["next_cursor"]["seq"].as_u64().unwrap();
        for message in messages {
            let last_seq = message["last_seq"].as_u64().unwrap();
            assert!(last_seq > after_seq, "stale projection re-delivered: {message}");
            seen_ids.push(message["projection_id"].as_str().unwrap().to_owned());
        }
        if !appended {
            // A new Turn committed between pages must still be delivered exactly
            // once by the same forward cursor.
            let (_, turn2) = store
                .start_turn(
                    &typed_session,
                    nomifun_agent_contracts::EventProducerId::from("session_api"),
                    nomifun_agent_contracts::IdempotencyKey::from("settlement-turn-two"),
                    nomifun_agent_contracts::OperationId::from("settlement-turn-two"),
                    nomifun_agent_contracts::StrictJsonValue(
                        json!({"content":"concurrent append during pagination"}),
                    ),
                )
                .await
                .unwrap();
            let appended_seq = turn2.record.unwrap().seq;
            assert!(
                appended_seq > after_seq,
                "an event committed mid-walk must sit after the live cursor"
            );
            appended = true;
        }
        if messages.is_empty() {
            assert_eq!(next_seq, after_seq, "empty page must not advance the cursor");
            break;
        }
        assert!(next_seq > after_seq, "cursor must move forward: {page}");
        after_seq = next_seq;
        assert!(seen_ids.len() < 32, "messages cursor did not converge");
    }

    let committed: std::collections::BTreeSet<String> = sqlx::query_scalar(
        "SELECT projection_id FROM agent_messages WHERE session_id = ?",
    )
    .bind(&session_id)
    .fetch_all(services.database.pool())
    .await
    .unwrap()
    .into_iter()
    .collect();
    let delivered: std::collections::BTreeSet<_> = seen_ids.iter().cloned().collect();
    assert_eq!(
        delivered.len(),
        seen_ids.len(),
        "a projection must not be delivered twice: {seen_ids:?}"
    );
    assert_eq!(
        delivered, committed,
        "forward cursor must deliver every committed projection exactly once"
    );
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn events_and_history_cursors_stay_stable_while_events_append() {
    let client = LocalJsonClient(TRUST);
    const TRUST: &str = "settlement-cursor-concurrent-append";

    let (router, services) = common::build_local_trust_app(TRUST).await;
    let provider =
        create_chat_provider(&router, TRUST, "Concurrent append", "step-3.7-flash", 1).await;
    let model = json!({"provider_id":provider["provider_id"],"model":"step-3.7-flash"});
    let (status, preset) = client.call(
        router.clone(),
        "POST",
        "/api/agent-presets/from-template/chat.minimal",
        json!({
            "display_name":"Concurrent append","reuse_existing":true,
            "model_route_refs":{},"chat_route_records":{},"model":model
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let preset_id = preset["data"]["preset"]["preset_id"].as_str().unwrap();
    let (status, session) = client.call(
        router.clone(),
        "POST",
        "/api/agent-sessions",
        json!({"preset_id":preset_id,"model":model,"title":"Concurrent append"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let session_id = session["data"]["agent_session_id"].as_str().unwrap().to_owned();
    let typed_session = nomifun_agent_contracts::AgentSessionId::from(session_id.clone());
    let store = nomifun_agent_session::AgentSessionStore::from_pool(
        services.database.pool().clone(),
    )
    .await
    .unwrap();
    append_settlement_events(&store, &typed_session, "one").await;

    // Canonical event feed: an event committed between pages lands exactly once
    // on the same cursor without reordering or skipping earlier events.
    let mut after_seq = 0_u64;
    let mut seen = Vec::<Value>::new();
    let mut appended = false;
    let mut turn_two_started: Option<nomifun_agent_contracts::EventId> = None;
    loop {
        let (status, page) = client.call(
            router.clone(),
            "GET",
            &format!("/api/agent-sessions/{session_id}/events?after_seq={after_seq}&limit=2"),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let data = &page["data"];
        let events = data["events"].as_array().unwrap();
        if events.is_empty() {
            assert_eq!(data["next_cursor"]["seq"].as_u64().unwrap(), after_seq);
            break;
        }
        for event in events {
            let seq = event["seq"].as_u64().unwrap();
            assert_eq!(seq, after_seq + 1, "event feed must stay contiguous: {event}");
            after_seq = seq;
            seen.push(event.clone());
        }
        assert_eq!(data["next_cursor"]["seq"].as_u64().unwrap(), after_seq);
        if !appended {
            let (_, turn_two) = store
                .start_turn(
                    &typed_session,
                    nomifun_agent_contracts::EventProducerId::from("session_api"),
                    nomifun_agent_contracts::IdempotencyKey::from("settlement-turn-two"),
                    nomifun_agent_contracts::OperationId::from("settlement-turn-two"),
                    nomifun_agent_contracts::StrictJsonValue(
                        json!({"content":"concurrent append during event pagination"}),
                    ),
                )
                .await
                .unwrap();
            turn_two_started = Some(turn_two.record.unwrap().event_id);
            appended = true;
        }
        assert!(seen.len() < 32, "event cursor did not converge");
    }
    let committed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_events WHERE session_id = ?",
    )
    .bind(&session_id)
    .fetch_one(services.database.pool())
    .await
    .unwrap();
    assert_eq!(seen.len() as i64, committed);
    assert_eq!(seen.last().unwrap()["seq"].as_u64().unwrap(), after_seq);
    assert!(
        seen.iter().any(|event| event["kind"] == "turn/started"
            && event["correlation_id"] == "settlement-turn-two"),
        "the concurrently appended Turn must appear exactly once on the live cursor"
    );

    // The Session allows only one active Turn; settle turn two so the history
    // walk can append a later Turn through the same lifecycle.
    let turn_two_terminal = nomifun_agent_contracts::SessionEventAppend {
        agent_session_id: typed_session.clone(),
        event_id: nomifun_agent_contracts::EventId::from("turn-completed-two"),
        producer_id: nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
        idempotency_key: nomifun_agent_contracts::IdempotencyKey::from("turn-completed-two"),
        semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
            kind: nomifun_agent_contracts::SessionEventKind("turn/completed".to_owned()),
            kind_version: 1,
            correlation_id: nomifun_agent_contracts::CorrelationId::from(
                "settlement-turn-two".to_owned(),
            ),
            causation_event_id: turn_two_started,
            payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                nomifun_agent_contracts::StrictJsonValue(json!({
                    "model_steps":0,"finish_reason":"completed","finished_at_ms":0
                })),
            ),
        },
    };
    store
        .append_turn_terminal(
            &turn_two_terminal,
            &nomifun_agent_contracts::OperationId::from("settlement-turn-two"),
        )
        .await
        .unwrap();

    // Backward message history: a projection committed after the walk started
    // stays outside the anchored window instead of shifting or duplicating rows.
    // The history view also derives one turn_summary row per durable Turn;
    // its message_id is the turn/started event id rather than a projection
    // correlation.
    let anchored: std::collections::BTreeSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT json_extract(projection_json, '$.correlation_id') FROM agent_messages \
         WHERE session_id = ? \
         AND presentation_intent IN ('message','tool','agent_transition') \
         UNION \
         SELECT started_event_id FROM agent_turns \
         WHERE session_id = ? AND source_message_id IS NOT NULL",
    )
    .bind(&session_id)
    .bind(&session_id)
    .fetch_all(services.database.pool())
    .await
    .unwrap()
    .into_iter()
    .filter(|id| {
        uuid::Uuid::parse_str(id)
            .ok()
            .is_some_and(|uuid| uuid.get_version_num() == 7)
    })
    .collect();
    let mut cursor: Option<String> = None;
    let mut items = Vec::<Value>::new();
    let mut appended_correlation: Option<String> = None;
    loop {
        let path = cursor.as_ref().map_or_else(
            || format!("/api/agent-sessions/{session_id}/message-history?page_size=2"),
            |cursor| {
                format!(
                    "/api/agent-sessions/{session_id}/message-history?page_size=2&cursor={cursor}"
                )
            },
        );
        let (status, page) = client.call(router.clone(), "GET", &path, json!({})).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let data = &page["data"];
        for item in data["items"].as_array().unwrap() {
            items.push(item.clone());
        }
        if appended_correlation.is_none() {
            let (appended_message, _) = store
                .start_turn(
                    &typed_session,
                    nomifun_agent_contracts::EventProducerId::from("session_api"),
                    nomifun_agent_contracts::IdempotencyKey::from("settlement-turn-three"),
                    nomifun_agent_contracts::OperationId::from("settlement-turn-three"),
                    nomifun_agent_contracts::StrictJsonValue(
                        json!({"content":"concurrent append during history pagination"}),
                    ),
                )
                .await
                .unwrap();
            appended_correlation = Some(
                appended_message
                    .record
                    .unwrap()
                    .correlation_id
                    .as_ref()
                    .to_owned(),
            );
        }
        if !data["has_more"].as_bool().unwrap() {
            break;
        }
        let last = items.last().unwrap();
        cursor = Some(format!(
            "{}:{}",
            last["created_at"].as_i64().unwrap(),
            last["message_id"].as_str().unwrap()
        ));
        assert!(items.len() < 32, "history cursor did not converge");
    }
    let seen_ids: std::collections::BTreeSet<String> = items
        .iter()
        .map(|item| item["message_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        seen_ids.len(),
        items.len(),
        "history walk must not repeat rows: {seen_ids:?}"
    );
    let appended_correlation = appended_correlation.unwrap();
    assert!(
        !seen_ids.contains(&appended_correlation),
        "a message committed after the walk started must stay outside the anchored window"
    );
    assert_eq!(
        seen_ids, anchored,
        "the anchored window must deliver exactly the projections committed before the walk"
    );
    // A fresh first page sees the appended Turn at the head (its derived
    // turn_summary sorts newest) alongside its source message, without
    // invalidating anything the earlier walk already delivered.
    let (status, fresh) = client.call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/message-history?page_size=2"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{fresh}");
    let fresh_ids: Vec<&str> = fresh["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["message_id"].as_str().unwrap())
        .collect();
    assert!(
        fresh_ids.contains(&appended_correlation.as_str()),
        "the appended message must surface on the next fresh history read: {fresh_ids:?}"
    );
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn effectful_coding_session_switches_same_workspace_to_read_only_with_new_resource_definition() {
    use nomifun_agent_contracts::{AgentSessionId, OperationId};
    use nomifun_agent_session::{AgentEffectState, AgentSessionStore, TurnReceiptStatus};
    use std::collections::BTreeSet;
    const TRUST: &str = "effectful-workspace-agent-switch";
    const CONTENT: &str = "SOURCE_WORKSPACE_WRITE_ONCE\n";
    const REPLY: &str = "SOURCE_WORKSPACE_WRITE_DONE";
    const CALL: &str = "effectful-source-write";
    const TARGET_CALL: &str = "effectful-target-write";
    async fn call(router: &axum::Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(path)
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json");
        if method == "PUT" && path.ends_with("/agent") {
            request = request.header("idempotency-key", uuid::Uuid::now_v7().to_string());
        }
        let response = router.clone().oneshot(request.body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }
    let upstream = wiremock::MockServer::start().await;
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = requests.clone();
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let body = request.body_json::<Value>().unwrap();
            let mut requests = captured.lock().unwrap();
            requests.push(body.clone());
            assert!(requests.len() <= 4, "each Agent must settle exactly one requested write");
            let call_id = if requests.len() <= 2 { CALL } else { TARGET_CALL };
            let (delta, finish) = if requests.len() == 1 || requests.len() == 3 {
                assert!(body["tools"].as_array().unwrap().iter().any(|tool| tool["function"]["name"] == "write_file"));
                (json!({"role":"assistant","tool_calls":[{"index":0,"id":call_id,"type":"function",
                    "function":{"name":"write_file","arguments":json!({"path":"switch-proof.txt","content":CONTENT}).to_string()}}]}), "tool_calls")
            } else {
                assert!(body["messages"].as_array().unwrap().iter().any(|message| message["role"] == "tool" && message["tool_call_id"] == call_id),
                    "the actual owner result must reach the next source model request");
                (json!({"role":"assistant","content":REPLY}), "stop")
            };
            let first = json!({"id":"effectful-switch","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
            let last = json!({"id":"effectful-switch","choices":[{"index":0,"delta":{},"finish_reason":finish}]});
            wiremock::ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
                .set_body_string(format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n"))
        }).mount(&upstream).await;
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let (status, provider) = call(&router, "POST", "/api/providers", json!({
        "platform":"stepfun-plan","name":"Effectful workspace switch","base_url":format!("{}/v1",upstream.uri()),
        "auth_scheme":"bearer","credentials":{"api_keys":["fixture-only"]},"enabled":true,
        "initial_model":{"model":"workspace-switch-model","enabled":true,"capabilities":[{
            "task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default",
            "context_limit":1000000,"output_limit":4096
        }]}
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let model = json!({"provider_id":provider["data"]["provider_id"],"model":"workspace-switch-model"});
    let (status, source) = call(&router, "POST", "/api/agent-presets/from-template/coding.codex", json!({
        "display_name":"Source writer","reuse_existing":false,"model":model,
    })).await;
    assert_eq!(status, StatusCode::OK, "{source}");
    let workspace = tempfile::tempdir().unwrap();
    let (status, created) = call(&router, "POST", "/api/agent-sessions", json!({
        "preset_id":source["data"]["preset"]["preset_id"],"model":model,
        "title":"Keep this effectful Session","workspace":workspace.path().to_str().unwrap(),
        "resource_selections":[
            {"resource_kind":"workspace","resource_id":"default-workspace"},
            {"resource_kind":"process_session","resource_id":"managed-process-session"},
            {"resource_kind":"project_memory","resource_id":"default-project-memory"}
        ]
    })).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let sid = created["data"]["agent_session_id"].as_str().unwrap();
    let session = AgentSessionId::from(sid.to_owned());
    let store = AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    let source_binding = store.get_live_session(&session).await.unwrap().agent_binding;
    let source_workspace = source_binding.typed_resource_bindings.iter()
        .find(|binding| binding.resource_kind.as_ref() == "workspace").unwrap().clone();
    assert_eq!(source_workspace.operations, BTreeSet::from(["read".to_owned(),"write".to_owned()]));
    let (status, started) = call(&router, "POST", &format!("/api/agent-sessions/{sid}/turns"), json!({
        "idempotency_key":uuid::Uuid::now_v7().to_string(),
        "input":{"content":format!("请使用 write_file 一次创建 switch-proof.txt，正文准确为 {CONTENT}。完成这一个写入后直接回复 {REPLY}，不要读取文件、执行命令或重复写入。")}
    })).await;
    assert_eq!(status, StatusCode::OK, "{started}");
    let operation = OperationId::from(started["data"]["operation_id"].as_str().unwrap().to_owned());
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let receipt = store.read_turn_receipt(&session, &operation).await.unwrap();
            if receipt.status == TurnReceiptStatus::Completed { break; }
            assert_eq!(receipt.status, TurnReceiptStatus::Running, "{receipt:?}");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("the actual source write must settle and the native Turn must complete");
    assert_eq!(std::fs::read(workspace.path().join("switch-proof.txt")).unwrap(), CONTENT.as_bytes());
    let source_effects = store.list_effects(&session).await.unwrap();
    assert_eq!(source_effects.len(), 1);
    assert_eq!(source_effects[0].state, AgentEffectState::Returned);
    assert_eq!(source_effects[0].resource_binding_id.as_ref(), Some(&source_workspace.binding_id));
    let source_resource_digest: String = sqlx::query_scalar(
        "SELECT binding_digest FROM agent_session_resources WHERE session_id=? AND binding_id=?")
        .bind(sid).bind(source_workspace.binding_id.as_ref()).fetch_one(services.database.pool()).await.unwrap();
    let (status, target) = call(&router, "POST", "/api/agent-presets/from-template/chat.minimal", json!({
        "display_name":"Read-only workspace target","reuse_existing":false,"model":model,
    })).await;
    assert_eq!(status, StatusCode::OK, "{target}");
    let mut draft = target["data"]["draft"].clone();
    draft["document"]["enabled_capabilities"] = json!([{
        "capability":{"id":"workspace.files"},"action_allowlist":["workspace.files/read"]
    }]);
    let target_id = draft["preset_id"].as_str().unwrap().to_owned();
    let (status, saved) = call(&router, "POST", &format!("/api/agent-presets/{target_id}/revisions"), json!({
        "expected_current_revision":draft["current_revision"],"draft":draft,"reason":"Read-only same-workspace switch regression"
    })).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let selection = json!({"kind":"preset","preset_id":target_id});
    let (status, preview) = call(&router, "POST", &format!("/api/agent-sessions/{sid}/agent-switch/preview"), json!({"selection":selection})).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["data"]["can_apply"], true, "{preview}");
    let (status, switched) = call(&router, "PUT", &format!("/api/agent-sessions/{sid}/agent"), json!({
        "selection":selection,"handoff_mode":"context_only","expected_binding_version":preview["data"]["expected_binding_version"]
    })).await;
    assert_eq!(status, StatusCode::OK, "the real producer must derive a new read-only definition for the same physical workspace: {switched}");
    assert_eq!(switched["data"]["conversation"]["conversation_id"], sid);
    let target_binding = store.get_live_session(&session).await.unwrap().agent_binding;
    assert_eq!(target_binding.typed_resource_bindings.len(), 1);
    let target_workspace = &target_binding.typed_resource_bindings[0];
    assert_eq!(target_workspace.resource_id, source_workspace.resource_id);
    assert_eq!(target_workspace.owner_id, source_workspace.owner_id);
    assert_eq!(target_workspace.typed_parameters, source_workspace.typed_parameters);
    assert_eq!(target_workspace.connection_config_ref, source_workspace.connection_config_ref);
    assert_eq!(target_workspace.operations, BTreeSet::from(["read".to_owned()]));
    assert_ne!(target_workspace.binding_id, source_workspace.binding_id);
    assert_eq!(store.list_effects(&session).await.unwrap(), source_effects);
    let retained_digest: String = sqlx::query_scalar(
        "SELECT binding_digest FROM agent_session_resources WHERE session_id=? AND binding_id=?")
        .bind(sid).bind(source_workspace.binding_id.as_ref()).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(retained_digest, source_resource_digest);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM pragma_foreign_key_check")
        .fetch_one(services.database.pool()).await.unwrap(), 0);
    assert_eq!(requests.lock().unwrap().len(), 2, "switching must not replay the completed write");

    let minimal_selection = json!({"kind":"template","template_key":"chat.minimal"});
    let coding_selection = json!({"kind":"template","template_key":"coding.codex"});
    // A useful project stays selected across a lower Agent with no file grant.
    for selection in [&minimal_selection, &coding_selection] {
        let (status, preview) = call(&router, "POST", &format!("/api/agent-sessions/{sid}/agent-switch/preview"),
            json!({"selection":selection})).await;
        assert_eq!(status, StatusCode::OK, "{preview}");
        assert_eq!(preview["data"]["can_apply"], true, "{preview}");
        let (status, switched) = call(&router, "PUT", &format!("/api/agent-sessions/{sid}/agent"), json!({
            "selection":selection,"handoff_mode":"context_only",
            "expected_binding_version":preview["data"]["expected_binding_version"]
        })).await;
        assert_eq!(status, StatusCode::OK, "{switched}");
        let binding = store.get_live_session(&session).await.unwrap().agent_binding;
        let selected_workspace = binding.typed_resource_bindings.iter().find(|binding| binding.resource_kind.as_ref() == "workspace").unwrap();
        assert_eq!(selected_workspace.resource_id, source_workspace.resource_id);
        assert_eq!(selected_workspace.typed_parameters, source_workspace.typed_parameters);
        if selection == &minimal_selection {
            assert!(selected_workspace.operations.is_empty(), "keeping a project selection grants no file Actions");
        } else {
            assert_eq!(selected_workspace.operations, source_workspace.operations);
            let process = binding.typed_resource_bindings.iter().find(|binding| binding.resource_kind.as_ref() == "process_session").unwrap();
            assert_eq!(process.typed_parameters, selected_workspace.typed_parameters);
        }
    }

    // A low-level Agent can ignore the old project's assets. Losing that
    // external directory must not block the change or cause us to recreate it.
    // Use a Session that has not started a native host, so the fixture can
    // remove its own project on Windows without a process cwd holding it open.
    let original_session = session;
    let unavailable_workspace = tempfile::tempdir().unwrap();
    let external_root = unavailable_workspace.path().to_path_buf();
    let (status, unavailable) = call(&router, "POST", "/api/agent-sessions", json!({
        "preset_id":source["data"]["preset"]["preset_id"], "model":model,
        "workspace":unavailable_workspace.path().to_str().unwrap(),
        "resource_selections":[
            {"resource_kind":"workspace","resource_id":"default-workspace"},
            {"resource_kind":"process_session","resource_id":"managed-process-session"},
            {"resource_kind":"project_memory","resource_id":"default-project-memory"}
        ]
    })).await;
    assert_eq!(status, StatusCode::OK, "{unavailable}");
    let sid = unavailable["data"]["agent_session_id"].as_str().unwrap();
    let session = AgentSessionId::from(sid.to_owned());
    unavailable_workspace.close().unwrap();
    let (status, preview) = call(&router, "POST", &format!("/api/agent-sessions/{sid}/agent-switch/preview"),
        json!({"selection":minimal_selection})).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["data"]["can_apply"], true, "{preview}");
    let (status, minimal) = call(&router, "PUT", &format!("/api/agent-sessions/{sid}/agent"), json!({
        "selection":minimal_selection,"handoff_mode":"context_only",
        "expected_binding_version":preview["data"]["expected_binding_version"]
    })).await;
    assert_eq!(status, StatusCode::OK, "an unavailable unused project must not block a downgrade: {minimal}");
    assert!(store.get_live_session(&session).await.unwrap().agent_binding.typed_resource_bindings.is_empty());

    // Re-selecting the higher Agent supplies its real infrastructure bindings.
    // The managed directory is materialized by the ordinary next-Turn path.
    let (status, preview) = call(&router, "POST", &format!("/api/agent-sessions/{sid}/agent-switch/preview"),
        json!({"selection":coding_selection})).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["data"]["can_apply"], true, "{preview}");
    let (status, coding) = call(&router, "PUT", &format!("/api/agent-sessions/{sid}/agent"), json!({
        "selection":coding_selection,"handoff_mode":"context_only",
        "expected_binding_version":preview["data"]["expected_binding_version"]
    })).await;
    assert_eq!(status, StatusCode::OK, "{coding}");
    let coding_binding = store.get_live_session(&session).await.unwrap().agent_binding;
    assert_eq!(coding_binding.typed_resource_bindings.iter().map(|binding| binding.resource_kind.as_ref()).collect::<BTreeSet<_>>(),
        BTreeSet::from(["workspace", "process_session", "project_memory"]));
    for resource in coding_binding.typed_resource_bindings.iter().filter(|binding|
        matches!(binding.resource_kind.as_ref(), "workspace" | "process_session")) {
        assert_eq!(resource.typed_parameters, nomifun_agent_contracts::managed_workspace_parameters());
    }
    let managed_root = services.work_dir.join("conversations").join(sid);
    assert_eq!(coding["data"]["conversation"]["extra"]["workspace"], managed_root.to_str().unwrap());
    let (status, started) = call(&router, "POST", &format!("/api/agent-sessions/{sid}/turns"), json!({
        "idempotency_key":uuid::Uuid::now_v7().to_string(),
        "input":{"content":format!("请使用 write_file 一次创建 switch-proof.txt，正文准确为 {CONTENT}。完成这一个写入后直接回复 {REPLY}，不要读取文件、执行命令或重复写入。")}
    })).await;
    assert_eq!(status, StatusCode::OK, "{started}");
    let operation = OperationId::from(started["data"]["operation_id"].as_str().unwrap().to_owned());
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let receipt = store.read_turn_receipt(&session, &operation).await.unwrap();
            if receipt.status == TurnReceiptStatus::Completed { break; }
            assert_eq!(receipt.status, TurnReceiptStatus::Running, "{receipt:?}");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("the upgraded Agent must execute and settle its real Workspace write");
    assert_eq!(std::fs::read(managed_root.join("switch-proof.txt")).unwrap(), CONTENT.as_bytes());
    assert!(!external_root.exists(), "switching never recreates an unavailable external project");
    let effects = store.list_effects(&session).await.unwrap();
    assert_eq!(effects.len(), 1);
    assert_eq!(store.list_effects(&original_session).await.unwrap(), source_effects, "the original effect receipt remains unchanged");
    assert!(effects.iter().all(|effect| effect.state == AgentEffectState::Returned));
    assert_eq!(requests.lock().unwrap().len(), 4);
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn started_agent_session_switches_model_then_agent_in_place_with_segmented_history() {
    let client = LocalJsonClient(TRUST);
    const TRUST: &str = "started-session-model-switch";
    const FIRST_REPLY: &str = "FIRST_MODEL_REPLY";
    const SECOND_REPLY: &str = "SECOND_MODEL_REPLY";
    const THIRD_REPLY: &str = "TARGET_AGENT_REPLY";
    const FOURTH_REPLY: &str = "CONFIG_DOWNGRADE_REPLY";



    async fn wait_for_reply(client: &LocalJsonClient<'_>, router: axum::Router, session_id: &str, reply: &str) {
        let mut last_history = Value::Null;
        for _ in 0..600 {
            let (status, history) = client.call(
                router.clone(),
                "GET",
                &format!("/api/agent-sessions/{session_id}/message-history?page_size=100"),
                json!({}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{history}");
            if history["data"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["content"]["content"] == reply)
            {
                return;
            }
            last_history = history;
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let (_, events) = client.call(
            router,
            "GET",
            &format!("/api/agent-sessions/{session_id}/events?after_seq=0&limit=500"),
            json!({}),
        )
        .await;
        let failure = events["data"]["events"]
            .as_array()
            .and_then(|events| {
                events
                    .iter()
                    .rev()
                    .find(|event| event["kind"] == "turn/failed")
            })
            .and_then(|event| event["payload"]["value"]["message"].as_str());
        panic!(
            "reply {reply} did not become durable: failure={failure:?}; history={last_history}"
        );
    }

    let upstream = wiremock::MockServer::start().await;
    let model_requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = Arc::clone(&model_requests);
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let body = request.body_json::<Value>().unwrap();
            let reply = if body.to_string().contains("fourth turn") {
                FOURTH_REPLY
            } else if body.to_string().contains("third turn") {
                THIRD_REPLY
            } else if body["model"] == "model-two" {
                SECOND_REPLY
            } else {
                FIRST_REPLY
            };
            captured.lock().unwrap().push(body);
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(format!(
                    "data: {{\"id\":\"model-switch\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{reply}\"}},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"model-switch\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
                ))
        })
        .mount(&upstream)
        .await;

    let (router, services) = common::build_local_trust_app(TRUST).await;
    let (status, provider_one) = client.call(
        router.clone(),
        "POST",
        "/api/providers",
        json!({
            "platform": "stepfun-plan",
            "name": "Session model one",
            "base_url": format!("{}/v1", upstream.uri()),
            "auth_scheme": "bearer",
            "credentials": { "api_keys": ["fixture-one"] },
            "enabled": true,
            "initial_model": {
                "model": "model-one",
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
    assert_eq!(status, StatusCode::CREATED, "{provider_one}");
    let (status, provider_two) = client.call(
        router.clone(),
        "POST",
        "/api/providers",
        json!({
            "platform": "stepfun-plan",
            "name": "Session model two",
            "base_url": format!("{}/v1", upstream.uri()),
            "auth_scheme": "bearer",
            "credentials": { "api_keys": ["fixture-two"] },
            "enabled": true,
            "initial_model": {
                "model": "model-two",
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
    assert_eq!(status, StatusCode::CREATED, "{provider_two}");
    let first_model = json!({
        "provider_id": provider_one["data"]["provider_id"],
        "model": "model-one"
    });
    let second_model = json!({
        "provider_id": provider_two["data"]["provider_id"],
        "model": "model-two"
    });
    let (status, preset) = client.call(
        router.clone(),
        "POST",
        "/api/agent-presets/from-template/coding.codex",
        json!({
            "display_name": "Model-switch coding Agent",
            "reuse_existing": false,
            "model_route_refs": {},
            "chat_route_records": {},
            "model": first_model
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let preset_id = preset["data"]["preset"]["preset_id"].as_str().unwrap();
    let resources = json!([
        {"resource_kind":"workspace", "resource_id":"default-workspace"},
        {"resource_kind":"process_session", "resource_id":"managed-process-session"},
        {"resource_kind":"project_memory", "resource_id":"default-project-memory"}
    ]);
    let (status, session) = client.call(
        router.clone(),
        "POST",
        "/api/agent-sessions",
        json!({
            "preset_id": preset_id,
            "title": "Keep this conversation",
            "model": first_model,
            "resource_selections": resources
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let session_id = session["data"]["agent_session_id"].as_str().unwrap();
    let (status, initial_projection) = client.call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/projection"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{initial_projection}");
    let workspace = initial_projection["data"]["extra"]["workspace"]
        .as_str()
        .expect("coding Session workspace");
    tokio::fs::create_dir_all(workspace).await.unwrap();

    let first_key = uuid::Uuid::now_v7().to_string();
    let (status, first_turn) = client.call(
        router.clone(),
        "POST",
        &format!("/api/agent-sessions/{session_id}/turns"),
        json!({
            "idempotency_key": first_key,
            "input": { "content": "first turn" }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first_turn}");
    wait_for_reply(&client, router.clone(), session_id, FIRST_REPLY).await;
    let (status, before) = client.call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/projection"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{before}");

    let (status, switched) = client.call(
        router.clone(),
        "PUT",
        &format!("/api/agent-sessions/{session_id}/model"),
        second_model.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{switched}");
    assert_eq!(
        switched["data"]["model"]["provider_id"],
        second_model["provider_id"]
    );
    assert_eq!(switched["data"]["model"]["model"], "model-two");
    for field in [
        "preset_name",
        "instructions",
        "included_skills",
        "enabled_capabilities",
        "enabled_capability_actions",
        "required_resource_kinds",
    ] {
        assert_eq!(
            switched["data"]["agent_snapshot"][field],
            before["data"]["agent_snapshot"][field],
            "model switching must preserve Agent field {field}"
        );
    }
    let before_binding = &before["data"]["agent_snapshot"]["canonical_binding"];
    let switched_binding = &switched["data"]["agent_snapshot"]["canonical_binding"];
    assert_eq!(
        switched_binding["typed_resource_bindings"],
        before_binding["typed_resource_bindings"]
    );
    assert_eq!(
        switched_binding["binding_version"].as_u64(),
        before_binding["binding_version"].as_u64().map(|version| version + 1)
    );
    let (status, repeated) = client.call(
        router.clone(),
        "PUT",
        &format!("/api/agent-sessions/{session_id}/model"),
        second_model.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{repeated}");
    assert_eq!(
        repeated["data"]["agent_snapshot"]["canonical_binding"]["binding_version"],
        switched_binding["binding_version"],
        "reselecting the current model must be idempotent"
    );

    // Normal model configuration changes must be usable on the next Turn
    // without reselecting the same model or rebuilding this conversation.
    let (status, configured_model) = client.call(
        router.clone(), "PUT", "/api/provider-models", json!({
            "provider_id": second_model["provider_id"],
            "model": {
                "model": "model-two", "enabled": true,
                "capabilities": [{
                    "task": "chat", "traits": ["vision_input"],
                    "context_limit": 1000000, "output_limit": 4096,
                    "protocol": "openai.chat_text", "connection_role": "default",
                    "provider_params": {"reasoning_effort": "medium"}
                }]
            }
        }),
    ).await;
    assert_eq!(status, StatusCode::OK, "{configured_model}");
    let (status, replay) = client.call(
        router.clone(), "POST", &format!("/api/agent-sessions/{session_id}/turns"),
        json!({"idempotency_key": first_key, "input": {"content": "first turn"}}),
    ).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["data"]["operation_id"], first_turn["data"]["operation_id"]);
    assert_eq!(model_requests.lock().unwrap().len(), 1, "redelivery must not call a new model");
    let (status, conflict) = client.call(
        router.clone(), "POST", &format!("/api/agent-sessions/{session_id}/turns"),
        json!({"idempotency_key": first_key, "input": {"content": "changed old turn"}}),
    ).await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");

    // The UI warms the selected Session after a model save, before accepting
    // its next task. A warm runtime must not freeze the old canonical snapshot
    // while remembering the new provider revision.
    let (status,warmed)=client.call(router.clone(),"POST",
        &format!("/api/agent-sessions/{session_id}/warmup"),json!({})).await;
    assert_eq!(status,StatusCode::OK,"{warmed}");

    let (status, second_turn) = client.call(
        router.clone(),
        "POST",
        &format!("/api/agent-sessions/{session_id}/turns"),
        json!({
            "idempotency_key": uuid::Uuid::now_v7().to_string(),
            "input": { "content": "second turn" }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second_turn}");
    wait_for_reply(&client, router.clone(), session_id, SECOND_REPLY).await;
    {
        let wire=model_requests.lock().unwrap();
        assert_eq!(wire[1]["reasoning_effort"],"medium","new model parameter must reach the next actual request");
        assert_eq!(wire[1]["max_tokens"],4096);
    }
    let (status,config_refreshed)=client.call(router.clone(),"GET",&format!("/api/agent-sessions/{session_id}/projection"),json!({})).await;
    assert_eq!(status,StatusCode::OK,"{config_refreshed}");
    assert_eq!(config_refreshed["data"]["agent_snapshot"]["canonical_binding"]["binding_version"].as_u64(),
        switched_binding["binding_version"].as_u64().map(|version|version+1));
    assert_eq!(config_refreshed["data"]["agent_snapshot"]["canonical_binding"]["typed_resource_bindings"],
        switched_binding["typed_resource_bindings"]);
    let (status, reasoning) = client.call(
        router.clone(), "PUT",
        &format!("/api/agent-sessions/{session_id}/reasoning-effort"),
        json!({"reasoning_effort": "high"}),
    ).await;
    assert_eq!(status, StatusCode::OK, "{reasoning}");

    // Add one exact, canonical closed Turn with a structured plan so the switch
    // exercises deterministic continue_task export/import without asking a
    // model to author the handoff summary.
    let store = nomifun_agent_session::AgentSessionStore::from_pool(
        services.database.pool().clone(),
    )
    .await
    .unwrap();
    let session_contract = nomifun_agent_contracts::AgentSessionId::from(session_id.to_owned());
    let second_operation = nomifun_agent_contracts::OperationId::from(
        second_turn["data"]["operation_id"].as_str().unwrap().to_owned(),
    );
    let second_facts = store
        .chat_causality_facts(&session_contract, &second_operation)
        .await
        .unwrap();
    assert!(second_facts.event_payloads.values().any(|payload| {
        let event=&payload["event"];
        event["event"]=="execution_budget_prepared" && event["context_window_tokens"]==1_000_000
            && event["max_output_tokens"]==4096
    }),"updated context/output configuration must be effective at the runtime boundary");
    let source_engine_binding = second_facts
        .events
        .iter()
        .filter(|event| {
            event.kind.0 == "runtime/progress-recorded"
                && event.correlation_id.as_ref() == second_operation.as_ref()
        })
        .filter_map(|event| second_facts.event_payloads.get(event.event_id.as_ref()))
        .filter_map(|payload| payload.get("event"))
        .filter_map(|event| {
            serde_json::from_value::<nomifun_agent_runtime::AgentEngineEvent>(event.clone()).ok()
        })
        .find_map(|event| match event {
            nomifun_agent_runtime::AgentEngineEvent::TurnStarted { binding, .. } => Some(binding),
            _ => None,
        })
        .expect("source Turn engine binding");
    assert_eq!(
        source_engine_binding.resolved_snapshot_ref(),
        &store
            .get_live_session(&session_contract)
            .await
            .unwrap()
            .agent_binding
            .resolved_snapshot_ref,
        "the synthetic handoff source must use the exact current binding",
    );
    let handoff_operation = nomifun_agent_contracts::OperationId::from(
        uuid::Uuid::now_v7().to_string(),
    );
    let (_, handoff_started) = store
        .start_turn(
            &session_contract,
            nomifun_agent_contracts::EventProducerId::from("session_api"),
            nomifun_agent_contracts::IdempotencyKey::from(uuid::Uuid::now_v7().to_string()),
            handoff_operation.clone(),
            nomifun_agent_contracts::StrictJsonValue(json!({
                "content": "continue this structured task"
            })),
        )
        .await
        .unwrap();
    let handoff_started_event = handoff_started.ack.unwrap().event_id;
    let handoff_events = vec![
        nomifun_agent_runtime::AgentEngineEvent::TurnStarted {
            binding: source_engine_binding,
            turn_operation_id: handoff_operation.clone(),
        },
        nomifun_agent_runtime::AgentEngineEvent::PlanUpdated {
            plan: nomifun_agent_runtime::AgentPlan {
                revision: 1,
                explanation: "Continue after the Agent boundary".to_owned(),
                exact_actions:Vec::new(),
                steps: vec![nomifun_agent_runtime::AgentPlanStep {
                    step: "Re-read and verify the current workspace".to_owned(),
                    status: nomifun_agent_runtime::AgentPlanStatus::Pending,
                }],
                needs_replan: true,
                requirements: vec![nomifun_agent_runtime::AgentTaskRequirement {
                    id: "req-handoff".to_owned(),
                    description: "Keep the exact requirement across Agents".to_owned(),
                    source: nomifun_agent_runtime::AgentInputCitation {
                        input: 0,
                        quote: "continue this structured task".to_owned(),
                    },
                    origin: None,
                }],
            },
        },
        nomifun_agent_runtime::AgentEngineEvent::TurnCompleted {
            model_steps: 1,
            finish_reason: nomifun_chat_model_broker::ChatFinishReason::Completed,
        },
    ];
    let mut handoff_cause = handoff_started_event;
    for (index, event) in handoff_events.into_iter().enumerate() {
        let event_id = nomifun_agent_contracts::EventId::from(format!(
            "handoff-progress:{}:{index}",
            handoff_operation.as_ref()
        ));
        store
            .append_event(&nomifun_agent_contracts::SessionEventAppend {
                agent_session_id: session_contract.clone(),
                event_id: event_id.clone(),
                producer_id: nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
                idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(format!(
                    "handoff-progress:{}:{index}",
                    handoff_operation.as_ref()
                )),
                semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                    kind: nomifun_agent_contracts::SessionEventKind(
                        "runtime/progress-recorded".to_owned(),
                    ),
                    kind_version: 1,
                    correlation_id: nomifun_agent_contracts::CorrelationId::from(
                        handoff_operation.as_ref().to_owned(),
                    ),
                    causation_event_id: Some(handoff_cause),
                    payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                        nomifun_agent_contracts::StrictJsonValue(json!({
                            "runtime_binding_id": format!("nomi:{}", session_id),
                            "producer_seq": index + 1,
                            "event": event,
                        })),
                    ),
                },
            })
            .await
            .unwrap();
        handoff_cause = event_id;
    }
    store
        .append_turn_terminal(
            &nomifun_agent_contracts::SessionEventAppend {
                agent_session_id: session_contract.clone(),
                event_id: nomifun_agent_contracts::EventId::from(format!(
                    "handoff-terminal:{}",
                    handoff_operation.as_ref()
                )),
                producer_id: nomifun_agent_contracts::EventProducerId::from(
                    "runtime_supervisor",
                ),
                idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(format!(
                    "handoff-terminal:{}",
                    handoff_operation.as_ref()
                )),
                semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                    kind: nomifun_agent_contracts::SessionEventKind("turn/completed".to_owned()),
                    kind_version: 1,
                    correlation_id: nomifun_agent_contracts::CorrelationId::from(
                        handoff_operation.as_ref().to_owned(),
                    ),
                    causation_event_id: Some(handoff_cause),
                    payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                        nomifun_agent_contracts::StrictJsonValue(json!({})),
                    ),
                },
            },
            &handoff_operation,
        )
        .await
        .unwrap();

    let (status, target_preset) = client.call(
        router.clone(),
        "POST",
        "/api/agent-presets/from-template/chat.minimal",
        json!({
            "display_name": "Target handoff Agent",
            "reuse_existing": false,
            "model_route_refs": {},
            "chat_route_records": {},
            "model": second_model
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{target_preset}");
    let target_preset_id = target_preset["data"]["preset"]["preset_id"]
        .as_str()
        .unwrap();
    let selection = json!({ "kind": "preset", "preset_id": target_preset_id });
    let (status, preview) = client.call(
        router.clone(),
        "POST",
        &format!("/api/agent-sessions/{session_id}/agent-switch/preview"),
        json!({ "selection": selection }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["data"]["can_apply"], true, "{preview}");
    assert_eq!(preview["data"]["model"]["preserved"], true);
    assert_eq!(preview["data"]["handoff"]["completion_gate_inherited"], false);
    assert_eq!(preview["data"]["handoff"]["available"], true);
    assert_eq!(preview["data"]["handoff"]["requirement_count"], 1);
    let expected_binding_version = preview["data"]["expected_binding_version"]
        .as_u64()
        .unwrap();
    let switch_request = json!({
        "selection": selection,
        "handoff_mode": "continue_task",
        "expected_binding_version": expected_binding_version
    });
    let switch_key = uuid::Uuid::now_v7().to_string();
    let (status, switched_agent) = client.call_with_idempotency(
        router.clone(),
        "PUT",
        &format!("/api/agent-sessions/{session_id}/agent"),
        switch_request.clone(),
        &switch_key,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{switched_agent}");
    assert_eq!(switched_agent["data"]["conversation"]["conversation_id"], session_id);
    assert_eq!(switched_agent["data"]["conversation"]["name"], "Keep this conversation");
    assert_eq!(switched_agent["data"]["effective_from"], "next_turn");
    assert_eq!(switched_agent["data"]["handoff"]["available"], true);
    assert_eq!(switched_agent["data"]["handoff"]["completion_gate_inherited"], false);
    assert_eq!(switched_agent["data"]["transition_id"], switch_key);
    assert_eq!(
        switched_agent["data"]["binding_version"],
        expected_binding_version + 1
    );
    let (status, switched_capabilities) = client.call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/capabilities"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{switched_capabilities}");
    assert_eq!(switched_capabilities["data"]["generation"], 1);
    assert_eq!(
        switched_capabilities["data"]["state_source"],
        "canonical_agent_store"
    );

    let (status, replayed_switch) = client.call_with_idempotency(
        router.clone(),
        "PUT",
        &format!("/api/agent-sessions/{session_id}/agent"),
        switch_request,
        &switch_key,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replayed_switch}");
    assert_eq!(replayed_switch["data"]["transition_id"], switch_key);
    assert_eq!(
        replayed_switch["data"]["binding_version"],
        switched_agent["data"]["binding_version"]
    );

    let (status, raw_messages_after_switch) = client.call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/messages?after_seq=0&limit=500"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw_messages_after_switch}");
    assert!(
        raw_messages_after_switch["data"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["presentation_intent"] == "agent_transition"),
        "{raw_messages_after_switch}"
    );

    let (status, history_after_switch) = client.call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/message-history?page_size=100"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{history_after_switch}");
    let history_items = history_after_switch["data"]["items"].as_array().unwrap();
    assert!(history_items.iter().any(|message| message["content"]["content"] == FIRST_REPLY));
    assert!(history_items.iter().any(|message| message["content"]["content"] == SECOND_REPLY));
    assert!(
        history_items.iter().any(|message| {
            message["type"] == "tips"
                && message["content"]["agent_transition"]["effective_from"] == "next_turn"
                && message["content"]["agent_transition"]["next_preset_id"] == target_preset_id
        }),
        "{history_after_switch}"
    );

    let (status, third_turn) = client.call(
        router.clone(),
        "POST",
        &format!("/api/agent-sessions/{session_id}/turns"),
        json!({
            "idempotency_key": uuid::Uuid::now_v7().to_string(),
            "input": { "content": "third turn" }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{third_turn}");
    wait_for_reply(&client, router.clone(), session_id, THIRD_REPLY).await;

    let (status, events_after_switch) = client.call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/events?after_seq=0&limit=500"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{events_after_switch}");
    let transition = events_after_switch["data"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["kind"] == "session/agent-binding-changed")
        .expect("canonical Agent transition event");
    assert_eq!(transition["payload"]["value"]["effective_after_seq"].is_u64(), true);
    let target_snapshot = switched_agent["data"]["conversation"]["agent_snapshot"]
        ["canonical_binding"]["resolved_snapshot_ref"]["snapshot_digest"]
        .clone();
    assert!(events_after_switch["data"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["kind"] == "runtime/progress-recorded")
        .filter_map(|event| event["payload"]["value"]["event"]["binding"]
            ["resolved_snapshot_ref"]["snapshot_digest"].as_str())
        .any(|digest| target_snapshot == digest));

    let requests = model_requests.lock().unwrap();
    let models = requests
        .iter()
        .map(|request| request["model"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(models, vec!["model-one", "model-two", "model-two"]);
    assert_eq!(requests[2]["reasoning_effort"], "high",
        "the explicit Session intelligence setting must reach the actual request");
    assert!(
        requests[1].to_string().contains(FIRST_REPLY),
        "the replacement Runtime must continue from the same durable history"
    );
    let target_request = requests[2].to_string();
    assert!(target_request.contains(FIRST_REPLY));
    assert!(target_request.contains(SECOND_REPLY));
    assert!(target_request.contains("Historical cross-Agent handoff"));
    assert!(target_request.contains("Keep the exact requirement across Agents"));
    assert!(target_request.contains("not inherited by the target completion gate"));
    assert!(
        !requests[2]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "tool"),
        "pre-transition tool authority must not enter target-Agent history"
    );
    drop(requests);

    let (status, minimal_preview) = client.call(
        router.clone(),
        "POST",
        &format!("/api/agent-sessions/{session_id}/agent-switch/preview"),
        json!({ "selection": { "kind": "template", "template_key": "chat.minimal" } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{minimal_preview}");
    assert_eq!(minimal_preview["data"]["can_apply"], true);
    let (status, minimal_switch) = client.call_with_idempotency(
        router.clone(),
        "PUT",
        &format!("/api/agent-sessions/{session_id}/agent"),
        json!({
            "selection": { "kind": "template", "template_key": "chat.minimal" },
            "handoff_mode": "context_only",
            "expected_binding_version": minimal_preview["data"]["expected_binding_version"],
        }),
        &uuid::Uuid::now_v7().to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{minimal_switch}");
    assert_eq!(minimal_switch["data"]["conversation"]["extra"]["official_template_key"], "chat.minimal");
    let minimal_preset_id = minimal_switch["data"]["conversation"]["preset_id"].as_str().unwrap();
    let (status, localized_history) = client.call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/message-history?page_size=100"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{localized_history}");
    assert!(localized_history["data"]["items"].as_array().unwrap().iter().any(|message| {
        message["content"]["agent_transition"]["next_preset_id"] == minimal_preset_id
            && message["content"]["agent_transition"]["next_template_key"] == "chat.minimal"
    }), "{localized_history}");

    // Removing reasoning support must also remove the incompatible Session
    // override. Unsetting the output limit must also use provider defaults,
    // including when the UI warms a host before the next Turn.
    let (status, downgraded_model) = client.call(
        router.clone(), "PUT", "/api/provider-models", json!({
            "provider_id": second_model["provider_id"],
            "model": {
                "model": "model-two", "enabled": true,
                "capabilities": [{
                    "task": "chat", "traits": [],
                    "context_limit": 1000000,
                    "protocol": "openai.chat_text", "connection_role": "default",
                    "provider_params": {}
                }]
            }
        }),
    ).await;
    assert_eq!(status, StatusCode::OK, "{downgraded_model}");
    // Technical reasoning support is provider-observed health, not a user
    // Chat input trait. Supply that host fact in this isolated fixture.
    {
        use nomifun_db::{IProviderModelCapabilityRepository, SqliteProviderModelCapabilityRepository};
        let provider_id = second_model["provider_id"].as_str().unwrap();
        let revision: i64 = sqlx::query_scalar("SELECT config_revision FROM providers WHERE provider_id = ?")
            .bind(provider_id).fetch_one(services.database.pool()).await.unwrap();
        assert!(SqliteProviderModelCapabilityRepository::new(services.database.pool().clone())
            .mark_technical_capability_unsupported(provider_id, revision, "model-two", "chat", "reasoning")
            .await.unwrap());
    }
    let (status,warmed_default)=client.call(router.clone(),"POST",
        &format!("/api/agent-sessions/{session_id}/warmup"),json!({})).await;
    assert_eq!(status,StatusCode::OK,"{warmed_default}");
    let (status, fourth_turn) = client.call(
        router.clone(), "POST", &format!("/api/agent-sessions/{session_id}/turns"),
        json!({"idempotency_key": uuid::Uuid::now_v7().to_string(), "input": {"content": "fourth turn"}}),
    ).await;
    assert_eq!(status, StatusCode::OK, "{fourth_turn}");
    wait_for_reply(&client, router.clone(), session_id, FOURTH_REPLY).await;
    let (status, downgraded_projection) = client.call(
        router.clone(), "GET", &format!("/api/agent-sessions/{session_id}/projection"), json!({}),
    ).await;
    assert_eq!(status, StatusCode::OK, "{downgraded_projection}");
    assert!(downgraded_projection["data"]["reasoning_effort"].is_null());
    {
        let requests = model_requests.lock().unwrap();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[3]["model"], "model-two");
        assert!(requests[3].get("max_tokens").is_none(),"provider default must reach the actual wire without a synthetic 4096 cap");
        assert!(requests[3].get("reasoning_effort").is_none(),
            "removed reasoning capability must be reflected on the next actual request");
    }

    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

async fn wait_knowledge_turn_terminal_and_native_idle(services:&AppServices,session_id:&str,accepted_turn:&Value) {
    let session=nomifun_agent_contracts::AgentSessionId::from(session_id.to_owned());
    let operation=nomifun_agent_contracts::OperationId::from(accepted_turn["data"]["operation_id"]
        .as_str().expect("the accepted Knowledge input returns its exact canonical operation").to_owned());
    let store=nomifun_agent_session::AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    let settled=tokio::time::timeout(std::time::Duration::from_secs(15),async {loop {
        let receipt=store.read_turn_receipt(&session,&operation).await.unwrap();
        if receipt.terminal_event.is_some() {
            assert_eq!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Completed,
                "the exact Knowledge Turn failed before its idle boundary: {receipt:?}");
        }
        let head=store.head(&session).await.unwrap();
        let native_idle=services.agent_runtime_sessions.active_turn_generation(session_id).is_none()
            &&match services.agent_runtime_sessions.get_runtime(session_id) {
                Some(runtime)=>runtime.is_transport_healthy()&&matches!(runtime.status(),None|Some(
                    nomifun_common::ConversationStatus::Pending|nomifun_common::ConversationStatus::Finished)),
                None=>!services.agent_runtime_sessions.has_owned_runtime(session_id),
            };
        if receipt.terminal_event.is_some()&&head.status=="ready"&&head.active_turn_id.is_none()&&native_idle {break;}
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }}).await;
    assert!(settled.is_ok(),"exact Knowledge Turn {operation:?} did not release its canonical/native owner: receipt={:?}, head={:?}, native_generation={:?}, runtime_status={:?}",
        store.read_turn_receipt(&session,&operation).await.unwrap(),store.head(&session).await.unwrap(),
        services.agent_runtime_sessions.active_turn_generation(session_id),
        services.agent_runtime_sessions.get_runtime(session_id).and_then(|runtime|runtime.status()));
}

#[tokio::test]
async fn canonical_session_updates_knowledge_retrieves_on_demand_and_writes_back_automatically() {
    let client = LocalJsonClient(TRUST);
    const TRUST: &str = "canonical-knowledge-tool";
    const QUERY: &str = "NOMIFUN_KNOWLEDGE_QUERY_9233";
    const RESULT_MARKER: &str = "KNOWLEDGE_TOOL_RESULT_9233";
    const REPLY: &str = "KNOWLEDGE_SEARCH_WAS_USED";
    const WRITEBACK_MARKER: &str = "AUTOMATIC_WRITEBACK_WAS_USED";
    const DISABLED_REPLY: &str = "KNOWLEDGE_WAS_DISABLED";
    const READ_ONLY_REPLY: &str = "KNOWLEDGE_WAS_READ_ONLY";


    let upstream = wiremock::MockServer::start().await;
    let model_requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = Arc::clone(&model_requests);
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let body = request.body_json::<Value>().unwrap();
            captured.lock().unwrap().push(body.clone());
            let body_text = body.to_string();
            if body_text.contains("knowledge-base curator for NomiFun") {
                let prompt = body["messages"]
                    .as_array()
                    .and_then(|messages| messages.last())
                    .and_then(|message| message["content"].as_str())
                    .expect("write-back request must carry the extraction prompt");
                let marker = "- kb_id: ";
                let start = prompt.find(marker).expect("mounted kb_id") + marker.len();
                let kb_id = &prompt[start..start + 36];
                let content = json!({
                    "candidates": [{
                        "kb_id": kb_id,
                        "rel_path": "patterns/automatic-writeback.md",
                        "content": format!("# Durable result\n\n{WRITEBACK_MARKER}")
                    }]
                })
                .to_string();
                let frame = json!({
                    "id": "knowledge-writeback-round",
                    "choices": [{
                        "index": 0,
                        "delta": {"content": content},
                        "finish_reason": null
                    }]
                });
                let done = json!({
                    "id": "knowledge-writeback-round",
                    "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
                });
                return wiremock::ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(format!(
                        "data: {frame}\n\ndata: {done}\n\ndata: [DONE]\n\n"
                    ));
            }
            let messages = body["messages"].as_array().unwrap();
            let tool_results = messages
                .iter()
                .filter(|message| message["role"] == "tool")
                .collect::<Vec<_>>();
            let search_tool = body["tools"].as_array().and_then(|tools| {
                tools.iter().find(|tool| {
                    tool["function"]["description"]
                        .as_str()
                        .is_some_and(|description| {
                            description.contains("Action: knowledge/search")
                        })
                })
            });
            let write_tool_available = body["tools"].as_array().is_some_and(|tools| {
                tools.iter().any(|tool| {
                    tool["function"]["description"]
                        .as_str()
                        .is_some_and(|description| {
                            description.contains("Action: knowledge/write")
                        })
                })
            });
            if search_tool.is_none() {
                let frame = json!({
                    "id": "knowledge-disabled-round",
                    "choices": [{
                        "index": 0,
                        "delta": {"content": DISABLED_REPLY},
                        "finish_reason": null
                    }]
                });
                let done = json!({
                    "id": "knowledge-disabled-round",
                    "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
                });
                return wiremock::ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(format!(
                        "data: {frame}\n\ndata: {done}\n\ndata: [DONE]\n\n"
                    ));
            }
            if !write_tool_available {
                let frame = json!({
                    "id": "knowledge-read-only-round",
                    "choices": [{
                        "index": 0,
                        "delta": {"content": READ_ONLY_REPLY},
                        "finish_reason": null
                    }]
                });
                let done = json!({
                    "id": "knowledge-read-only-round",
                    "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
                });
                return wiremock::ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(format!(
                        "data: {frame}\n\ndata: {done}\n\ndata: [DONE]\n\n"
                    ));
            }
            let (frame, finish_reason, response_id) = if tool_results.is_empty() {
                let search = search_tool.expect("Knowledge search tool");
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

    let (status, provider) = client.call(
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
    let (status, minimal) = client.call(
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
    let (status, read_only_preset) = client.call(
        router.clone(),
        "POST",
        "/api/agent-presets",
        json!({
            "display_name": "Read-only Knowledge Agent",
            "description": "Canonical Knowledge regression fixture",
            "document": document.clone()
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{read_only_preset}");
    let read_only_preset_id = read_only_preset["data"]["preset"]["preset_id"]
        .as_str()
        .unwrap();
    let (status, rejected_writeback) = client.call(
        router.clone(),
        "POST",
        "/api/agent-sessions",
        json!({
            "preset_id": read_only_preset_id,
            "model": model,
            "resource_selections": [{
                "resource_kind": "knowledge_base",
                "resource_id": base_a.knowledge_base_id
            }],
            "knowledge_policy": {
                "writeback": true,
                "writeback_eagerness": "auto"
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{rejected_writeback}");
    assert_eq!(rejected_writeback["code"], "KNOWLEDGE_WRITEBACK_NOT_AVAILABLE");

    document["enabled_capabilities"] = json!([{
        "capability": {"id": "knowledge"},
        "action_allowlist": [
            "knowledge/autogen",
            "knowledge/read",
            "knowledge/search",
            "knowledge/write"
        ]
    }]);
    let (status, preset) = client.call(
        router.clone(),
        "POST",
        "/api/agent-presets",
        json!({
            "display_name": "Write-back Knowledge Agent",
            "description": "Canonical Knowledge write-back fixture",
            "document": document
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let preset_id = preset["data"]["preset"]["preset_id"].as_str().unwrap();

    let (status, session) = client.call(
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
            ],
            "knowledge_policy": {
                "writeback": true,
                "writeback_eagerness": "auto"
            }
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
    assert!(resources.iter().all(|resource| {
        resource["typed_parameters"]["knowledge_enabled"] == "true"
            && resource["typed_parameters"]["knowledge_writeback"] == "true"
            && resource["typed_parameters"]["knowledge_writeback_eagerness"] == "auto"
    }));

    let session_id = session["data"]["agent_session_id"].as_str().unwrap();
    let (status, consumers) = client.call(
        router.clone(),
        "GET",
        &format!(
            "/api/knowledge/bases/{}/consumers",
            base_a.knowledge_base_id
        ),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{consumers}");
    assert!(consumers["data"].as_array().unwrap().iter().any(|consumer| {
        consumer["target_kind"] == "conversation"
            && consumer["target_id"] == session_id
            && consumer["enabled"] == true
    }));
    let (status, mounted_delete) = client.call(
        router.clone(),
        "DELETE",
        &format!(
            "/api/knowledge/bases/{}",
            base_a.knowledge_base_id
        ),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{mounted_delete}");
    let (status, projection) = client.call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/projection"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{projection}");
    assert_eq!(
        projection["data"]["agent_snapshot"]["knowledge_policy"],
        json!({
            "enabled": true,
            "writeback": true,
            "eagerness": "auto",
            "grounded": true
        })
    );
    let (status, disabled) = client.call(
        router.clone(),
        "PUT",
        &format!("/api/agent-sessions/{session_id}/knowledge"),
        json!({
            "enabled": false,
            "writeback": false,
            "writeback_eagerness": "manual",
            "kb_ids": [base_a.knowledge_base_id, base_b.knowledge_base_id]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{disabled}");
    assert_eq!(disabled["data"]["enabled"], false);
    assert_eq!(disabled["data"]["kb_ids"].as_array().unwrap().len(), 2);
    let (status, disabled_projection) = client.call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/projection"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{disabled_projection}");
    assert_eq!(
        disabled_projection["data"]["agent_snapshot"]["knowledge_policy"],
        json!({
            "enabled": false,
            "writeback": false,
            "grounded": false
        })
    );

    let (status, manual) = client.call(
        router.clone(),
        "PUT",
        &format!("/api/agent-sessions/{session_id}/knowledge"),
        json!({
            "enabled": true,
            "writeback": true,
            "writeback_eagerness": "manual",
            "kb_ids": [base_a.knowledge_base_id, base_b.knowledge_base_id]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{manual}");
    assert_eq!(manual["data"]["writeback_eagerness"], "manual");

    let (status, automatic) = client.call(
        router.clone(),
        "PUT",
        &format!("/api/agent-sessions/{session_id}/knowledge"),
        json!({
            "enabled": true,
            "writeback": true,
            "writeback_eagerness": "auto",
            "kb_ids": [base_a.knowledge_base_id, base_b.knowledge_base_id]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{automatic}");
    assert_eq!(automatic["data"]["writeback"], true);
    assert_eq!(automatic["data"]["writeback_eagerness"], "auto");
    let (status, retired_binding) = client.call(
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

    let (status, turn) = client.call(
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
            let (status, history) = client.call(
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
        let (_, events) = client.call(
            router.clone(),
            "GET",
            &format!("/api/agent-sessions/{session_id}/events?after_seq=0&limit=200"),
            json!({}),
        )
        .await;
        let (_, history) = client.call(
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

    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while model_requests.lock().unwrap().len() < 4 {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("automatic write-back extraction must run before the turn becomes idle");

    let requests = model_requests.lock().unwrap();
    assert_eq!(requests.len(), 4, "search, read, final, and automatic write-back rounds are required");
    let first = &requests[0];
    let prompt = first["messages"].to_string();
    assert!(prompt.contains("Use knowledge/search and knowledge/read when the current task needs facts"), "{first}");
    assert!(prompt.contains("write-back is enabled in automatic mode"), "{first}");
    assert!(prompt.contains("Python handbook"), "{first}");
    assert!(prompt.contains("Engineering notes"), "{first}");
    assert!(!prompt.contains("nomifun_knowledge_retrieval"), "{first}");
    assert!(!prompt.contains(RESULT_MARKER), "document bodies must wait for a real tool request: {first}");
    assert!(first["tools"].as_array().unwrap().iter().any(|tool| {
        tool["function"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("Action: knowledge/search"))
    }));
    assert!(first["tools"].as_array().unwrap().iter().any(|tool| {
        tool["function"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("Action: knowledge/write"))
    }));
    assert!(requests[1].to_string().contains("knowledge-search-call"), "{}", requests[1]);
    assert!(requests[1].to_string().contains(RESULT_MARKER), "actual search results must reach the model: {}", requests[1]);
    assert!(requests[2].to_string().contains(RESULT_MARKER), "{}", requests[2]);
    assert!(requests[3].to_string().contains(REPLY), "{}", requests[3]);
    drop(requests);

    let written = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if let Ok(written) = services
                .knowledge_service
                .read_file(
                    base_a.knowledge_base_id.as_str(),
                    "patterns/automatic-writeback.md",
                )
                .await
            {
                break written;
            }
            if let Ok(written) = services
                .knowledge_service
                .read_file(
                    base_b.knowledge_base_id.as_str(),
                    "patterns/automatic-writeback.md",
                )
                .await
            {
                break written;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("automatic write-back must publish a durable Knowledge file");
    assert!(written.content.contains(WRITEBACK_MARKER));

    wait_knowledge_turn_terminal_and_native_idle(&services,session_id,&turn).await;
    let (status, disabled_after_runtime) = client.call(
        router.clone(),
        "PUT",
        &format!("/api/agent-sessions/{session_id}/knowledge"),
        json!({
            "enabled": false,
            "writeback": false,
            "writeback_eagerness": "manual",
            "kb_ids": [base_a.knowledge_base_id, base_b.knowledge_base_id]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{disabled_after_runtime}");
    let (status, disabled_turn) = client.call(
        router.clone(),
        "POST",
        &format!("/api/agent-sessions/{session_id}/turns"),
        json!({
            "idempotency_key": uuid::Uuid::now_v7().to_string(),
            "input": {"content": "回答一个不应读取知识库的问题"}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{disabled_turn}");
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let (status, history) = client.call(
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
                .any(|message| message["content"]["content"] == DISABLED_REPLY)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("Knowledge-disabled reply must become durable");
    wait_knowledge_turn_terminal_and_native_idle(&services,session_id,&disabled_turn).await;
    let requests = model_requests.lock().unwrap();
    let disabled_request = requests.last().unwrap();
    assert!(!disabled_request.to_string().contains("nomifun_knowledge_retrieval"));
    assert!(disabled_request["tools"].as_array().is_none_or(|tools| {
        !tools.iter().any(|tool| {
            tool["function"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("Action: knowledge/"))
        })
    }));
    drop(requests);

    let (status, read_only) = client.call(
        router.clone(),
        "PUT",
        &format!("/api/agent-sessions/{session_id}/knowledge"),
        json!({
            "enabled": true,
            "writeback": false,
            "writeback_eagerness": "manual",
            "kb_ids": [base_a.knowledge_base_id, base_b.knowledge_base_id]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{read_only}");
    let (status, read_only_turn) = client.call(
        router.clone(),
        "POST",
        &format!("/api/agent-sessions/{session_id}/turns"),
        json!({
            "idempotency_key": uuid::Uuid::now_v7().to_string(),
            "input": {"content": format!("再次查询 {QUERY}")}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{read_only_turn}");
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let (_, history) = client.call(
                router.clone(),
                "GET",
                &format!("/api/agent-sessions/{session_id}/message-history?page_size=50"),
                json!({}),
            )
            .await;
            if history["data"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["content"]["content"] == READ_ONLY_REPLY)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("read-only Knowledge reply must become durable");
    wait_knowledge_turn_terminal_and_native_idle(&services,session_id,&read_only_turn).await;
    let requests = model_requests.lock().unwrap();
    let read_only_request = requests.last().unwrap();
    assert!(!read_only_request.to_string().contains("nomifun_knowledge_retrieval"));
    let tools = read_only_request["tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| {
        tool["function"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("Action: knowledge/search"))
    }));
    assert!(!tools.iter().any(|tool| {
        tool["function"]["description"]
            .as_str()
            .is_some_and(|description| {
                description.contains("Action: knowledge/write")
                    || description.contains("Action: knowledge/autogen")
            })
    }));
    drop(requests);

    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[path = "common/mod.rs"]
mod common;
#[path = "support/model_management.rs"]
mod model_management;
#[path = "support/session_contract_evolution.rs"]
mod session_contract_evolution;

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
async fn installation_token_is_limited_to_the_app_hosting_control_plane() {
    let trust_secret = "headless-product-local-trust";
    let installation_token = "headless-product-installation-token";
    let (router, services) = common::build_local_trust_app(trust_secret).await;
    services
        .instance_token_validator
        .set_token(nomifun_auth::token_sha256_hex(installation_token));

    for path in ["/api/apps"] {
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
                .uri("/api/apps/import")
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
        ("/api/apps", "wrong-installation-token", StatusCode::FORBIDDEN),
        ("/api/apps", owner_jwt.as_str(), StatusCode::OK),
        ("/api/capabilities", installation_token, StatusCode::FORBIDDEN),
        ("/api/webui/access-token", installation_token, StatusCode::FORBIDDEN),
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

    for local_proof in [None, Some("wrong-local-trust"), Some(trust_secret)] {
        let mut request = Request::builder().uri("/api/apps");
        if let Some(proof) = local_proof {
            request = request.header("x-nomi-local-trust", proof);
        }
        let response = router.clone().oneshot(
            request.body(Body::empty()).expect("build local-trust application request"),
        ).await.expect("dispatch local-trust application request");
        assert_eq!(
            response.status(),
            if local_proof == Some(trust_secret) { StatusCode::OK } else { StatusCode::FORBIDDEN },
            "only the current Host's LocalTrust proof grants application access",
        );
    }

    let source = tempfile::tempdir().expect("create application source fixture");
    fs::write(source.path().join("index.html"), "<!doctype html><title>Ordinary application</title>")
        .expect("write ordinary application fixture");
    let imported = router.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/api/apps/import")
            .header("authorization", format!("Bearer {installation_token}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&json!({
                "source": source.path(), "import_kind": "artifact", "name": "Ordinary application",
            })).expect("serialize application import")))
            .expect("build authenticated application import"),
    ).await.expect("dispatch authenticated application import");
    assert_eq!(imported.status(), StatusCode::OK, "the supported installation bearer must reach the real hosting owner");
    let imported_body = axum::body::to_bytes(imported.into_body(), 4 * 1024 * 1024)
        .await.expect("read imported application response");
    let imported_value: Value = serde_json::from_slice(&imported_body).expect("imported application JSON");
    let app_id = imported_value["data"]["id"].as_str().expect("real application identity");
    let application = services.app_hosting.get(app_id).await.expect("get actual imported application");
    assert_eq!(application.state, nomifun_app_hosting::AppState::Stopped);
    assert!(source.path().join("index.html").is_file(), "import preserves the original source");

    let deleted = router.clone().oneshot(
        Request::builder()
            .method("DELETE")
            .uri(format!("/api/apps/{app_id}"))
            .header("x-nomi-local-trust", trust_secret)
            .body(Body::empty())
            .expect("build trusted application cleanup"),
    ).await.expect("dispatch trusted application cleanup");
    assert_eq!(deleted.status(), StatusCode::OK, "Host LocalTrust retains application control-plane grants");
    assert!(source.path().join("index.html").is_file(), "cleanup preserves the original source");

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
        ("workspace.process", "workspace.process/start"),
        ("workspace.vcs", "workspace.vcs/commit"),
        ("agent.collaboration", "agent/delegate"),
        ("requirements", "requirements/read"),
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
    assert!(general_enabled.iter().all(|selection| selection["capability"]["id"] != "agent.tool-discovery"),
        "tool discovery is automatic runtime infrastructure, not an authored grant");
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
        ("workspace.process", "workspace.process/start"),
        ("workspace.artifacts", "workspace.artifacts/publish"),
        ("project.memory", "project.memory/read"),
        ("agent.collaboration", "agent/delegate"),
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
    let client = LocalJsonClient(TRUST);
    const TRUST: &str = "configured-agent-workbench-test";
    let (router, services) = common::build_local_trust_app(TRUST).await;
    create_chat_provider(&router, TRUST, "Configured Agent fixture", "configured-chat", 0).await;
    let (_, before) = client.call(router.clone(), "GET", "/api/agent-preset-templates?source=official", json!({})).await;
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
    let (status, created) = client.call(router.clone(), "POST", "/api/agent-presets", json!({
        "display_name":"My adjusted assistant", "description":"Custom capability scope", "document":document,
    })).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["data"]["preset"]["source"], "user");
    assert_eq!(created["data"]["revision"]["reference"]["revision"], 1);
    assert!(created["data"]["draft"]["source_template_key"].is_null());
    let id = created["data"]["preset"]["preset_id"].as_str().unwrap();
    let (status, reloaded) = client.call(router.clone(), "GET", &format!("/api/agent-presets/{id}/editor"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reloaded["data"]["draft"]["document"]["enabled_capabilities"], created["data"]["revision"]["document"]["enabled_capabilities"]);
    assert_eq!(reloaded["data"]["draft"]["document"]["enabled_capabilities"].as_array().unwrap().len(), 3);
    let mut invalid = document;
    invalid["enabled_capabilities"] = json!([{"capability":{"id":"missing.capability"}}]);
    let (status, _) = client.call(router.clone(), "POST", "/api/agent-presets", json!({"display_name":"Must not persist", "document":invalid})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, after) = client.call(router, "GET", "/api/agent-preset-templates?source=official", json!({})).await;
    assert_eq!(after["data"]["user_presets"].as_array().unwrap().len(), before_count + 1);
    assert_eq!(after["data"]["official_templates"], official_before);
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn official_template_creation_requires_explicit_persistence_intent() {
    const TRUST: &str = "official-agent-explicit-intent";
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let (status, before) = LocalJsonClient(TRUST).call(
        router.clone(), "GET", "/api/agent-preset-templates", json!({}),
    ).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    let presets_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_presets")
        .fetch_one(services.database.pool()).await.unwrap();
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
    assert_eq!(library["data"]["official_presets"], before["data"]["official_presets"]);
    let persisted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_presets")
        .fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(persisted, presets_before, "rejected ambiguous requests must not write hidden or visible configurations");
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn official_agent_read_surfaces_load_on_the_full_product_graph_without_a_model() {
    let client = LocalJsonClient(TRUST);
    const TRUST: &str = "official-agent-read-surfaces";
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let (status, catalog) = client.call(router.clone(), "GET", "/api/agent-catalog", json!({})).await;
    assert_eq!(status, StatusCode::OK, "full host catalog failed: {catalog}");
    let modules = catalog["data"]["modules"].as_array().unwrap();
    assert!(!modules.is_empty(), "the product fixture must use the real module catalog");
    let (status, library) = client.call(router.clone(), "GET", "/api/agent-preset-templates?source=official", json!({})).await;
    assert_eq!(status, StatusCode::OK, "official Agent library failed: {library}");
    for (key, preset) in library["data"]["official_presets"].as_object().unwrap() {
        let id = preset["preset_id"].as_str().unwrap();
        let (status, editor) = client.call(router.clone(), "GET", &format!("/api/agent-presets/{id}/editor"), json!({})).await;
        assert_eq!(status, StatusCode::OK, "official {key} capability defaults failed: {editor}");
        assert_eq!(editor["data"]["draft"]["source_template_key"], key.as_str());
        assert!(editor["data"]["revision"].is_null(), "reading {key} must not prepare a saved Revision");
        assert_eq!(editor["data"]["draft"]["document"]["model_route_refs"], json!({}),
            "reading {key} must not require or select a model");
        let document = editor["data"]["draft"]["document"].as_object().unwrap();
        for field in ["chat_route_records", "system_role_provider_overrides", "starter_prompts", "context_order", "middleware_order"] {
            assert!(!document.contains_key(field), "{key}: the real compact editor wire omits empty {field}; renderer reads must normalize its documented default");
        }
    }
    let (status, created) = client.call(router.clone(), "POST", "/api/companion/companions", json!({
        "name": "Read-only Agent options", "character": "mochi"
    })).await;
    assert_eq!(status, StatusCode::CREATED, "Companion creation failed: {created}");
    let id = created["data"]["companion_id"].as_str().unwrap();
    let path = format!("/api/product-agent-bindings/companion/{id}");
    let (status, options) = client.call(router.clone(), "GET", &path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "Companion Agent options failed: {options}");
    assert_eq!(options["data"]["selection"], json!({ "kind": "template", "template_key": "companion.default" }));
    assert_eq!(options["data"]["needs_model"], true);
    let companion_option = options["data"]["options"].as_array().unwrap().iter()
        .find(|option| option["selection"]["template_key"] == "companion.default").unwrap();
    assert_eq!(companion_option["available"], true, "the shipped Companion must remain selectable before model configuration: {companion_option}");
    let provider = create_chat_provider(&router, TRUST, "Read-only model", "read-surfaces-chat", 0).await;
    let (status, with_model) = client.call(router.clone(), "GET", &format!("{path}?provider_id={}&model=read-surfaces-chat",
        provider["provider_id"].as_str().unwrap()), json!({})).await;
    assert_eq!(status, StatusCode::OK, "model-scoped Companion options failed: {with_model}");
    assert_eq!(with_model["data"]["selection"], options["data"]["selection"]);
    assert_eq!(with_model["data"]["needs_model"], false);
    let companion_option = with_model["data"]["options"].as_array().unwrap().iter()
        .find(|option| option["selection"]["template_key"] == "companion.default").unwrap();
    assert_eq!(companion_option["available"], true, "missing optional native resources must not block the normal Companion: {companion_option}");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_preset_revisions")
        .fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(count, 0, "these real host reads must not create executable Agent revisions");
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn product_agent_selection_precedes_models_and_reports_host_capability_availability() {
    let client = LocalJsonClient(TRUST);
    const TRUST: &str = "product-selection-preflight";
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let (_, created) = client.call(router.clone(), "POST", "/api/companion/companions", json!({ "name": "Choose Agent first", "character": "mochi" })).await;
    let companion_id = created["data"]["companion_id"].as_str().unwrap();
    let chosen = json!({ "kind": "template", "template_key": "chat.minimal" });
    let mut paths = Vec::new();
    for kind in ["customer", "creative_studio_canvas"] {
        let path = format!("/api/product-agent-bindings/{kind}/{companion_id}");
        let (status, saved) = client.call(router.clone(), "PUT", &path, json!({ "selection": chosen })).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["data"]["needs_model"], true);
        let (status, reloaded) = client.call(router.clone(), "GET", &path, json!({})).await;
        assert_eq!(status, StatusCode::OK, "{reloaded}");
        assert_eq!(reloaded["data"]["selection"], chosen);
        paths.push(path);
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_preset_revisions").fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(count, 0, "model-free selection may initialize editable official identities but must not allocate executable revisions or select a default model");
    let upstream = wiremock::MockServer::start().await;
    let (status, provider) = client.call(router.clone(), "POST", "/api/providers", json!({
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
        let (status, options) = client.call(router.clone(), "GET", &query, json!({})).await;
        assert_eq!(status, StatusCode::OK, "{options}");
        let general = options["data"]["options"].as_array().unwrap().iter().find(|item| item["selection"]["template_key"] == "assistant.general").unwrap();
        assert_eq!(general["available"], true, "{general}");
        assert!(general["reason"].is_null(), "optional Browser and Computer resources must not block the remaining General Agent tools");
        let coding = options["data"]["options"].as_array().unwrap().iter().find(|item| item["selection"]["template_key"] == "coding.codex").unwrap();
        assert_eq!(coding["available"], true, "{coding}");
        assert!(coding["reason"].is_null(), "{coding}");
        let (_, after) = client.call(router.clone(), "GET", path, json!({})).await;
        assert_eq!(after["data"]["selection"], chosen, "availability checks must not change the saved Agent");
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_preset_revisions").fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(count, 0, "availability checks must not allocate executable Agent revisions");
    let coding = json!({ "kind": "template", "template_key": "coding.codex" });
    for path in &paths {
        let (status, saved) = client.call(router.clone(), "PUT", path, json!({
            "selection": coding, "model": model,
            "resource_selections": [
                {"resource_kind":"workspace", "resource_id":"default-workspace"},
                {"resource_kind":"process_session", "resource_id":"managed-process-session"},
                {"resource_kind":"project_memory", "resource_id":"default-project-memory"}
            ]
        })).await;
        assert_eq!(status, StatusCode::OK, "a chat-only model must support the headless-safe Coding Agent: {saved}");
        assert_eq!(saved["data"]["needs_model"], false);
        let (status, reloaded) = client.call(router.clone(), "GET", path, json!({})).await;
        assert_eq!(status, StatusCode::OK, "{reloaded}");
        assert_eq!(reloaded["data"]["selection"], coding);
    }
    assert!(upstream.received_requests().await.unwrap().is_empty());
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn canonical_coding_session_rejects_retired_overrides_and_invalid_capability_selection() {
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
        ("mcp-selection", json!({"mcp_server_ids": []})),
    ] {
        let response = router.clone().oneshot(Request::builder().method("PUT")
            .uri(format!("/api/agent-sessions/{id}/{suffix}"))
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "retired route remained: {suffix}");
    }
    let invalid_selection = router.clone().oneshot(Request::builder().method("PUT")
        .uri(format!("/api/agent-sessions/{id}/capability-selection"))
        .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
        .body(Body::from(r#"{"capability_selection":{}}"#)).unwrap()).await.unwrap();
    assert_eq!(invalid_selection.status(), StatusCode::UNPROCESSABLE_ENTITY,
        "current capability selection requires the typed selection and binding-version CAS");
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
    let client = LocalJsonClient(TRUST);
    const TRUST: &str = "creative-product-agent";
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let upstream = wiremock::MockServer::start().await;
    let (status, provider) = client.call(router.clone(), "POST", "/api/providers", json!({
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
    let (status, canvas) = client.call(router.clone(), "POST", "/api/creative-studio/canvases",
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
    let (status, session) = client.call(router.clone(), "POST",
        "/api/creative-studio/canvas-agent-sessions/resolve", json!({
            "canvas_id": canvas_id,
            "session_id": session_id,
            "model": { "provider_id": provider_id, "model": "step-3.7-flash" },
            "pending_turn_idempotency_key": pending_key
        })).await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    let conversation_id = session["data"]["binding"]["conversation_id"].as_str().unwrap();
    let (status, projected) = client.call(
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
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path_regex(".*/chat/completions$"))
        .respond_with(wiremock::ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string("data: {\"id\":\"creative\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"CREATIVE_SKILL_OK\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"creative\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"))
        .mount(&upstream).await;
    for (skill, expected_state) in [("creative-studio-canvas", "completed"), ("not-selected", "failed")] {
        let (status, turn) = client.call(router.clone(), "POST",
            &format!("/api/agent-sessions/{conversation_id}/turns"), json!({
                "idempotency_key": uuid::Uuid::now_v7().to_string(),
                "input": { "content": "Discuss the current canvas without changing it.", "inject_skills": [skill] }
            })).await;
        assert_eq!(status, StatusCode::OK, "{turn}");
        let operation = turn["data"]["operation_id"].as_str().unwrap();
        let terminal = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                let row: (String, Option<String>) = sqlx::query_as(
                    "SELECT turn.state, COALESCE(terminal.inline_json,json_extract(CAST(payload.body AS TEXT),'$.value')) \
                     FROM agent_turns turn LEFT JOIN agent_events terminal ON terminal.event_id=turn.terminal_event_id AND terminal.session_id=turn.session_id \
                     LEFT JOIN agent_payloads payload ON payload.payload_id=terminal.payload_id AND payload.session_id=turn.session_id \
                     WHERE turn.session_id = ? AND turn.operation_id = ?",
                ).bind(conversation_id).bind(operation).fetch_one(services.database.pool()).await.unwrap();
                if matches!(row.0.as_str(), "completed" | "failed" | "cancelled") { break row; }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }).await.expect("Creative Studio Skill turn must settle");
        assert_eq!(terminal.0, expected_state, "{terminal:?}");
        if expected_state == "failed" {
            let error: Value = serde_json::from_str(terminal.1.as_deref().unwrap()).unwrap();
            assert_eq!(error["error"]["code"], "NOMIFUN_STATE_INCONSISTENT");
            assert_eq!(error["error"]["ownership"], "nomifun");
            assert_eq!(error["error"]["retryable"], false);
            let (status, restored) = client.call(router.clone(), "POST",
                "/api/creative-studio/canvas-agent-sessions/resolve", json!({
                    "canvas_id": canvas_id,
                    "session_id": session_id,
                    "model": { "provider_id": provider_id, "model": "step-3.7-flash" },
                    "pending_turn_idempotency_key": pending_key
                })).await;
            assert_eq!(status, StatusCode::OK, "{restored}");
            let history = restored["data"]["history"].as_array().unwrap();
            assert_eq!(history.len(), 4, "both terminal turns must survive reload");
            assert_eq!(history[1]["status"], "complete");
            assert_eq!(history[3]["status"], "failed", "failed Skill admission must not become a blank completed reply");
            assert_eq!(history[3]["errorMessage"], error["error"]["message"]);
            let source: String = sqlx::query_scalar(
                "SELECT source_message_id FROM agent_turns WHERE session_id = ? AND operation_id = ?")
                .bind(conversation_id).bind(operation).fetch_one(services.database.pool()).await.unwrap();
            assert_eq!(history[2]["id"], source);
            let assistant: String = sqlx::query_scalar(
                "SELECT json_extract(projection_json, '$.correlation_id') FROM agent_messages \
                 WHERE session_id = ? AND presentation_intent = 'message' \
                   AND json_extract(projection_json, '$.state') = 'completed' ORDER BY first_seq DESC LIMIT 1")
                .bind(conversation_id).fetch_one(services.database.pool()).await.unwrap();
            assert_eq!(history[3]["id"], assistant, "reload must preserve the original assistant identity");
        }
    }
    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "unselected Skill must fail before provider invocation");
    let request = requests[0].body_json::<Value>().unwrap();
    let instructions = request["messages"].as_array().unwrap().iter()
        .filter(|message| message["role"] == "system")
        .map(|message| message["content"].to_string()).collect::<Vec<_>>().join("\n");
    assert!(instructions.contains("# Creative Studio Canvas Planning"));
    assert!(!instructions.contains("# Creative Studio Layout Organizer"));
    assert!(!instructions.contains("# Creative Studio Template Designer"));
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn creative_canvas_new_session_rebinds_exact_target_after_stale_product_selection() {
    let client = LocalJsonClient(TRUST);
    const TRUST: &str = "creative-canvas-stale-target-binding";

    let (router, services) = common::build_local_trust_app(TRUST).await;
    let upstream = wiremock::MockServer::start().await;
    let (status, provider) = client.call(router.clone(), "POST", "/api/providers", json!({
        "platform": "stepfun-plan", "name": "Canvas target fixture",
        "base_url": format!("{}/step_plan/v1", upstream.uri()),
        "auth_scheme": "bearer", "credentials": { "api_keys": ["fixture-only"] },
        "enabled": true, "initial_model": {
            "model": "step-3.7-flash", "enabled": true,
            "capabilities": [{ "task": "chat", "traits": [],
                "protocol": "openai.chat_text", "connection_role": "default", "provider_params": {} }]
        }
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let provider_id = provider["data"]["provider_id"].as_str().unwrap();
    let (status, other) = client.call(router.clone(), "POST", "/api/creative-studio/canvases",
        json!({ "title": "Other owned Canvas" })).await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    let other_id = other["data"]["canvas"]["canvasId"].as_str().unwrap();

    for selected_canvas in [Some(other_id), None] {
        let (status, canvas) = client.call(router.clone(), "POST", "/api/creative-studio/canvases",
            json!({
                "title": "Target Canvas",
                "agentKickoff": { "prompt": "plan", "model": {
                    "providerId": provider_id, "model": "step-3.7-flash"
                } }
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

        let mut selections = vec![
            json!({"resource_kind":"asset_library", "resource_id":"creative-studio-assets"}),
            json!({"resource_kind":"process_session", "resource_id":"managed-process-session"}),
            json!({"resource_kind":"project_memory", "resource_id":"default-project-memory"}),
            json!({"resource_kind":"workspace", "resource_id":"default-workspace"}),
        ];
        if let Some(id) = selected_canvas {
            selections.push(json!({"resource_kind":"canvas", "resource_id":id}));
        }
        let (status, saved) = client.call(router.clone(), "PUT",
            &format!("/api/product-agent-bindings/creative_studio_canvas/{canvas_id}"),
            json!({
                "selection": {"kind":"template", "template_key":"creative-studio.default"},
                "model": {"provider_id":provider_id, "model":"step-3.7-flash"},
                "resource_selections": selections
            })).await;
        assert_eq!(status, StatusCode::OK, "formal product selection fixture: {saved}");

        let (status, resolved) = client.call(router.clone(), "POST",
            "/api/creative-studio/canvas-agent-sessions/resolve", json!({
                "canvas_id": canvas_id, "session_id": session_id,
                "model": {"provider_id": provider_id, "model": "step-3.7-flash"},
                "pending_turn_idempotency_key": pending_key
            })).await;
        assert_eq!(status, StatusCode::CREATED, "{resolved}");
        let conversation_id = resolved["data"]["binding"]["conversation_id"].as_str().unwrap();
        let raw: String = sqlx::query_scalar(
            "SELECT agent_binding_json FROM agent_sessions WHERE agent_session_id = ?")
            .bind(conversation_id).fetch_one(services.database.pool()).await.unwrap();
        let binding: Value = serde_json::from_str(&raw).unwrap();
        let resources = binding["typed_resource_bindings"].as_array().unwrap();
        let resource = |kind: &str| resources.iter().find(|value| value["resource_kind"] == kind)
            .and_then(|value| value["resource_id"].as_str());
        assert_eq!(resource("canvas"), Some(canvas_id), "new Session must bind its own Canvas");
        assert_eq!(resource("asset_library"), Some("creative-studio-assets"));
        assert_eq!(resource("workspace"), Some("default-workspace"));
        assert_eq!(resource("process_session"), Some("managed-process-session"));
        assert_eq!(resource("project_memory"), Some("default-project-memory"));
    }
    assert!(upstream.received_requests().await.unwrap().is_empty());
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn companion_implicit_entry_binds_its_exact_owner_resources() {
    let client = LocalJsonClient(TRUST);
    const TRUST: &str = "companion-implicit-resource-binding";

    let (router, services) = common::build_local_trust_app(TRUST).await;
    common::materialize_builtin_skills_for_fixture(&services).await;
    let upstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path_regex(".*/chat/completions$"))
        .respond_with(wiremock::ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string("data: {\"id\":\"companion\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"companion\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"))
        .mount(&upstream).await;
    let (status, provider) = client.call(router.clone(), "POST", "/api/providers", json!({
        "platform": "stepfun-plan", "name": "Companion binding fixture",
        "base_url": format!("{}/step_plan/v1", upstream.uri()),
        "auth_scheme": "bearer", "credentials": { "api_keys": ["fixture-no-network"] },
        "enabled": true, "initial_model": {
            "model": "step-3.7-flash", "enabled": true,
            "capabilities": [{ "task": "chat", "traits": [],
                "protocol": "openai.chat_text", "connection_role": "default", "provider_params": {} }]
        }
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let provider_id = provider["data"]["provider_id"].as_str().unwrap();
    let (status, companion) = client.call(router.clone(), "POST", "/api/companion/companions",
        json!({ "name": "Implicit binding companion", "character": "mochi" })).await;
    assert_eq!(status, StatusCode::CREATED, "{companion}");
    let companion_id = companion["data"]["companion_id"].as_str().unwrap();
    let (status, patched) = client.call(router.clone(), "PATCH",
        &format!("/api/companion/companions/{companion_id}"),
        json!({ "model": { "provider_id": provider_id, "model": "step-3.7-flash" } })).await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    let (status, thread) = client.call(router.clone(), "POST",
        &format!("/api/companion/companions/{companion_id}/companion/threads"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{thread}");
    let session_id = thread["data"]["conversation_id"].as_str().unwrap();
    let raw: String = sqlx::query_scalar(
        "SELECT agent_binding_json FROM agent_sessions WHERE agent_session_id = ?",
    )
    .bind(session_id)
    .fetch_one(services.database.pool())
    .await
    .unwrap();
    let binding: Value = serde_json::from_str(&raw).unwrap();
    let resources = binding["typed_resource_bindings"].as_array().unwrap();
    let resource = |kind: &str| {
        resources.iter().find(|value| value["resource_kind"] == kind)
            .map(|value| value["resource_id"].as_str().unwrap())
    };
    assert_eq!(resource("companion"), Some(companion_id));
    assert_eq!(resource("companion_memory"), Some(companion_id));
    assert_eq!(resource("scheduler"), Some("installation-scheduler"));
    for kind in ["workspace", "process_session", "project_memory", "asset_library"] {
        assert!(resource(kind).is_some(), "full Companion must receive host-owned {kind}");
    }
    let workspace = resources.iter().find(|value| value["resource_kind"] == "workspace").unwrap();
    let process = resources.iter().find(|value| value["resource_kind"] == "process_session").unwrap();
    assert_eq!(workspace["typed_parameters"]["workspace_root"], process["typed_parameters"]["workspace_root"]);
    for absent in ["channel", "robot", "knowledge_base", "ssh_host", "canvas"] {
        assert_eq!(resource(absent), None, "optional {absent} must not be invented");
    }
    let (status, unrelated) = client.call(router.clone(), "POST", "/api/companion/companions",
        json!({ "name": "Unrelated companion must stay private", "character": "mochi" })).await;
    assert_eq!(status, StatusCode::CREATED, "{unrelated}");
    let (status, turn) = client.call(router.clone(), "POST",
        &format!("/api/agent-sessions/{session_id}/turns"), json!({
            "idempotency_key": uuid::Uuid::now_v7().to_string(),
            "input": { "content": "Tell me your name." }
        })).await;
    assert_eq!(status, StatusCode::OK, "{turn}");
    let terminal = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let kind: Option<String> = sqlx::query_scalar(
                "SELECT kind FROM agent_events WHERE session_id = ? AND kind IN ('turn/completed', 'turn/failed', 'turn/cancelled') ORDER BY seq DESC LIMIT 1",
            ).bind(session_id).fetch_optional(services.database.pool()).await.unwrap();
            if let Some(kind) = kind { break kind; }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }).await.expect("Companion turn must reach a durable terminal");
    assert_eq!(terminal, "turn/completed");
    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let model_request = requests[0].body_json::<Value>().unwrap();
    let instructions = model_request["messages"].as_array().unwrap().iter()
        .filter(|message| message["role"] == "system")
        .map(|message| message["content"].to_string()).collect::<Vec<_>>().join("\n");
    assert!(instructions.contains("Implicit binding companion"), "selected persona must reach the model");
    assert!(!instructions.contains("Unrelated companion must stay private"));
    let (status, patched) = client.call(router.clone(), "PATCH",
        &format!("/api/companion/companions/{companion_id}"),
        json!({ "name": "Renamed bound companion" })).await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    let (status, turn) = client.call(router.clone(), "POST",
        &format!("/api/agent-sessions/{session_id}/turns"), json!({
            "idempotency_key": uuid::Uuid::now_v7().to_string(),
            "input": { "content": "Tell me your current name." }
        })).await;
    assert_eq!(status, StatusCode::OK, "{turn}");
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let completed: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM agent_events WHERE session_id = ? AND kind = 'turn/completed'",
            ).bind(session_id).fetch_one(services.database.pool()).await.unwrap();
            if completed == 2 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }).await.expect("second Companion turn must complete");
    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let second = requests[1].body_json::<Value>().unwrap();
    let instructions = second["messages"].as_array().unwrap().iter()
        .filter(|message| message["role"] == "system")
        .map(|message| message["content"].to_string()).collect::<Vec<_>>().join("\n");
    assert!(instructions.contains("Renamed bound companion"), "BeforeTurn persona must refresh");
    assert!(!instructions.contains("Implicit binding companion"));
    assert!(!instructions.contains("Unrelated companion must stay private"));
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

use common::companion_agent::{COMPANION_AGENT_TEST_TRUST, companion_agent_test_request, save_companion_core_agent_for_fixture};


#[tokio::test]
async fn companion_options_survive_a_first_launch_selection_default_save_failure() {
    let call = companion_agent_test_request;
    let (router, services) = common::build_local_trust_app(COMPANION_AGENT_TEST_TRUST).await;
    common::materialize_builtin_skills_for_fixture(&services).await;
    save_companion_core_agent_for_fixture(router.clone()).await;
    let (_, library) = call(router.clone(), "GET", "/api/agent-preset-templates", json!({})).await;
    let official_id = library["data"]["official_presets"]["companion.default"]["preset_id"].as_str().unwrap();
    let (_, editor) = call(router.clone(), "GET", &format!("/api/agent-presets/{official_id}/editor"), json!({})).await;
    let (status, personal) = call(router.clone(), "POST", "/api/agent-presets", json!({
        "display_name": "Personal first-launch companion",
        "document": editor["data"]["draft"]["document"],
    })).await;
    assert_eq!(status, StatusCode::OK, "{personal}");
    let personal_id = personal["data"]["preset"]["preset_id"].as_str().unwrap();
    let provider = create_chat_provider(&router, COMPANION_AGENT_TEST_TRUST, "First-launch selection", "first-launch-chat", 0).await;
    let model = json!({ "provider_id": provider["provider_id"], "model": "first-launch-chat" });
    let (status, companion) = call(router.clone(), "POST", "/api/companion/companions", json!({
        "name": "Partially saved personal Agent", "character": "mochi",
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{companion}");
    let companion_id = companion["data"]["companion_id"].as_str().unwrap();
    let path = format!("/api/product-agent-bindings/companion/{companion_id}");
    sqlx::query("CREATE TRIGGER fixture_first_launch_default_failure BEFORE INSERT ON product_agent_selections BEGIN SELECT RAISE(FAIL, 'fixture first-launch default write failure'); END")
        .execute(services.database.pool()).await.unwrap();
    let selection = json!({ "kind": "preset", "preset_id": personal_id });
    let (status, partial) = call(router.clone(), "PUT", &path, json!({ "selection": selection, "model": model })).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{partial}");
    let bindings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_bindings WHERE target_kind = 'companion' AND target_id = ?")
        .bind(companion_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(bindings, 1, "a first-launch default failure can leave a real saved executable binding");
    sqlx::query("DROP TRIGGER fixture_first_launch_default_failure").execute(services.database.pool()).await.unwrap();
    let artifacts_before: (i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_preset_revisions), (SELECT COUNT(*) FROM agent_runtime_snapshots)")
        .fetch_one(services.database.pool()).await.unwrap();
    let (status, before_launch_options) = call(router.clone(), "GET", &path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "the partially saved default must remain explicitly recoverable before launch: {before_launch_options}");
    assert!(before_launch_options["data"]["selection"].is_null(), "an internal execution variant must not appear as the editable personal Agent");
    let artifacts_after: (i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_preset_revisions), (SELECT COUNT(*) FROM agent_runtime_snapshots)")
        .fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(artifacts_after, artifacts_before, "reading recovery options must not prepare executable revisions or snapshots");
    let (status, patched) = call(router.clone(), "PATCH", &format!("/api/companion/companions/{companion_id}"), json!({ "model": model })).await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    let (status, thread) = call(router.clone(), "POST", &format!("/api/companion/companions/{companion_id}/companion/threads"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{thread}");
    let session_id = thread["data"]["conversation_id"].as_str().unwrap();
    const RETAINED_INPUT: &str = "Preserve this accepted input when explicitly restoring the Agent choice";
    let (status, accepted) = call(router.clone(), "POST", &format!("/api/agent-sessions/{session_id}/turns"), json!({
        "idempotency_key": uuid::Uuid::now_v7().to_string(),
        "input": { "content": RETAINED_INPUT, "inject_skills": ["fixture-unselected-skill"] },
    })).await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    let operation = accepted["data"]["operation_id"].as_str().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let state: String = sqlx::query_scalar("SELECT state FROM agent_turns WHERE session_id = ? AND operation_id = ?")
                .bind(session_id).bind(operation).fetch_one(services.database.pool()).await.unwrap();
            if state == "failed" { break; }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }).await.expect("the known pre-model Skill rejection must settle before explicit Agent recovery");
    let failure_json: String = sqlx::query_scalar("SELECT COALESCE(terminal.inline_json,json_extract(CAST(payload.body AS TEXT),'$.value')) \
        FROM agent_turns turn JOIN agent_events terminal ON terminal.event_id=turn.terminal_event_id AND terminal.session_id=turn.session_id \
        LEFT JOIN agent_payloads payload ON payload.payload_id=terminal.payload_id AND payload.session_id=turn.session_id \
        WHERE turn.session_id = ? AND turn.operation_id = ?")
        .bind(session_id).bind(operation).fetch_one(services.database.pool()).await.unwrap();
    let failure: Value = serde_json::from_str(&failure_json).unwrap();
    assert_eq!(failure["model_steps"], 0, "the deliberate Skill rejection must precede any provider request");
    assert_eq!(failure["error"]["code"], "NOMIFUN_STATE_INCONSISTENT");
    assert!(failure["error"]["detail"].as_str().unwrap().contains("requested Skill is not in this Agent Snapshot"),
        "an unrelated preparation failure must not stand in for the deliberate rejection: {failure}");
    let (status, before_history) = call(router.clone(), "GET", &format!("/api/agent-sessions/{session_id}/message-history?page_size=50"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{before_history}");
    assert!(before_history.to_string().contains(RETAINED_INPUT), "the fixture must contain real canonical input history");
    let (status, before) = call(router.clone(), "GET", &format!("/api/agent-sessions/{session_id}"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    let (status, options) = call(router.clone(), "GET", &path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "a valid current-generation binding must not block explicit Agent recovery: {options}");
    assert!(options["data"]["selection"].is_null(), "an unprovable authored source must not become a guessed template or hidden cache ID: {options}");
    assert!(options["data"]["options"].as_array().unwrap().iter().any(|option|
        option["selection"] == selection && option["available"] == true), "the original editable Agent must remain an explicit recovery candidate");
    let (status, after) = call(router.clone(), "GET", &format!("/api/agent-sessions/{session_id}"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["data"]["session"]["agent_binding"], before["data"]["session"]["agent_binding"],
        "listing recovery options must preserve the frozen Agent and Session history");
    let (status, repaired) = call(router.clone(), "PUT", &path, json!({
        "selection": selection, "model": model, "conversation_id":session_id, "update_default":true,
    })).await;
    assert_eq!(status, StatusCode::OK, "explicitly choosing the personal Agent must repair its future default: {repaired}");
    assert_eq!(repaired["data"]["conversation"]["conversation_id"], session_id);
    let (status, repaired_session) = call(router.clone(), "GET", &format!("/api/agent-sessions/{session_id}"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{repaired_session}");
    assert_eq!(repaired_session["data"]["session"]["agent_binding"]["typed_resource_bindings"],
        before["data"]["session"]["agent_binding"]["typed_resource_bindings"]);
    assert_eq!(repaired["data"]["conversation"]["model"]["provider_id"], model["provider_id"]);
    assert_eq!(repaired["data"]["conversation"]["model"]["model"], model["model"]);
    let transitions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id = ? AND kind = 'session/agent-binding-changed'")
        .bind(session_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(transitions, 1, "the user's explicit choice must establish one canonical Agent transition");
    let (status, after_history) = call(router.clone(), "GET", &format!("/api/agent-sessions/{session_id}/message-history?page_size=50"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{after_history}");
    for retained in before_history["data"]["items"].as_array().unwrap() {
        assert_eq!(after_history["data"]["items"].as_array().unwrap().iter()
            .find(|message| message["message_id"] == retained["message_id"]), Some(retained),
            "explicit recovery must preserve each existing canonical history item alongside its new transition notice");
    }
    // Current options validate the exact frozen Session; the path without a
    // conversation_id reads the independently saved future authoring default.
    let active_options_path = format!("{path}?conversation_id={session_id}");
    let (status, options) = call(router.clone(), "GET", &active_options_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{options}");
    assert_eq!(options["data"]["selection"], selection);
    let snapshot_id = repaired_session["data"]["session"]["agent_binding"]["resolved_snapshot_ref"]["snapshot_id"].as_str().unwrap();
    let damaged_row = sqlx::query("UPDATE agent_runtime_snapshots SET envelope_json = '{}' WHERE snapshot_id = ?")
        .bind(snapshot_id).execute(services.database.pool()).await.unwrap();
    assert_eq!(damaged_row.rows_affected(), 1, "damage the exact current frozen Snapshot, not an unused role cache");
    let (status, damaged) = call(router.clone(), "GET", &active_options_path, json!({})).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "supported storage damage must not become a nullable authoring origin: {damaged}");
    assert_eq!(damaged["code"], "PRESET_REVISION_SAVE_FAILED");
    let (status, future_default) = call(router.clone(), "GET", &path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "future authoring intent does not read or authorize the damaged current Session: {future_default}");
    assert_eq!(future_default["data"]["selection"], selection);
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

const COMPANION_REPLY: &str = "COMPANION_CHAT_WITHOUT_DEVICE_OK";

struct StartedCompanion {
    router: axum::Router,
    services: AppServices,
    upstream: wiremock::MockServer,
    provider_id: String,
    companion_id: String,
    conversation_id: String,
    preset_id: String,
    snapshot: Value,
    history: Value,
    unchanged: Value,
}

async fn original_history_fact_fingerprints(services:&AppServices,session:&str,through:i64)->Vec<(i64,String,String)> {
    let rows:Vec<(i64,String,String)>=sqlx::query_as(
        "SELECT e.seq,e.event_id,json_object(\
          'session_id',e.session_id,'producer_id',e.producer_id,'idempotency_key',e.idempotency_key,\
          'kind',e.kind,'kind_version',e.kind_version,'correlation_id',e.correlation_id,\
          'causation_event_id',e.causation_event_id,'inline_json',e.inline_json,'payload_id',e.payload_id,\
          'payload_session_id',p.session_id,'media_type',p.media_type,'byte_len',p.byte_len,\
          'digest',p.digest,'body_hex',hex(p.body)) \
         FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id \
         WHERE e.session_id=? AND e.seq<=? ORDER BY e.seq")
        .bind(session).bind(through).fetch_all(services.database.pool()).await.unwrap();
    rows.into_iter().map(|(seq,id,raw)|(seq,id,format!("{:x}",
        <sha2::Sha256 as sha2::Digest>::digest(raw.as_bytes())))).collect()
}

fn history_dto_changed_paths(before:&Value,after:&Value,prefix:&str,paths:&mut Vec<String>) {
    if before==after {return;}
    match(before,after) {
        (Value::Object(left),Value::Object(right))=>{
            let keys=left.keys().chain(right.keys()).collect::<std::collections::BTreeSet<_>>();
            for key in keys {history_dto_changed_paths(left.get(key).unwrap_or(&Value::Null),
                right.get(key).unwrap_or(&Value::Null),&format!("{prefix}/{key}"),paths);}
        },
        (Value::Array(left),Value::Array(right)) if left.len()==right.len()=>{
            for(index,(left,right))in left.iter().zip(right).enumerate(){history_dto_changed_paths(left,right,&format!("{prefix}/{index}"),paths);}
        },
        _=>paths.push(prefix.to_owned()),
    }
}

fn assert_history_identity_and_content_preserved(before:&Value,after:&Value) {
    for original in before["data"]["items"].as_array().unwrap() {
        let id=original["message_id"].as_str().unwrap();
        let current=after["data"]["items"].as_array().unwrap().iter().find(|item|item["message_id"]==id)
            .unwrap_or_else(||panic!("the original canonical message {id} disappeared from history"));
        let mut changed=Vec::new();history_dto_changed_paths(original,current,"",&mut changed);
        if !changed.is_empty() {
            eprintln!("HISTORY_DTO_COMPARISON message={id} changed_paths={} prior_watermark={} current_watermark={}",
                json!(changed),original.pointer("/content/timeline/through_seq").unwrap_or(&Value::Null),
                current.pointer("/content/timeline/through_seq").unwrap_or(&Value::Null));
        }
        // A new binding event advances this read snapshot's observation bound.
        // IDs, text, type, status and every timeline segment remain identical.
        assert!(changed.iter().all(|path|path=="/content/timeline/through_seq"),
            "history changed outside its observed timeline watermark: message={id}, paths={changed:?}");
        if let Some(previous)=original.pointer("/content/timeline/through_seq").and_then(Value::as_u64) {
            assert!(current.pointer("/content/timeline/through_seq").and_then(Value::as_u64).is_some_and(|seq|seq>=previous));
        }
        let(mut old,mut new)=(original.clone(),current.clone());
        for value in [&mut old,&mut new] {
            if let Some(timeline)=value.pointer_mut("/content/timeline").and_then(Value::as_object_mut){timeline.remove("through_seq");}
        }
        assert_eq!(old,new,"the original HTTP identity, content and timeline segments must remain exact");
    }
}

/// Exercise real initial admission, concurrent ensure, failure cleanup and history
/// once per isolated fixture; both Agent-edit paths begin from this same state.
async fn started_companion_fixture() -> StartedCompanion {
    let call = companion_agent_test_request;
    let (router, services) = common::build_local_trust_app(COMPANION_AGENT_TEST_TRUST).await;
    common::materialize_builtin_skills_for_fixture(&services).await;
    let upstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path_regex(".*/chat/completions$"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(format!(
                    "data: {{\"id\":\"companion\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{COMPANION_REPLY}\"}},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"companion\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
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
        json!({ "name": "Agent-bound companion", "character": "mochi" })).await;
    assert_eq!(status, StatusCode::CREATED, "{companion}");
    let companion_id = companion["data"]["companion_id"].as_str().unwrap();
    let (status, patched) = call(router.clone(), "PATCH",
        &format!("/api/companion/companions/{companion_id}"),
        json!({ "model": { "provider_id": provider_id, "model": "step-3.7-flash" } })).await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    save_companion_core_agent_for_fixture(router.clone()).await;
    let (status, authored_library) = call(router.clone(), "GET", "/api/agent-preset-templates", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{authored_library}");
    let authored_id = authored_library["data"]["official_presets"]["companion.default"]["preset_id"].as_str().unwrap();
    let authored_selection = json!({ "kind":"preset", "preset_id":authored_id });
    let (status, selected_default) = call(router.clone(), "PUT",
        &format!("/api/product-agent-bindings/companion/{companion_id}"), json!({
            "selection": authored_selection,
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
    assert_eq!(thread["data"]["conversation_id"], other_thread["data"]["conversation_id"], "concurrent desktop receive ensures share the same receive Session");
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
        "companion.memory", "knowledge",
    ]));
    for (module, expected) in [
        ("companion", &["companion/evolve", "companion/learn"][..]),
        ("companion.memory", &["companion.memory/recall", "companion.memory/write"][..]),
        ("knowledge", &["knowledge/read", "knowledge/search"][..]),
        ("channel.messaging", &["channel.messaging/reply"][..]),
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
                "SELECT turn.state, COALESCE(terminal.inline_json,json_extract(CAST(payload.body AS TEXT),'$.value')) \
                 FROM agent_turns turn LEFT JOIN agent_events terminal ON terminal.event_id=turn.terminal_event_id AND terminal.session_id=turn.session_id \
                 LEFT JOIN agent_payloads payload ON payload.payload_id=terminal.payload_id AND payload.session_id=turn.session_id \
                 WHERE turn.session_id = ? AND turn.operation_id = ?",
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
    let failed_error: Value = serde_json::from_str(&failed_error).expect("canonical pre-model failure diagnostics");
    assert_eq!(failed_error["model_steps"], 0, "an unselected Skill must fail before the model starts");
    assert_eq!(failed_error["error"]["code"], "NOMIFUN_STATE_INCONSISTENT");
    assert_eq!(failed_error["error"]["agentTemplateKey"], "companion.default");
    let detail = failed_error["error"]["detail"].as_str().expect("native Skill admission diagnostic");
    assert!(detail.contains("Agent Skills: requested Skill is not in this Agent Snapshot"), "{failed_error}");
    assert!(!detail.contains("turn no longer admits progress"), "cleanup masked the preparation failure: {failed_error}");
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
                .find(|message| message["content"]["content"] == COMPANION_REPLY)
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
        .find(|message| message["content"]["content"] == COMPANION_REPLY)
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
    assert_eq!(session_count, 1, "concurrent desktop receive ensures create one canonical Session");
    let (status, options) = call(router.clone(), "GET",
        &format!("/api/product-agent-bindings/companion/{companion_id}"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{options}");
    assert_eq!(options["data"]["selection"], authored_selection,
        "the explicitly saved official configuration remains a Preset selection");
    let minimal_option = options["data"]["options"].as_array().unwrap().iter()
        .find(|option| option["selection"]["template_key"] == "chat.minimal").unwrap();
    assert_eq!(minimal_option["available"], false);
    assert_eq!(minimal_option["reason"], "companion", "Agents must retain the Companion persona contract");
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
    assert_eq!(selected["code"], "COMPANION_AGENT_REQUIRED");
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

    StartedCompanion {
        router, services, upstream,
        provider_id: provider_id.to_owned(),
        companion_id: companion_id.to_owned(),
        conversation_id: conversation_id.to_owned(),
        preset_id,
        snapshot: snapshot.clone(),
        history: durable_history,
        unchanged,
    }
}

#[tokio::test]
async fn one_companion_agent_serves_independent_desktop_and_device_sessions() {
    let call = companion_agent_test_request;
    let fixture = started_companion_fixture().await;
    let router = fixture.router;
    let services = fixture.services;
    let companion_id = fixture.companion_id.as_str();
    let desktop_id = fixture.conversation_id.as_str();
    let original_through:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=?")
        .bind(desktop_id).fetch_one(services.database.pool()).await.unwrap();
    assert!(original_through>0);
    let original_facts=original_history_fact_fingerprints(&services,desktop_id,original_through).await;
    let model = json!({"provider_id":fixture.provider_id,"model":"step-3.7-flash"});
    let agent_path = format!("/api/product-agent-bindings/companion/{companion_id}");
    let desktop_projection = format!("/api/agent-sessions/{desktop_id}/projection");
    let memory = services.companion_service.add_memory("preference", "ROBOT_PROFILE_MEMORY_PRESERVED", &[], Some(companion_id))
        .await.unwrap();
    let (status, library) = call(router.clone(), "GET", "/api/agent-preset-templates", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{library}");
    let shared_preset = library["data"]["official_presets"]["companion.default"]["preset_id"].as_str().unwrap();
    let (status, editor) = call(router.clone(), "GET",
        &format!("/api/agent-presets/{shared_preset}/editor"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{editor}");
    assert_eq!(editor["data"]["draft"]["source_template_key"], "companion.default");
    let mut detached = editor["data"]["draft"].clone();
    detached["document"]["enabled_capabilities"].as_array_mut().unwrap()
        .retain(|capability| capability["capability"]["id"] != "companion.memory");
    let revision_path = format!("/api/agent-presets/{shared_preset}/revisions");
    let (status, rejected) = call(router.clone(), "POST", &revision_path, json!({
        "expected_current_revision":editor["data"]["revision"]["reference"], "draft":detached,
    })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{rejected}");
    assert_eq!(rejected["code"], "PRESET_REVISION_SAVE_FAILED");
    // One authored Agent grants both ordinary work and optional device Actions.
    // Device identity is selected when the physical Session opens, never by a
    // second weaker preset or an implicit desktop delegation.
    let mut shared_draft = editor["data"]["draft"].clone();
    shared_draft["document"]["enabled_capabilities"].as_array_mut().unwrap().extend([
        json!({"capability":{"id":"robot"},"action_allowlist":["robot/device","robot/display","robot/motion","robot/vision"]}),
        json!({"capability":{"id":"workspace.files"},"action_allowlist":["workspace.files/read"]}),
    ]);
    let (status, saved) = call(router.clone(), "POST", &revision_path, json!({
        "expected_current_revision":editor["data"]["revision"]["reference"], "draft":shared_draft,
    })).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let selection = json!({"kind":"preset", "preset_id":shared_preset});
    let (status, applied) = call(router.clone(), "PUT", &agent_path, json!({
        "selection":selection,"model":model,"conversation_id":desktop_id,"update_default":true,
    })).await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    let (status, desktop_before_device) = call(router.clone(), "GET", &desktop_projection, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{desktop_before_device}");
    let (status,history_before_device)=call(router.clone(),"GET",
        &format!("/api/agent-sessions/{desktop_id}/message-history?page_size=50"),json!({})).await;
    assert_eq!(status,StatusCode::OK,"{history_before_device}");
    assert_history_identity_and_content_preserved(&fixture.history,&history_before_device);
    assert_eq!(original_history_fact_fingerprints(&services,desktop_id,original_through).await,original_facts,
        "the explicit Agent transition never rewrites original Event metadata, payload references or payload bytes");
    let (status, retired_target) = call(router.clone(), "GET",
        &format!("/api/product-agent-bindings/robot/{companion_id}"), json!({})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{retired_target}");
    assert_eq!(retired_target["code"], "CAPABILITY_UNAVAILABLE_ON_PLATFORM");
    let registry = &services.robot.as_ref().unwrap().registry;
    let (device, _) = registry.upsert_on_report(nomifun_robot::registry::RobotReport {
        robot_id:"coexisting-device".into(), client_id:"coexisting-client".into(),
        board:"fixture-board".into(), firmware_version:"test".into(),
    }, 1).await.unwrap();
    registry.claim(device.activation_code.as_deref().unwrap(), companion_id).await.unwrap();
    // A real host connection fence with an intentionally empty advertised
    // catalog proves the selected device grants no invented hardware tools.
    let (device_tx, _device_rx) = tokio::sync::mpsc::channel(4);
    registry.connect(&device.robot_id, companion_id, "coexisting-connection").await.unwrap();
    services.robot.as_ref().unwrap().tools.attach(&device.robot_id, std::sync::Arc::new(
        nomifun_robot::mcp_bridge::RobotMcpClient::new(device_tx, "coexisting-connection".into())), Vec::new()).await;
    let robot = services.companion_service.ensure_session_for_role(
        companion_id, nomifun_companion::CompanionRole::Robot, Some(&device.robot_id)).await.unwrap();
    let robot_id = robot.conversation_id.as_str();
    assert_ne!(robot_id, desktop_id, "physical inputs have a separate canonical execution lane");
    let (status, projected) = call(router.clone(), "GET",
        &format!("/api/agent-sessions/{robot_id}/projection"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{projected}");
    assert_eq!(projected["data"]["extra"]["companion_role"], "robot");
    assert_eq!(projected["data"]["extra"]["robot_id"], device.robot_id);
    assert_eq!(projected["data"]["extra"]["official_template_key"], "companion.default");
    assert_eq!(projected["data"]["extra"]["product_agent_target_kind"], "companion");
    assert_eq!(projected["data"]["agent_snapshot"]["resolved_model"],
        desktop_before_device["data"]["agent_snapshot"]["resolved_model"], "all entries share the selected main model");
    assert_eq!(projected["data"]["agent_snapshot"]["enabled_capability_actions"],
        desktop_before_device["data"]["agent_snapshot"]["enabled_capability_actions"],
        "the device entry preserves the complete shared Agent capability selection");
    assert_eq!(projected["data"]["agent_snapshot"]["enabled_capability_actions"]["workspace.files"],
        json!(["workspace.files/read"]), "ordinary file authority survives the physical entry");
    let resources = projected["data"]["agent_snapshot"]["canonical_binding"]["typed_resource_bindings"].as_array().unwrap();
    assert!(resources.iter().any(|resource| resource["resource_kind"] == "robot" && resource["resource_id"] == device.robot_id));
    for kind in ["companion", "companion_memory"] {
        assert!(resources.iter().any(|resource| resource["resource_kind"] == kind && resource["resource_id"] == companion_id));
    }
    assert!(resources.iter().any(|resource| resource["resource_kind"] == "workspace"
        && resource["owner_id"] == services.authoritative_user_id.as_ref()),
        "ordinary work retains its own real owner-bound resource");
    let (status, desktop_unchanged) = call(router.clone(), "GET", &desktop_projection, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{desktop_unchanged}");
    assert_eq!(desktop_unchanged["data"]["agent_snapshot"]["canonical_binding"],
        desktop_before_device["data"]["agent_snapshot"]["canonical_binding"], "opening a device Session cannot mutate an existing Session binding");
    let (status, history) = call(router.clone(), "GET",
        &format!("/api/agent-sessions/{desktop_id}/message-history?page_size=50"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(history["data"]["items"],history_before_device["data"]["items"],
        "opening another device Session does not advance or change the desktop history snapshot");
    assert_history_identity_and_content_preserved(&fixture.history,&history);
    assert_eq!(original_history_fact_fingerprints(&services,desktop_id,original_through).await,original_facts,
        "independent entries preserve the complete original Event/Payload prefix byte for byte");
    assert_eq!(memory.companion_id.as_deref(), Some(companion_id));
    assert_eq!(fixture.upstream.received_requests().await.unwrap().len(), 1, "configuration and resource assembly perform no model call");
    let (status, turn) = call(router.clone(), "POST", &format!("/api/agent-sessions/{robot_id}/turns"), json!({
        "idempotency_key":uuid::Uuid::now_v7().to_string(),"input":{"content":"Say hello as this companion"},
    })).await;
    assert_eq!(status, StatusCode::OK, "{turn}");
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let completed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/completed'")
                .bind(robot_id).fetch_one(services.database.pool()).await.unwrap();
            if completed == 1 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }).await.expect("the device entry must complete its own ordinary conversation");
    let requests = fixture.upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let actual = requests[1].body_json::<Value>().unwrap();
    let system = actual["messages"].as_array().unwrap().iter().filter(|message| message["role"] == "system")
        .map(|message| message["content"].to_string()).collect::<Vec<_>>().join("\n");
    assert!(system.contains("Agent-bound companion"), "the device entry shares the exact Companion persona");
    assert!(system.contains("当前物理机器人"), "canonical entry identity supplies physical reply context");
    assert!(system.contains("ROBOT_PROFILE_MEMORY_PRESERVED"), "the device entry reads the same owner's durable memory");
    assert!(system.contains("Current frozen physical Robot tool mapping (data, not additional authority): []"),
        "the selected device's empty catalog grants no invented hardware tools");
    let names = actual["tools"].as_array().into_iter().flatten().filter_map(|tool|
        tool["function"]["name"].as_str()).collect::<Vec<_>>();
    assert!(names.iter().all(|name| !name.starts_with("robot_")), "the empty physical device catalog has no callable tool surface");
    let (status, authorized) = call(router.clone(), "PUT", &agent_path, json!({
        "selection":selection, "model":model, "conversation_id":desktop_id, "update_default":false,
        "resource_selections":[{"resource_kind":"robot", "resource_id":device.robot_id}],
    })).await;
    assert_eq!(status, StatusCode::OK, "{authorized}");
    let (status, desktop_authorized) = call(router.clone(), "GET", &desktop_projection, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{desktop_authorized}");
    assert_eq!(desktop_authorized["data"]["extra"]["companion_role"], "companion",
        "explicit device authorization does not retarget the desktop input lane");
    assert!(desktop_authorized["data"]["agent_snapshot"]["enabled_capabilities"].as_array().unwrap().contains(&json!("robot")),
        "the explicitly selected Agent retains its Robot capability");
    let resources = desktop_authorized["data"]["agent_snapshot"]["canonical_binding"]["typed_resource_bindings"].as_array().unwrap();
    assert!(resources.iter().any(|resource| resource["resource_kind"] == "robot" && resource["resource_id"] == device.robot_id
        && resource["owner_id"] == services.authoritative_user_id.as_ref()),
        "desktop hardware access still requires the selected real owned device");
    let (status, defaults) = call(router.clone(), "GET", &agent_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{defaults}");
    assert_eq!(defaults["data"]["selection"], selection,
        "explicit Session resource changes preserve the one shared Agent default");
    assert_eq!(fixture.upstream.received_requests().await.unwrap().len(), 2, "authorizing capabilities performs no model call");
    let thread_path = format!("/api/companion/companions/{companion_id}/companion/threads");
    let (status, extra) = call(router.clone(), "POST", &thread_path, json!({
        "new_session":true, "creation_key":uuid::Uuid::now_v7().to_string(), "title":"Separate desktop topic",
    })).await;
    assert_eq!(status, StatusCode::OK, "{extra}");
    assert_ne!(extra["data"]["conversation_id"], desktop_id);
    assert_ne!(extra["data"]["conversation_id"], robot_id);
    let (status, receive) = call(router.clone(), "POST", &thread_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{receive}");
    assert_eq!(receive["data"]["conversation_id"], desktop_id, "new viewed topics do not retarget the desktop receive lane");
    let reopened_robot = services.companion_service.ensure_session_for_role(
        companion_id, nomifun_companion::CompanionRole::Robot, Some(&device.robot_id)).await.unwrap();
    assert_eq!(reopened_robot.conversation_id, robot_id, "desktop topic selection cannot retarget a device");
    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions WHERE state='live'")
        .fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(sessions, 3, "multiple desktop topics and the device Session coexist");
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn directed_companion_creation_is_read_only_before_its_execution_link() {
    let call = companion_agent_test_request;
    let fixture = started_companion_fixture().await;
    let router = fixture.router;
    let services = fixture.services;
    let owner = nomifun_agent_contracts::PrincipalRef {
        principal_kind:"user".into(), principal_id:services.authoritative_user_id.to_string(),
    };
    let binding: nomifun_agent_contracts::AgentBindingValue = serde_json::from_value(
        fixture.snapshot["canonical_binding"].clone()).unwrap();
    let active = fixture.snapshot["enabled_capabilities"].as_array().unwrap().iter()
        .map(|capability| capability.as_str().unwrap().to_owned()).collect();
    let canonical = nomifun_conversation::CanonicalAgentSessionOwner::from_pool(
        services.database.pool().clone()).await.unwrap();
    // This is the committed opening produced before the Attempt-link write.
    // It models restart in that gap without authoring UI metadata or a second ledger.
    let opened = canonical.open_with_companion_scope(owner, binding, Some("Unlinked directed task".into()), active,
        nomifun_agent_contracts::CompanionSessionOrigin {
            companion_id:fixture.companion_id.clone(), role:nomifun_agent_contracts::CompanionSessionRole::Companion,
            robot_id:None,
        }, &format!("companion-directed:fixture:{}", uuid::Uuid::now_v7()), nomifun_common::now_ms()).await.unwrap();
    let id = opened.session.agent_session_id.as_ref();
    let (status, projection) = call(router.clone(), "GET",
        &format!("/api/agent-sessions/{id}/projection"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{projection}");
    assert_eq!(projection["data"]["extra"]["companion_associated_task"], true);
    assert!(projection["data"]["execution_attempt_id"].is_null(), "the fixture must precede its actual Attempt link");
    let threads = services.companion_service.list_companion_threads(&fixture.companion_id).await.unwrap();
    assert!(!threads.iter().any(|thread| thread.conversation_id == id), "a partially created task is not a desktop receive topic");
    let model = json!({"provider_id":fixture.provider_id,"model":"step-3.7-flash"});
    let (status, agent) = call(router.clone(), "PUT",
        &format!("/api/product-agent-bindings/companion/{}", fixture.companion_id), json!({
            "selection":{"kind":"template","template_key":"companion.default"}, "conversation_id":id,
        })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{agent}");
    assert_eq!(agent["code"], "AGENT_EXECUTION_ATTEMPT_READ_ONLY");
    let (status, switched_model) = call(router.clone(), "PUT",
        &format!("/api/agent-sessions/{id}/model"), model).await;
    assert_eq!(status, StatusCode::CONFLICT, "{switched_model}");
    assert_eq!(switched_model["code"], "AGENT_EXECUTION_ATTEMPT_READ_ONLY");
    let (status, input) = call(router.clone(), "POST",
        &format!("/api/agent-sessions/{id}/turns"), json!({
            "idempotency_key":uuid::Uuid::now_v7().to_string(), "input":{"content":"Do not start an unrelated task Turn"},
        })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{input}");
    assert_eq!(input["code"], "AGENT_EXECUTION_ATTEMPT_READ_ONLY");
    let turns: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?")
        .bind(id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(turns, 0, "rejected configuration and input probes never create a Turn");
    assert_eq!(fixture.upstream.received_requests().await.unwrap().len(), 1, "only the fixture's original desktop Turn reached the model");
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn companion_agent_settings_switch_in_place_and_preserve_persona_history_and_model() {
    let call = companion_agent_test_request;
    let fixture = started_companion_fixture().await;
    let router = fixture.router;
    let services = fixture.services;
    let upstream = fixture.upstream;
    let provider_id = fixture.provider_id.as_str();
    let companion_id = fixture.companion_id.as_str();
    let conversation_id = fixture.conversation_id.as_str();
    let preset_id = fixture.preset_id;
    let path = format!("/api/companion/companions/{companion_id}/companion/threads");

    // A personal Companion Agent can customize its instructions without
    // creating another Session or granting access to another Companion.
    let (status, official_editor) = call(router.clone(), "GET",
        &format!("/api/agent-presets/{preset_id}/editor"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{official_editor}");
    let mut document = official_editor["data"]["draft"]["document"].clone();
    document["instructions"] = json!("COMPANION_CUSTOM_INSTRUCTION_V1");
    let (status, custom) = call(router.clone(), "POST", "/api/agent-presets", json!({
        "display_name": "Personal Companion Agent", "document": document,
    })).await;
    assert_eq!(status, StatusCode::OK, "{custom}");
    let custom_id = custom["data"]["preset"]["preset_id"].as_str().unwrap();
    let custom_selection = json!({ "kind": "preset", "preset_id": custom_id });
    let (status, options) = call(router.clone(), "GET",
        &format!("/api/product-agent-bindings/companion/{companion_id}?provider_id={provider_id}&model=step-3.7-flash"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{options}");
    assert_eq!(options["data"]["options"].as_array().unwrap().iter()
        .find(|option| option["selection"] == custom_selection).unwrap()["available"], true);

    let (status, other) = call(router.clone(), "POST", "/api/companion/companions",
        json!({"name":"Another Companion", "character":"mochi"})).await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    let other_id = other["data"]["companion_id"].as_str().unwrap();
    let (status, cross_target) = call(router.clone(), "PUT",
        &format!("/api/product-agent-bindings/companion/{other_id}"), json!({
            "selection": custom_selection, "conversation_id": conversation_id,
        })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{cross_target}");
    let (status, missing_profile) = call(router.clone(), "PUT",
        &format!("/api/product-agent-bindings/companion/{}", uuid::Uuid::now_v7()), json!({
            "selection": custom_selection,
        })).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "model-free writes must validate the real profile: {missing_profile}");

    let (status, other_provider) = call(router.clone(), "POST", "/api/providers", json!({
        "platform": "stepfun-plan", "name": "Future Companion default",
        "base_url": format!("{}/step_plan/v1", upstream.uri()),
        "auth_scheme": "bearer", "credentials": {"api_keys":["test-only"]}, "enabled": true,
        "initial_model":{"model":"step-3.7-flash","enabled":true,
            "capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","provider_params":{}}]},
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{other_provider}");
    let future_model = json!({"provider_id":other_provider["data"]["provider_id"],"model":"step-3.7-flash"});
    let (status, differently_routed) = call(router.clone(), "POST", "/api/agent-presets/from-template/companion.default", json!({
        "display_name":"companion.default", "reuse_existing":true, "model":future_model,
    })).await;
    assert_eq!(status, StatusCode::OK, "{differently_routed}");
    let (status, custom_editor) = call(router.clone(), "GET", &format!("/api/agent-presets/{custom_id}/editor"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{custom_editor}");
    let mut routed_draft = custom_editor["data"]["draft"].clone();
    for field in ["model_route_refs", "chat_route_records"] {
        routed_draft["document"][field] = differently_routed["data"]["draft"]["document"][field].clone();
    }
    let (status, routed) = call(router.clone(), "POST", &format!("/api/agent-presets/{custom_id}/revisions"), json!({
        "expected_current_revision":routed_draft["current_revision"], "draft":routed_draft,
        "reason":"Authored model differs from the frozen Companion Session",
    })).await;
    assert_eq!(status, StatusCode::OK, "{routed}");
    let product_path = format!("/api/product-agent-bindings/companion/{companion_id}");
    sqlx::query("CREATE TRIGGER fixture_companion_default_failure BEFORE INSERT ON product_agent_selections WHEN NEW.target_kind = 'companion' BEGIN SELECT RAISE(FAIL, 'fixture default write failure'); END")
        .execute(services.database.pool()).await.unwrap();
    let (status, applied) = call(router.clone(), "PUT", &product_path, json!({"conversation_id":conversation_id, "update_default":true,
        "selection": custom_selection, "model": future_model,
    })).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{applied}");
    assert_eq!(applied["code"], "COMPANION_AGENT_DEFAULT_SAVE_FAILED");
    assert_eq!(applied["details"]["active_agent_applied"], true);
    let (status, authoritative) = call(router.clone(), "GET", &format!("{product_path}?conversation_id={conversation_id}"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{authoritative}");
    assert_eq!(authoritative["data"]["selection"], custom_selection,
        "an active model variant must retain the original authorial selection, including after default persistence failure");
    let (status, repeated_failure) = call(router.clone(), "PUT", &product_path, json!({"conversation_id":conversation_id, "update_default":true, "selection":custom_selection})).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{repeated_failure}");
    assert_eq!(repeated_failure["code"], "COMPANION_AGENT_DEFAULT_SAVE_FAILED");
    sqlx::query("DROP TRIGGER fixture_companion_default_failure").execute(services.database.pool()).await.unwrap();
    let (status, retry) = call(router.clone(), "PUT", &product_path, json!({"conversation_id":conversation_id, "update_default":true, "selection":custom_selection,"model":future_model})).await;
    assert_eq!(status, StatusCode::OK, "explicit retry must reconcile defaults without switching again: {retry}");
    let (status, current) = call(router.clone(), "GET",
        &format!("/api/agent-sessions/{conversation_id}/projection"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{current}");
    assert_ne!(current["data"]["preset_id"], custom_id, "a different authored Chat route must produce an immutable Session-only variant");
    assert!(current["data"]["agent_snapshot"]["instructions"].as_str().unwrap().contains("COMPANION_CUSTOM_INSTRUCTION_V1"),
        "the exact immutable model variant must preserve the selected author's instructions");
    assert_eq!(current["data"]["extra"]["companion_session"], true);
    assert_eq!(current["data"]["extra"]["companion_id"], companion_id);
    assert_eq!(current["data"]["agent_snapshot"]["resolved_model"]["provider_id"], provider_id,
        "changing the profile Agent must preserve this Session's frozen model");
    let resources = current["data"]["agent_snapshot"]["canonical_binding"]["typed_resource_bindings"].as_array().unwrap();
    assert!(resources.iter().filter(|resource| matches!(resource["resource_kind"].as_str(), Some("companion"|"companion_memory")))
        .all(|resource| resource["resource_id"] == companion_id));
    let (status, reopened) = call(router.clone(), "POST", &path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{reopened}");
    assert_eq!(reopened["data"]["conversation_id"], conversation_id);
    let (status, turn) = call(router.clone(), "POST",
        &format!("/api/agent-sessions/{conversation_id}/turns"), json!({
            "idempotency_key": uuid::Uuid::now_v7().to_string(), "input":{"content":"use the personal Agent"},
        })).await;
    assert_eq!(status, StatusCode::OK, "{turn}");
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let completed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id = ? AND kind = 'turn/completed'")
                .bind(conversation_id).fetch_one(services.database.pool()).await.unwrap();
            if completed == 2 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }).await.expect("custom Companion turn must complete");
    let requests = upstream.received_requests().await.unwrap();
    let actual = requests.last().unwrap().body_json::<Value>().unwrap();
    let system = actual["messages"].as_array().unwrap().iter().filter(|message| message["role"] == "system")
        .map(|message| message["content"].to_string()).collect::<Vec<_>>().join("\n");
    assert!(system.contains("COMPANION_CUSTOM_INSTRUCTION_V1"), "custom Agent instructions must reach the real model: {system}");
    assert!(system.contains("Agent-bound companion"), "the exact Companion persona must remain effective");

    // Editing the selected preset is frozen until an explicit reapply. Reapply
    // uses a canonical Agent transition and retains all previous messages.
    let (status, editor) = call(router.clone(), "GET", &format!("/api/agent-presets/{custom_id}/editor"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{editor}");
    let mut draft = editor["data"]["draft"].clone();
    draft["document"]["instructions"] = json!("COMPANION_CUSTOM_INSTRUCTION_V2");
    let (status, saved) = call(router.clone(), "POST", &format!("/api/agent-presets/{custom_id}/revisions"), json!({
        "expected_current_revision": draft["current_revision"], "draft": draft, "reason":"Companion settings reapply regression",
    })).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (status, frozen) = call(router.clone(), "GET",
        &format!("/api/agent-sessions/{conversation_id}/projection"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{frozen}");
    assert_eq!(frozen["data"]["agent_snapshot"]["canonical_binding"]["preset_revision_ref"], current["data"]["agent_snapshot"]["canonical_binding"]["preset_revision_ref"]);
    let (status, reapplied) = call(router.clone(), "PUT", &product_path, json!({"update_default":true,
        "selection": custom_selection, "conversation_id": conversation_id,
    })).await;
    assert_eq!(status, StatusCode::OK, "model-free reapply must preserve the active model: {reapplied}");
    let (status, latest) = call(router.clone(), "GET",
        &format!("/api/agent-sessions/{conversation_id}/projection"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{latest}");
    assert_ne!(latest["data"]["agent_snapshot"]["canonical_binding"]["preset_revision_ref"], frozen["data"]["agent_snapshot"]["canonical_binding"]["preset_revision_ref"]);
    let transition_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id = ? AND kind = 'session/agent-binding-changed'")
        .bind(conversation_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(transition_count, 2);
    let (status, noop) = call(router.clone(), "PUT", &product_path, json!({"conversation_id":conversation_id, "update_default":true, "selection":custom_selection})).await;
    assert_eq!(status, StatusCode::OK, "exact reapply must be a successful no-op: {noop}");
    let after_noop: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id = ? AND kind = 'session/agent-binding-changed'")
        .bind(conversation_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(after_noop, transition_count);
    let (status, turn) = call(router.clone(), "POST",
        &format!("/api/agent-sessions/{conversation_id}/turns"), json!({
            "idempotency_key":uuid::Uuid::now_v7().to_string(), "input":{"content":"use the reapplied Agent"},
        })).await;
    assert_eq!(status, StatusCode::OK, "{turn}");
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let completed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id = ? AND kind = 'turn/completed'")
                .bind(conversation_id).fetch_one(services.database.pool()).await.unwrap();
            if completed == 3 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }).await.expect("reapplied Companion turn must complete");
    let requests = upstream.received_requests().await.unwrap();
    let actual = requests.last().unwrap().body_json::<Value>().unwrap();
    let system = actual["messages"].as_array().unwrap().iter().filter(|message| message["role"] == "system")
        .map(|message| message["content"].to_string()).collect::<Vec<_>>().join("\n");
    assert!(system.contains("COMPANION_CUSTOM_INSTRUCTION_V2"), "explicit reapply must reach the actual model: {system}");
    assert!(!system.contains("COMPANION_CUSTOM_INSTRUCTION_V1"), "the old Agent instructions must not remain active");
    let (status, history) = call(router.clone(), "GET",
        &format!("/api/agent-sessions/{conversation_id}/message-history?page_size=100"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert!(history["data"]["items"].as_array().unwrap().iter().any(|message| message["content"]["content"] == COMPANION_REPLY));

    // Equal compiled contracts can belong to different authored Agents. The
    // explicit source choice must survive cache reuse and later source edits.
    let (status, author) = call(router.clone(), "GET", &format!("/api/agent-presets/{custom_id}/editor"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{author}");
    let (status, alias) = call(router.clone(), "POST", "/api/agent-presets", json!({
        "display_name":"Personal Companion Agent", "document":author["data"]["draft"]["document"],
    })).await;
    assert_eq!(status, StatusCode::OK, "{alias}");
    let alias_id = alias["data"]["preset"]["preset_id"].as_str().unwrap();
    let alias_selection = json!({"kind":"preset","preset_id":alias_id});
    let (status, selected_alias) = call(router.clone(), "PUT", &product_path, json!({"conversation_id":conversation_id, "update_default":true, "selection":alias_selection})).await;
    assert_eq!(status, StatusCode::OK, "{selected_alias}");
    let (status, alias_options) = call(router.clone(), "GET", &format!("{product_path}?conversation_id={conversation_id}"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{alias_options}");
    assert_eq!(alias_options["data"]["selection"], alias_selection);
    let alias_transitions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id = ? AND kind = 'session/agent-binding-changed'")
        .bind(conversation_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(alias_transitions, 3, "a distinct authored selection needs an exact immutable target transition");
    let (status, alias_noop) = call(router.clone(), "PUT", &product_path, json!({"conversation_id":conversation_id, "update_default":true, "selection":alias_selection})).await;
    assert_eq!(status, StatusCode::OK, "{alias_noop}");
    let alias_noop_transitions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id = ? AND kind = 'session/agent-binding-changed'")
        .bind(conversation_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(alias_noop_transitions, alias_transitions);
    let mut alias_draft = alias["data"]["draft"].clone();
    alias_draft["document"]["instructions"] = json!("COMPANION_CUSTOM_INSTRUCTION_V3");
    let (status, saved_alias) = call(router.clone(), "POST", &format!("/api/agent-presets/{alias_id}/revisions"), json!({
        "expected_current_revision":alias_draft["current_revision"],"draft":alias_draft,
    })).await;
    assert_eq!(status, StatusCode::OK, "{saved_alias}");
    let (status, alias_reapplied) = call(router.clone(), "PUT", &product_path, json!({"conversation_id":conversation_id, "update_default":true, "selection":alias_selection})).await;
    assert_eq!(status, StatusCode::OK, "{alias_reapplied}");
    let (status, turn) = call(router.clone(), "POST", &format!("/api/agent-sessions/{conversation_id}/turns"), json!({
        "idempotency_key":uuid::Uuid::now_v7().to_string(),"input":{"content":"use the selected authored Agent"},
    })).await;
    assert_eq!(status, StatusCode::OK, "{turn}");
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let completed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id = ? AND kind = 'turn/completed'")
                .bind(conversation_id).fetch_one(services.database.pool()).await.unwrap();
            if completed == 4 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }).await.expect("reapplied authored alias must complete");
    let requests = upstream.received_requests().await.unwrap();
    let actual = requests.last().unwrap().body_json::<Value>().unwrap();
    let system = actual["messages"].as_array().unwrap().iter().filter(|message| message["role"] == "system")
        .map(|message| message["content"].to_string()).collect::<Vec<_>>().join("\n");
    assert!(system.contains("COMPANION_CUSTOM_INSTRUCTION_V3"), "the selected source's edits must reach the model: {system}");

    let (status, generic) = call(router.clone(), "POST", "/api/agent-sessions", json!({
        "preset_id":custom_id, "model":{"provider_id":provider_id,"model":"step-3.7-flash"},
        "resource_selections":[
            {"resource_kind":"companion","resource_id":companion_id},
            {"resource_kind":"companion_memory","resource_id":companion_id},
            {"resource_kind":"scheduler","resource_id":"installation-scheduler"},
        ],
    })).await;
    assert_eq!(status, StatusCode::OK, "{generic}");
    let generic_id = generic["data"]["agent_session_id"].as_str().unwrap();
    let (status, generic_projection) = call(router.clone(), "GET",
        &format!("/api/agent-sessions/{generic_id}/projection"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{generic_projection}");
    assert!(generic_projection["data"]["extra"]["companion_session"].is_null(),
        "a resource grant alone must not turn a personal Agent launch into a Companion product Session");

    let (status, future_default) = call(router.clone(), "PUT",
        &format!("/api/product-agent-bindings/companion/{other_id}"), json!({
            "selection":custom_selection, "model":{"provider_id":provider_id,"model":"step-3.7-flash"},
        })).await;
    assert_eq!(status, StatusCode::OK, "{future_default}");
    let (status, configured_other) = call(router.clone(), "PATCH",
        &format!("/api/companion/companions/{other_id}"), json!({"model":{"provider_id":provider_id,"model":"step-3.7-flash"}})).await;
    assert_eq!(status, StatusCode::OK, "{configured_other}");
    let (status, future_thread) = call(router.clone(), "POST",
        &format!("/api/companion/companions/{other_id}/companion/threads"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{future_thread}");
    let future_id = future_thread["data"]["conversation_id"].as_str().unwrap();
    let (status, future_projection) = call(router.clone(), "GET",
        &format!("/api/agent-sessions/{future_id}/projection"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{future_projection}");
    assert_eq!(future_projection["data"]["extra"]["companion_id"], other_id);
    assert_eq!(future_projection["data"]["agent_snapshot"]["preset_name"], "Personal Companion Agent",
        "the resolver must honor saved personal selections for newly opened Companion Sessions");

    let session = nomifun_agent_contracts::AgentSessionId::from(conversation_id.to_owned());
    let store = nomifun_agent_session::AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    store.start_turn(&session, nomifun_agent_contracts::EventProducerId::from("session_api"),
        nomifun_agent_contracts::IdempotencyKey::from(uuid::Uuid::now_v7().to_string()),
        nomifun_agent_contracts::OperationId::from(uuid::Uuid::now_v7().to_string()),
        nomifun_agent_contracts::StrictJsonValue(json!({"content":"active settings fence"}))).await.unwrap();
    let before_default = sqlx::query_scalar::<_,String>("SELECT selection_json FROM product_agent_selections WHERE target_kind = 'companion' AND target_id = ?")
        .bind(companion_id).fetch_one(services.database.pool()).await.unwrap();
    let (status, busy) = call(router.clone(), "PUT", &product_path, json!({"conversation_id":conversation_id, "update_default":true,
        "selection":{"kind":"template","template_key":"companion.default"}, "model":future_model,
    })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{busy}");
    assert_eq!(busy["code"], "AGENT_SESSION_TURN_ACTIVE");
    let after_default = sqlx::query_scalar::<_,String>("SELECT selection_json FROM product_agent_selections WHERE target_kind = 'companion' AND target_id = ?")
        .bind(companion_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(after_default, before_default, "busy changes must not mutate the saved default");
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn companion_official_agent_edits_require_an_exact_preview_and_preserve_history() {
    let call = companion_agent_test_request;
    let fixture = started_companion_fixture().await;
    let router = fixture.router;
    let services = fixture.services;
    let upstream = fixture.upstream;
    let provider_id = fixture.provider_id.as_str();
    let companion_id = fixture.companion_id.as_str();
    let conversation_id = fixture.conversation_id.as_str();
    let enabled = fixture.snapshot["enabled_capabilities"].as_array().unwrap().iter()
        .map(|capability| capability.as_str().unwrap()).collect::<std::collections::BTreeSet<_>>();
    let path = format!("/api/companion/companions/{companion_id}/companion/threads");
    let original_through: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=?")
        .bind(conversation_id).fetch_one(services.database.pool()).await.unwrap();
    let original_facts = original_history_fact_fingerprints(&services, conversation_id, original_through).await;
    let unchanged = fixture.unchanged;

    // The library's owner-scoped official ID is directly editable. Saving a
    // revision must not silently replace this existing Companion Session.
    const ADDED_RULE: &str = "OFFICIAL_COMPANION_EXTENDED_RULES_V2";
    let (status, library) = call(router.clone(), "GET", "/api/agent-preset-templates", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{library}");
    let official_id = library["data"]["official_presets"]["companion.default"]["preset_id"]
        .as_str().expect("editable owner-scoped official Companion ID");
    let (status, editor) = call(router.clone(), "GET",
        &format!("/api/agent-presets/{official_id}/editor"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{editor}");
    assert_eq!(editor["data"]["preset"]["source"], "official");
    let (status, recommendation) = call(router.clone(), "GET",
        &format!("/api/agent-presets/{official_id}/recommended-capabilities"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{recommendation}");
    let mut draft = recommendation["data"]["draft"].clone();
    let gained_modules = recommendation["data"]["added_capabilities"].as_array().unwrap().iter()
        .map(|selection| selection["capability"]["id"].as_str().unwrap())
        .filter(|id| !enabled.contains(id)).collect::<std::collections::BTreeSet<_>>();
    let original_instructions = draft["document"]["instructions"].as_str().unwrap().to_owned();
    draft["document"]["instructions"] = json!(format!("{original_instructions}\nOFFICIAL_COMPANION_EXTENDED_RULES_V1"));
    let (status, saved) = call(router.clone(), "POST",
        &format!("/api/agent-presets/{official_id}/revisions"), json!({
            "expected_current_revision": editor["data"]["revision"]["reference"], "draft": draft,
        })).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["data"]["preset"]["preset_id"], official_id);
    assert_eq!(saved["data"]["preset"]["source"], "official");
    assert_eq!(saved["data"]["revision"]["document"]["enabled_capabilities"],
        draft["document"]["enabled_capabilities"]);
    assert_eq!(saved["data"]["revision"]["document"]["instructions"], draft["document"]["instructions"]);

    let old_binding = unchanged["data"]["agent_snapshot"]["canonical_binding"].clone();
    let (status, reopened) = call(router.clone(), "POST", &path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{reopened}");
    assert_eq!(reopened["data"]["conversation_id"], conversation_id,
        "reopening the Companion must retain its existing Session after the official edit");
    let session_path = format!("/api/agent-sessions/{conversation_id}");
    let (status, frozen) = call(router.clone(), "GET", &session_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{frozen}");
    assert_eq!(frozen["data"]["session"]["agent_binding"], old_binding);
    let selection = json!({ "kind": "preset", "preset_id": official_id });
    let preview_path = format!("{session_path}/agent-switch/preview");
    let (status, preview) = call(router.clone(), "POST", &preview_path,
        json!({"selection": selection})).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["data"]["can_apply"], true, "{preview}");
    assert_eq!(preview["data"]["capabilities"]["gained"].as_array().unwrap().iter()
        .map(|value| value.as_str().unwrap()).collect::<std::collections::BTreeSet<_>>(), gained_modules);
    assert_eq!(preview["data"]["model"]["preserved"], true);
    assert_eq!(preview["data"]["model"]["provider_id"], provider_id);
    assert_eq!(preview["data"]["model"]["model"], "step-3.7-flash");
    assert_eq!(preview["data"]["resources"]["dropped"], json!([]));
    let (status, still_frozen) = call(router.clone(), "GET", &session_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{still_frozen}");
    assert_eq!(still_frozen["data"]["session"]["agent_binding"], old_binding,
        "preview must preserve the frozen Session binding");

    // A second edit invalidates the exact preview, even though the source
    // Session's binding version has not changed.
    draft["current_revision"] = saved["data"]["revision"]["reference"].clone();
    draft["document"]["instructions"] = json!(format!("{original_instructions}\n{ADDED_RULE}"));
    let (status, saved_again) = call(router.clone(), "POST",
        &format!("/api/agent-presets/{official_id}/revisions"), json!({
            "expected_current_revision": saved["data"]["revision"]["reference"], "draft": draft,
        })).await;
    assert_eq!(status, StatusCode::OK, "{saved_again}");
    let switch_path = format!("{session_path}/agent");
    let (status, stale) = call(router.clone(), "PUT", &switch_path, json!({
        "selection": selection, "handoff_mode": "context_only",
        "expected_binding_version": preview["data"]["expected_binding_version"],
        "expected_resolved_snapshot_ref": preview["data"]["target"]["resolved_snapshot_ref"],
    })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["code"], "AGENT_SESSION_BINDING_CHANGED");
    assert_eq!(stale["details"]["stage"], "target_preview");
    let (status, rejected_binding) = call(router.clone(), "GET", &session_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{rejected_binding}");
    assert_eq!(rejected_binding["data"]["session"]["agent_binding"], old_binding);

    let (status, current_preview) = call(router.clone(), "POST", &preview_path,
        json!({"selection": selection})).await;
    assert_eq!(status, StatusCode::OK, "{current_preview}");
    assert_eq!(current_preview["data"]["can_apply"], true, "{current_preview}");
    assert_ne!(current_preview["data"]["target"]["resolved_snapshot_ref"],
        preview["data"]["target"]["resolved_snapshot_ref"]);
    let (status, applied) = call(router.clone(), "PUT", &switch_path, json!({
        "selection": selection, "handoff_mode": "context_only",
        "expected_binding_version": current_preview["data"]["expected_binding_version"],
        "expected_resolved_snapshot_ref": current_preview["data"]["target"]["resolved_snapshot_ref"],
    })).await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    assert_eq!(applied["data"]["conversation"]["conversation_id"], conversation_id);
    assert_eq!(applied["data"]["effective_from"], "next_turn");
    let refreshed = &applied["data"]["conversation"]["agent_snapshot"];
    assert!(refreshed["instructions"].as_str().unwrap().contains(ADDED_RULE),
        "the applied immutable Snapshot must preserve the edited official instructions before Runtime assembly");
    let new_binding = &refreshed["canonical_binding"];
    assert_eq!(new_binding["binding_version"].as_u64(),
        Some(old_binding["binding_version"].as_u64().unwrap() + 1));
    let new_resources = new_binding["typed_resource_bindings"].as_array().unwrap();
    for previous in old_binding["typed_resource_bindings"].as_array().unwrap() {
        let next = new_resources.iter().find(|resource|
            resource["resource_kind"] == previous["resource_kind"]).unwrap();
        for field in ["resource_id", "owner_id", "typed_parameters", "connection_config_ref"] {
            assert_eq!(next[field], previous[field], "refresh must keep the original resource choice: {field}");
        }
    }
    for kind in ["workspace", "process_session", "project_memory", "asset_library", "scheduler"] {
        assert!(new_resources.iter().any(|resource| resource["resource_kind"] == kind),
            "an upgraded Companion must automatically receive {kind}");
    }
    let workspace = new_resources.iter().find(|resource| resource["resource_kind"] == "workspace").unwrap();
    let process = new_resources.iter().find(|resource| resource["resource_kind"] == "process_session").unwrap();
    assert_eq!(workspace["typed_parameters"]["workspace_root"], process["typed_parameters"]["workspace_root"]);
    assert_eq!(new_binding["resolved_snapshot_ref"], current_preview["data"]["target"]["resolved_snapshot_ref"]);
    assert_eq!(new_binding["preset_revision_ref"]["preset_id"], current_preview["data"]["target"]["preset_id"]);
    assert_eq!(new_binding["preset_revision_ref"]["revision"], current_preview["data"]["target"]["preset_revision"]);
    assert_eq!(refreshed["enabled_capability_actions"]["web.research"], json!(["web.research/fetch","web.research/search"]));
    let (status, preserved_history) = call(router.clone(), "GET",
        &format!("{session_path}/message-history?page_size=50"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{preserved_history}");
    assert_history_identity_and_content_preserved(&fixture.history, &preserved_history);
    assert_eq!(original_history_fact_fingerprints(&services, conversation_id, original_through).await,
        original_facts, "explicitly applying the saved configuration cannot rewrite any original Event or Payload byte");

    let (status, next_turn) = call(router.clone(), "POST", &format!("{session_path}/turns"), json!({
        "idempotency_key": uuid::Uuid::now_v7().to_string(),
        "input": {"content": "Continue with your extended official capabilities."},
    })).await;
    assert_eq!(status, StatusCode::OK, "{next_turn}");
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let completed: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM agent_events WHERE session_id = ? AND kind = 'turn/completed'",
            ).bind(conversation_id).fetch_one(services.database.pool()).await.unwrap();
            if completed == 2 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }).await.expect("refreshed Companion turn must complete using the same Session");
    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let continued = requests[1].body_json::<Value>().unwrap();
    assert_eq!(continued["model"], "step-3.7-flash");
    let rules = continued["messages"].as_array().unwrap().iter()
        .filter(|message| message["role"] == "system")
        .map(|message| message["content"].to_string()).collect::<Vec<_>>().join("\n");
    assert!(rules.contains(ADDED_RULE), "the applied official instructions must reach the next real model request");
    assert!(continued["messages"].to_string().contains(COMPANION_REPLY), "the same canonical history must reach the refreshed Runtime");
    let (status, library) = call(router.clone(), "GET", "/api/agent-preset-templates", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{library}");
    assert_eq!(library["data"]["official_presets"]["companion.default"]["preset_id"], official_id);
    assert_eq!(library["data"]["user_presets"], json!([]), "editing the official Agent must not require a copy");
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
        "creation.media", "creative.workshop", "office",
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
    let request = json!({ "display_name": "chat.minimal", "reuse_existing": true,
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
    assert_eq!(original["data"]["preset"]["display_name"], "chat.minimal");

    let projection = router.clone().oneshot(Request::builder()
        .uri(format!("/api/agent-sessions/{session_id}/projection"))
        .header("x-nomi-local-trust", TRUST)
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(projection.status(), StatusCode::OK);
    let projection: Value = serde_json::from_slice(&axum::body::to_bytes(
        projection.into_body(), 4 * 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(projection["data"]["extra"]["official_template_key"], "chat.minimal");

    assert!(upstream.received_requests().await.unwrap().is_empty(),
        "Agent preparation must not execute the selected commercial model");

    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn agent_session_model_selection_is_exact_persistent_and_keeps_the_agent_unchanged() {
    let client = LocalJsonClient(TRUST);
    const TRUST: &str = "agent-model-selection-test";
    async fn delete(router: axum::Router, path: &str, key: &str) -> (StatusCode, Value) {
        let response = router.oneshot(Request::builder().method("DELETE").uri(path)
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
            .header("Idempotency-Key", key).body(Body::from("{}")).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    let (router, services) = common::build_local_trust_app(TRUST).await;
    fs::create_dir_all(&services.work_dir).unwrap();
    fs::write(
        services.work_dir.join("work-root-must-not-leak.txt"),
        b"installation-owned",
    )
    .unwrap();
    create_chat_provider(&router, TRUST, "Primary model fixture", "primary-chat", 0).await;
    let fallback = create_chat_provider(&router, TRUST, "Fallback model fixture", "fallback-chat", 1).await;
    let (status, original) = client.call(router.clone(), "POST", "/api/agent-presets", json!({
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
    let selection = json!({ "provider_id": fallback["provider_id"], "model": "fallback-chat" });
    let mut sessions = Vec::new();
    let mut session_ids = Vec::new();
    let mut session_workspaces = Vec::new();
    for _ in 0..2 {
        let (status, result) = client.call(router.clone(), "POST", "/api/agent-sessions", json!({
            "preset_id": preset_id, "title": "Chosen model", "model": selection,
            "resource_selections": [{ "resource_kind": "workspace", "resource_id": "default-workspace" }]
        })).await;
        assert_eq!(status, StatusCode::OK, "{result}");
        let id = result["data"]["agent_session_id"].as_str().unwrap();
        let (status, observation) = client.call(router.clone(), "GET", &format!("/api/agent-sessions/{id}"), json!({})).await;
        assert_eq!(status, StatusCode::OK);
        let binding = result["data"]["agent_binding"].clone();
        assert_eq!(observation["data"]["session"]["agent_binding"], binding);
        let (status, projection) = client.call(
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
        assert_eq!(projection["data"]["extra"]["temp_workspace_id"], id);
        let workspace = PathBuf::from(
            projection["data"]["extra"]["workspace"]
                .as_str()
                .expect("managed workspace path"),
        );
        assert_eq!(
            workspace.parent(),
            Some(services.work_dir.join("conversations").as_path()),
        );
        assert_eq!(workspace.file_name().and_then(|name| name.to_str()), Some(id));
        assert!(workspace.is_dir(), "managed workspace must exist at Session creation");
        let (status, listing) = client.call(
            router.clone(),
            "GET",
            &format!("/api/agent-sessions/{id}/workspace?path=."),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{listing}");
        assert!(
            listing["data"].as_array().is_some_and(|entries| entries.iter().all(|entry|
                entry["name"] != "work-root-must-not-leak.txt"
            )),
            "a managed Session listing must not expose siblings from the installation work root",
        );
        if sessions.is_empty() {
            let original_workspace = projection["data"]["extra"]["workspace"].clone();
            let (status, enabled) = client.call(
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

            let (status, refreshed) = client.call(
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

            let (status, disabled) = client.call(
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
        session_ids.push(id.to_owned());
        session_workspaces.push(workspace);
        let variant = binding["preset_revision_ref"]["preset_id"].as_str().unwrap();
        let (status, editor) = client.call(router.clone(), "GET", &format!("/api/agent-presets/{variant}/editor"), json!({})).await;
        assert_eq!(status, StatusCode::OK);
        let record = &editor["data"]["revision"]["document"]["chat_route_records"]["agent_chat"];
        assert_eq!(record["primary"]["provider_id"], selection["provider_id"]);
        assert_eq!(record["primary"]["model"], selection["model"]);
        assert_eq!(record["failovers"], json!([]));
        let mut expected_capabilities = original["revision"]["document"]["enabled_capabilities"].as_array().unwrap().clone();
        expected_capabilities.push(json!({
            "capability": {"id": "agent.tool-discovery"},
            "action_allowlist": ["tool.discovery.rank"]
        }));
        expected_capabilities.sort_by(|left, right| left["capability"]["id"].as_str().cmp(&right["capability"]["id"].as_str()));
        assert_eq!(editor["data"]["revision"]["document"]["enabled_capabilities"], json!(expected_capabilities),
            "Session compilation may add only the exact automatic discovery grant");
        for field in ["skill_bindings", "system_role_provider_overrides", "persona", "instructions"] {
            assert_eq!(editor["data"]["revision"]["document"][field], original["revision"]["document"][field], "must retain {field}");
        }
        sessions.push(binding);
    }
    assert_eq!(
        sessions[0]["typed_resource_bindings"],
        sessions[1]["typed_resource_bindings"],
        "same model resolves the same owner-scoped resources"
    );
    assert_ne!(
        session_workspaces[0], session_workspaces[1],
        "default Sessions must never share one writable workspace",
    );
    let (status, fork) = client.call(
        router.clone(),
        "POST",
        &format!("/api/agent-sessions/{}/forks", session_ids[1]),
        json!({
            "target_agent_binding": sessions[1],
            "parent_through_seq": 0,
            "title": "Isolated child",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{fork}");
    let child_id = fork["data"]["child_agent_session_id"]
        .as_str()
        .expect("fork child Session id");
    let (status, child_projection) = client.call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{child_id}/projection"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{child_projection}");
    let child_workspace = PathBuf::from(
        child_projection["data"]["extra"]["workspace"]
            .as_str()
            .expect("fork child workspace"),
    );
    assert!(child_workspace.is_dir(), "fork must materialize its workspace before returning");
    assert_ne!(
        child_workspace, session_workspaces[1],
        "a fork of a managed Session must receive its own writable workspace",
    );
    let artifact_source = session_workspaces[0].join("artifact-source.txt");
    fs::write(&artifact_source,b"managed artifact before delete").unwrap();
    let artifact_store = nomifun_file::WorkspaceArtifactStore::new(&session_workspaces[0]).unwrap();
    let artifact = artifact_store.publish("artifact-source.txt",None).unwrap();
    let cached = artifact_store.read(&artifact.artifact_id,0,1024).unwrap();
    assert!(cached.complete);
    assert_eq!(cached.sha256,artifact.artifact_id);
    let delete_path = format!("/api/agent-sessions/{}",session_ids[0]);
    let delete_key = "managed-workspace-live-artifact-delete";
    let (first_status,first_delete) = delete(router.clone(),&delete_path,delete_key).await;
    if first_status != StatusCode::OK {
        assert_eq!(first_status,StatusCode::CONFLICT,"{first_delete}");
        assert_eq!(first_delete["code"],"AGENT_SESSION_WORKSPACE_CLEANUP_FAILED");
        let state:String = sqlx::query_scalar("SELECT state FROM agent_sessions WHERE agent_session_id=?")
            .bind(&session_ids[0]).fetch_one(services.database.pool()).await.unwrap();
        assert_eq!(state,"deleting","failed cleanup must retain the delete fence: {first_delete}");
        if let Ok(read) = artifact_store.read(&artifact.artifact_id,0,1024) {
            assert_eq!(read.sha256,artifact.artifact_id,"live reader cannot switch identity during failed cleanup");
        }
    }
    drop(artifact_store);
    let (status,deleted) = delete(router.clone(),&delete_path,delete_key).await;
    assert_eq!(status,StatusCode::OK,"{deleted}");
    if first_status == StatusCode::OK {
        assert_eq!(deleted["data"]["deleted_at"],first_delete["data"]["deleted_at"],
            "exact delete replay must retain the original tombstone");
    }
    let deleted_state:String = sqlx::query_scalar("SELECT state FROM agent_sessions WHERE agent_session_id=?")
        .bind(&session_ids[0]).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(deleted_state,"deleted");
    assert!(!session_workspaces[0].exists(), "delete must reclaim only its managed workspace");
    assert!(session_workspaces[1].is_dir(), "deleting one Session must preserve its sibling");
    let custom_workspace = services
        .work_dir
        .parent()
        .expect("test work root parent")
        .join("user-selected-project");
    fs::create_dir_all(&custom_workspace).unwrap();
    fs::write(custom_workspace.join("keep.txt"), b"user-owned").unwrap();
    let (status, custom) = client.call(
        router.clone(),
        "POST",
        "/api/agent-sessions",
        json!({
            "preset_id": preset_id,
            "title": "Custom workspace",
            "model": selection,
            "resource_selections": [{ "resource_kind": "workspace", "resource_id": "default-workspace" }],
            "workspace": custom_workspace.to_string_lossy(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{custom}");
    let custom_id = custom["data"]["agent_session_id"]
        .as_str()
        .expect("custom Session id");
    let (status, custom_projection) = client.call(
        router.clone(),
        "GET",
        &format!("/api/agent-sessions/{custom_id}/projection"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{custom_projection}");
    assert_eq!(custom_projection["data"]["extra"]["custom_workspace"], true);
    assert_eq!(
        custom_projection["data"]["extra"]["is_temporary_workspace"],
        false,
    );
    let (status, custom_deleted) = client.call(
        router.clone(),
        "DELETE",
        &format!("/api/agent-sessions/{custom_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{custom_deleted}");
    assert_eq!(
        fs::read_to_string(custom_workspace.join("keep.txt")).unwrap(),
        "user-owned",
        "Session deletion must never reclaim a user-selected workspace",
    );
    let (_, reloaded) = client.call(router.clone(), "GET", &format!("/api/agent-presets/{preset_id}/editor"), json!({})).await;
    assert_eq!(reloaded["data"]["revision"], original["revision"]);
    let (_, library) = client.call(router.clone(), "GET", "/api/agent-preset-templates", json!({})).await;
    assert_eq!(library["data"]["user_presets"].as_array().unwrap().len(), 1, "internal model variants stay out of the Agent library");
    let (status, official) = client.call(router.clone(), "POST", "/api/agent-presets/from-template/chat.minimal", json!({
        "display_name": "Official chosen model", "model_route_refs": {}, "chat_route_records": {},
        "model": selection, "reuse_existing": true
    })).await;
    assert_eq!(status, StatusCode::OK, "{official}");
    assert_eq!(official["data"]["revision"]["document"]["chat_route_records"]["agent_chat"]["primary"]["model"], selection["model"]);
    let (status, _) = client.call(router, "POST", "/api/agent-sessions", json!({
        "preset_id": preset_id, "model": { "provider_id": selection["provider_id"], "model": "nonexistent-model" }
    })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "an unknown model cannot silently fall back");
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn startup_recovers_a_deleting_managed_workspace_after_live_artifact_release() {
    const TRUST: &str = "deleting-workspace-startup-recovery";
    async fn call(
        router: &axum::Router,
        method: &str,
        path: &str,
        key: Option<&str>,
        body: Value,
    ) -> (StatusCode,Value) {
        let mut request = Request::builder().method(method).uri(path)
            .header("x-nomi-local-trust",TRUST).header("content-type","application/json");
        if let Some(key) = key { request = request.header("Idempotency-Key",key); }
        let response = router.clone().oneshot(request.body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(),4 * 1024 * 1024).await.unwrap();
        (status,serde_json::from_slice(&bytes).unwrap())
    }

    let root = tempfile::tempdir().unwrap();
    let config = AppConfig {
        data_dir:root.path().join("data"),work_dir:root.path().join("work"),
        auth_policy:AuthPolicy::TrustLocalToken,local_trust_secret:Some(TRUST.into()),
        ..Default::default()
    };
    fs::create_dir_all(&config.data_dir).unwrap();
    let database = nomifun_db::init_database(&config.database_path()).await.unwrap();
    let first = AppServices::from_config(database,&config).await.unwrap();
    let router = create_router(&first).await;
    let provider = create_chat_provider(&router,TRUST,"Delete recovery provider","delete-recovery-model",0).await;
    let model = json!({"provider_id":provider["provider_id"],"model":"delete-recovery-model"});
    let (status,preset) = call(&router,"POST","/api/agent-presets/from-template/coding.codex",None,json!({
        "reuse_existing":false,"display_name":"Delete recovery preset","model":model
    })).await;
    assert_eq!(status,StatusCode::OK,"{preset}");
    let preset_id = preset["data"]["preset"]["preset_id"].as_str().unwrap();
    let (status,session) = call(&router,"POST","/api/agent-sessions",None,json!({
        "preset_id":preset_id,"title":"Delete recovery Session","model":model,
        "resource_selections":[
            {"resource_kind":"workspace","resource_id":"default-workspace"},
            {"resource_kind":"process_session","resource_id":"managed-process-session"},
            {"resource_kind":"project_memory","resource_id":"default-project-memory"}
        ]
    })).await;
    assert_eq!(status,StatusCode::OK,"{session}");
    let session_id = session["data"]["agent_session_id"].as_str().unwrap().to_owned();
    let (status,projection) = call(&router,"GET",&format!("/api/agent-sessions/{session_id}/projection"),None,json!({})).await;
    assert_eq!(status,StatusCode::OK,"{projection}");
    let workspace = PathBuf::from(projection["data"]["extra"]["workspace"].as_str().unwrap());
    fs::write(workspace.join("source.txt"),b"startup recovery artifact").unwrap();
    let artifact_store = nomifun_file::WorkspaceArtifactStore::new(&workspace).unwrap();
    let artifact = artifact_store.publish("source.txt",None).unwrap();
    assert!(artifact_store.read(&artifact.artifact_id,0,1024).unwrap().complete);
    let delete_path = format!("/api/agent-sessions/{session_id}");
    let (status,blocked) = call(&router,"DELETE",&delete_path,Some("startup-delete-recovery"),json!({})).await;
    assert_eq!(status,StatusCode::CONFLICT,"{blocked}");
    assert_eq!(blocked["code"],"AGENT_SESSION_WORKSPACE_CLEANUP_FAILED");
    let state:String = sqlx::query_scalar("SELECT state FROM agent_sessions WHERE agent_session_id=?")
        .bind(&session_id).fetch_one(first.database.pool()).await.unwrap();
    assert_eq!(state,"deleting");
    assert!(workspace.exists());

    drop(artifact_store);
    drop(router);
    first.shutdown_browser_platform().await.unwrap();
    first.database.close().await;
    drop(first);

    let reopened = nomifun_db::init_database(&config.database_path()).await.unwrap();
    let second = AppServices::from_config(reopened,&config).await.unwrap();
    let restored_router = create_router(&second).await;
    let state:String = sqlx::query_scalar("SELECT state FROM agent_sessions WHERE agent_session_id=?")
        .bind(&session_id).fetch_one(second.database.pool()).await.unwrap();
    assert_eq!(state,"deleted","startup recovery must complete the fenced deletion before routes publish");
    assert!(!workspace.exists());
    let (status,replayed) = call(&restored_router,"DELETE",&delete_path,Some("startup-delete-recovery"),json!({})).await;
    assert_eq!(status,StatusCode::OK,"{replayed}");
    assert_eq!(replayed["data"]["state"],"deleted");
    drop(restored_router);
    second.shutdown_browser_platform().await.unwrap();
    second.database.close().await;
}

#[tokio::test]
async fn nomi_core_agent_settings_template_and_binding_surface_is_persistent() {
    const TRUST: &str = "agent-settings-local-trust";
    let (router, services) = common::build_local_trust_app(TRUST).await;
    create_chat_provider(&router, TRUST, "Primary settings fixture", "primary-chat", 0).await;
    create_chat_provider(&router, TRUST, "Fallback settings fixture", "fallback-chat", 1).await;
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-presets/from-template/chat.minimal")
                .header("x-nomi-local-trust", TRUST)
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
        "primary-chat",
        "the first configured Chat model should be selected"
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
                .header("x-nomi-local-trust", TRUST)
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
    let historical_status = historical.status();
    let historical: Value = serde_json::from_slice(&axum::body::to_bytes(
        historical.into_body(), 4 * 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(historical_status, StatusCode::OK, "retiring an Agent must preserve existing Session history: {historical}");
    assert_eq!(historical["data"]["session"]["agent_binding"]["resolved_snapshot_ref"], expected_snapshot);

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

#[cfg(target_os = "linux")]
#[path = "support/missing_default_workspace.rs"]
mod missing_default_workspace;
