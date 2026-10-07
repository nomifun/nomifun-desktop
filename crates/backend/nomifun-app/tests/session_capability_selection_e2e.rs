//! Composer extensions use immutable Session configurations for every preset.
//! All files and transports are fixture-owned; native execution uses local scripts.

use std::{fs, path::Path, sync::{Arc, Mutex}, time::Duration};

use axum::{Router, body::Body, http::{Request, StatusCode}};
use nomifun_agent_contracts::{
    AgentSessionId, CorrelationId, EventId, EventProducerId, IdempotencyKey,
    OperationId, SemanticSessionEventDraft, SessionEventAppend, SessionEventKind,
    SessionEventPayloadRef, StrictJsonValue,
};
use nomifun_agent_session::{AgentSessionStore, NativePauseState, TurnReceiptStatus};
use nomifun_app::{AppConfig, compatibility::{AppServices, create_router}};
use nomifun_auth::AuthPolicy;
use nomifun_skill_library::SkillPaths;
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{method, path}};

const TRUST: &str = "session-capability-selection-fixture";
const SKILL_A: &str = "fixture-review";
const SKILL_B: &str = "fixture-write";
const AUTO_SKILL: &str = "fixture-auto";
const UNKNOWN_SERVER: &str = "0195f7c0-7b6a-7c21-8f4a-1234567890fe";

async fn response(router: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let response = router.clone().oneshot(Request::builder().method(method).uri(uri)
        .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024).await.unwrap();
    let body = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
    (status, body)
}

async fn call(router: &Router, method: &str, uri: &str, body: Value) -> Value {
    let (status, value) = response(router, method, uri, body).await;
    assert!(matches!(status, StatusCode::OK | StatusCode::CREATED), "{method} {uri}: {status} {value}");
    value["data"].clone()
}

fn write_skill(parent: &Path, name: &str) {
    let directory = parent.join(name);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("SKILL.md"), format!(
        "---\nname: {name}\ndescription: Fixture {name} instructions\n---\nUse the fixture instructions for {name}.\n"
    )).unwrap();
}

fn selection(skills: &[&str], servers: &[&str]) -> Value {
    json!({ "skill_names": skills, "mcp_server_ids": servers })
}

fn selection_path(id: &str) -> String {
    format!("/api/agent-sessions/{id}/capability-selection")
}

fn non_mcp_resources(binding: &Value) -> Vec<Value> {
    binding["typed_resource_bindings"].as_array().unwrap().iter()
        .filter(|resource| resource["resource_kind"] != "mcp_server").cloned().collect()
}

