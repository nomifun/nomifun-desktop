//! E2E integration tests with mock Agent runtimes.
//!
//! Tests the message flow, confirmation system, and auxiliary routes
//! with a mock AgentRuntimeRegistry that provides in-memory agents.

mod common;

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tower::ServiceExt;

use async_trait::async_trait;
use nomifun_ai_agent::runtime_handle::{AgentRuntimeHandle, AgentRuntimeControl, MockAgentRuntime};
use nomifun_ai_agent::protocol::events::TextEventData;
use nomifun_ai_agent::types::{AgentRuntimeBuildOptions, SendMessageData};
use nomifun_ai_agent::{AgentStreamEvent, AgentRuntimeRegistry};
use nomifun_common::{AgentKillReason, AgentType, AppError, Confirmation, ConversationStatus, TimestampMs, now_ms};

use common::{body_json, get_with_token, json_with_token, setup_and_login};

const MESSAGE_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678901";
const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8abc-012345679987";
const MODEL: &str = "mock-agent-model";

// ── Mock Agent ──────────────────────────────────────────────────

struct MockAgent {
    conversation_id: String,
    workspace: String,
    event_tx: broadcast::Sender<AgentStreamEvent>,
    confirmations: Mutex<Vec<Confirmation>>,
    approvals: Mutex<std::collections::HashMap<String, bool>>,
    last_activity: AtomicI64,
}

impl MockAgent {
    fn new(conversation_id: &str, workspace: &str) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            conversation_id: conversation_id.to_owned(),
            workspace: workspace.to_owned(),
            event_tx,
            confirmations: Mutex::new(vec![]),
            approvals: Mutex::new(std::collections::HashMap::new()),
            last_activity: AtomicI64::new(now_ms()),
        }
    }
}

#[async_trait]
impl AgentRuntimeControl for MockAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Nomi
    }

    fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    fn workspace(&self) -> &str {
        &self.workspace
    }

    fn status(&self) -> Option<ConversationStatus> {
        Some(ConversationStatus::Running)
    }

    fn is_transport_healthy(&self) -> bool {
        true
    }

    fn last_activity_at(&self) -> TimestampMs {
        self.last_activity.load(Ordering::Relaxed)
    }

    fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent> {
        self.event_tx.subscribe()
    }

    async fn send_message(&self, _data: SendMessageData) -> Result<(), nomifun_ai_agent::AgentSendError> {
        self.last_activity.store(now_ms(), Ordering::Relaxed);
        // Emit a text event and finish
        let _ = self.event_tx.send(AgentStreamEvent::Text(TextEventData {
            content: "Mock response".into(),
        }));
        let _ = self.event_tx.send(AgentStreamEvent::Finish(
            nomifun_ai_agent::protocol::events::FinishEventData::default(),
        ));
        Ok(())
    }

    async fn cancel(&self) -> Result<(), AppError> {
        Ok(())
    }

    fn kill(&self, _reason: Option<AgentKillReason>) -> Result<(), AppError> {
        Ok(())
    }
}

#[async_trait]
impl MockAgentRuntime for MockAgent {
    fn get_confirmations(&self) -> Vec<Confirmation> {
        self.confirmations.lock().unwrap().clone()
    }

    fn check_approval(&self, action: &str, _command_type: Option<&str>) -> bool {
        self.approvals.lock().unwrap().get(action).copied().unwrap_or(false)
    }

    fn confirm(&self, _msg_id: &str, call_id: &str, _data: Value, always_allow: bool) -> Result<(), AppError> {
        let mut confs = self.confirmations.lock().unwrap();
        confs.retain(|c| c.call_id != call_id);
        if always_allow {
            self.approvals.lock().unwrap().insert("test_action".to_owned(), true);
        }
        Ok(())
    }
}

// ── Mock Agent Runtime Registry ────────────────────────────────────

struct MockAgentRuntimeRegistry {
    agents: Mutex<std::collections::HashMap<String, AgentRuntimeHandle>>,
}

