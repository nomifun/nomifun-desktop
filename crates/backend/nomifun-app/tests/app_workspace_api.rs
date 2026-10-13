//! Current application HTTP records, ordinary canonical Sessions and manual
//! import use the production owners. Opening a Session does not run a model.
use std::sync::Arc;
use axum::{Router, body::Body, http::{Request, StatusCode}};
use http_body_util::BodyExt;
use nomifun_app::{AppConfig, AuthPolicy, compatibility::{AppServices, create_router}};
use nomifun_api_types::{AgentChatModelSelectionDto, ConversationResponse};
use serde_json::{Value, json};
use tower::ServiceExt;

const TRUST: &str = "application-workspace-isolated-test";

#[tokio::test]
async fn explicit_application_workflow_refresh_preserves_legacy_blank_revision_and_session_history() {
    let temporary = tempfile::tempdir().unwrap();
    let (services, router) = fixture(temporary.path()).await;
    let model = seed_model(&services).await;
    let editor = data(&router, "POST", "/api/agent-presets/from-template/app.developer", json!({
        "model":model,"reuse_existing":true,"display_name":"app.developer"})).await;
    let preset = editor["preset"]["preset_id"].as_str().unwrap();
    let mut draft = editor["draft"].clone();
    draft["document"]["instructions"] = json!("");
    let saved = data(&router, "POST", &format!("/api/agent-presets/{preset}/revisions"), json!({
        "expected_current_revision":editor["revision"]["reference"],"draft":draft,
        "reason":"Current-generation blank application role regression fixture"})).await;
    let revision = saved["revision"]["reference"]["revision"].as_u64().unwrap();
    let old_revision = data(&router, "GET", &format!("/api/agent-presets/{preset}/revisions/{revision}"), Value::Null).await;
    assert_eq!(old_revision["document"]["instructions"], "");
    let app = data(&router, "POST", "/api/apps", json!({"name":"保留应用开发历史"})).await;
    let app_id = app["id"].as_str().unwrap();
    let opened = data(&router, "POST", &format!("/api/apps/{app_id}/development-session"), json!({"model":model})).await;
    let session = opened["conversation"]["conversation_id"].as_str().unwrap();
    let initial_binding = opened["conversation"]["agent_snapshot"]["canonical_binding"].clone();
    let mut old_binding = initial_binding.clone();
    old_binding["preset_revision_ref"] = saved["revision"]["reference"].clone();
    old_binding["resolved_snapshot_ref"] = saved["resolved_snapshot_ref"].clone();
    old_binding["binding_version"] = json!(initial_binding["binding_version"].as_u64().unwrap()+1);
    let store = nomifun_agent_session::AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    let session_id = nomifun_agent_contracts::AgentSessionId::from(session.to_owned());
    let principal = nomifun_agent_contracts::PrincipalRef { principal_kind:"user".into(), principal_id:services.authoritative_user_id.as_ref().into() };
    // Seed a genuine current-generation blank role through the same canonical
    // transition, rather than rewriting an immutable Snapshot or Session row.
    let active = store.active_capability_ids(&session_id).await.unwrap();
    let transition = uuid::Uuid::now_v7().to_string();
    store.replace_session_agent_binding(&principal, &session_id, nomifun_agent_session::ReplaceSessionAgentBinding {
        expected:serde_json::from_value(initial_binding).unwrap(), replacement:serde_json::from_value(old_binding.clone()).unwrap(),
        previous_agent_label:"app.developer".into(),next_agent_label:"app.developer".into(),
        transition_id:transition.clone().into(),request_digest:nomifun_agent_contracts::digest_payload(&old_binding).unwrap(),
        idempotency_key:transition.into(),handoff_mode:nomifun_agent_contracts::AgentHandoffMode::ContextOnly,
        handoff:None,initial_active_capability_ids:active,
    }).await.unwrap();
    let source = std::path::Path::new(app["directory"].as_str().unwrap()).join("user-work.txt");
    std::fs::write(&source, "APPLICATION_SOURCE_UNCHANGED").unwrap();
    let old_snapshot_id = old_binding["resolved_snapshot_ref"]["snapshot_id"].as_str().unwrap();
    let old_snapshot: String = sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id = ?")
        .bind(old_snapshot_id).fetch_one(services.database.pool()).await.unwrap();
    let history = data(&router, "GET", &format!("/api/agent-sessions/{session}/events"), Value::Null).await;
    let before = canonical_fact_bytes(&services, session).await;
    let reopened = data(&router, "POST", &format!("/api/apps/{app_id}/development-session"), json!({})).await;
    assert_eq!(reopened["conversation"]["agent_snapshot"]["canonical_binding"], old_binding);
    assert_eq!(canonical_fact_bytes(&services, session).await, before, "ordinary reopen must never update the frozen application role");

    let refreshed = data(&router, "POST", &format!("/api/apps/{app_id}/development-session"), json!({"refresh_workflow":true})).await;
    assert_eq!(refreshed["app"]["id"], app_id);
    assert_eq!(refreshed["conversation"]["conversation_id"], session);
    let new_binding = &refreshed["conversation"]["agent_snapshot"]["canonical_binding"];
    assert_ne!(new_binding["resolved_snapshot_ref"], old_binding["resolved_snapshot_ref"]);
    assert_eq!(new_binding["typed_resource_bindings"], old_binding["typed_resource_bindings"]);
    assert_eq!(new_binding["binding_version"].as_u64().unwrap(), old_binding["binding_version"].as_u64().unwrap()+1);
    assert_eq!(refreshed["conversation"]["model"], opened["conversation"]["model"]);
    let after = data(&router, "GET", &format!("/api/agent-sessions/{session}/events"), Value::Null).await;
    let prior = history["events"].as_array().unwrap();
    assert_eq!(&after["events"].as_array().unwrap()[..prior.len()], prior.as_slice(), "existing canonical events/payloads must remain byte-equivalent");
    assert_eq!(data(&router,"GET",&format!("/api/agent-presets/{preset}/revisions/{revision}"),Value::Null).await, old_revision);
    let snapshot_after: String = sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id = ?")
        .bind(old_snapshot_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(old_snapshot, snapshot_after);
    let facts = canonical_fact_bytes(&services, session).await;
    let repeated = data(&router,"POST",&format!("/api/apps/{app_id}/development-session"),json!({"refresh_workflow":true})).await;
    assert_eq!(repeated["conversation"]["agent_snapshot"]["canonical_binding"], *new_binding);
    assert_eq!(canonical_fact_bytes(&services, session).await, facts, "the current application workflow refresh is a no-op");
    assert_eq!(std::fs::read_to_string(source).unwrap(), "APPLICATION_SOURCE_UNCHANGED");
    services.cleanup_after_startup_failure(anyhow::anyhow!("application role refresh fixture completed")).await;
}

/// Preserve the complete stored values, including raw JSON/blob bytes, rather
/// than comparing a projection that could conceal a rewrite or lost fields.
async fn canonical_fact_bytes(services: &AppServices, session: &str) -> std::collections::BTreeMap<String, Vec<String>> {
    use sqlx::Row;
    let mut facts = std::collections::BTreeMap::new();
    for table in ["agent_sessions", "agent_turns", "agent_events", "agent_payloads", "agent_effects", "agent_session_resources"] {
        let columns = sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(services.database.pool()).await.unwrap();
        let fields = columns.iter().map(|row| {
            let name: String = row.get("name");
            let quoted = format!("\"{}\"", name.replace('"', "\"\""));
            format!("typeof({quoted}) || ':' || hex({quoted})")
        }).collect::<Vec<_>>().join(" || '|' || ");
        let identity = if table == "agent_sessions" { "agent_session_id" } else { "session_id" };
        let bytes: Vec<String> = sqlx::query_scalar(&format!("SELECT {fields} FROM {table} WHERE {identity} = ? ORDER BY rowid"))
            .bind(session).fetch_all(services.database.pool()).await.unwrap();
        facts.insert(table.to_owned(), bytes);
    }
    facts
}

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
    assert_eq!(response["success"], true, "{response}");
    response["data"].clone()
}