struct Fixture {
    router: Router,
    services: AppServices,
    server_a: String,
    server_b: String,
    chat: MockServer,
    _mcp: MockServer,
    _directory: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::Builder::new().prefix("session-capability-selection-").tempdir().unwrap();
        let db = nomifun_db::init_database_memory().await.unwrap();
        let mut services = AppServices::from_config(db, &AppConfig {
            data_dir: directory.path().join("data"), work_dir: directory.path().join("work"),
            auth_policy: AuthPolicy::TrustLocalToken, local_trust_secret: Some(Arc::from(TRUST)),
            ..AppConfig::default()
        }).await.unwrap();
        // Ignore any developer-machine builtin override. Publication reads
        // only this fixture's explicit paths, and never a user's library.
        let skills = SkillPaths {
            data_dir: services.data_dir.clone(), user_skills_dir: services.data_dir.join("skills"),
            cron_skills_dir: services.data_dir.join("cron/skills"),
            builtin_skills_dir: services.data_dir.join("builtin-skills"),
            builtin_rules_dir: services.data_dir.join("builtin-rules"),
        };
        for path in [&skills.user_skills_dir, &skills.cron_skills_dir, &skills.builtin_skills_dir, &skills.builtin_rules_dir] {
            fs::create_dir_all(path).unwrap();
        }
        write_skill(&skills.user_skills_dir, SKILL_A);
        write_skill(&skills.user_skills_dir, SKILL_B);
        write_skill(&skills.builtin_skills_dir.join("auto-inject"), AUTO_SKILL);
        services.skill_paths = Arc::new(skills);
        let router = create_router(&services).await;
        let chat = MockServer::start().await;
        call(&router, "POST", "/api/providers", json!({
            "platform": "openai", "name": "Selection-only Chat fixture",
            "base_url": format!("{}/v1", chat.uri()), "auth_scheme": "bearer",
            "credentials": { "api_keys": ["test-only"] }, "enabled": true,
            "initial_model": { "model": "fixture-chat", "enabled": true, "capabilities": [{
                "task": "chat", "traits": [], "protocol": "openai.chat_text",
                "connection_role": "default", "provider_params": {}
            }] }
        })).await;
        let mcp = MockServer::start().await;
        Mock::given(method("POST")).and(path("/mcp"))
            .respond_with(|request: &wiremock::Request| {
                let request: Value = request.body_json().unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "notifications/initialized" => return ResponseTemplate::new(202),
                    "initialize" => json!({
                        "protocolVersion": nomifun_mcp::owner::MCP_PROTOCOL_VERSION,
                        "capabilities": {}, "serverInfo": { "name": "fixture", "version": "1.0.0" }
                    }),
                    "tools/list" => json!({ "tools": [{
                        "name": "lookup", "description": "Fixture lookup",
                        "inputSchema": { "type": "object", "additionalProperties": false }
                    }] }),
                    "tools/call" => {
                        assert_eq!(request["params"]["name"], "lookup");
                        assert_eq!(request["params"]["arguments"], json!({}));
                        json!({ "isError": false, "content": [{ "type": "text", "text": "NATIVE_MCP_RESULT_OK" }] })
                    }
                    other => panic!("selection must not execute MCP tools: {other}"),
                };
                ResponseTemplate::new(200).set_body_json(json!({
                    "jsonrpc": "2.0", "id": request["id"], "result": result
                }))
            }).mount(&mcp).await;
        let server_a = Self::ready_mcp(&router, &mcp, "selection-server-a").await;
        let server_b = Self::ready_mcp(&router, &mcp, "selection-server-b").await;
        Self { router, services, server_a, server_b, chat, _mcp: mcp, _directory: directory }
    }

    async fn ready_mcp(router: &Router, mcp: &MockServer, name: &str) -> String {
        let transport = json!({ "type": "http", "url": format!("{}/mcp", mcp.uri()) });
        let server = call(router, "POST", "/api/mcp/servers", json!({ "name": name, "transport": transport })).await;
        let id = server["mcp_server_id"].as_str().unwrap().to_owned();
        call(router, "POST", &format!("/api/mcp/servers/{id}/toggle"), Value::Null).await;
        let result = call(router, "POST", "/api/mcp/test-connection", json!({
            "mcp_server_id": id, "name": name, "transport": transport
        })).await;
        assert_eq!(result["success"], true, "{result}");
        id
    }

    async fn preset(&self, workspace: bool) -> Value {
        let enabled = if workspace { json!([{
            "capability": { "id": "workspace.files" },
            "action_allowlist": ["workspace.files/read", "workspace.files/search"]
        }]) } else { json!([]) };
        call(&self.router, "POST", "/api/agent-presets", json!({
            "display_name": "Personal preset without extension gates", "document": {
                "schema_version": "1.0.0", "model_route_refs": {}, "chat_route_records": {},
                "enabled_capabilities": enabled, "skill_bindings": [],
                "system_role_provider_overrides": {}, "persona": "Fixture assistant",
                "instructions": "Keep these authored instructions", "starter_prompts": []
            }
        })).await
    }

    async fn create(&self, preset: &Value, extensions: Option<Value>, workspace: bool) -> Value {
        let mut request = json!({ "preset_id": preset["preset"]["preset_id"], "title": "Composer selection" });
        if let Some(extensions) = extensions { request["session_capabilities"] = extensions; }
        if workspace {
            let root = self._directory.path().join("selected-workspace");
            fs::create_dir_all(&root).unwrap();
            request["workspace"] = json!(root);
            request["resource_selections"] = json!([{ "resource_kind": "workspace", "resource_id": "default-workspace" }]);
        }
        call(&self.router, "POST", "/api/agent-sessions", request).await
    }

    async fn binding(&self, id: &str) -> Value {
        call(&self.router, "GET", &format!("/api/agent-sessions/{id}"), Value::Null).await["session"]["agent_binding"].clone()
    }

    async fn state(&self, id: &str) -> Value {
        call(&self.router, "GET", &selection_path(id), Value::Null).await
    }

    async fn finish(self) {
        assert!(self._mcp.received_requests().await.unwrap().iter().all(|request|
            request.body_json::<Value>().unwrap()["method"] != "tools/call"),
            "selection alone must not execute an MCP tool");
        self.finish_with_chat_count(0).await;
    }

    async fn finish_with_chat_count(self, count: usize) {
        assert_eq!(self.chat.received_requests().await.unwrap().len(), count, "unexpected Chat provider calls");
        self.services.shutdown_browser_platform().await.unwrap();
        self.services.database.close().await;
    }
}

