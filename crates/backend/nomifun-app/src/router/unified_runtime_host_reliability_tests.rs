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
        Self::build_delivery(scenario, vec![], vec![], None, false).await
    }

    async fn with_delivery(scenario: &str, files: Vec<String>, inject_skills: Vec<String>, origin: Option<String>) -> Self {
        Self::build_delivery(scenario, files, inject_skills, origin, true).await
    }

    async fn build_delivery(scenario: &str, files: Vec<String>, inject_skills: Vec<String>, origin: Option<String>, explicit_metadata: bool) -> Self {
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
        let mut delivery = json!({ "content":"cancel before model dispatch", "admission":{
            "route_identity":host.route, "resolved_snapshot_ref":host.snapshot_ref
        }});
        if explicit_metadata {
            delivery["files"] = json!(files);
            delivery["inject_skills"] = json!(inject_skills);
            delivery["origin"] = json!(origin);
        }
        let (input, _) = owner.canonical().store().start_turn(&id.to_owned().into(), "session_api".into(),
            "cleanup-retry-turn".into(), "cleanup-retry-turn".into(), StrictJsonValue(delivery)).await.unwrap();
        let root = input.record.unwrap().event_id;
        let message = SendMessageData { content:"cancel before model dispatch".into(), msg_id:"cleanup-wire".into(),
            source_message_id:Some(root.as_ref().into()), files, inject_skills, origin };
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
async fn admitted_workspace_context_reaches_the_formal_turn_without_a_probe() {
    let fixture=Fixture::new("admitted-workspace-context").await;
    let prepared=fixture.host.prepare_turn(&fixture.message,CancellationToken::new()).await.unwrap();
    let expected=admitted_workspace_context(&fixture.host.options.workspace).unwrap();
    assert_eq!(prepared.model_request.input.instructions.iter().filter(|instruction|*instruction==&expected).count(),1);
    let session=&fixture.host.options.conversation_id;
    let models:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='context/model-visible-applied'")
        .bind(session).fetch_one(fixture.pool()).await.unwrap();
    let effects:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_effects WHERE session_id=?")
        .bind(session).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(models,0,"context preparation must not spend a model request");
    assert_eq!(effects,0,"admitted workspace data must not trigger a cwd or listing command");
    fixture.finish().await;
}

#[tokio::test]
async fn cancellation_receipt_identity_rejects_altered_empty_delivery() {
    cancellation_identity_scenario(false).await;
}

#[tokio::test]
async fn claimed_preparation_cancel_keeps_same_cleanup_owner() {
    claimed_preparation_scenario(true).await;
}

#[tokio::test]
async fn claimed_preparation_drop_keeps_same_cleanup_owner() {
    claimed_preparation_scenario(false).await;
}

#[tokio::test]
async fn claimed_preparation_cannot_confirm_another_execution_owner() {
    let fixture = Fixture::new("claimed-foreign-owner").await;
    let store = fixture.host.session_host.canonical_store().unwrap();
    let foreign = uuid::Uuid::now_v7().to_string();
    let lease = store.claim_native_execution(nomifun_agent_session::NativeExecutionClaim {
        owner:fixture.host.principal.clone(), agent_session_id:fixture.host.options.conversation_id.clone().into(),
        operation_id:"cleanup-retry-turn".into(), snapshot:fixture.host.snapshot_ref.clone(),
        active_set_generation:0, holder:foreign.clone(), expected_fence:0, checkpoint:None,
    }).await.unwrap();
    store.cancel_active_turn(&fixture.host.options.conversation_id.clone().into(),
        "cancel-foreign-execution".into(), "session_api".into()).await.unwrap();
    let before = store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap();
    assert!(fixture.host.cleanup_turn(&fixture.message).await.is_err());
    assert!(fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.is_err());
    assert!(fixture.host.active.lock().await.is_none());
    assert_eq!(store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap(), before);
    let actual: (i64, Option<String>, i64) = sqlx::query_as("SELECT execution_generation,execution_owner,execution_fence FROM agent_turns WHERE session_id=?")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(actual, (lease.generation() as i64, Some(foreign), 0));
    fixture.assert_settled().await;
    fixture.finish().await;
}

async fn claimed_preparation_scenario(canonical_cancel: bool) {
    use std::{future::Future, task::Poll};
    let fixture = Fixture::new(if canonical_cancel {"claimed-canonical-cancel"} else {"claimed-driver-drop"}).await;
    let cancellation = CancellationToken::new();
    let mut preparation = Box::pin(fixture.host.admit_preparation(&fixture.message, cancellation.clone()));
    let claimed: (i64, Option<String>) = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let pending = std::future::poll_fn(|context| Poll::Ready(preparation.as_mut().poll(context).is_pending())).await;
            assert!(pending, "preparation must expose its real database await before caller drop");
            let claim: (i64, Option<String>) = sqlx::query_as("SELECT execution_generation,execution_owner FROM agent_turns WHERE session_id=?")
                .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
            if claim.0 > 0 && claim.1.is_some() { break claim; }
        }
    }).await.expect("native claim must become visible within the preparation budget");
    let active_at_claim = fixture.host.active.lock().await.is_some();
    let store = fixture.host.session_host.canonical_store().unwrap();
    if canonical_cancel {
        store.cancel_active_turn(&fixture.host.options.conversation_id.clone().into(),
            "cancel-during-native-claim".into(), "session_api".into()).await.unwrap();
    }
    cancellation.cancel();
    drop(preparation);
    println!("CLAIMED_PREPARATION canonical_cancel={canonical_cancel} generation={} active_at_claim={active_at_claim}", claimed.0);
    fixture.host.cleanup_turn(&fixture.message).await
        .expect("the original claimed preparation must retain its cleanup owner");
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.unwrap();
    fixture.assert_settled().await;
    let final_claim: (i64, Option<String>, i64) = sqlx::query_as("SELECT execution_generation,execution_owner,execution_fence FROM agent_turns WHERE session_id=?")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(final_claim.0, claimed.0);
    assert_eq!(final_claim.1, claimed.1);
    assert_eq!(final_claim.2, 0, "cleanup must not recover or replace the original execution fence");
    let claim_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/execution-claimed'")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(claim_count, 1);
    let before_repeat = store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap();
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    assert_eq!(store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap(), before_repeat);
    fixture.finish().await;
}

