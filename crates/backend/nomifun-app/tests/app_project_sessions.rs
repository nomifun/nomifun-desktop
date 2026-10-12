//! Application projects reuse ordinary canonical Sessions. These tests make
//! no provider requests and operate only on isolated installation data.
use std::{collections::BTreeMap, sync::Arc};
use axum::{Router, body::Body, http::{Request, StatusCode}};
use http_body_util::BodyExt;
use nomifun_app::{AppConfig, AuthPolicy, compatibility::{AppServices, create_router}};
use nomifun_api_types::AgentChatModelSelectionDto;
use serde_json::{Value, json};
use tower::ServiceExt;

const TRUST: &str = "app-project-session-isolated-test";

async fn request(router: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = router.clone().oneshot(Request::builder().method(method).uri(path)
        .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({"error":String::from_utf8_lossy(&bytes)})))
}
async fn data(router: &Router, method: &str, path: &str, body: Value) -> Value {
    let (status, response) = request(router, method, path, body).await;
    assert!(status.is_success(), "{method} {path}: {status}: {response}");
    assert_eq!(response["success"], true);
    response["data"].clone()
}
async fn fixture(root: &std::path::Path) -> (AppServices, Router, AgentChatModelSelectionDto) {
    fixture_with_model_route(root, "https://api.openai.invalid/v1", "openai.responses", "/responses").await
}
async fn fixture_with_model_route(root: &std::path::Path, base_url: &str, protocol: &str, endpoint: &str)
    -> (AppServices, Router, AgentChatModelSelectionDto) {
    use nomifun_db::{CreateProviderParams, IProviderRepository, NewProviderModel, NewProviderModelCapability, SqliteProviderRepository};
    let database = nomifun_db::init_database_memory().await.unwrap();
    let services = AppServices::from_config(database, &AppConfig {
        data_dir:root.join("data"), work_dir:root.join("work"), auth_policy:AuthPolicy::TrustLocalToken,
        local_trust_secret:Some(Arc::from(TRUST)), ..Default::default()
    }).await.unwrap();
    let provider_id = uuid::Uuid::now_v7().to_string();
    let encrypted = nomifun_common::encrypt_string(r#"{"api_keys":["project-opening-test"]}"#, &services.encryption_key).unwrap();
    let capabilities = [NewProviderModelCapability { task:"chat", traits:r#"["vision_input","function_calling","reasoning","web_search"]"#,
        protocol, connection_role:"default", endpoint:Some(endpoint), provider_params:"{}", context_limit:Some(128_000), ..Default::default() }];
    SqliteProviderRepository::new(services.database.pool().clone()).create(CreateProviderParams {
        provider_id:Some(&provider_id), platform:"openai", name:"Project Session storage fixture", base_url,
        auth_scheme:"bearer", credentials_encrypted:&encrypted, enabled:true, bedrock_config:None, sort_order:Some(-10_000),
    }, &NewProviderModel { model:"project-opening-test", enabled:true, sort_order:-10_000,
        description:Some("Deterministic storage or scripted-provider fixture; never live generation evidence."), capabilities:&capabilities }, &[]).await.unwrap();
    let router = create_router(&services).await;
    (services, router, AgentChatModelSelectionDto { provider_id, model:"project-opening-test".into() })
}

async fn canonical_bytes(services: &AppServices, session: &str) -> BTreeMap<String, Vec<String>> {
    use sqlx::Row;
    let mut facts = BTreeMap::new();
    for table in ["agent_sessions", "agent_turns", "agent_events", "agent_payloads", "agent_effects", "agent_session_resources", "agent_messages"] {
        let columns = sqlx::query(&format!("PRAGMA table_info({table})")).fetch_all(services.database.pool()).await.unwrap();
        let fields = columns.iter().map(|row| {
            let name: String = row.get("name");
            let quoted = format!("\"{}\"", name.replace('"', "\"\""));
            format!("typeof({quoted}) || ':' || hex({quoted})")
        }).collect::<Vec<_>>().join(" || '|' || ");
        let identity = if table == "agent_sessions" { "agent_session_id" } else { "session_id" };
        facts.insert(table.into(), sqlx::query_scalar(&format!("SELECT {fields} FROM {table} WHERE {identity}=? ORDER BY rowid"))
            .bind(session).fetch_all(services.database.pool()).await.unwrap());
    }
    facts
}

async fn record_cancelled_input(services: &AppServices, session: &str, content: &str) {
    let owner = nomifun_agent_contracts::PrincipalRef { principal_kind:"user".into(), principal_id:services.authoritative_user_id.as_ref().into() };
    let session_id = nomifun_agent_contracts::AgentSessionId::from(session.to_owned());
    let canonical = nomifun_conversation::CanonicalAgentSessionOwner::from_pool(services.database.pool().clone()).await.unwrap();
    // Exercise the actual canonical input and cancellation writers. No
    // Runtime is started and this fixture never fabricates a successful Turn.
    canonical.start_turn(&owner, &session_id, "project-history-input", json!({"content":content})).await.unwrap();
    canonical.cancel(&owner, &session_id, "project-history-cancel").await.unwrap();
}

#[tokio::test]
async fn project_sessions_are_independent_paginated_history_and_navigation_never_rewrites_facts() {
    let temporary = tempfile::tempdir().unwrap();
    let (services, router, model) = fixture(temporary.path()).await;
    let app = data(&router, "POST", "/api/apps", json!({"name":"应用项目"})).await;
    let id = app["id"].as_str().unwrap();
    let endpoint = format!("/api/apps/{id}/development-session");
    let first = data(&router, "POST", &endpoint, json!({"model":model})).await;
    let first_id = first["conversation"]["conversation_id"].as_str().unwrap();
    record_cancelled_input(&services, first_id, "FIRST_PROJECT_SESSION_HISTORY").await;
    let source = std::path::Path::new(app["directory"].as_str().unwrap()).join("README.md");
    std::fs::write(&source, "PROJECT_SOURCE_PRESERVED").unwrap();
    data(&router, "PATCH", &format!("/api/agent-sessions/{first_id}"), json!({"archived":true,"name":"初次开发"})).await;
    let first_bytes = canonical_bytes(&services, first_id).await;
    let key = uuid::Uuid::now_v7().to_string();
    let second = data(&router, "POST", &endpoint, json!({"new_session":true,"creation_key":key,"model":model,"reasoning_effort":"high"})).await;
    let second_id = second["conversation"]["conversation_id"].as_str().unwrap();
    record_cancelled_input(&services, second_id, "SECOND_PROJECT_SESSION_HISTORY").await;
    assert_ne!(first_id, second_id);
    assert_eq!(second["app"]["development_session_id"], second_id);
    assert_eq!(second["conversation"]["extra"]["workspace"], first["conversation"]["extra"]["workspace"]);
    assert_eq!(second["conversation"]["reasoning_effort"], "high");
    assert_eq!(canonical_bytes(&services, first_id).await, first_bytes, "new project Sessions cannot mirror or rewrite old history");
    let second_bytes = canonical_bytes(&services, second_id).await;
    let first_history = data(&router, "GET", &format!("/api/agent-sessions/{first_id}/events"), Value::Null).await;
    let second_history = data(&router, "GET", &format!("/api/agent-sessions/{second_id}/events"), Value::Null).await;
    assert!(first_history.to_string().contains("FIRST_PROJECT_SESSION_HISTORY"));
    assert!(!first_history.to_string().contains("SECOND_PROJECT_SESSION_HISTORY"));
    assert!(second_history.to_string().contains("SECOND_PROJECT_SESSION_HISTORY"));
    assert!(!second_history.to_string().contains("FIRST_PROJECT_SESSION_HISTORY"));
    let first_page = data(&router, "GET", &format!("/api/apps/{id}/development-sessions?limit=1"), Value::Null).await;
    assert_eq!(first_page["total"], 2);
    assert_eq!(first_page["has_more"], true);
    assert_eq!(first_page["items"][0]["conversation"]["conversation_id"], second_id);
    assert_eq!(first_page["items"][0]["editable_in_app"], true);
    assert_eq!(first_page["items"][0]["archived"], false);
    assert_eq!(first_page["current_session_id"], second_id);
    assert_eq!(first_page["project_directory"], app["directory"]);
    let cursor = first_page["next_cursor"].as_str().unwrap();
    let next = data(&router, "GET", &format!("/api/apps/{id}/development-sessions?limit=1&cursor={cursor}"), Value::Null).await;
    assert_eq!(next["total"], 2);
    assert_eq!(next["has_more"], false);
    assert!(next["next_cursor"].is_null());
    assert_eq!(next["items"][0]["conversation"]["conversation_id"], first_id, "archived ordinary Session remains project history");
    assert_eq!(next["items"][0]["archived"], true);
    let selected = data(&router, "POST", &endpoint, json!({"session_id":first_id})).await;
    assert_eq!(selected["app"]["development_session_id"], first_id);
    assert_eq!(selected["conversation"]["conversation_id"], first_id);
    assert_eq!(canonical_bytes(&services, first_id).await, first_bytes);
    assert_eq!(canonical_bytes(&services, second_id).await, second_bytes);
    // Renaming the application changes no exact creation receipt. Retrying
    // the same command opens its original Session even after navigation.
    data(&router, "PUT", &format!("/api/apps/{id}"), json!({"name":"重新命名的项目"})).await;
    let replay = data(&router, "POST", &endpoint, json!({"new_session":true,"creation_key":key,"model":model,"reasoning_effort":"high"})).await;
    assert_eq!(replay["conversation"]["conversation_id"], second_id);
    assert_eq!(canonical_bytes(&services, first_id).await, first_bytes);
    assert_eq!(canonical_bytes(&services, second_id).await, second_bytes);
    let (status, _) = request(&router, "POST", &endpoint, json!({"new_session":true,"creation_key":key,"reasoning_effort":"low"})).await;
    assert!(!status.is_success(), "a changed request cannot reuse another exact creation receipt");
    assert_eq!(std::fs::read_to_string(source).unwrap(), "PROJECT_SOURCE_PRESERVED");
    let history = data(&router, "GET", &format!("/api/apps/{id}/development-sessions"), Value::Null).await;
    assert_eq!(history["total"], 2, "replay must not create duplicate Sessions");
    data(&router, "DELETE", &format!("/api/apps/{id}"), Value::Null).await;
    assert_eq!(canonical_bytes(&services, first_id).await, first_bytes);
    assert_eq!(canonical_bytes(&services, second_id).await, second_bytes);
    services.cleanup_after_startup_failure(anyhow::anyhow!("project Session fixture finished")).await;
}

#[tokio::test]
async fn project_history_includes_ordinary_foreign_agents_but_app_selection_stays_exact() {
    let temporary = tempfile::tempdir().unwrap();
    let (services, router, model) = fixture(temporary.path()).await;
    let app = data(&router, "POST", "/api/apps", json!({"name":"工程"})).await;
    let id = app["id"].as_str().unwrap();
    let own = data(&router, "POST", &format!("/api/apps/{id}/development-session"), json!({"model":model})).await;
    let own_id = own["conversation"]["conversation_id"].as_str().unwrap();
    let editor = data(&router, "POST", "/api/agent-presets/from-template/chat.minimal", json!({"model":model,"reuse_existing":true,"display_name":"chat.minimal"})).await;
    let foreign = data(&router, "POST", "/api/agent-sessions", json!({"preset_id":editor["preset"]["preset_id"],"title":"普通项目讨论",
        "model":model,"workspace":app["directory"],"resource_selections":[]})).await;
    let foreign_id = foreign["agent_session_id"].as_str().unwrap();
    let before = canonical_bytes(&services, foreign_id).await;
    let history = data(&router, "GET", &format!("/api/apps/{id}/development-sessions"), Value::Null).await;
    assert_eq!(history["total"], 2);
    let item = history["items"].as_array().unwrap().iter().find(|item| item["conversation"]["conversation_id"] == foreign_id).unwrap();
    assert_eq!(item["editable_in_app"], false);
    let (status, _) = request(&router, "POST", &format!("/api/apps/{id}/development-session"), json!({"session_id":foreign_id})).await;
    assert!(!status.is_success(), "history visibility cannot replace the frozen Agent");
    assert_eq!(data(&router, "GET", &format!("/api/apps/{id}"), Value::Null).await["development_session_id"], own_id);
    assert_eq!(canonical_bytes(&services, foreign_id).await, before);
    services.cleanup_after_startup_failure(anyhow::anyhow!("foreign project Session fixture finished")).await;
}

#[tokio::test]
async fn project_selection_and_new_session_requests_cannot_invent_or_cross_project_membership() {
    let temporary = tempfile::tempdir().unwrap();
    let (services, router, model) = fixture(temporary.path()).await;
    let first = data(&router, "POST", "/api/apps", json!({"name":"第一项目"})).await;
    let second = data(&router, "POST", "/api/apps", json!({"name":"第二项目"})).await;
    let first_id = first["id"].as_str().unwrap();
    let second_id = second["id"].as_str().unwrap();
    assert_ne!(first["directory"], second["directory"], "one application always owns one independent project directory");
    let selected = data(&router, "POST", &format!("/api/apps/{second_id}/development-session"), json!({"model":model})).await;
    let session = selected["conversation"]["conversation_id"].as_str().unwrap();
    let before = canonical_bytes(&services, session).await;
    for body in [json!({"session_id":session}),json!({"new_session":true}),json!({"creation_key":uuid::Uuid::now_v7().to_string()}),
        json!({"new_session":true,"creation_key":"invalid"}),json!({"new_session":true,"creation_key":uuid::Uuid::now_v7().to_string(),"session_id":session})] {
        let (status, _) = request(&router, "POST", &format!("/api/apps/{first_id}/development-session"), body).await;
        assert!(!status.is_success());
    }
    let empty = data(&router, "GET", &format!("/api/apps/{first_id}/development-sessions"), Value::Null).await;
    assert_eq!(empty["total"], 0);
    assert!(empty["current_session_id"].is_null());
    assert_eq!(canonical_bytes(&services, session).await, before);
    let second_history = data(&router, "GET", &format!("/api/apps/{second_id}/development-sessions"), Value::Null).await;
    assert_eq!(second_history["total"], 1);
    assert_eq!(second_history["items"][0]["conversation"]["conversation_id"], session);
    for query in ["limit=0", "limit=10001", "cursor=not-a-session"] {
        let (status, _) = request(&router, "GET", &format!("/api/apps/{first_id}/development-sessions?{query}"), Value::Null).await;
        assert!(!status.is_success());
    }
    services.cleanup_after_startup_failure(anyhow::anyhow!("project Session boundary fixture finished")).await;
}

#[tokio::test]
async fn project_memory_namespace_survives_explicit_agent_switch_without_rebinding_default_sessions() {
    let temporary = tempfile::tempdir().unwrap();
    let (services, router, model) = fixture(temporary.path()).await;
    let editor = data(&router, "POST", "/api/agent-presets/from-template/coding.codex", json!({
        "model":model,"reuse_existing":true,"display_name":"coding.codex"})).await;
    let ordinary = data(&router, "POST", "/api/agent-sessions", json!({"preset_id":editor["preset"]["preset_id"],
        "model":model,"title":"Existing ordinary managed workspace", "resource_selections":[
            {"resource_kind":"workspace","resource_id":"default-workspace"},
            {"resource_kind":"process_session","resource_id":"managed-process-session"},
            {"resource_kind":"project_memory","resource_id":"default-project-memory"}]})).await;
    let ordinary_id = ordinary["agent_session_id"].as_str().unwrap();
    let ordinary_projection = data(&router, "GET", &format!("/api/agent-sessions/{ordinary_id}/projection"), Value::Null).await;
    let memory = |projection: &Value| projection["agent_snapshot"]["canonical_binding"]["typed_resource_bindings"].as_array().unwrap()
        .iter().find(|resource| resource["resource_kind"] == "project_memory").unwrap().clone();
    assert_eq!(memory(&ordinary_projection)["resource_id"], "default-project-memory");
    let ordinary_before = canonical_bytes(&services, ordinary_id).await;
    let app = data(&router, "POST", "/api/apps", json!({"name":"Shared memory project"})).await;
    let id = app["id"].as_str().unwrap();
    let first = data(&router, "POST", &format!("/api/apps/{id}/development-session"), json!({"model":model})).await;
    let next = data(&router, "POST", &format!("/api/apps/{id}/development-session"), json!({"new_session":true,
        "creation_key":uuid::Uuid::now_v7().to_string(),"model":model})).await;
    let session = next["conversation"]["conversation_id"].as_str().unwrap();
    let expected_memory = memory(&next["conversation"]);
    assert_eq!(expected_memory, memory(&first["conversation"]));
    assert!(expected_memory["resource_id"].as_str().unwrap().starts_with("project-workspace-memory-"));
    let before_events = data(&router, "GET", &format!("/api/agent-sessions/{session}/events"), Value::Null).await;
    let selection = json!({"kind":"preset","preset_id":editor["preset"]["preset_id"]});
    let preview = data(&router, "POST", &format!("/api/agent-sessions/{session}/agent-switch/preview"), json!({"selection":selection})).await;
    assert_eq!(preview["can_apply"], true);
    let command = json!({"selection":selection,"handoff_mode":"context_only","expected_binding_version":preview["expected_binding_version"]});
    let response = router.clone().oneshot(Request::builder().method("PUT").uri(format!("/api/agent-sessions/{session}/agent"))
        .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
        .header("Idempotency-Key", uuid::Uuid::now_v7().to_string())
        .body(Body::from(serde_json::to_vec(&command).unwrap())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let switched: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(status.is_success(), "{status}: {switched}");
    assert_eq!(memory(&switched["data"]["conversation"]), expected_memory);
    assert_eq!(switched["data"]["conversation"]["extra"]["workspace"], app["directory"]);
    let after_events = data(&router, "GET", &format!("/api/agent-sessions/{session}/events"), Value::Null).await;
    let prefix = before_events["events"].as_array().unwrap();
    assert_eq!(&after_events["events"].as_array().unwrap()[..prefix.len()], prefix);
    assert_eq!(canonical_bytes(&services, ordinary_id).await, ordinary_before, "creating or switching an application cannot rewrite a different frozen default Session");
    let history = data(&router, "GET", &format!("/api/apps/{id}/development-sessions"), Value::Null).await;
    let current = history["items"].as_array().unwrap().iter().find(|item| item["conversation"]["conversation_id"] == session).unwrap();
    assert_eq!(current["editable_in_app"], false, "normal Agent switch preserves project membership but does not bypass the application's Agent entry");
    services.cleanup_after_startup_failure(anyhow::anyhow!("project memory Agent-switch fixture finished")).await;
}

async fn start_protected_target_turn(services: &AppServices, session: &str, unknown: bool) {
    use nomifun_agent_contracts::{AgentSessionId, SemanticSessionEventDraft, SessionEventAppend, SessionEventKind, SessionEventPayloadRef, StrictJsonValue};
    use nomifun_agent_session::{AgentSessionStore, EffectEventRequest, EffectStrategy, EffectTerminalState};
    let store = AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    let session = AgentSessionId::from(session.to_owned());
    let operation = format!("project-import-protected-target:{unknown}");
    let (_, started) = store.start_turn(&session, "session_api".into(), operation.clone().into(),
        operation.clone().into(), StrictJsonValue(json!({"content":"Canonical target protection fault injection"}))).await.unwrap();
    let turn_event = started.ack.unwrap().event_id;
    if !unknown { return; }
    let workspace = store.get_live_session(&session).await.unwrap().agent_binding.typed_resource_bindings.into_iter()
        .find(|resource| resource.resource_kind.as_ref() == "workspace").unwrap();
    // This is actual canonical uncertainty injection, with a complete causal
    // tool/effect chain. It never creates a successful provider/tool result.
    let tool = store.append_event(&SessionEventAppend { agent_session_id:session.clone(), event_id:"project-import-unknown-tool".into(),
        producer_id:"runtime-supervisor".into(), idempotency_key:"project-import-unknown-tool".into(), semantic_event:SemanticSessionEventDraft {
            kind:SessionEventKind("tool/call-started".into()), kind_version:1, correlation_id:"project-import-unknown-operation".into(), causation_event_id:Some(turn_event.clone()),
            payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"operation_id":"project-import-unknown-operation",
                "call_id":"project-import-unknown-call","capability_id":"workspace.vcs","action_id":"workspace.vcs/push"}))) }
    }).await.unwrap().ack.unwrap().event_id;
    let effect = EffectEventRequest { agent_session_id:session.clone(), effect_id:"project-import-unknown-effect".into(), turn_id:operation.clone().into(),
        operation_id:"project-import-unknown-operation".into(), owner_domain:"workspace".into(), capability_module:"workspace.vcs".into(), action_id:"workspace.vcs/push".into(),
        resource_binding_id:Some(workspace.binding_id), resource_key:Some("project-import-unknown-target".into()), input_digest:"8".repeat(64).into(), recorded_at:2000,
        event_id:"project-import-unknown-started".into(), producer_id:"capability-host".into(), idempotency_key:"project-import-unknown-effect".into(),
        correlation_id:"project-import-unknown-effect".into(), strategy:EffectStrategy::ExternalUncertainEffect, causation_event_id:Some(tool),
        payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"fault":"unknown target effect"}))) };
    let effect_started = store.record_effect_started(effect.clone()).await.unwrap().ack.unwrap().event_id;
    store.record_effect_terminal(EffectEventRequest { event_id:"project-import-unknown-terminal".into(), producer_id:"application-owner".into(),
        causation_event_id:Some(effect_started), recorded_at:2010, ..effect }, EffectTerminalState::Uncertain).await.unwrap();
    store.append_event(&SessionEventAppend { agent_session_id:session.clone(), event_id:"project-import-protected-turn-terminal".into(),
        producer_id:"runtime-supervisor".into(), idempotency_key:"project-import-protected-turn-terminal".into(), semantic_event:SemanticSessionEventDraft {
            kind:SessionEventKind("turn/failed".into()), kind_version:1, correlation_id:operation.into(), causation_event_id:Some(turn_event),
            payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"error":"Unknown target effect remains unsettled"}))) } }).await.unwrap();
    assert!(store.head(&session).await.unwrap().active_turn_id.is_none());
    assert!(store.has_unsettled_effects(&session).await.unwrap());
}