fn chat_response(call: Option<(&str, &str, Value)>, text: &str) -> ResponseTemplate {
    let (delta, finish) = match call {
        Some((id, name, arguments)) => (json!({ "role": "assistant", "tool_calls": [{
            "index": 0, "id": id, "type": "function", "function": { "name": name, "arguments": arguments.to_string() }
        }] }), "tool_calls"),
        None => (json!({ "role": "assistant", "content": text }), "stop"),
    };
    let first = json!({ "id": "fixture-native-selection", "choices": [{ "index": 0, "delta": delta, "finish_reason": null }] });
    let last = json!({ "id": "fixture-native-selection", "choices": [{ "index": 0, "delta": {}, "finish_reason": finish }] });
    ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
        .set_body_string(format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n"))
}

#[tokio::test]
async fn native_turn_discovers_mcp_reads_frozen_skill_resource_and_commits_tool_result_and_text() {
    const RESOURCE_PROOF: &str = "NATIVE_FROZEN_SKILL_RESOURCE_OK";
    const REPLY: &str = "NATIVE_SELECTION_TURN_COMPLETE";
    let fixture = Fixture::new().await;
    let reference = fixture.services.skill_paths.user_skills_dir.join(SKILL_A).join("references/proof.txt");
    fs::create_dir_all(reference.parent().unwrap()).unwrap();
    fs::write(&reference, RESOURCE_PROOF).unwrap();
    let captured = nomifun_skill_library::frozen::capture_selected(&fixture.services.skill_paths, &[SKILL_A.to_owned()]).await.unwrap();
    let provenance = format!("library:{SKILL_A}:{}", captured[0].source_digest.as_ref());
    let resource_id = format!("skill-{}", nomifun_agent_contracts::digest_bytes(format!("{provenance}\0references/proof.txt").as_bytes()).as_ref());
    let server_id = nomifun_api_types::McpServerId::parse(&fixture.server_a).unwrap();
    let capability_id = nomifun_mcp::canonical_mcp_tool_capability_id(&server_id, "lookup").unwrap();
    let tool_name = nomifun_agent_contracts::tool_presentation::namespaced_tool_name("mcp", "selection-server-a", "lookup", capability_id.as_bytes());
    let preset = fixture.preset(false).await;
    let session = fixture.create(&preset, Some(selection(&[SKILL_A], &[&fixture.server_a])), false).await;
    let id = session["agent_session_id"].as_str().unwrap();
    // The already-open Session must consume captured bytes, even when the
    // mutable Library source is edited before its first actual model turn.
    fs::write(&reference, "MUTABLE_LIBRARY_BYTES_MUST_NOT_BE_READ").unwrap();
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let observed = requests.clone();
    let expected_tool = tool_name.clone();
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let request = request.body_json::<Value>().unwrap();
            let mut requests = observed.lock().unwrap();
            requests.push(request.clone());
            let messages = request["messages"].as_array().unwrap();
            let result = |id: &str| messages.iter().find(|message| message["role"] == "tool" && message["tool_call_id"] == id)
                .unwrap_or_else(|| panic!("expected native tool receipt {id}: {request}"));
            match requests.len() {
                1 => {
                    assert!(messages.iter().any(|message| message["content"].to_string().contains(&format!("Selected Skill {SKILL_A} for this accepted request"))),
                        "saved composer selection must be injected when the caller supplies no inject_skills");
                    assert!(request["tools"].as_array().unwrap().iter().any(|tool| tool["function"]["name"] == "ToolSearch"));
                    chat_response(Some(("fixture-search", "ToolSearch", json!({ "query": expected_tool }))), "")
                }
                2 => {
                    assert!(result("fixture-search")["content"].to_string().contains(&expected_tool));
                    chat_response(Some(("fixture-skill-read", "read_context_resource", json!({ "id": resource_id }))), "")
                }
                3 => {
                    let read = result("fixture-skill-read")["content"].to_string();
                    assert!(read.contains(RESOURCE_PROOF), "actual native reader must return the frozen supporting resource: {read}");
                    assert!(!read.contains("MUTABLE_LIBRARY_BYTES_MUST_NOT_BE_READ"));
                    chat_response(Some(("fixture-mcp-call", &expected_tool, json!({}))), "")
                }
                4 => {
                    assert!(result("fixture-mcp-call")["content"].to_string().contains("NATIVE_MCP_RESULT_OK"));
                    chat_response(None, REPLY)
                }
                count => panic!("native selection must finish after four Chat steps, got {count}: {request}"),
            }
        }).mount(&fixture.chat).await;
    let accepted = call(&fixture.router, "POST", &format!("/api/agent-sessions/{id}/turns"), json!({
        "idempotency_key": uuid::Uuid::now_v7().to_string(),
        "input": { "content": "Use my selected Skill reference and the authorized MCP lookup, then report the result." }
    })).await;
    let store = AgentSessionStore::from_pool(fixture.services.database.pool().clone()).await.unwrap();
    let session_id = AgentSessionId::from(id);
    let operation = OperationId::from(accepted["operation_id"].as_str().unwrap());
    let finished = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let receipt = store.read_turn_receipt(&session_id, &operation).await.unwrap();
            if receipt.status == TurnReceiptStatus::Completed { break Ok(()); }
            if receipt.status != TurnReceiptStatus::Running { break Err(format!("native selection failed: {receipt:?}")); }
            let head = store.head(&session_id).await.unwrap();
            if head.status == "paused" || head.status == "reconciliation" {
                break Err(format!("native selection did not finish: {head:?}"));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await;
    if !matches!(&finished, Ok(Ok(()))) {
        eprintln!("NATIVE RECEIPT {:?}", store.read_turn_receipt(&session_id, &operation).await.unwrap());
        eprintln!("NATIVE EXECUTION {:?}", response(&fixture.router, "GET", &format!("/api/agent-sessions/{id}/execution"), Value::Null).await);
        let facts = store.turn_output_facts(&session_id, &operation).await.unwrap();
        for event in &facts.events {
            let payload = facts.event_payloads.get(event.event_id.as_ref()).map(Value::to_string).unwrap_or_default();
            eprintln!("NATIVE EVENT {} {} {}", event.seq, event.kind.0, payload.chars().take(1400).collect::<String>());
        }
        for (index, request) in requests.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().enumerate() {
            eprintln!("CHAT STEP {} tools={}", index + 1, json!(request["tools"].as_array().unwrap().iter().map(|tool| &tool["function"]["name"]).collect::<Vec<_>>()));
            for message in request["messages"].as_array().unwrap().iter().filter(|message| message["role"] == "tool") {
                eprintln!("CHAT RECEIPT {} {}", message["tool_call_id"], message["content"].to_string().chars().take(1400).collect::<String>());
            }
        }
    }
    finished.expect("native selection turn must finish").expect("native selection must remain executable until its terminal receipt");
    assert_eq!(requests.lock().unwrap().len(), 4);
    let facts = store.turn_output_facts(&session_id, &operation).await.unwrap();
    assert!(facts.event_payloads.values().any(|payload| payload.to_string().contains(REPLY)), "final text must exist in canonical output facts");
    assert!(facts.event_payloads.values().any(|payload| payload.to_string().contains("NATIVE_MCP_RESULT_OK")), "MCP result must exist in canonical output facts");
    assert!(facts.events.iter().any(|event| event.kind.0 == "turn/completed"));
    assert_eq!(fixture._mcp.received_requests().await.unwrap().iter().filter(|request|
        request.body_json::<Value>().unwrap()["method"] == "tools/call").count(), 1);
    fixture.finish_with_chat_count(4).await;
}