#[tokio::test]
async fn cancellation_receipt_identity_preserves_unopened_attachment_delivery() {
    cancellation_identity_scenario(true).await;
}

async fn cancellation_identity_scenario(attachments: bool) {
    let fixture = if attachments {
        Fixture::with_delivery("cancel-identity-attachments", vec!["cancel-note.txt".into(), "cancel-context.txt".into()],
            vec![], Some("fixture-source".into())).await
    } else {
        Fixture::with_delivery("cancel-identity-empty", vec![], vec![], None).await
    };
    let store = fixture.host.session_host.canonical_store().unwrap();
    let (_, terminal) = store.cancel_active_turn(&fixture.host.options.conversation_id.clone().into(),
        "cancel-delivery-identity".into(), "session_api".into()).await.unwrap();
    let original_terminal = terminal.record.unwrap().event_id;
    let before = store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap();
    let mut altered_files = fixture.message.clone();
    if attachments { altered_files.files.reverse(); }
    else { altered_files.files.push("unaccepted.txt".into()); }
    let mut altered_skills = fixture.message.clone();
    altered_skills.inject_skills.push("unaccepted-skill".into());
    let mut altered_origin = fixture.message.clone();
    altered_origin.origin = Some("altered-origin".into());
    let mut missing_origin = fixture.message.clone();
    missing_origin.origin = None;
    let mut failures = Vec::new();
    for (field, candidate) in [("files", altered_files), ("inject_skills", altered_skills), ("origin", altered_origin)] {
        let cleanup_rejected = fixture.host.cleanup_turn(&candidate).await.is_err();
        let terminal_rejected = fixture.host.record_event(&candidate, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.is_err();
        if !cleanup_rejected || !terminal_rejected { failures.push((field, cleanup_rejected, terminal_rejected)); }
    }
    if attachments && fixture.host.cleanup_turn(&missing_origin).await.is_ok() {
        failures.push(("missing_origin", false, false));
    }
    assert_eq!(store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap(), before);
    assert!(fixture.host.active.lock().await.is_none());
    // An accepted cancelled delivery closes through the actual SDK without
    // reading its attachment references or dispatching a model or tool.
    fixture.runtime.send_message(fixture.message.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(8), fixture.runtime.cancel()).await.unwrap().unwrap();
    let receipt = store.read_turn_receipt(&fixture.host.options.conversation_id.clone().into(),
        &"cleanup-retry-turn".into()).await.unwrap();
    assert_eq!(receipt.terminal_event.unwrap().event_id, original_terminal);
    assert_eq!(store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap(), before);
    let lease: (i64, Option<String>) = sqlx::query_as("SELECT execution_generation,execution_owner FROM agent_turns WHERE session_id=?")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(lease, (0, None));
    fixture.assert_settled().await;
    println!("CANCEL_DELIVERY_IDENTITY attachments={attachments} unaccepted_fields={failures:?}");
    fixture.finish().await;
    assert!(failures.is_empty(), "cancel acknowledgement accepted altered delivery fields: {failures:?}");
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

#[tokio::test]
async fn steering_cleanup_retry_retains_tail_after_private_record_failure() {
    steering_cleanup_scenario(SteeringCleanupFault::PrivateRecord).await;
}

#[tokio::test]
async fn steering_cleanup_retry_retains_tail_after_public_projection_failure() {
    steering_cleanup_scenario(SteeringCleanupFault::PublicProjection).await;
}

#[tokio::test]
async fn steering_cleanup_retry_retains_deferred_receipt_until_ack() {
    steering_cleanup_scenario(SteeringCleanupFault::DeferredReceipt).await;
}

#[derive(Clone, Copy)]
enum SteeringCleanupFault { PrivateRecord, PublicProjection, DeferredReceipt }

#[tokio::test]
async fn steering_cleanup_retry_acknowledges_exact_cancelled_root_before_first_poll() {
    let fixture = Fixture::new("cancelled-before-first-poll").await;
    let store = fixture.host.session_host.canonical_store().unwrap();
    let (_, cancelled) = store.cancel_active_turn(&fixture.host.options.conversation_id.clone().into(),
        "cancel-before-first-poll".into(), "session_api".into()).await.unwrap();
    let cancelled_id = cancelled.record.unwrap().event_id;
    let before = store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap();
    assert_eq!(fixture.state().await, "cancelled");
    assert!(fixture.host.active.lock().await.is_none());
    let mut wrong_root = fixture.message.clone();
    wrong_root.source_message_id = Some(uuid::Uuid::now_v7().to_string());
    assert!(fixture.host.cleanup_turn(&wrong_root).await.is_err());
    assert!(fixture.host.record_event(&wrong_root, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.is_err());
    let mut wrong_text = fixture.message.clone();
    wrong_text.content = "different accepted body".into();
    assert!(fixture.host.cleanup_turn(&wrong_text).await.is_err());
    assert!(fixture.host.record_event(&wrong_text, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.is_err());
    assert!(fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:1}).await.is_err());
    assert!(fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCompleted {
        model_steps:0, finish_reason:ChatFinishReason::Completed,
    }).await.is_err());
    let mut foreign_owner = fixture.host.options.clone();
    foreign_owner.user_id = uuid::Uuid::now_v7().to_string();
    assert!(fixture.host.session_host.confirm_cancelled_before_execution(&foreign_owner,
        &fixture.host.binding, &fixture.host.snapshot_ref, &fixture.message).await.is_err());
    let mut foreign_session = fixture.host.options.clone();
    foreign_session.conversation_id = uuid::Uuid::now_v7().to_string();
    assert!(fixture.host.session_host.confirm_cancelled_before_execution(&foreign_session,
        &fixture.host.binding, &fixture.host.snapshot_ref, &fixture.message).await.is_err());
    let mut foreign_snapshot = fixture.host.snapshot_ref.clone();
    foreign_snapshot.snapshot_digest = "0".repeat(64).into();
    assert!(fixture.host.session_host.confirm_cancelled_before_execution(&fixture.host.options,
        &fixture.host.binding, &foreign_snapshot, &fixture.message).await.is_err());
    let mut foreign_binding = fixture.host.binding.clone();
    foreign_binding.build_digest = "0".repeat(64);
    assert!(fixture.host.session_host.confirm_cancelled_before_execution(&fixture.host.options,
        &foreign_binding, &fixture.host.snapshot_ref, &fixture.message).await.is_err());
    // On this current-thread runtime send returns after spawning; no yield
    // occurs before cancel sets the token, so the SDK's biased cancel wins
    // before the first driver poll. The canonical cancel is already durable.
    fixture.runtime.send_message(fixture.message.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(8), fixture.runtime.cancel()).await
        .expect("cancel before first driver poll must remain bounded")
        .expect("a durable cancel before first driver poll must not strand teardown");
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.unwrap();
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    // The special witness must not turn later altered messages into a match.
    assert!(fixture.host.cleanup_turn(&wrong_text).await.is_err());
    assert!(fixture.host.record_event(&wrong_root, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.is_err());
    let receipt = store.read_turn_receipt(&fixture.host.options.conversation_id.clone().into(),
        &"cleanup-retry-turn".into()).await.unwrap();
    assert_eq!(receipt.terminal_event.unwrap().event_id, cancelled_id);
    assert_eq!(store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap(), before);
    let lease: (i64, Option<String>) = sqlx::query_as("SELECT execution_generation,execution_owner FROM agent_turns WHERE session_id=?")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(lease, (0, None));
    fixture.assert_settled().await;
    fixture.finish().await;
}

#[tokio::test]
async fn steering_cleanup_retry_cannot_acknowledge_completed_root_as_cancelled() {
    let fixture = Fixture::new("completed-before-first-poll").await;
    let store = fixture.host.session_host.canonical_store().unwrap();
    let started = store.read_turn_receipt(&fixture.host.options.conversation_id.clone().into(),
        &"cleanup-retry-turn".into()).await.unwrap().started_event.unwrap();
    let completion_id: EventId = "fixture-completed-before-poll".into();
    store.append_turn_terminal(&SessionEventAppend {
        agent_session_id: fixture.host.options.conversation_id.clone().into(), event_id: completion_id,
        producer_id:"session_api".into(), idempotency_key:"fixture-completed-before-poll".into(),
        runtime_binding_id:None, runtime_producer_seq:None,
        semantic_event:SemanticSessionEventDraft { kind:SessionEventKind("turn/completed".into()), kind_version:1,
            correlation_id:"cleanup-retry-turn".into(), causation_event_id:Some(started.event_id),
            payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"model_steps":0,"finish_reason":ChatFinishReason::Completed}))),
        },
    }, &"cleanup-retry-turn".into()).await.unwrap();
    let before = store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap();
    assert!(fixture.host.cleanup_turn(&fixture.message).await.is_err());
    assert!(fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.is_err());
    assert_eq!(fixture.state().await, "completed");
    assert_eq!(store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap(), before);
    assert!(!fixture.host.terminal_already_recorded(fixture.host.root(&fixture.message)).unwrap());
    fixture.finish().await;
}