async fn fixture(root: &std::path::Path) -> (AppServices, Router) {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let services = AppServices::from_config(database, &AppConfig {
        data_dir: root.join("data"), work_dir: root.join("work"), auth_policy: AuthPolicy::TrustLocalToken,
        local_trust_secret: Some(Arc::from(TRUST)), ..Default::default()
    }).await.unwrap();
    let router = create_router(&services).await;
    (services, router)
}

async fn seed_model(services: &AppServices) -> AgentChatModelSelectionDto {
    seed_model_route(services, "https://api.openai.invalid/v1", "openai.responses", "/responses").await
}

async fn seed_model_route(services: &AppServices, base_url: &str, protocol: &str, endpoint: &str) -> AgentChatModelSelectionDto {
    use nomifun_db::{CreateProviderParams, IProviderRepository, NewProviderModel, NewProviderModelCapability, SqliteProviderRepository};
    let provider_id = uuid::Uuid::now_v7().to_string();
    let encrypted = nomifun_common::encrypt_string(r#"{"api_keys":["opening-only-test"]}"#, &services.encryption_key).unwrap();
    let capabilities = [NewProviderModelCapability { task: "chat", traits: r#"["vision_input","function_calling","reasoning","web_search"]"#,
        protocol, connection_role: "default", endpoint: Some(endpoint), provider_params: "{}", context_limit: Some(128_000), ..Default::default() }];
    SqliteProviderRepository::new(services.database.pool().clone()).create(CreateProviderParams {
        provider_id: Some(&provider_id), platform: "openai", name: "Application Session opening fixture",
        base_url, auth_scheme: "bearer", credentials_encrypted: &encrypted,
        enabled: true, bedrock_config: None, sort_order: Some(-10_000),
    }, &NewProviderModel { model: "application-opening-test", enabled: true, sort_order: -10_000,
        description: Some("No model requests are made in this storage and Session admission test."), capabilities: &capabilities }, &[]).await.unwrap();
    AgentChatModelSelectionDto { provider_id, model: "application-opening-test".into() }
}

#[tokio::test]
async fn manual_import_and_metadata_edit_do_not_require_an_agent_or_model() {
    let temporary = tempfile::tempdir().unwrap();
    let (services, router) = fixture(temporary.path()).await;
    let app = data(&router, "POST", "/api/apps", json!({})).await;
    assert_eq!(app["name"], "新应用");
    assert!(app["development_session_id"].is_null());
    let id = app["id"].as_str().unwrap();
    let source = temporary.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("index.html"), "<h1>Manual imported application</h1>").unwrap();
    let app = data(&router, "POST", &format!("/api/apps/{id}/import"), json!({"source":source,"name":"工具","import_kind":"artifact"})).await;
    assert_eq!(app["id"], id);
    assert_eq!(app["import_kind"], "artifact");
    let app = data(&router, "PUT", &format!("/api/apps/{id}"), json!({"name":"桌面工具","description":"可搜索的说明","tags":["效率","工具"],"settings":app["settings"]})).await;
    let unchanged = data(&router, "PUT", &format!("/api/apps/{id}"), json!({"name":"桌面工具","settings":app["settings"]})).await;
    assert_eq!(unchanged["description"], "可搜索的说明");
    assert_eq!(unchanged["tags"], json!(["效率","工具"]));
    assert!(unchanged["development_session_id"].is_null());
    let (status, _) = request(&router, "PUT", &format!("/api/apps/{id}"), json!({"name":"wrong","settings":app["settings"],"development_session_id":uuid::Uuid::now_v7().to_string()})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "metadata editing must not let a client invent a Session reference");
    data(&router, "DELETE", &format!("/api/apps/{id}"), Value::Null).await;
    assert!(source.join("index.html").exists());
    services.cleanup_after_startup_failure(anyhow::anyhow!("workspace fixture completed")).await;
}