#[tokio::test]
async fn minimal_personal_preset_launches_selected_mcp_and_skill_without_preset_gates() {
    let fixture = Fixture::new().await;
    let preset = fixture.preset(false).await;
    assert_eq!(preset["revision"]["document"]["enabled_capabilities"], json!([]));
    assert_eq!(preset["revision"]["document"]["skill_bindings"], json!([]));
    let selected = selection(&[SKILL_A], &[&fixture.server_a]);
    let session = fixture.create(&preset, Some(selected.clone()), false).await;
    let id = session["agent_session_id"].as_str().unwrap();
    let state = fixture.state(id).await;
    assert_eq!(state["selection"], selected);
    assert_eq!(state["editable"], true);
    assert_eq!(state["binding_version"], session["agent_binding"]["binding_version"]);
    let resources = session["agent_binding"]["typed_resource_bindings"].as_array().unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0]["resource_kind"], "mcp_server");
    assert_eq!(resources[0]["resource_id"], fixture.server_a);
    assert_eq!(resources[0]["operations"], json!(["connect", "invoke", "read"]));
    let capabilities = call(&fixture.router, "GET", &format!("/api/agent-sessions/{id}/capabilities"), Value::Null).await;
    assert!(capabilities["enabled_capabilities"].as_array().unwrap().iter()
        .any(|value| value.as_str().is_some_and(nomifun_mcp::is_namespaced_mcp_tool_capability)));
    let original = call(&fixture.router, "GET", &format!("/api/agent-presets/{}/editor", preset["preset"]["preset_id"].as_str().unwrap()), Value::Null).await;
    assert_eq!(original["draft"]["document"], preset["revision"]["document"], "Session extensions must not edit the authored preset");
    fixture.finish().await;
}