impl MockAgentRuntimeRegistry {
    fn new() -> Self {
        Self {
            agents: Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn insert(&self, conv_id: &str, workspace: &str) -> Arc<MockAgent> {
        let agent = Arc::new(MockAgent::new(conv_id, workspace));
        self.agents
            .lock()
            .unwrap()
            .insert(conv_id.to_owned(), AgentRuntimeHandle::Mock(agent.clone()));
        agent
    }
}

#[async_trait::async_trait]
impl AgentRuntimeRegistry for MockAgentRuntimeRegistry {
    fn get_runtime(&self, conversation_id: &str) -> Option<AgentRuntimeHandle> {
        self.agents.lock().unwrap().get(conversation_id).cloned()
    }

    async fn get_or_create_runtime(
        &self,
        conversation_id: &str,
        options: AgentRuntimeBuildOptions,
    ) -> Result<AgentRuntimeHandle, AppError> {
        let mut agents = self.agents.lock().unwrap();
        if let Some(existing) = agents.get(conversation_id) {
            return Ok(existing.clone());
        }
        let instance =
            AgentRuntimeHandle::Mock(Arc::new(MockAgent::new(conversation_id, &options.workspace)));
        agents.insert(conversation_id.to_owned(), instance.clone());
        Ok(instance)
    }

    fn terminate(&self, conversation_id: &str, _reason: Option<AgentKillReason>) -> Result<(), AppError> {
        self.agents.lock().unwrap().remove(conversation_id);
        Ok(())
    }

    fn terminate_and_wait_result(
        &self,
        conversation_id: &str,
        reason: Option<AgentKillReason>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send>> {
        let result = self.terminate(conversation_id, reason);
        Box::pin(std::future::ready(result))
    }

    fn terminate_all(&self) {
        self.agents.lock().unwrap().clear();
    }

    fn active_runtime_count(&self) -> usize {
        self.agents.lock().unwrap().len()
    }
}

// ── Test App builder with mock agents ───────────────────────────

async fn build_app_with_mock_runtime_registry() -> (axum::Router, nomifun_app::AppServices, Arc<MockAgentRuntimeRegistry>) {
    let db = nomifun_db::init_database_memory().await.unwrap();
    let services = nomifun_app::AppServices::from_config(db, &nomifun_app::AppConfig::default())
        .await
        .unwrap();
    seed_mock_provider(&services).await;

    let runtime_registry = Arc::new(MockAgentRuntimeRegistry::new());
    let services = services.with_agent_runtime_registry(runtime_registry.clone());

    let router = nomifun_app::create_router(&services).await;
    (router, services, runtime_registry)
}

/// Nomi is the only engine, and its runtime options are refused without a
/// canonical provider/model pair — even when the registry that consumes them is
/// a mock. Every fixture conversation below pins this pair.
async fn seed_mock_provider(services: &nomifun_app::AppServices) {
    let credentials_encrypted =
        nomifun_common::encrypt_string(r#"{"api_keys":["test-only"]}"#, &[0x42; 32]).unwrap();
    nomifun_db::sqlx::query(
        "INSERT OR IGNORE INTO providers (\
            provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, enabled, \
            created_at, updated_at\
         ) VALUES (?, 'openai', 'mock-agent-fixture', 'https://example.invalid', 'bearer', ?, 1, 1, 1)",
    )
    .bind(PROVIDER_ID)
    .bind(&credentials_encrypted)
    .execute(services.database.pool())
    .await
    .unwrap();
    common::seed_openai_chat_model(services.database.pool(), PROVIDER_ID, MODEL).await;
}

async fn create_conversation(app: &mut axum::Router, token: &str, csrf: &str, name: &str) -> String {
    let body = json!({
        "type": "nomi",
        "name": name,
        "model": {
            "provider_id": PROVIDER_ID,
            "model": MODEL,
            "use_model": MODEL,
        },
        "extra": {}
    });
    let req = common::json_with_token("POST", "/api/conversations", body, token, csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let json = common::body_json(resp).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "mock-runtime conversation creation failed: {json}"
    );
    json["data"]["conversation_id"]
        .as_str()
        .expect("created conversation response must expose conversation_id")
        .to_owned()
}

// ── Message flow with mock agent ────────────────────────────────

#[tokio::test]
async fn send_message_with_mock_agent_returns_202() {
    let (mut app, services, _runtime_registry) = build_app_with_mock_runtime_registry().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "Pass123!").await;
    let conv_id = create_conversation(&mut app, &token, &csrf, "Mock Agent Test").await;

    let req = json_with_token(
        "POST",
        &format!("/api/conversations/{conv_id}/messages"),
        json!({ "content": "Hello mock agent" }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let json = body_json(resp).await;
    assert_eq!(status, StatusCode::ACCEPTED, "send message failed: {json}");
    assert_eq!(
        json["success"],
        true,
        "send message failed: {json}"
    );
}

#[tokio::test]
async fn stop_stream_with_mock_agent() {
    let (mut app, services, runtime_registry) = build_app_with_mock_runtime_registry().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "Pass123!").await;
    let conv_id = create_conversation(&mut app, &token, &csrf, "Stop Test").await;
    runtime_registry.insert(&conv_id, "/mock-workspace");

    let req = json_with_token(
        "POST",
        &format!("/api/conversations/{conv_id}/cancel"),
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let json = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "cancel failed: {json}");
    assert_eq!(
        json["success"],
        true,
        "cancel failed: {json}"
    );
}

#[tokio::test]
async fn warmup_with_mock_agent() {
    let (mut app, services, _runtime_registry) = build_app_with_mock_runtime_registry().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "Pass123!").await;
    let conv_id = create_conversation(&mut app, &token, &csrf, "Warmup Test").await;

    let req = json_with_token(
        "POST",
        &format!("/api/conversations/{conv_id}/warmup"),
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let json = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "warmup failed: {json}");
}

// ── Confirmation system with mock agent ─────────────────────────

#[tokio::test]
async fn list_confirmations_empty() {
    let (mut app, services, runtime_registry) = build_app_with_mock_runtime_registry().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "Pass123!").await;
    let conv_id = create_conversation(&mut app, &token, &csrf, "Confirm Test").await;
    runtime_registry.insert(&conv_id, "/mock-workspace");

    let req = get_with_token(&format!("/api/conversations/{conv_id}/confirmations"), &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert!(json["data"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn confirm_and_check_approval() {
    let (mut app, services, runtime_registry) = build_app_with_mock_runtime_registry().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "Pass123!").await;
    let conv_id = create_conversation(&mut app, &token, &csrf, "Approval Test").await;
    let agent = runtime_registry.insert(&conv_id, "/mock-workspace");

    // Pre-populate a pending confirmation so the confirm endpoint can find it
    agent.confirmations.lock().unwrap().push(Confirmation {
        id: "conf-1".into(),
        call_id: "call-42".into(),
        title: Some("Allow file edit".into()),
        action: Some("test_action".into()),
        description: String::new(),
        command_type: None,
        options: vec![],
        screenshot: None,
    });

    // Confirm a call with alwaysAllow=true
    let req = json_with_token(
        "POST",
        &format!("/api/conversations/{conv_id}/confirmations/call-42/confirm"),
        json!({ "msg_id": MESSAGE_ID, "data": { "value": "allow" }, "always_allow": true }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let json = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "confirm failed: {json}");

    // Check approval — should be approved for "test_action"
    let req = get_with_token(
        &format!("/api/conversations/{conv_id}/approvals/check?action=test_action"),
        &token,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["approved"], true);
}

#[tokio::test]
async fn check_approval_not_set() {
    let (mut app, services, runtime_registry) = build_app_with_mock_runtime_registry().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "Pass123!").await;
    let conv_id = create_conversation(&mut app, &token, &csrf, "Approval NotSet").await;
    runtime_registry.insert(&conv_id, "/mock-workspace");

    let req = get_with_token(
        &format!("/api/conversations/{conv_id}/approvals/check?action=unknown_action"),
        &token,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["data"]["approved"], false);
}

// ── Auxiliary routes with mock agent ────────────────────────────

#[tokio::test]
async fn slash_commands_with_mock_returns_empty() {
    let (mut app, services, runtime_registry) = build_app_with_mock_runtime_registry().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "Pass123!").await;
    let conv_id = create_conversation(&mut app, &token, &csrf, "Slash Mock Test").await;
    runtime_registry.insert(&conv_id, "/mock-workspace");

    let req = get_with_token(&format!("/api/conversations/{conv_id}/slash-commands"), &token);
    let resp = app.oneshot(req).await.unwrap();
    // The route dispatches straight through `AgentRuntimeHandle`, which has no
    // per-engine downcast left: the mock's slash-command surface is an empty
    // list, so this is an exact 200 rather than the old "200 or 500".
    let status = resp.status();
    let json = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "slash-commands failed: {json}");
    assert_eq!(json["success"], true);
    assert!(json["data"].as_array().unwrap().is_empty());
}


#[tokio::test]
async fn side_question_with_mock_agent() {
    let (mut app, services, runtime_registry) = build_app_with_mock_runtime_registry().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "Pass123!").await;
    let conv_id = create_conversation(&mut app, &token, &csrf, "Side Q Mock").await;
    runtime_registry.insert(&conv_id, "/mock-workspace");

    let req = json_with_token(
        "POST",
        &format!("/api/conversations/{conv_id}/side-question"),
        json!({ "question": "What is this code?" }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    // Side-questions have no implementing backend, so the mock reports the same
    // honest `unsupported` every real variant does — an exact 200 with that
    // status, not the old "200 or 500" downcast-failure window.
    let status = resp.status();
    let json = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "side-question failed: {json}");
    assert_eq!(json["data"]["status"], "unsupported");
    assert!(json["data"]["answer"].is_null());
}