#[tokio::test]
async fn scripted_runtime_import_uses_real_project_fences_and_hosted_effect_receipts() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use nomifun_agent_session::{AgentEffectState, AgentSessionStore, TurnReceiptStatus};
    // Provider text here is a deterministic protocol driver. Acceptance is
    // proved by the real Runtime, source bytes and canonical Effect receipts,
    // never by generated prose or a simulated application state.
    for protection in [None, Some(false), Some(true)] {
        let temporary = tempfile::tempdir().unwrap();
        let upstream = wiremock::MockServer::start().await;
        let (services, router, model) = fixture_with_model_route(temporary.path(), &format!("{}/v1", upstream.uri()),
            "openai.chat_text", "/chat/completions").await;
        let source = temporary.path().join("import-source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("index.html"), "EXTERNAL_IMPORT_SOURCE").unwrap();
        let current = data(&router, "POST", "/api/apps", json!({"name":"Importing application"})).await;
        let current_id = current["id"].as_str().unwrap();
        let opened = data(&router, "POST", &format!("/api/apps/{current_id}/development-session"), json!({"model":model})).await;
        let current_session = opened["conversation"]["conversation_id"].as_str().unwrap();
        let target = if protection.is_some() { data(&router, "POST", "/api/apps", json!({"name":"Protected target"})).await } else { current.clone() };
        let target_id = target["id"].as_str().unwrap();
        let target_root = std::path::Path::new(target["directory"].as_str().unwrap());
        std::fs::write(target_root.join("index.html"), "CURRENT_TARGET_BYTES").unwrap();
        let mut target_session = None;
        if let Some(unknown) = protection {
            let target_opened = data(&router, "POST", &format!("/api/apps/{target_id}/development-session"), json!({"model":model})).await;
            let session = target_opened["conversation"]["conversation_id"].as_str().unwrap().to_owned();
            start_protected_target_turn(&services, &session, unknown).await;
            target_session = Some(session);
        }
        let target_facts = if let Some(session) = &target_session { Some(canonical_bytes(&services, session).await) } else { None };
        let arguments = json!({"app_id":target_id,"path":source,"import_kind":"artifact"}).to_string();
        let rounds = Arc::new(AtomicUsize::new(0));
        let seen = rounds.clone();
        wiremock::Mock::given(wiremock::matchers::method("POST")).and(wiremock::matchers::path("/v1/chat/completions"))
            .respond_with(move |request: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                let first = seen.fetch_add(1, Ordering::SeqCst) == 0;
                if first {
                    assert!(body["tools"].as_array().unwrap().iter().any(|tool| tool["function"]["name"] == "import_app"),
                        "the selected application's import schema must be exposed before the first model step");
                }
                let delta = if first { json!({"tool_calls":[{"index":0,"id":"scripted-project-import","type":"function",
                    "function":{"name":"import_app","arguments":arguments}}]}) }
                    else { json!({"content":if protection.is_some() { "The import was rejected because the target has active or unknown work. No target files were changed." }
                        else { "The import tool completed. The existing application contains the imported source." }}) };
                let chunk = json!({"id":"scripted-project-import-provider","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
                let terminal = json!({"id":"scripted-project-import-provider","choices":[{"index":0,"delta":{},"finish_reason":if first {"tool_calls"} else {"stop"}}]});
                wiremock::ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
                    .set_body_string(format!("data: {chunk}\n\ndata: {terminal}\n\ndata: [DONE]\n\n"))
            }).mount(&upstream).await;
        let accepted = data(&router, "POST", &format!("/api/agent-sessions/{current_session}/turns"), json!({
            "idempotency_key":"scripted-project-import-input", "input":{"content":format!("Use import_app to import this external folder into application {target_id}. Report the actual tool result.")}})).await;
        let store = AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
        let session = nomifun_agent_contracts::AgentSessionId::from(current_session.to_owned());
        let operation = nomifun_agent_contracts::OperationId::from(accepted["operation_id"].as_str().unwrap().to_owned());
        let terminal = tokio::time::timeout(std::time::Duration::from_secs(40), async {
            loop {
                let receipt = store.read_turn_receipt(&session, &operation).await.unwrap();
                if matches!(receipt.status, TurnReceiptStatus::Completed | TurnReceiptStatus::Failed | TurnReceiptStatus::Cancelled) { break receipt; }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }).await.unwrap();
        assert!(rounds.load(Ordering::SeqCst) >= 2);
        let imports = store.list_effects(&session).await.unwrap().into_iter().filter(|effect|
            effect.turn_id == operation && effect.action_id.as_ref() == "app.hosting/import").collect::<Vec<_>>();
        assert_eq!(imports.len(), 1, "the real import operation must have one exact HostedEffect receipt");
        if protection.is_none() {
            assert_eq!(terminal.status, TurnReceiptStatus::Completed);
            assert_eq!(imports[0].state, AgentEffectState::Returned);
            assert_eq!(std::fs::read_to_string(target_root.join("index.html")).unwrap(), "EXTERNAL_IMPORT_SOURCE");
            let saved = data(&router, "GET", &format!("/api/apps/{current_id}"), Value::Null).await;
            assert_eq!(saved["id"], current_id);
            assert_eq!(data(&router, "GET", "/api/apps", Value::Null).await.as_array().unwrap().len(), 1);
        } else {
            assert_eq!(imports[0].state, AgentEffectState::Rejected);
            assert_eq!(imports[0].bounded_observation.as_ref().unwrap()["rejected_before_dispatch"], true);
            assert_eq!(std::fs::read_to_string(target_root.join("index.html")).unwrap(), "CURRENT_TARGET_BYTES");
            let target_session = target_session.as_ref().unwrap();
            assert_eq!(canonical_bytes(&services, target_session).await, target_facts.unwrap());
        }
        assert_eq!(std::fs::read_to_string(source.join("index.html")).unwrap(), "EXTERNAL_IMPORT_SOURCE");
        services.cleanup_after_startup_failure(anyhow::anyhow!("scripted import pipeline fixture finished")).await;
    }
}

#[tokio::test]
async fn repeated_import_of_one_external_source_creates_independent_application_projects() {
    let temporary = tempfile::tempdir().unwrap();
    let (services, router, model) = fixture(temporary.path()).await;
    let source = temporary.path().join("shared-external-source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("index.html"), "ORIGINAL_EXTERNAL_SOURCE").unwrap();
    let first = data(&router, "POST", "/api/apps/import", json!({"source":source,"import_kind":"artifact","name":"First independent app"})).await;
    let second = data(&router, "POST", "/api/apps/import", json!({"source":source,"import_kind":"artifact","name":"Second independent app"})).await;
    let first_id = first["id"].as_str().unwrap();
    let second_id = second["id"].as_str().unwrap();
    assert_ne!(first_id, second_id);
    assert_ne!(first["directory"], second["directory"]);
    assert_ne!(std::path::Path::new(first["directory"].as_str().unwrap()), source);
    assert_ne!(std::path::Path::new(second["directory"].as_str().unwrap()), source);
    let first_session = data(&router, "POST", &format!("/api/apps/{first_id}/development-session"), json!({"model":model})).await;
    let next_first_session = data(&router, "POST", &format!("/api/apps/{first_id}/development-session"), json!({"new_session":true,
        "creation_key":uuid::Uuid::now_v7().to_string(),"model":model})).await;
    let second_session = data(&router, "POST", &format!("/api/apps/{second_id}/development-session"), json!({"model":model})).await;
    assert_eq!(first_session["conversation"]["extra"]["workspace"], first["directory"]);
    assert_eq!(next_first_session["conversation"]["extra"]["workspace"], first["directory"]);
    assert_eq!(second_session["conversation"]["extra"]["workspace"], second["directory"]);
    let memory = |response: &Value| response["conversation"]["agent_snapshot"]["canonical_binding"]["typed_resource_bindings"].as_array().unwrap()
        .iter().find(|resource| resource["resource_kind"] == "project_memory").unwrap()["resource_id"].clone();
    assert_eq!(memory(&first_session), memory(&next_first_session));
    assert_ne!(memory(&first_session), memory(&second_session));
    std::fs::write(std::path::Path::new(first["directory"].as_str().unwrap()).join("index.html"), "ONLY_FIRST_APP_WAS_EDITED").unwrap();
    assert_eq!(std::fs::read_to_string(std::path::Path::new(second["directory"].as_str().unwrap()).join("index.html")).unwrap(), "ORIGINAL_EXTERNAL_SOURCE");
    assert_eq!(std::fs::read_to_string(source.join("index.html")).unwrap(), "ORIGINAL_EXTERNAL_SOURCE");
    let first_history = data(&router, "GET", &format!("/api/apps/{first_id}/development-sessions"), Value::Null).await;
    let second_history = data(&router, "GET", &format!("/api/apps/{second_id}/development-sessions"), Value::Null).await;
    assert_eq!(first_history["total"], 2);
    assert_eq!(second_history["total"], 1);
    assert!(first_history["items"].as_array().unwrap().iter().all(|item| item["conversation"]["extra"]["workspace"] == first["directory"]));
    assert!(second_history["items"].as_array().unwrap().iter().all(|item| item["conversation"]["extra"]["workspace"] == second["directory"]));
    let (status, _) = request(&router, "POST", &format!("/api/apps/{first_id}/development-session"),
        json!({"session_id":second_session["conversation"]["conversation_id"]})).await;
    assert!(!status.is_success());
    services.cleanup_after_startup_failure(anyhow::anyhow!("independent imported project fixture finished")).await;
}