#[tokio::test]
async fn omitted_selection_uses_global_ready_servers_and_auto_skills_explicit_empty_narrows() {
    let fixture = Fixture::new().await;
    let preset = fixture.preset(false).await;
    let default = fixture.create(&preset, None, false).await;
    let state = fixture.state(default["agent_session_id"].as_str().unwrap()).await;
    let mut servers = vec![fixture.server_a.clone(), fixture.server_b.clone()];
    servers.sort();
    assert_eq!(state["selection"]["mcp_server_ids"], json!(servers));
    assert!(state["selection"]["skill_names"].as_array().unwrap().contains(&json!(AUTO_SKILL)));
    assert!(!state["selection"]["skill_names"].as_array().unwrap().contains(&json!(SKILL_A)));
    let narrowed = fixture.create(&preset, Some(selection(&[], &[])), false).await;
    assert_eq!(fixture.state(narrowed["agent_session_id"].as_str().unwrap()).await["selection"], selection(&[], &[]));
    assert!(narrowed["agent_binding"]["typed_resource_bindings"].as_array().unwrap().is_empty());
    fixture.finish().await;
}

#[tokio::test]
async fn apply_preserves_workspace_and_noop_preserves_binding_and_transition_count() {
    let fixture = Fixture::new().await;
    let preset = fixture.preset(true).await;
    let session = fixture.create(&preset, Some(selection(&[SKILL_A], &[&fixture.server_a])), true).await;
    let id = session["agent_session_id"].as_str().unwrap();
    let before = session["agent_binding"].clone();
    let retained = non_mcp_resources(&before);
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0]["resource_kind"], "workspace");
    let selected = selection(&[SKILL_B], &[&fixture.server_b]);
    let applied = call(&fixture.router, "PUT", &selection_path(id), json!({
        "selection": selected, "expected_binding_version": before["binding_version"]
    })).await;
    assert_eq!(applied["selection"], selected);
    assert_eq!(applied["binding_version"].as_u64().unwrap(), before["binding_version"].as_u64().unwrap() + 1);
    let after = fixture.binding(id).await;
    assert_eq!(non_mcp_resources(&after), retained, "workspace authority and policy must remain byte-for-byte exact");
    assert_ne!(after["resolved_snapshot_ref"], before["resolved_snapshot_ref"]);
    let noop = call(&fixture.router, "PUT", &selection_path(id), json!({
        "selection": selected, "expected_binding_version": applied["binding_version"]
    })).await;
    assert_eq!(noop, applied);
    assert_eq!(fixture.binding(id).await, after);
    let transitions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='session/agent-binding-changed'")
        .bind(id).fetch_one(fixture.services.database.pool()).await.unwrap();
    assert_eq!(transitions, 1, "no-op must not create another canonical transition");
    let (status, stale) = response(&fixture.router, "PUT", &selection_path(id), json!({
        "selection": selection(&[], &[]), "expected_binding_version": before["binding_version"]
    })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["code"], "AGENT_SESSION_BINDING_CHANGED");
    assert_eq!(fixture.binding(id).await, after);
    fixture.finish().await;
}