async fn steering_cleanup_scenario(fault: SteeringCleanupFault) {
    let scenario = match fault {
        SteeringCleanupFault::PrivateRecord => "steering-private-record",
        SteeringCleanupFault::PublicProjection => "steering-public-projection",
        SteeringCleanupFault::DeferredReceipt => "steering-deferred-receipt",
    };
    let fixture = Fixture::new(scenario).await;
    fixture.host.admit_preparation(&fixture.message, CancellationToken::new()).await.unwrap();
    let (operation, generation) = {
        let active = fixture.host.active.lock().await;
        let turn = active.as_ref().unwrap();
        (turn.operation.clone(), turn.epoch as u64)
    };
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnStarted {
        binding: fixture.host.engine_binding.clone(), turn_operation_id: operation.clone().into(),
    }).await.unwrap();
    let store = fixture.host.session_host.canonical_store().unwrap();
    let queued_text = "Keep the correction queued.\n中文纠正：保留原指令";
    let (_, accepted) = store.steer_active_turn(&fixture.host.options.conversation_id.clone().into(),
        "pending-steering".into(), "session_api".into(), StrictJsonValue(json!({"content":queued_text}))).await.unwrap();
    let receipt = accepted.record.unwrap().event_id.as_ref().to_owned();
    let delivery = nomifun_ai_agent::RuntimeSteerDelivery {
        receipt_operation_id: receipt.clone(), wire_turn_id: fixture.message.msg_id.clone(),
        turn_generation: generation, text: queued_text.into(), files: vec![], inject_skills: vec![],
    };
    assert!(fixture.host.queue_steer(delivery.clone()).await.unwrap());
    assert!(fixture.host.queue_steer(delivery.clone()).await.unwrap());
    let tail = "Partial response before cancellation.\n中文尾部：只写一次";
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::OutputTextDelta {
        step: 1, text: tail.into(),
    }).await.unwrap();
    assert_eq!(fixture.host.active.lock().await.as_ref().unwrap().journal.sequence(), 2);
    let predicate = match fault {
        SteeringCleanupFault::PrivateRecord => "NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='output_text_delta'",
        SteeringCleanupFault::PublicProjection => "NEW.kind='message/content-part'",
        SteeringCleanupFault::DeferredReceipt => "NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='steering_deferred'",
    };
    sqlx::query(&format!("CREATE TRIGGER reject_steering_tail BEFORE INSERT ON agent_events WHEN {predicate} BEGIN SELECT RAISE(FAIL,'fixture steering tail write failure'); END"))
        .execute(fixture.pool()).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_steering_cleanup_pause BEFORE INSERT ON agent_events WHEN NEW.kind='turn/paused' BEGIN SELECT RAISE(FAIL,'fixture steering pause write failure'); END")
        .execute(fixture.pool()).await.unwrap();
    let failure = fixture.host.cleanup_turn(&fixture.message).await.unwrap_err();
    assert!(failure.to_string().contains("fixture steering tail write failure"));
    assert!(!fixture.host.queue_steer(delivery).await.unwrap(), "cleanup must keep the inbox closed");
    let pause_failure = fixture.host.suspend_after_cleanup_failure(&fixture.message).await.unwrap_err();
    assert!(pause_failure.to_string().contains("fixture steering pause write failure"));
    assert_eq!(fixture.state().await, "running");
    {
        let active = fixture.host.active.lock().await;
        let turn = active.as_ref().unwrap();
        assert!(turn.cleanup_started);
        assert!(!turn.cleanup_proven);
        assert!(!turn.steering.permits_resource_dispatch());
        assert_eq!(turn.steering.pending_receipt_ids(), vec![receipt.clone()]);
    }
    let public_parts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='message/content-part'")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(public_parts, i64::from(matches!(fault, SteeringCleanupFault::DeferredReceipt)));
    let private_parts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='output_text_delta'")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(private_parts, i64::from(!matches!(fault, SteeringCleanupFault::PrivateRecord)));
    sqlx::query("DROP TRIGGER reject_steering_tail").execute(fixture.pool()).await.unwrap();
    sqlx::query("DROP TRIGGER reject_steering_cleanup_pause").execute(fixture.pool()).await.unwrap();
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    {
        let active = fixture.host.active.lock().await;
        let turn = active.as_ref().unwrap();
        assert!(turn.cleanup_proven);
        assert!(!turn.steering.permits_resource_dispatch());
        assert!(turn.steering.pending_receipt_ids().is_empty());
    }
    let parts: Vec<String> = sqlx::query_scalar("SELECT json_extract(inline_json,'$.content') FROM agent_events WHERE session_id=? AND kind='message/content-part' ORDER BY seq")
        .bind(&fixture.host.options.conversation_id).fetch_all(fixture.pool()).await.unwrap();
    assert_eq!(parts, vec![tail]);
    let private: Vec<String> = sqlx::query_scalar("SELECT json_extract(inline_json,'$.event.text') FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='output_text_delta'")
        .bind(&fixture.host.options.conversation_id).fetch_all(fixture.pool()).await.unwrap();
    assert_eq!(private, vec![tail]);
    let deferred: Vec<String> = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='steering_deferred'")
        .bind(&fixture.host.options.conversation_id).fetch_all(fixture.pool()).await.unwrap();
    assert_eq!(deferred.len(), 1);
    let deferred: Value = serde_json::from_str(&deferred[0]).unwrap();
    let inputs = deferred["event"]["inputs"].as_array().unwrap();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0]["receipt_operation_id"], receipt);
    assert_eq!(inputs[0]["message_id"], inputs[0]["receipt_operation_id"]);
    assert_eq!(inputs[0]["text"], queued_text);
    let recorded_input: AgentSteeringInput = serde_json::from_value(inputs[0].clone()).unwrap();
    assert!(recorded_input.files.is_empty());
    assert!(recorded_input.inject_skills.is_empty());
    let cleanup_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='host_cleanup_proven'")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(cleanup_count, 1);
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.unwrap();
    fixture.assert_settled().await;
    let before_repeat = store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap();
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    assert_eq!(store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap(), before_repeat);
    fixture.finish().await;
}
