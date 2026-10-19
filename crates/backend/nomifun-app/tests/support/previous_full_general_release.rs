//! Exercise the complete previous General publication through the real HTTP
//! control plane and Runtime. The JSON fixture contains only public bundled
//! publication metadata extracted from the previous release: no user records,
//! model identity or credentials. In particular, its old schemas and roles are
//! not reconstructed by adding an exec Action to today's contracts.

use std::sync::{Arc, Mutex};

use axum::{Router, body::Body, http::{Request, StatusCode}};
use nomifun_agent_contracts::{
    AgentPresetRevision, AgentSessionId, CapabilitySelection, EffectClass,
    PluginSourceKind, PresetRevisionRef, ResolvedSnapshotEnvelope, digest_payload,
};
use nomifun_agent_kernel::{AgentPresetCompiler, CompileRequest};
use nomifun_agent_session::{AgentSessionStore, TurnReceiptStatus};
use nomifun_api_types::{AgentPresetEditorResponse, AgentPresetLibraryResponse, AgentPresetRevisionDto, OfficialPresetKeyDto};
use nomifun_app::{AppConfig, AuthPolicy, compatibility::{AppServices, build_module_states, create_router_with_states}};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{method, path}};

const REPLY: &str = "PREVIOUS_FULL_GENERAL_RELEASE_OK";