#[tokio::test]
async fn competing_selection_updates_only_commit_one_expected_binding_version() {
    let fixture = Fixture::new().await;
    let preset = fixture.preset(false).await;
    let session = fixture.create(&preset, Some(selection(&[], &[])), false).await;
    let id = session["agent_session_id"].as_str().unwrap();
    let path = selection_path(id);
    let version = session["agent_binding"]["binding_version"].clone();
    let (first, second) = tokio::join!(
        response(&fixture.router, "PUT", &path, json!({ "selection": selection(&[SKILL_A], &[&fixture.server_a]), "expected_binding_version": version })),
        response(&fixture.router, "PUT", &path, json!({ "selection": selection(&[SKILL_B], &[&fixture.server_b]), "expected_binding_version": version }))
    );
    let (winner, rejected) = if first.0 == StatusCode::OK { (first, second) } else { (second, first) };
    assert_eq!(winner.0, StatusCode::OK, "{winner:?}");
    assert_eq!(rejected.0, StatusCode::CONFLICT, "{rejected:?}");
    assert_eq!(rejected.1["code"], "AGENT_SESSION_BINDING_CHANGED");
    let state = fixture.state(id).await;
    assert_eq!(state, winner.1["data"]);
    assert_eq!(state["binding_version"].as_u64().unwrap(), version.as_u64().unwrap() + 1);
    fixture.finish().await;
}