#[tokio::test]
async fn fixed_application_agent_uses_one_scoped_session_and_deletion_purges_its_facts() {
    let temporary = tempfile::tempdir().unwrap();
    let (services, router) = fixture(temporary.path()).await;
    let model = seed_model(&services).await;
    let app = data(&router, "POST", "/api/apps", json!({"name":"待办通知","description":"会话开发","tags":["工具"]})).await;
    let id = app["id"].as_str().unwrap();
    let opened = data(&router, "POST", &format!("/api/apps/{id}/development-session"), json!({"model":model,"reasoning_effort":"high"})).await;
    let session: ConversationResponse = serde_json::from_value(opened["conversation"].clone()).unwrap();
    assert_eq!(session.session_purpose, nomifun_agent_contracts::SessionPurpose::Conversation);
    assert_eq!(session.app_project_id.as_deref(), Some(id));
    assert_eq!(session.reasoning_effort, Some(nomifun_api_types::SessionReasoningEffortDto::High));
    assert_eq!(session.extra["official_template_key"], "app.developer");
    assert_eq!(std::path::Path::new(session.extra["workspace"].as_str().unwrap()), std::path::Path::new(app["directory"].as_str().unwrap()));
    assert_eq!(opened["app"]["development_session_id"], session.conversation_id);
    assert!(opened["app"]["settings"].get("exclusive_session_ids").is_none());
    let reopened = data(&router, "POST", &format!("/api/apps/{id}/development-session"), json!({})).await;
    assert_eq!(reopened["conversation"]["conversation_id"], session.conversation_id);
    data(&router, "DELETE", &format!("/api/apps/{id}"), Value::Null).await;
    assert!(!std::path::Path::new(app["directory"].as_str().unwrap()).exists());
    let (status, _) = request(&router, "GET", &format!("/api/agent-sessions/{}/events", session.conversation_id), Value::Null).await;
    assert!(!status.is_success());
    assert!(canonical_fact_bytes(&services, &session.conversation_id).await.iter()
        .filter(|(table, _)| table.as_str() != "agent_sessions").all(|(_, rows)| rows.is_empty()));
    services.cleanup_after_startup_failure(anyhow::anyhow!("workspace fixture completed")).await;
}

#[tokio::test]
async fn unavailable_application_history_remains_readable_without_recreating_its_deleted_project() {
    let temporary = tempfile::tempdir().unwrap();
    let (services, router) = fixture(temporary.path()).await;
    let model = seed_model(&services).await;
    let app = data(&router, "POST", "/api/apps", json!({"name":"Historical application removed before scoped cleanup"})).await;
    let id = app["id"].as_str().unwrap();
    let opened = data(&router, "POST", &format!("/api/apps/{id}/development-session"), json!({"model":model})).await;
    let session = opened["conversation"]["conversation_id"].as_str().unwrap();
    let principal = nomifun_agent_contracts::PrincipalRef { principal_kind:"user".into(), principal_id:services.authoritative_user_id.as_ref().into() };
    let canonical = nomifun_conversation::CanonicalAgentSessionOwner::from_pool(services.database.pool().clone()).await.unwrap();
    let session_id: nomifun_agent_contracts::AgentSessionId = session.into();
    canonical.start_turn(&principal, &session_id, "preserved-unavailable-application-input", json!({
        "content":"Retained canonical application history from an unavailable project"})).await.unwrap();
    canonical.cancel(&principal, &session_id, "preserved-unavailable-application-cancel").await.unwrap();
    let projection = data(&router, "GET", &format!("/api/agent-sessions/{session}/projection"), Value::Null).await;
    let events = data(&router, "GET", &format!("/api/agent-sessions/{session}/events"), Value::Null).await;
    let before = canonical_fact_bytes(&services, session).await;
    let directory = std::path::Path::new(app["directory"].as_str().unwrap());
    // This direct existing hosting owner simulates the former application-only
    // deletion contract. The current HTTP App delete always cascades its scope.
    services.app_hosting.delete_with(id, |_| async { Ok(()) }).await.unwrap();
    assert!(!directory.exists());
    assert!(matches!(services.app_hosting.get(id).await, Err(nomifun_app_hosting::AppError::NotFound(_))));
    let (status, _) = request(&router, "GET", &format!("/api/apps/{id}"), Value::Null).await;
    assert!(!status.is_success());
    let after_projection = data(&router, "GET", &format!("/api/agent-sessions/{session}/projection"), Value::Null).await;
    assert_eq!(after_projection, projection);
    assert_eq!(after_projection["app_project_id"], id);
    assert_eq!(data(&router, "GET", &format!("/api/agent-sessions/{session}/events"), Value::Null).await, events);
    assert_eq!(canonical_fact_bytes(&services, session).await, before);
    assert!(!directory.exists(), "read-only unavailable history must never materialize a deleted application directory");
    services.cleanup_after_startup_failure(anyhow::anyhow!("unavailable application history fixture completed")).await;
}