async fn response(router: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let response = router.clone().oneshot(Request::builder().method(method).uri(uri)
        .header("x-nomi-local-trust", super::LOCAL_TRUST_SECRET)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn call(router: &Router, method: &str, uri: &str, body: Value) -> Value {
    let (status, value) = response(router, method, uri, body).await;
    assert!(status.is_success(), "{method} {uri}: {status} {value}");
    value["data"].clone()
}

async fn artifact_counts(pool: &nomifun_db::SqlitePool) -> (i64, i64, i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_presets),
        (SELECT COUNT(*) FROM agent_preset_revisions), (SELECT COUNT(*) FROM agent_runtime_snapshots),
        (SELECT COUNT(*) FROM agent_sessions), (SELECT COUNT(*) FROM agent_turns)")
        .fetch_one(pool).await.unwrap()
}

async fn load_artifacts(pool: &nomifun_db::SqlitePool, reference: &PresetRevisionRef)
    -> (AgentPresetRevision, ResolvedSnapshotEnvelope)
{
    let (payload, created_by, created_at, reason): (String, String, i64, Option<String>) = sqlx::query_as(
        "SELECT payload_json,created_by,created_at,reason FROM agent_preset_revisions WHERE revision_id=?")
        .bind(reference.revision_id()).fetch_one(pool).await.unwrap();
    let locks: Vec<String> = sqlx::query_scalar(
        "SELECT lock_json FROM agent_preset_contribution_locks WHERE revision_id=? ORDER BY contribution_id")
        .bind(reference.revision_id()).fetch_all(pool).await.unwrap();
    let snapshot: String = sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots
        WHERE json_extract(content_json,'$.preset_revision_ref.preset_id')=?
        AND json_extract(content_json,'$.preset_revision_ref.revision')=?")
        .bind(reference.preset_id.as_ref()).bind(i64::try_from(reference.revision).unwrap())
        .fetch_one(pool).await.unwrap();
    let mut contribution_locks = locks.iter().map(|lock| serde_json::from_str(lock).unwrap()).collect::<Vec<_>>();
    contribution_locks.sort();
    (AgentPresetRevision { reference: reference.clone(), payload: serde_json::from_str(&payload).unwrap(),
        contribution_locks,
        created_by: created_by.into(), created_at_ms: created_at, reason },
        serde_json::from_str(&snapshot).unwrap())
}

async fn publish_previous(
    pool: &nomifun_db::SqlitePool,
    current: &AgentPresetRevision,
    current_snapshot: &ResolvedSnapshotEnvelope,
    drift: Option<&str>,
) -> (AgentPresetRevision, ResolvedSnapshotEnvelope) {
    let metadata: Value = serde_json::from_str(include_str!("../fixtures/general-before-bounded-launch.json")).unwrap();
    let mut content = serde_json::to_value(&current_snapshot.content).unwrap();
    for (key, value) in metadata.as_object().unwrap() {
        assert!(["schema_version", "resolver_version", "required_runtime_protocol_version", "required_runtime_profile",
            "runtime_feature_inventory_digest", "required_runtime_features", "canonical_schema_manifest_digest",
            "target_contribution_manifest_digest", "enabled_capabilities", "resolved_role_providers",
            "required_resource_kinds", "capability_allowlist"].contains(&key.as_str()),
            "the previous-release fixture must contain only public publication metadata");
        content[key] = value.clone();
    }
    let mut snapshot = current_snapshot.clone();
    snapshot.content = serde_json::from_value(content).unwrap();
    assert_eq!(snapshot.content.enabled_capabilities.len(), 19);
    assert_eq!(snapshot.content.resolved_role_providers.len(), 2);
    assert!(snapshot.content.enabled_capabilities.iter()
        .all(|capability| capability.resolved_source.source_kind == PluginSourceKind::Bundled));
    assert!(snapshot.content.resolved_role_providers.values()
        .all(|provider| provider.source.source_kind == PluginSourceKind::Bundled));
    assert!(snapshot.content.skill_locks.is_empty() && snapshot.content.mcp_tool_locks.is_empty());
    assert_ne!(snapshot.content.canonical_schema_manifest_digest, current_snapshot.content.canonical_schema_manifest_digest,
        "this regression must cover the real previous global Schema manifest");
    let process = snapshot.content.enabled_capabilities.iter_mut()
        .find(|capability| capability.capability.id.as_ref() == "workspace.process").unwrap();
    assert!(process.action_allowlist.contains(&"workspace.process/exec".into()));
    match drift {
        Some("schema") => snapshot.content.canonical_schema_manifest_digest = digest_payload(&"unrecognized schema manifest").unwrap(),
        Some("authority") => process.actions.iter_mut()
            .find(|action| action.action_id.as_ref() == "workspace.process/start").unwrap().effect_class = EffectClass::Destructive,
        None => {},
        Some(other) => panic!("unknown drift {other}"),
    }
    let mut revision = current.clone();
    revision.payload.enabled_capabilities = snapshot.content.enabled_capabilities.iter()
        .map(|capability| CapabilitySelection { capability: capability.capability.clone(), action_allowlist: capability.action_allowlist.clone() })
        .collect();
    revision.contribution_locks = snapshot.content.enabled_capabilities.iter()
        .map(|capability| capability.contribution_lock.clone()).collect();
    revision.contribution_locks.sort();
    let latest: i64 = sqlx::query_scalar("SELECT MAX(revision_no) FROM agent_preset_revisions WHERE preset_id=?")
        .bind(current.reference.preset_id.as_ref()).fetch_one(pool).await.unwrap();
    revision.reference.revision = u64::try_from(latest).unwrap() + 1;
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    snapshot.content.preset_revision_ref = revision.reference.clone();
    snapshot.content.chat_route_identity = revision.chat_route_identity().unwrap();
    snapshot.snapshot_ref.snapshot_id = format!("fixture.previous-general:{}", uuid::Uuid::now_v7()).into();
    snapshot.snapshot_ref.snapshot_digest = digest_payload(&snapshot.content).unwrap();
    revision.validate().unwrap();
    snapshot.validate().unwrap();
    // The canonical model-profile algorithm consumes the frozen old contracts;
    // it must not resolve their schemas against today's Registry.
    snapshot = AgentPresetCompiler::derive_model_snapshot(&revision, &snapshot, CompileRequest {
        revision: revision.clone(), principal: snapshot.actor.clone(), scene: snapshot.scene.clone(),
        surface: snapshot.surface.clone(), audience: snapshot.audience.clone(),
        created_at_ms: snapshot.created_at_ms, resolver_run_id: snapshot.resolver_run_id.clone(),
    }).unwrap();
    let mut tx = pool.begin().await.unwrap();
    nomifun_db::insert_agent_preset_revision_tx(&mut tx, &revision, &snapshot).await.unwrap();
    sqlx::query("UPDATE agent_presets SET current_stable_revision=? WHERE preset_id=?")
        .bind(i64::try_from(revision.reference.revision).unwrap()).bind(revision.reference.preset_id.as_ref())
        .execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    (revision, snapshot)
}

fn process_actions(editor: &AgentPresetEditorResponse) -> &std::collections::BTreeSet<String> {
    &editor.draft.document.enabled_capabilities.iter()
        .find(|capability| capability.capability.id == "workspace.process").unwrap().action_allowlist
}

#[tokio::test]
async fn previous_full_general_release_opens_and_runs() {
    let root = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let mut services = AppServices::from_config(database, &AppConfig {
        data_dir: root.path().join("data"), work_dir: root.path().join("work"),
        auth_policy: AuthPolicy::TrustLocalToken,
        local_trust_secret: Some(Arc::from(super::LOCAL_TRUST_SECRET)), ..AppConfig::default()
    }).await.unwrap();
    services.browser_resources = Some(Arc::new(super::BrowserResourceService::new(Arc::new(super::CatalogGateBrowserFactory))));
    let (states, _channels) = build_module_states(&services).await;
    let router = create_router_with_states(&services, states);
    let upstream = MockServer::start().await;
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = requests.clone();
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            captured.lock().unwrap().push(request.body_json::<Value>().unwrap());
            ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
                .set_body_string(format!("data: {{\"id\":\"full-general\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{REPLY}\"}},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"full-general\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"))
        }).mount(&upstream).await;
    let provider = call(&router, "POST", "/api/providers", json!({
        "platform":"stepfun-plan", "name":"Previous General local fixture", "base_url":format!("{}/v1",upstream.uri()),
        "auth_scheme":"bearer", "credentials":{"api_keys":["isolated-fixture-only"]}, "enabled":true,
        "initial_model":{"model":"previous-full-general", "enabled":true, "capabilities":[{
            "task":"chat", "traits":[], "protocol":"openai.chat_text", "connection_role":"default", "provider_params":{}}]}
    })).await;
    let model = json!({"provider_id":provider["provider_id"], "model":"previous-full-general"});
    let prepared = call(&router, "POST", "/api/agent-presets/from-template/assistant.general", json!({
        "reuse_existing":true, "display_name":"General", "model":model,
        "model_route_refs":{}, "chat_route_records":{}})).await;
    let library: AgentPresetLibraryResponse = super::get_data(&router, "/api/agent-preset-templates?source=official").await;
    let official = &library.official_presets[&OfficialPresetKeyDto::AssistantGeneral];
    let preset_id = &official.preset_id;
    let mut editor: AgentPresetEditorResponse = super::get_data(&router, &format!("/api/agent-presets/{preset_id}/editor")).await;
    editor.draft.document = serde_json::from_value(prepared["draft"]["document"].clone()).unwrap();
    let saved = call(&router, "POST", &format!("/api/agent-presets/{preset_id}/revisions"), json!({
        "expected_current_revision":editor.draft.current_revision, "draft":editor.draft, "reason":"Isolated saved General fixture"})).await;
    let reference: PresetRevisionRef = serde_json::from_value(saved["revision"]["reference"].clone()).unwrap();
    let pool = services.database.pool();
    let (current, current_snapshot) = load_artifacts(pool, &reference).await;
    assert_eq!(current_snapshot.content.enabled_capabilities.len(), 19);
    let (first_previous, first_snapshot) = publish_previous(pool, &current, &current_snapshot, None).await;
    let before = artifact_counts(pool).await;
    let latest: AgentPresetEditorResponse = super::get_data(&router, &format!("/api/agent-presets/{preset_id}/editor")).await;
    assert!(!process_actions(&latest).contains("workspace.process/exec"), "the latest workbench draft must be usable");
    assert!(process_actions(&latest).contains("workspace.process/start"));
    assert_eq!(artifact_counts(pool).await, before, "opening the workbench must not write artifacts");
    let historical: AgentPresetEditorResponse = super::get_data(&router,
        &format!("/api/agent-presets/{preset_id}/editor?revision={}",first_previous.reference.revision)).await;
    assert!(process_actions(&historical).contains("workspace.process/exec"));
    assert_eq!(load_artifacts(pool, &first_previous.reference).await, (first_previous.clone(), first_snapshot));

    for drift in ["schema", "authority"] {
        let (revision, snapshot) = publish_previous(pool, &current, &current_snapshot, Some(drift)).await;
        let before = artifact_counts(pool).await;
        let (status, error) = response(&router, "POST", "/api/agent-presets/from-template/assistant.general", json!({
            "reuse_existing":true, "display_name":"General", "model":model,
            "model_route_refs":{}, "chat_route_records":{}})).await;
        assert_eq!(status, StatusCode::CONFLICT, "{drift}: {error}");
        assert_eq!(error["code"], "AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED", "{drift}: {error}");
        assert_eq!(artifact_counts(pool).await, before, "{drift}: rejected preparation cannot create artifacts, Sessions or Turns");
        assert_eq!(load_artifacts(pool, &revision.reference).await, (revision, snapshot));
        assert!(requests.lock().unwrap().is_empty(), "rejected contracts cannot invoke the model");
    }
    let (previous, previous_snapshot) = publish_previous(pool, &current, &current_snapshot, None).await;
    let prepared = call(&router, "POST", "/api/agent-presets/from-template/assistant.general", json!({
        "reuse_existing":true, "display_name":"General", "model":model,
        "model_route_refs":{}, "chat_route_records":{}})).await;
    let session = call(&router, "POST", "/api/agent-sessions", json!({
        "preset_id":prepared["preset"]["preset_id"], "title":"Previous full General release regression",
        "resource_selections":super::official_session_resources("assistant.general", "", ""), "model":model})).await;
    assert_eq!(session["state"], "ready");
    let session_id = session["agent_session_id"].as_str().unwrap();
    let id = AgentSessionId::from(session_id);
    let store = AgentSessionStore::from_pool(pool.clone()).await.unwrap();
    let opened = store.get_live_session(&id).await.unwrap();
    let (_, frozen) = load_artifacts(pool, &opened.agent_binding.preset_revision_ref).await;
    let expected = nomifun_agent_contracts::bounded_process_launch_payload(&previous.payload);
    let actual: AgentPresetRevisionDto = super::get_data(&router, &format!("/api/agent-presets/{}/revisions/{}",
        opened.agent_binding.preset_revision_ref.preset_id.as_ref(), opened.agent_binding.preset_revision_ref.revision)).await;
    for selection in &expected.enabled_capabilities {
        assert!(actual.document.enabled_capabilities.iter().any(|capability|
            capability.capability.id == selection.capability.id.as_ref()
                && capability.action_allowlist == selection.action_allowlist.iter().map(|action|action.as_ref().to_owned()).collect()),
            "every one of the General's 19 original module grants must survive");
    }
    assert!(frozen.content.enabled_capabilities.iter().flat_map(|capability| &capability.actions)
        .all(|action| action.action_id.as_ref() != "workspace.process/exec"));
    let accepted = call(&router, "POST", &format!("/api/agent-sessions/{session_id}/turns"), json!({
        "idempotency_key":uuid::Uuid::now_v7().to_string(), "input":{"content":"Reply with the fixed fixture confirmation."}})).await;
    let operation = nomifun_agent_contracts::OperationId::from(accepted["operation_id"].as_str().unwrap());
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let history = call(&router, "GET", &format!("/api/agent-sessions/{session_id}/message-history?page_size=100"), Value::Null).await;
            let head = store.head(&id).await.unwrap();
            if head.active_turn_id.is_none() && head.status != "running" {
                let receipt = store.read_turn_receipt(&id, &operation).await.unwrap();
                assert_eq!(receipt.status, TurnReceiptStatus::Completed, "{history}");
                assert!(history.to_string().contains(REPLY), "{history}");
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }).await.expect("the first real Runtime Turn must finish promptly");
    assert_eq!(requests.lock().unwrap().len(), 1);
    let before_repeat = artifact_counts(pool).await;
    let repeated = call(&router, "POST", "/api/agent-presets/from-template/assistant.general", json!({
        "reuse_existing":true, "display_name":"General", "model":model,
        "model_route_refs":{}, "chat_route_records":{}})).await;
    assert_eq!(repeated["preset"]["preset_id"], prepared["preset"]["preset_id"],
        "reopening the same saved General must reuse its proved execution contract");
    assert_eq!(artifact_counts(pool).await, before_repeat,
        "repeated preparation must not accumulate new revisions, Sessions or Turns");
    let reopened: AgentPresetEditorResponse = super::get_data(&router, &format!("/api/agent-presets/{preset_id}/editor")).await;
    assert!(!process_actions(&reopened).contains("workspace.process/exec"));
    assert_eq!(load_artifacts(pool, &previous.reference).await, (previous, previous_snapshot),
        "new preparation, Session creation and execution must retain the old immutable artifacts exactly");
    services.agent_runtime_sessions.terminate_all();
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}