#[tokio::test]
async fn unknown_disabled_servers_and_missing_skills_never_mutate_the_session() {
    let fixture = Fixture::new().await;
    let preset = fixture.preset(false).await;
    call(&fixture.router, "POST", &format!("/api/mcp/servers/{}/toggle", fixture.server_b), Value::Null).await;
    let session = fixture.create(&preset, Some(selection(&[SKILL_A], &[&fixture.server_a])), false).await;
    let id = session["agent_session_id"].as_str().unwrap();
    let before = fixture.binding(id).await;
    for selected in [selection(&[], &[UNKNOWN_SERVER]), selection(&[], &[&fixture.server_b]), selection(&["missing-fixture-skill"], &[])] {
        let (status, error) = response(&fixture.router, "PUT", &selection_path(id), json!({
            "selection": selected, "expected_binding_version": before["binding_version"]
        })).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        assert_eq!(error["code"], "SESSION_CAPABILITIES_INVALID");
        assert_eq!(fixture.binding(id).await, before);
        let (status, error) = response(&fixture.router, "POST", "/api/agent-sessions", json!({
            "preset_id": preset["preset"]["preset_id"], "session_capabilities": selected
        })).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        assert_eq!(error["code"], "SESSION_CAPABILITIES_INVALID");
    }
    fixture.finish().await;
}