#[cfg(windows)]
#[tokio::test]
async fn warmed_development_runtime_releases_its_owned_directory_before_application_deletion() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let temporary = tempfile::tempdir().unwrap();
    let (services, router) = fixture(temporary.path()).await;
    let model_server = wiremock::MockServer::start().await;
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    wiremock::Mock::given(wiremock::matchers::method("POST")).and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |_: &wiremock::Request| {
            let round = seen.fetch_add(1, Ordering::SeqCst);
            let delta = if round == 0 { json!({"tool_calls":[{"index":0,"id":"read-current",
                "type":"function","function":{"name":"current_app","arguments":"{}"}}]}) }
                else { json!({"content":"The current application is 手工应用. No files or process settings were changed."}) };
            let first = json!({"id":"application-lifecycle-test","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
            let last = json!({"id":"application-lifecycle-test","choices":[{"index":0,"delta":{},"finish_reason":if round==0 {"tool_calls"} else {"stop"}}]});
            wiremock::ResponseTemplate::new(200).insert_header("content-type","text/event-stream")
                .set_body_string(format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n"))
        }).mount(&model_server).await;
    let model = seed_model_route(&services, &format!("{}/v1",model_server.uri()), "openai.chat_text", "/chat/completions").await;
    let original = temporary.path().join("manual-artifact");
    std::fs::create_dir(&original).unwrap();
    std::fs::write(original.join("index.html"), "USER_ORIGINAL").unwrap();
    let app = data(&router, "POST", "/api/apps/import", json!({"source":original,"import_kind":"artifact","name":"手工应用"})).await;
    let id = app["id"].as_str().unwrap();
    let opened = data(&router, "POST", &format!("/api/apps/{id}/development-session"), json!({"model":model})).await;
    let session = opened["conversation"]["conversation_id"].as_str().unwrap();
    // Scripted provider responses are deterministic lifecycle fault coverage,
    // not evidence of live LLM generation. The real Runtime must run and close
    // one read-only current_app Turn without any process/workspace write.
    let accepted = data(&router, "POST", &format!("/api/agent-sessions/{session}/turns"), json!({
        "idempotency_key":"inspect-current-application", "input":{"content":"Only inspect the current application with current_app and report its identity; do not modify files or run the application."}})).await;
    let store = nomifun_agent_session::AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    let session_id = nomifun_agent_contracts::AgentSessionId::from(session.to_owned());
    let operation = nomifun_agent_contracts::OperationId::from(accepted["operation_id"].as_str().unwrap().to_owned());
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let receipt = store.read_turn_receipt(&session_id, &operation).await.unwrap();
            if receipt.status == nomifun_agent_session::TurnReceiptStatus::Completed { break; }
            assert_eq!(receipt.status, nomifun_agent_session::TurnReceiptStatus::Running, "{receipt:?}");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert!(requests.load(Ordering::SeqCst) >= 2);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let (status, result) = request(&router, "DELETE", &format!("/api/apps/{id}"), Value::Null).await;
    assert_eq!(status, StatusCode::OK, "the scoped Session must be cleaned before physical project deletion: {result}");
    assert!(!std::path::Path::new(app["directory"].as_str().unwrap()).exists());
    assert_eq!(std::fs::read_to_string(original.join("index.html")).unwrap(), "USER_ORIGINAL");
    let (status, _) = request(&router, "GET", &format!("/api/agent-sessions/{session}/events"), Value::Null).await;
    assert!(!status.is_success());
    assert!(canonical_fact_bytes(&services, session).await.iter()
        .filter(|(table, _)| table.as_str() != "agent_sessions").all(|(_, rows)| rows.is_empty()));
    services.cleanup_after_startup_failure(anyhow::anyhow!("warmed application deletion fixture completed")).await;
}

#[tokio::test]
async fn unknown_retired_ownership_does_not_convert_or_delete_a_real_canonical_session() {
    let temporary = tempfile::tempdir().unwrap();
    let (services, router) = fixture(temporary.path()).await;
    let model = seed_model(&services).await;
    let app = data(&router, "POST", "/api/apps", json!({"name":"保留作品与历史"})).await;
    let id = app["id"].as_str().unwrap();
    let opened = data(&router, "POST", &format!("/api/apps/{id}/development-session"), json!({"model":model})).await;
    let session = opened["conversation"]["conversation_id"].as_str().unwrap();
    let directory = std::path::Path::new(app["directory"].as_str().unwrap());
    std::fs::write(directory.join("work.txt"), "USER_WORK").unwrap();
    let before_events = data(&router, "GET", &format!("/api/agent-sessions/{session}/events"), Value::Null).await;
    let root = directory.parent().unwrap().parent().unwrap();
    let registry = root.join("registry").join(format!("{id}.json"));
    let mut stored: Value = serde_json::from_slice(&std::fs::read(&registry).unwrap()).unwrap();
    stored["record"]["settings"]["exclusive_session_ids"] = json!([session]);
    let registry_bytes = serde_json::to_vec_pretty(&stored).unwrap();
    std::fs::write(&registry, &registry_bytes).unwrap();
    let result = nomifun_app_hosting::HostingOwner::new(root.to_owned(), nomifun_app_hosting::LocalConnection {
        host_url:"http://127.0.0.1:5177".into(),local_trust:TRUST.into(),
    }).await;
    let error = match result { Ok(_) => panic!("a stored reference cannot author Session ownership"), Err(error) => error.to_string() };
    assert!(error.contains("exclusive_session_ids"), "{error}");
    assert_eq!(std::fs::read(&registry).unwrap(), registry_bytes);
    assert_eq!(std::fs::read_to_string(directory.join("work.txt")).unwrap(), "USER_WORK");
    let after_events = data(&router, "GET", &format!("/api/agent-sessions/{session}/events"), Value::Null).await;
    assert_eq!(before_events, after_events);
    let projection = data(&router, "GET", &format!("/api/agent-sessions/{session}/projection"), Value::Null).await;
    assert_eq!(projection["conversation_id"], session);
    services.cleanup_after_startup_failure(anyhow::anyhow!("registry retirement fixture completed")).await;
}

#[tokio::test]
async fn deletion_does_not_close_another_session_retaining_the_same_workspace() {
    let temporary = tempfile::tempdir().unwrap();
    let (services, router) = fixture(temporary.path()).await;
    let model = seed_model(&services).await;
    let app = data(&router, "POST", "/api/apps", json!({"name":"Shared workspace protection"})).await;
    let id = app["id"].as_str().unwrap();
    let opened = data(&router, "POST", &format!("/api/apps/{id}/development-session"), json!({"model":model})).await;
    let session = opened["conversation"]["conversation_id"].as_str().unwrap();
    let other_created = data(&router, "POST", "/api/agent-sessions", json!({
        "preset_id":opened["conversation"]["preset_id"], "model":model,
        "workspace":app["directory"], "title":"Independent ordinary same-root Session"
    })).await;
    let other = other_created["agent_session_id"].as_str().unwrap();
    let other_projection = data(&router, "GET", &format!("/api/agent-sessions/{other}/projection"), Value::Null).await;
    assert_eq!(other_projection["extra"]["workspace"], opened["conversation"]["extra"]["workspace"]);
    assert!(other_projection["app_project_id"].is_null());
    data(&router, "POST", &format!("/api/agent-sessions/{other}/warmup"), json!({})).await;
    assert!(services.agent_runtime_sessions.has_owned_runtime(other));
    let before = canonical_fact_bytes(&services, session).await;
    let other_before = canonical_fact_bytes(&services, other).await;
    let (status, response) = request(&router, "DELETE", &format!("/api/apps/{id}"), Value::Null).await;
    assert_eq!(status, StatusCode::CONFLICT, "another same-root owner must retain its resources instead of being killed for application deletion: {response}");
    assert!(std::path::Path::new(app["directory"].as_str().unwrap()).exists());
    assert!(services.agent_runtime_sessions.has_owned_runtime(other));
    assert_eq!(before, canonical_fact_bytes(&services, session).await);
    assert_eq!(other_before, canonical_fact_bytes(&services, other).await);
    // Explicit test cleanup of that exact independent owner allows retry;
    // application deletion itself never performs this close.
    services.agent_runtime_sessions.terminate_and_wait_result(other, None).await.unwrap();
    data(&router, "DELETE", &format!("/api/apps/{id}"), Value::Null).await;
    assert_eq!(other_before, canonical_fact_bytes(&services, other).await);
    services.cleanup_after_startup_failure(anyhow::anyhow!("shared application workspace fixture completed")).await;
}

#[tokio::test]
async fn manual_import_and_overwrite_reject_active_development_and_closed_unknown_effects() {
    use nomifun_agent_contracts::{AgentSessionId, CorrelationId, SemanticSessionEventDraft, SessionEventAppend,
        SessionEventKind, SessionEventPayloadRef, StrictJsonValue};
    use nomifun_agent_session::{AgentSessionStore, EffectEventRequest, EffectStrategy, EffectTerminalState};

    for unknown in [false, true] {
        let temporary = tempfile::tempdir().unwrap();
        let (services, router) = fixture(temporary.path()).await;
        let model = seed_model(&services).await;
        let app = data(&router, "POST", "/api/apps", json!({"name":"Protected workspace"})).await;
        let id = app["id"].as_str().unwrap();
        let directory = std::path::Path::new(app["directory"].as_str().unwrap());
        std::fs::write(directory.join("index.html"), "CURRENT_FILES").unwrap();
        let source = temporary.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("index.html"), "REPLACEMENT_FILES").unwrap();
        let opened = data(&router, "POST", &format!("/api/apps/{id}/development-session"), json!({"model":model})).await;
        let session_id = AgentSessionId::from(opened["conversation"]["conversation_id"].as_str().unwrap().to_owned());
        let store = AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
        // Canonical owner fault injection only. No provider response or
        // fabricated generation success is used for this protection test.
        let turn_id = format!("application-file-fence:{unknown}");
        let (_, started) = store.start_turn(&session_id, "session_api".into(), turn_id.clone().into(),
            turn_id.clone().into(), StrictJsonValue(json!({"content":"Continue developing this application"}))).await.unwrap();
        let turn_event = started.ack.unwrap().event_id;
        if unknown {
            let tool = SessionEventAppend { agent_session_id: session_id.clone(), event_id: "workspace-file-fence-tool".into(),
                producer_id: "runtime-supervisor".into(), idempotency_key: "workspace-file-fence-tool".into(),
                semantic_event: SemanticSessionEventDraft { kind: SessionEventKind("tool/call-started".into()), kind_version: 1,
                    correlation_id: CorrelationId::from("workspace-file-fence-operation"), causation_event_id: Some(turn_event.clone()),
                    payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"operation_id":"workspace-file-fence-operation",
                        "call_id":"workspace-file-fence-call","capability_id":"app.hosting","action_id":"app.hosting/start"}))) } };
            let tool_event = store.append_event(&tool).await.unwrap().ack.unwrap().event_id;
            let effect = EffectEventRequest { agent_session_id: session_id.clone(), effect_id: "workspace-file-fence-effect".into(),
                turn_id: turn_id.clone().into(), operation_id: "workspace-file-fence-operation".into(), owner_domain: "app.hosting".into(),
                capability_module: "app.hosting".into(), action_id: "app.hosting/start".into(), resource_binding_id: None,
                resource_key: Some(format!("app.hosting:{id}")), input_digest: "8".repeat(64).into(), recorded_at: 2000,
                event_id: "workspace-file-fence-started".into(), producer_id: "capability-host".into(),
                idempotency_key: "workspace-file-fence-effect".into(), correlation_id: "workspace-file-fence-effect".into(),
                strategy: EffectStrategy::ExternalUncertainEffect, causation_event_id: Some(tool_event),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"app_id":id}))) };
            let started_effect = store.record_effect_started(effect.clone()).await.unwrap().ack.unwrap().event_id;
            store.record_effect_terminal(EffectEventRequest { event_id: "workspace-file-fence-unknown".into(),
                producer_id: "application-owner".into(), causation_event_id: Some(started_effect),
                recorded_at: 2010, ..effect }, EffectTerminalState::Uncertain).await.unwrap();
            store.append_event(&SessionEventAppend { agent_session_id: session_id.clone(), event_id: "workspace-file-fence-terminal".into(),
                producer_id: "runtime-supervisor".into(), idempotency_key: "workspace-file-fence-terminal".into(),
                semantic_event: SemanticSessionEventDraft { kind: SessionEventKind("turn/failed".into()), kind_version: 1,
                    correlation_id: turn_id.clone().into(), causation_event_id: Some(turn_event),
                    payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"error":"Effect outcome not confirmed"}))) } }).await.unwrap();
            assert!(store.head(&session_id).await.unwrap().active_turn_id.is_none(), "the unknown-effect case must have a closed Turn");
            assert!(store.has_unsettled_effects(&session_id).await.unwrap());
        }
        for path in [format!("/api/apps/{id}/import"), format!("/api/apps/{id}/overwrite")] {
            let body = if path.ends_with("/overwrite") { json!({"source":source}) }
                else { json!({"source":source,"import_kind":"artifact"}) };
            let (status, response) = request(&router, "POST", &path, body).await;
            assert_eq!(status, StatusCode::CONFLICT, "{path}: {response}");
            assert_eq!(std::fs::read_to_string(directory.join("index.html")).unwrap(), "CURRENT_FILES");
        }
        let stored_before = canonical_fact_bytes(&services, session_id.as_ref()).await;
        let (status, response) = request(&router, "DELETE", &format!("/api/apps/{id}"), Value::Null).await;
        assert_eq!(status, StatusCode::CONFLICT, "active or unknown canonical work must block resource release and physical deletion: {response}");
        assert_eq!(std::fs::read_to_string(directory.join("index.html")).unwrap(), "CURRENT_FILES");
        assert_eq!(stored_before, canonical_fact_bytes(&services, session_id.as_ref()).await);
        data(&router, "GET", &format!("/api/apps/{id}"), Value::Null).await;
        let (status, response) = request(&router, "PUT", &format!("/api/apps/{id}"), json!({"detect_settings":true})).await;
        assert_eq!(status, StatusCode::CONFLICT, "configuration discovery must respect the same active/unknown-effect fence: {response}");
        assert!(!directory.join(".nomifun-static-server.cjs").exists());
        data(&router, "PUT", &format!("/api/apps/{id}"), json!({"name":"Metadata can still be edited","settings":app["settings"],"description":"Still readable"})).await;
        data(&router, "POST", &format!("/api/apps/{id}/operate"), json!({"operation":"stop"})).await;
        services.cleanup_after_startup_failure(anyhow::anyhow!("workspace fence fixture completed")).await;
    }
}

