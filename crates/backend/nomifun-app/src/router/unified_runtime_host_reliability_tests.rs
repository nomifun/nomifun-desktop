//! Cleanup faults on the host created by the installed official factory.
use super::*;
use axum::{body::Body, http::Request};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, Weak};
use std::time::Duration;
use tower::ServiceExt;

type HostMap = BTreeMap<String, Weak<ConversationRuntimeHost>>;
static HOSTS: OnceLock<Mutex<HostMap>> = OnceLock::new();

pub(super) fn capture_host(host: &Arc<ConversationRuntimeHost>) {
    let mut hosts = HOSTS.get_or_init(Default::default).lock().unwrap();
    hosts.retain(|_, host| host.strong_count() > 0);
    hosts.insert(host.options.conversation_id.clone(), Arc::downgrade(host));
}

async fn api(router: &axum::Router, path: &str, body: Value) -> Value {
    let response = router.clone().oneshot(Request::builder().method("POST").uri(path)
        .header("x-nomi-local-trust", "cleanup-fixture-local-trust")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(status.is_success(), "{path}: {status}: {value}");
    value["data"].clone()
}

struct Fixture {
    _directory: tempfile::TempDir,
    _router: axum::Router,
    runtime: nomifun_ai_agent::AgentRuntimeHandle,
    services: crate::services::AppServices,
    host: Arc<ConversationRuntimeHost>,
    message: SendMessageData,
    database_path: PathBuf,
}

impl Fixture {
    async fn new(scenario: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = std::env::var_os("NOMIFUN_RELIABILITY_EVIDENCE_DIR")
            .map(|root| PathBuf::from(root).join(scenario))
            .unwrap_or_else(|| directory.path().to_path_buf());
        let config = crate::AppConfig {
            data_dir: root.join("data"), work_dir: root.join("work"),
            auth_policy: nomifun_auth::AuthPolicy::TrustLocalToken,
            local_trust_secret: Some("cleanup-fixture-local-trust".into()),
            ..Default::default()
        };
        std::fs::create_dir_all(&config.data_dir).unwrap();
        let database_path = config.database_path();
        let database = nomifun_db::init_database(&database_path).await.unwrap();
        let services = crate::services::AppServices::from_config(database, &config).await.unwrap();
        let (states, _components) = super::super::state::try_build_module_states(&services).await.unwrap();
        let owner = states.nomi_core_agent_api.session_owner.clone();
        let router = super::super::create_router_with_states(&services, states);
        let provider = api(&router, "/api/providers", json!({
            "platform":"custom", "name":"cleanup fixture", "base_url":"http://127.0.0.1:9/v1",
            "auth_scheme":"bearer", "credentials":{"api_keys":["local-fixture-not-a-secret"]}, "enabled":true,
            "initial_model":{"model":"cleanup-fixture", "enabled":true, "capabilities":[{
                "task":"chat", "traits":[], "protocol":"openai.chat_text", "connection_role":"default", "output_limit":4096
            }]}
        })).await;
        let model = json!({"provider_id":provider["provider_id"], "model":"cleanup-fixture"});
        let editor = api(&router, "/api/agent-presets/from-template/chat.minimal", json!({
            "reuse_existing":false, "display_name":"Cleanup fixture", "model":model
        })).await;
        let preset = editor["preset"]["preset_id"].as_str().unwrap();
        let mut draft = editor["draft"].clone();
        draft["document"]["enabled_capabilities"] = json!([{
            "capability":{"id":"workspace.files"}, "action_allowlist":["workspace.files/read"]
        }]);
        api(&router, &format!("/api/agent-presets/{preset}/revisions"), json!({
            "expected_current_revision":draft["current_revision"], "draft":draft,
            "reason":"isolated cleanup fault regression"
        })).await;
        let session = api(&router, "/api/agent-sessions", json!({
            "preset_id":preset, "model":model,
            "resource_selections":[{"resource_kind":"workspace", "resource_id":"default-workspace"}]
        })).await;
        let id = session["agent_session_id"].as_str().unwrap();
        let projection = owner.get_session(services.authoritative_user_id.as_ref(), id).await.unwrap();
        let workspace = projection.extra["workspace"].as_str().unwrap().to_owned();
        let options = AgentRuntimeBuildOptions {
            user_id: services.authoritative_user_id.to_string(), agent_type: projection.r#type,
            workspace: workspace.clone(), model: projection.model, conversation_id: id.into(),
            delegation_policy: projection.delegation_policy, extra: projection.extra,
            conversation_created_at: Some(projection.created_at), device_mcp_servers: vec![],
            workspace_binding_lease: Some(nomifun_knowledge::WorkspaceBindingLease::acquire_unbound(
                std::path::Path::new(&workspace), id.to_owned()).unwrap()),
        };
        let runtime = services.official_runtime.factory()(options).await.unwrap();
        let host = HOSTS.get().unwrap().lock().unwrap().remove(id).unwrap().upgrade().unwrap();
        // Accept the durable root without starting a model task. This is the
        // cancellation-before-first-driver-poll boundary, not a UI scenario.
        let (input, _) = owner.canonical().store().start_turn(&id.to_owned().into(), "session_api".into(),
            "cleanup-retry-turn".into(), "cleanup-retry-turn".into(), StrictJsonValue(json!({
                "content":"cancel before model dispatch", "admission":{
                    "route_identity":host.route, "resolved_snapshot_ref":host.snapshot_ref
                }
            }))).await.unwrap();
        let root = input.record.unwrap().event_id;
        let message = SendMessageData { content:"cancel before model dispatch".into(), msg_id:"cleanup-wire".into(),
            source_message_id:Some(root.as_ref().into()), files:vec![], inject_skills:vec![], origin:None };
        println!("CLEANUP_FIXTURE scenario={scenario} session={id} operation=cleanup-retry-turn database={}", database_path.display());
        Self { _directory:directory, _router:router, runtime, services, host, message, database_path }
    }

    fn pool(&self) -> &nomifun_db::SqlitePool { self.services.database.pool() }

    async fn state(&self) -> String {
        sqlx::query_scalar("SELECT state FROM agent_turns WHERE session_id=?")
            .bind(&self.host.options.conversation_id).fetch_one(self.pool()).await.unwrap()
    }

    async fn finish(self) {
        self.runtime.kill_and_wait(None).await.unwrap();
        self.services.shutdown_nomi_core_host().await.unwrap();
    }

    async fn assert_settled(&self) {
        assert_eq!(self.state().await, "cancelled");
        let events: Vec<String> = sqlx::query_scalar("SELECT kind FROM agent_events WHERE session_id=? AND kind IN ('turn/cancelled','turn/completed','turn/failed') ORDER BY seq")
            .bind(&self.host.options.conversation_id).fetch_all(self.pool()).await.unwrap();
        assert_eq!(events, vec!["turn/cancelled"]);
        let effects: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_effects WHERE session_id=?")
            .bind(&self.host.options.conversation_id).fetch_one(self.pool()).await.unwrap();
        assert_eq!(effects, 0, "cleanup must not dispatch a model or effect");
        let models: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='context/model-visible-applied'")
            .bind(&self.host.options.conversation_id).fetch_one(self.pool()).await.unwrap();
        assert_eq!(models, 0, "cleanup must not admit a new model attempt");
        let head = self.host.session_host.canonical_store().unwrap()
            .head(&self.host.options.conversation_id.clone().into()).await.unwrap();
        assert_eq!(head.status, "ready");
        assert!(head.active_turn_id.is_none());
    }
}

#[tokio::test]
async fn cleanup_retry_keeps_half_committed_initialization_after_suspension_fault() {
    let fixture = Fixture::new("initialization").await;
    fixture.host.admit_preparation(&fixture.message, CancellationToken::new()).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_input_scope BEFORE INSERT ON agent_events WHEN NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='turn_input_scope' BEGIN SELECT RAISE(FAIL,'fixture input scope write failure'); END")
        .execute(fixture.pool()).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_cleanup_pause BEFORE INSERT ON agent_events WHEN NEW.kind='turn/paused' BEGIN SELECT RAISE(FAIL,'fixture pause write failure'); END")
        .execute(fixture.pool()).await.unwrap();
    let failure = fixture.host.cleanup_turn(&fixture.message).await.unwrap_err();
    assert!(failure.to_string().contains("fixture input scope write failure"));
    assert_eq!(fixture.host.active.lock().await.as_ref().unwrap().journal.sequence(), 1);
    assert!(fixture.host.suspend_after_cleanup_failure(&fixture.message).await.is_err());
    assert_eq!(fixture.state().await, "running");
    sqlx::query("DROP TRIGGER reject_input_scope").execute(fixture.pool()).await.unwrap();
    sqlx::query("DROP TRIGGER reject_cleanup_pause").execute(fixture.pool()).await.unwrap();
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.unwrap();
    fixture.assert_settled().await;
    let events: Vec<String> = sqlx::query_scalar("SELECT json_extract(inline_json,'$.event.event') FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' ORDER BY seq")
        .bind(&fixture.host.options.conversation_id).fetch_all(fixture.pool()).await.unwrap();
    assert_eq!(events, vec!["turn_started", "turn_input_scope", "host_cleanup_proven", "turn_cancelled"]);
    fixture.finish().await;
}

#[tokio::test]
async fn cleanup_retry_rejects_unclaimed_turn_under_writer_lock_then_settles() {
    let fixture = Fixture::new("unclaimed-writer-lock").await;
    assert!(fixture.host.active.lock().await.is_none());
    let writer_pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(&fixture.database_path)
            .busy_timeout(Duration::from_secs(2))).await.unwrap();
    let writer = writer_pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(8), fixture.host.cleanup_turn(&fixture.message)).await
        .expect("claim failure must be bounded");
    assert_eq!(fixture.state().await, "running");
    assert!(fixture.host.active.lock().await.is_none());
    assert!(result.is_err(), "cleanup must retain the canonical admission/claim error");
    assert!(result.unwrap_err().to_string().contains("locked"));
    assert!(fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.is_err());
    assert!(!fixture.host.terminal_already_recorded(fixture.host.root(&fixture.message)).unwrap());
    writer.rollback().await.unwrap();
    writer_pool.close().await;
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.unwrap();
    fixture.assert_settled().await;
    fixture.finish().await;
}

#[tokio::test]
async fn cleanup_retry_rejects_terminal_ack_without_admitted_turn() {
    let fixture = Fixture::new("unadmitted-terminal").await;
    assert!(fixture.host.active.lock().await.is_none());
    let result = fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await;
    assert_eq!(fixture.state().await, "running");
    assert!(result.is_err(), "missing host admission cannot acknowledge a canonical terminal");
    assert!(!fixture.host.terminal_already_recorded(fixture.host.root(&fixture.message)).unwrap());
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.unwrap();
    fixture.assert_settled().await;
    fixture.finish().await;
}