#[tokio::test]
async fn unselected_unfreezable_skill_does_not_block_explicit_empty_or_other_extensions() {
    const UNAVAILABLE: &str = "fixture-unavailable";
    let fixture = Fixture::new().await;
    write_skill(&fixture.services.skill_paths.user_skills_dir, UNAVAILABLE);
    fs::write(fixture.services.skill_paths.user_skills_dir.join(UNAVAILABLE).join("template.xlsx"), [0xff, 0xfe, 0x00, 0x80]).unwrap();
    let preset = fixture.preset(false).await;
    let session = fixture.create(&preset, Some(selection(&[], &[])), false).await;
    let id = session["agent_session_id"].as_str().unwrap();
    assert_eq!(fixture.state(id).await["selection"], selection(&[], &[]));
    let applied = call(&fixture.router, "PUT", &selection_path(id), json!({
        "selection": selection(&[SKILL_A], &[&fixture.server_a]),
        "expected_binding_version": session["agent_binding"]["binding_version"]
    })).await;
    let before = fixture.binding(id).await;
    let (status, error) = response(&fixture.router, "PUT", &selection_path(id), json!({
        "selection": selection(&[UNAVAILABLE], &[]), "expected_binding_version": applied["binding_version"]
    })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
    assert_eq!(error["code"], "SESSION_CAPABILITIES_INVALID");
    assert!(error["error"].as_str().unwrap().contains(UNAVAILABLE), "selected unavailable Skill should be identified: {error}");
    assert_eq!(fixture.binding(id).await, before);
    fixture.finish().await;
}

#[tokio::test]
async fn same_agent_preview_rejects_after_preserving_a_narrowed_extension_selection() {
    let fixture = Fixture::new().await;
    let preset = fixture.preset(false).await;
    let selected = selection(&[SKILL_A], &[&fixture.server_a]);
    let session = fixture.create(&preset, Some(selected.clone()), false).await;
    let id = session["agent_session_id"].as_str().unwrap();
    let (status, error) = response(&fixture.router, "POST", &format!("/api/agent-sessions/{id}/agent-switch/preview"), json!({
        "selection": { "kind": "preset", "preset_id": preset["preset"]["preset_id"] }
    })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["code"], "AGENT_SESSION_AGENT_UNCHANGED");
    assert_eq!(fixture.binding(id).await, session["agent_binding"]);
    assert_eq!(fixture.state(id).await["selection"], selected);
    fixture.finish().await;
}

#[tokio::test]
async fn running_paused_attempt_and_remote_sessions_report_readonly_and_reject_apply() {
    let fixture = Fixture::new().await;
    let preset = fixture.preset(false).await;
    let store = AgentSessionStore::from_pool(fixture.services.database.pool().clone()).await.unwrap();
    for paused in [false, true] {
        let session = fixture.create(&preset, Some(selection(&[], &[])), false).await;
        let id = session["agent_session_id"].as_str().unwrap();
        let operation = OperationId::from(uuid::Uuid::now_v7().to_string());
        let (_, started) = store.start_turn(&AgentSessionId::from(id), EventProducerId::from("session_api"),
            IdempotencyKey::from(operation.as_ref()), operation.clone(), StrictJsonValue(json!({ "content": "Fixture active task" }))).await.unwrap();
        if paused {
            let pause = NativePauseState {
                revision: 1, reason: "EXECUTION_USER_REQUESTED".into(), checkpoint_revision: 0,
                checkpoint_digest: None, execution_fence: 1, cleanup_proven: true,
                paused_at_ms: nomifun_common::now_ms(),
            };
            store.append_event(&SessionEventAppend {
                agent_session_id: id.into(), event_id: EventId::from(uuid::Uuid::now_v7().to_string()),
                producer_id: EventProducerId::from("runtime_supervisor"), idempotency_key: IdempotencyKey::from(format!("fixture-pause:{id}")),
                semantic_event: SemanticSessionEventDraft {
                    kind: SessionEventKind("turn/paused".into()), kind_version: 1,
                    correlation_id: CorrelationId::from(operation.as_ref()), causation_event_id: Some(started.ack.unwrap().event_id),
                    payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({ "pause": pause }))),
                },
            }).await.unwrap();
        }
        assert_eq!(fixture.state(id).await["editable"], false);
        let (status, error) = response(&fixture.router, "PUT", &selection_path(id), json!({
            "selection": selection(&[SKILL_A], &[&fixture.server_a]), "expected_binding_version": session["agent_binding"]["binding_version"]
        })).await;
        assert_eq!(status, StatusCode::CONFLICT, "{error}");
        assert_eq!(error["code"], "AGENT_SESSION_CAPABILITIES_READ_ONLY");
        assert_eq!(fixture.binding(id).await, session["agent_binding"]);
    }
    for relation in ["attempt", "automation"] {
        let attempt = fixture.create(&preset, Some(selection(&[], &[])), false).await;
        let id = attempt["agent_session_id"].as_str().unwrap();
        sqlx::query("INSERT INTO conversation_execution_links (conversation_id,execution_id,relation,step_id,attempt_id,created_at,updated_at) VALUES (?,?,?,?,?,1,1)")
            .bind(id).bind(uuid::Uuid::now_v7().to_string()).bind(relation).bind(uuid::Uuid::now_v7().to_string()).bind(uuid::Uuid::now_v7().to_string())
            .execute(fixture.services.database.pool()).await.unwrap();
        assert_eq!(fixture.state(id).await["editable"], false);
        let (status, error) = response(&fixture.router, "PUT", &selection_path(id), json!({
            "selection": selection(&[SKILL_A], &[&fixture.server_a]), "expected_binding_version": attempt["agent_binding"]["binding_version"]
        })).await;
        assert_eq!(status, StatusCode::CONFLICT, "{error}");
        assert_eq!(error["code"], "AGENT_SESSION_CAPABILITIES_READ_ONLY");
        assert_eq!(fixture.binding(id).await, attempt["agent_binding"]);
    }

    let local = fixture.create(&preset, Some(selection(&[], &[])), false).await;
    let binding = call(&fixture.router, "POST", "/api/remote-bindings", json!({
        "name": "Read-only remote selection", "agent_binding": local["agent_binding"]
    })).await;
    let (status, remote) = response(&fixture.router, "POST", "/api/remote/open", json!({
        "binding_id": binding["remote_binding_id"], "idempotency_key": "fixture-remote-selection"
    })).await;
    assert_eq!(status, StatusCode::OK, "{remote}");
    let id = remote["agent_session_id"].as_str().unwrap();
    assert_eq!(fixture.state(id).await["editable"], false);
    let (status, error) = response(&fixture.router, "PUT", &selection_path(id), json!({
        "selection": selection(&[SKILL_A], &[&fixture.server_a]), "expected_binding_version": remote["agent_binding"]["binding_version"]
    })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["code"], "AGENT_SESSION_CAPABILITIES_READ_ONLY");
    fixture.finish().await;
}