#[tokio::test]
async fn sparse_http_configuration_updates_current_fields_without_running_the_application() {
    let temporary = tempfile::tempdir().unwrap();
    let (services, router) = fixture(temporary.path()).await;
    let app = data(&router, "POST", "/api/apps", json!({})).await;
    let id = app["id"].as_str().unwrap();
    let directory = std::path::Path::new(app["directory"].as_str().unwrap());
    std::fs::create_dir(directory.join("snake-game")).unwrap();
    std::fs::write(directory.join("snake-game/index.html"), "<title>贪吃蛇</title>").unwrap();
    data(&router, "PUT", &format!("/api/apps/{id}"), json!({"name":"贪吃蛇","description":"已生成的实际应用","tags":["游戏"]})).await;
    let before = data(&router, "GET", &format!("/api/apps/{id}"), Value::Null).await;
    assert!(before["settings"]["start_command"].is_null(), "ordinary metadata saves and reads cannot auto-detect source");
    let detected = data(&router, "PUT", &format!("/api/apps/{id}"), json!({"detect_settings":true})).await;
    assert_eq!(detected["id"], id);
    assert_eq!(detected["name"], "贪吃蛇");
    assert_eq!(detected["description"], "已生成的实际应用");
    assert_eq!(detected["tags"], json!(["游戏"]));
    assert_eq!(detected["settings"]["project_directory"], "snake-game");
    assert!(detected["settings"]["start_command"].as_str().unwrap().contains(".nomifun-static-server.cjs"));
    assert!(detected["settings"]["web_url"].as_str().unwrap().starts_with("http://127.0.0.1:"));
    assert_eq!(detected["state"], "stopped");
    assert!(detected["pid"].is_null());
    let changed = data(&router, "PUT", &format!("/api/apps/{id}"), json!({"settings":{"web_url":null},"description":"只改指定字段"})).await;
    assert_eq!(changed["name"], detected["name"]);
    assert_eq!(changed["settings"]["start_command"], detected["settings"]["start_command"]);
    assert_eq!(changed["settings"]["environment"], detected["settings"]["environment"]);
    // This preset owns its web address, so the standard preset fills an empty
    // address while an ordinary unknown backend's explicit null stays null.
    let custom = data(&router, "PUT", &format!("/api/apps/{id}"), json!({"settings":{"start_command":"node worker.cjs","web_url":null}})).await;
    assert!(custom["settings"]["web_url"].is_null());
    assert_eq!(custom["settings"]["start_command"], "node worker.cjs");
    assert_eq!(custom["name"], "贪吃蛇");
    let full = data(&router, "PUT", &format!("/api/apps/{id}"), json!({"name":"贪吃蛇","settings":custom["settings"]})).await;
    assert_eq!(full["settings"], custom["settings"], "full settings clients remain compatible");
    services.cleanup_after_startup_failure(anyhow::anyhow!("sparse config fixture completed")).await;
}

#[tokio::test]
async fn completed_config_requires_the_exact_latest_successful_turn_and_preserves_canonical_facts() {
    use nomifun_agent_contracts::{AgentSessionId, SemanticSessionEventDraft, SessionEventAppend,
        SessionEventKind, SessionEventPayloadRef, StrictJsonValue};
    use nomifun_agent_session::{AgentSessionStore, EffectEventRequest, EffectStrategy, EffectTerminalState};
    let temporary = tempfile::tempdir().unwrap();
    let (services, router) = fixture(temporary.path()).await;
    let model = seed_model(&services).await;
    let app = data(&router, "POST", "/api/apps", json!({"name":"实际元数据"})).await;
    let id = app["id"].as_str().unwrap();
    let directory = std::path::Path::new(app["directory"].as_str().unwrap());
    std::fs::write(directory.join("index.html"), "SOURCE_PRESERVED").unwrap();
    let opened = data(&router, "POST", &format!("/api/apps/{id}/development-session"), json!({"model":model})).await;
    let session = opened["conversation"]["conversation_id"].as_str().unwrap();
    let session_id = AgentSessionId::from(session.to_owned());
    let store = AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    // This is an admission and immutable-history fixture, not LLM generation.
    // Notification IDs are accepted-message UUIDs. They must never be confused
    // with input idempotency keys or canonical operation IDs.
    for reference in [uuid::Uuid::now_v7().to_string(), session.to_owned()] {
        let turn = uuid::Uuid::now_v7().to_string();
        let (status, _) = request(&router, "PUT", &format!("/api/apps/{id}"), json!({"detect_settings":true,"completion":{"session_id":reference,"turn_id":turn}})).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(!directory.join(".nomifun-static-server.cjs").exists());
    }
    let first_key = uuid::Uuid::now_v7().to_string();
    let first_scoped_key = format!("user:{}:{session}:{first_key}", services.authoritative_user_id);
    let first_operation = format!("turn:{first_scoped_key}");
    let (message, started) = store.start_turn(&session_id, "session_api".into(), first_scoped_key.into(), first_operation.clone().into(),
        StrictJsonValue(json!({"content":"Develop this application"}))).await.unwrap();
    let first = message.ack.unwrap().event_id.as_ref().to_owned();
    assert_ne!(first, first_key);
    assert_ne!(first, first_operation);
    let turn_event = started.ack.unwrap().event_id;
    let (status, _) = request(&router, "PUT", &format!("/api/apps/{id}"), json!({"detect_settings":true,"completion":{"session_id":session,"turn_id":first}})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let workspace = store.observe(&session_id, None, 1).await.unwrap().session.agent_binding.typed_resource_bindings.into_iter()
        .find(|binding| binding.resource_kind.as_ref() == "workspace").unwrap();
    let tool_operation = format!("{first_operation}:tool:write-source");
    assert_ne!(tool_operation, first_operation, "file Effects identify the Tool operation separately from its parent Turn");
    let tool_event = store.append_event(&SessionEventAppend { agent_session_id: session_id.clone(), event_id: "config-file-tool-started".into(),
        producer_id: "runtime-supervisor".into(), idempotency_key: "config-file-tool-started".into(),
        semantic_event: SemanticSessionEventDraft { kind: SessionEventKind("tool/call-started".into()), kind_version: 1,
            correlation_id: tool_operation.clone().into(), causation_event_id: Some(turn_event.clone()),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"operation_id":tool_operation,
                "call_id":"config-file-write","capability_id":"workspace.files","action_id":"workspace.files/write"}))) } }).await.unwrap().ack.unwrap().event_id;
    let resource_identity = json!({"principal_kind":"user","owner_id":services.authoritative_user_id.as_ref(),
        "workspace_root":std::fs::canonicalize(directory).unwrap().to_string_lossy()});
    let effect = EffectEventRequest { agent_session_id: session_id.clone(), effect_id: "config-file-effect".into(),
        turn_id: first_operation.clone().into(), operation_id: tool_operation.into(), owner_domain: "workspace".into(),
        capability_module: "workspace.files".into(), action_id: "workspace.files/write".into(), resource_binding_id: Some(workspace.binding_id),
        resource_key: Some(format!("workspace:{}",nomifun_agent_contracts::digest_payload(&resource_identity).unwrap().as_ref())),
        input_digest: nomifun_agent_contracts::digest_payload(&json!({"path":"index.html","content":"SOURCE_PRESERVED"})).unwrap(),
        recorded_at: 2000, event_id: "config-file-effect-started".into(), producer_id: "capability_host".into(),
        idempotency_key: "config-file-effect-lifecycle".into(), correlation_id: "config-file-effect".into(),
        strategy: EffectStrategy::ManagedEffect, causation_event_id: Some(tool_event),
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))) };
    let effect_start = store.record_effect_started(effect.clone()).await.unwrap().ack.unwrap().event_id;
    std::fs::write(directory.join("index.html"), "SOURCE_PRESERVED").unwrap();
    store.record_effect_terminal(EffectEventRequest { event_id: "config-file-effect-succeeded".into(), producer_id: "owning_plugin".into(),
        causation_event_id: Some(effect_start), recorded_at: 2010,
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"path":"index.html","written":true,"bytes":16,
            "sha256":nomifun_agent_contracts::digest_bytes(b"SOURCE_PRESERVED")}))), ..effect }, EffectTerminalState::Succeeded).await.unwrap();
    store.append_event(&SessionEventAppend { agent_session_id: session_id.clone(), event_id: "config-turn-completed".into(),
        producer_id: "runtime-supervisor".into(), idempotency_key: "config-turn-completed".into(),
        semantic_event: SemanticSessionEventDraft { kind: SessionEventKind("turn/completed".into()), kind_version: 1,
            correlation_id: first_operation.clone().into(), causation_event_id: Some(turn_event),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"result":"actual test terminal"}))) } }).await.unwrap();
    let facts = canonical_fact_bytes(&services, session).await;
    for wrong_identity in [&first_key, &first_operation] {
        let (status, _) = request(&router, "PUT", &format!("/api/apps/{id}"), json!({"detect_settings":true,"completion":{"session_id":session,"turn_id":wrong_identity}})).await;
        assert_eq!(status, StatusCode::CONFLICT, "only the exact accepted source message is a notification ID");
        assert!(!directory.join(".nomifun-static-server.cjs").exists());
    }
    let saved = data(&router, "PUT", &format!("/api/apps/{id}"), json!({"detect_settings":true,"completion":{"session_id":session,"turn_id":first}})).await;
    assert_eq!(saved["completion_observation"], json!({"session_id":session,"turn_id":first,"files_changed":true}));
    assert_eq!(saved["development_session_id"], session);
    assert_eq!(saved["name"], "实际元数据");
    assert!(saved["settings"]["start_command"].is_string());
    assert_eq!(saved["state"], "stopped");
    assert!(saved["pid"].is_null());
    assert_eq!(facts, canonical_fact_bytes(&services, session).await, "configuration detection must not author or rewrite canonical history");
    assert_eq!(std::fs::read_to_string(directory.join("index.html")).unwrap(), "SOURCE_PRESERVED");
    let current = data(&router, "GET", &format!("/api/apps/{id}"), Value::Null).await;
    assert!(current.get("completion_observation").is_none(), "completion observations must never be persisted or returned by reads");
    let second_key = uuid::Uuid::now_v7().to_string();
    let second_scoped_key = format!("user:{}:{session}:{second_key}", services.authoritative_user_id);
    let second_operation = format!("turn:{second_scoped_key}");
    let (message, started) = store.start_turn(&session_id, "session_api".into(), second_scoped_key.into(), second_operation.into(),
        StrictJsonValue(json!({"content":"Continue editing"}))).await.unwrap();
    let second = message.ack.unwrap().event_id.as_ref().to_owned();
    assert_ne!(second, second_key);
    store.append_event(&SessionEventAppend { agent_session_id: session_id.clone(), event_id: "config-turn-failed".into(),
        producer_id: "runtime-supervisor".into(), idempotency_key: "config-turn-failed".into(),
        semantic_event: SemanticSessionEventDraft { kind: SessionEventKind("turn/failed".into()), kind_version: 1,
            correlation_id: format!("turn:user:{}:{session}:{second_key}", services.authoritative_user_id).into(), causation_event_id: Some(started.ack.unwrap().event_id),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"error":"test failure"}))) } }).await.unwrap();
    let before = canonical_fact_bytes(&services, session).await;
    for turn in [first, second] {
        let (status, response) = request(&router, "PUT", &format!("/api/apps/{id}"), json!({"detect_settings":true,"completion":{"session_id":session,"turn_id":turn}})).await;
        assert_eq!(status, StatusCode::CONFLICT, "stale or failed completion must be rejected: {response}");
    }
    assert_eq!(data(&router, "GET", &format!("/api/apps/{id}"), Value::Null).await["name"], "实际元数据");
    assert_eq!(before, canonical_fact_bytes(&services, session).await);
    services.cleanup_after_startup_failure(anyhow::anyhow!("completed config fixture completed")).await;
}

#[tokio::test]
async fn read_only_completed_turn_does_not_detect_configuration_or_write_a_static_preset() {
    use nomifun_agent_contracts::{AgentSessionId, SemanticSessionEventDraft, SessionEventAppend,
        SessionEventKind, SessionEventPayloadRef, StrictJsonValue};
    use nomifun_agent_session::AgentSessionStore;
    let temporary = tempfile::tempdir().unwrap();
    let (services, router) = fixture(temporary.path()).await;
    let model = seed_model(&services).await;
    let app = data(&router, "POST", "/api/apps", json!({"name":"现有应用"})).await;
    let id = app["id"].as_str().unwrap();
    let directory = std::path::Path::new(app["directory"].as_str().unwrap());
    std::fs::write(directory.join("index.html"), "EXISTING_APP_SOURCE").unwrap();
    std::fs::write(directory.join("user-data.json"), "EXISTING_BUSINESS_DATA").unwrap();
    let opened = data(&router, "POST", &format!("/api/apps/{id}/development-session"), json!({"model":model})).await;
    let before = data(&router, "GET", &format!("/api/apps/{id}"), Value::Null).await;
    let session = opened["conversation"]["conversation_id"].as_str().unwrap();
    let session_id = AgentSessionId::from(session.to_owned());
    let store = AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    let key = uuid::Uuid::now_v7().to_string();
    let scoped_key = format!("user:{}:{session}:{key}", services.authoritative_user_id);
    let operation = format!("turn:{scoped_key}");
    // Canonical admission fixture for a normal question, without model calls,
    // workspace mutations or fabricated generated-file receipts.
    let (message, started) = store.start_turn(&session_id, "session_api".into(), scoped_key.into(), operation.clone().into(),
        StrictJsonValue(json!({"content":"这个应用有什么作用？请只解释，不修改文件或配置。"}))).await.unwrap();
    let source = message.ack.unwrap().event_id;
    store.append_event(&SessionEventAppend { agent_session_id: session_id.clone(), event_id: "read-only-config-turn-completed".into(),
        producer_id: "runtime-supervisor".into(), idempotency_key: "read-only-config-turn-completed".into(),
        semantic_event: SemanticSessionEventDraft { kind: SessionEventKind("turn/completed".into()), kind_version: 1,
            correlation_id: operation.into(), causation_event_id: Some(started.ack.unwrap().event_id),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"result":"Read-only question completed"}))) } }).await.unwrap();
    assert!(store.list_effects(&session_id).await.unwrap().is_empty());
    let facts = canonical_fact_bytes(&services, session).await;
    let completed = data(&router, "PUT", &format!("/api/apps/{id}"), json!({"detect_settings":true,
        "completion":{"session_id":session,"turn_id":source}})).await;
    assert_eq!(completed["completion_observation"], json!({"session_id":session,"turn_id":source,"files_changed":false}));
    let mut completed_app = completed;
    completed_app.as_object_mut().unwrap().remove("completion_observation");
    assert_eq!(completed_app, before);
    assert!(!directory.join(".nomifun-static-server.cjs").exists());
    assert_eq!(std::fs::read_to_string(directory.join("index.html")).unwrap(), "EXISTING_APP_SOURCE");
    assert_eq!(std::fs::read_to_string(directory.join("user-data.json")).unwrap(), "EXISTING_BUSINESS_DATA");
    assert_eq!(facts, canonical_fact_bytes(&services, session).await);
    for edit in [json!({"name":"must not save"}), json!({"settings":{}})] {
        let mut body = json!({"detect_settings":true,"completion":{"session_id":session,"turn_id":source}});
        body.as_object_mut().unwrap().extend(edit.as_object().unwrap().clone());
        let (status, _) = request(&router, "PUT", &format!("/api/apps/{id}"), body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "completed notification cannot carry an unrelated settings save");
    }
    assert_eq!(data(&router, "GET", &format!("/api/apps/{id}"), Value::Null).await, before);
    assert!(!directory.join(".nomifun-static-server.cjs").exists());
    services.cleanup_after_startup_failure(anyhow::anyhow!("read-only completion fixture completed")).await;
}
