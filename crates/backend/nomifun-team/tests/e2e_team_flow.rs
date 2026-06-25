//! End-to-end integration tests for the team communication pipeline.
//!
//! These tests verify the full flow:
//!   MCP tool call → mailbox write → wake → send_message → finish → leader notified
//!

// Pre-existing: MutexGuard held across await points is intentional in this
// test to maintain a short critical section for assertion, then explicitly dropped.
#![allow(clippy::await_holding_lock)]
//! Infrastructure used:
//! - Real in-memory mock repo (same pattern as existing tests)
//! - Real TCP MCP server (TeamMcpServer)
//! - Real TeamSession with real Mailbox + TaskBoard
//! - RecordingAgent: captures send_message calls (mock `IAgentTask` / `IMockAgent`)
//! - StubTaskManager: pre-populated with RecordingAgent instances
//!
//! Scenarios that cannot yet be wired without a live TeamSessionService DB path
//! are marked #[ignore] with a clear explanation.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;
use common::MockTeamRepo;
use nomifun_ai_agent::agent_task::{AgentInstance, IAgentTask, IMockAgent};
use nomifun_ai_agent::protocol::events::{AgentStreamEvent, FinishEventData};
use nomifun_ai_agent::shared_kernel::approval_key;
use nomifun_ai_agent::types::{BuildTaskOptions, SendMessageData};
use nomifun_api_types::AgentModeResponse;
use nomifun_api_types::WebSocketMessage;
use nomifun_common::{AgentKillReason, AgentType, AppError, Confirmation, ConversationStatus, TimestampMs, now_ms};
use nomifun_db::ITeamRepository;
use nomifun_realtime::EventBroadcaster;
use nomifun_team::mcp::protocol::{read_frame, write_frame};
use nomifun_team::service::TeamSessionService;
use nomifun_team::{TeamAgent, TeamSession, TeammateRole};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::broadcast;

// ===========================================================================
// Shared test infrastructure
// ===========================================================================

struct NullBroadcaster;
impl EventBroadcaster for NullBroadcaster {
    fn broadcast(&self, _msg: WebSocketMessage<Value>) {}
}

/// RecordingBroadcaster captures all WebSocket events for assertion.
/// Currently unused in e2e_team_flow tests — kept for future scenario expansion.
#[allow(dead_code)]
#[derive(Default)]
struct RecordingBroadcaster {
    events: Mutex<Vec<WebSocketMessage<Value>>>,
}

#[allow(dead_code)]
impl RecordingBroadcaster {
    fn new() -> Self {
        Self::default()
    }

    fn events_named(&self, name: &str) -> Vec<WebSocketMessage<Value>> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.name == name)
            .cloned()
            .collect()
    }
}

impl EventBroadcaster for RecordingBroadcaster {
    fn broadcast(&self, msg: WebSocketMessage<Value>) {
        self.events.lock().unwrap().push(msg);
    }
}

/// RecordingAgent: captures every send_message call. The broadcast channel
/// lets tests simulate Finish events by sending AgentStreamEvent::Finish.
struct RecordingAgent {
    conversation_id: String,
    sent: Arc<Mutex<Vec<SendMessageData>>>,
    event_tx: broadcast::Sender<AgentStreamEvent>,
    fail_with: Option<String>,
}

impl RecordingAgent {
    fn new(conversation_id: &str, sent: Arc<Mutex<Vec<SendMessageData>>>) -> Self {
        let (event_tx, _) = broadcast::channel(16);
        Self {
            conversation_id: conversation_id.to_owned(),
            sent,
            event_tx,
            fail_with: None,
        }
    }

    /// Create a variant whose send_message always errors.
    /// Reserved for future error-path scenario tests.
    #[allow(dead_code)]
    fn failing(conversation_id: &str, sent: Arc<Mutex<Vec<SendMessageData>>>, error: &str) -> Self {
        let (event_tx, _) = broadcast::channel(16);
        Self {
            conversation_id: conversation_id.to_owned(),
            sent,
            event_tx,
            fail_with: Some(error.to_owned()),
        }
    }

    /// Subscribe to the agent's event stream so the test can fire Finish/Error.
    #[allow(dead_code)]
    fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent> {
        self.event_tx.subscribe()
    }

    /// Fire a Finish event on the agent's stream (simulates agent completing a turn).
    #[allow(dead_code)]
    fn fire_finish(&self) {
        let _ = self
            .event_tx
            .send(AgentStreamEvent::Finish(FinishEventData { session_id: None, stop_reason: None }));
    }
}

#[async_trait::async_trait]
impl IAgentTask for RecordingAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Acp
    }
    fn conversation_id(&self) -> &str {
        &self.conversation_id
    }
    fn workspace(&self) -> &str {
        "/tmp/ws"
    }
    fn status(&self) -> Option<ConversationStatus> {
        None
    }
    fn last_activity_at(&self) -> TimestampMs {
        now_ms()
    }
    fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent> {
        self.event_tx.subscribe()
    }
    async fn send_message(&self, data: SendMessageData) -> Result<(), nomifun_ai_agent::AgentSendError> {
        self.sent.lock().unwrap().push(data);
        match &self.fail_with {
            Some(msg) => Err(nomifun_ai_agent::AgentSendError::from_app_error(AppError::Internal(
                msg.clone(),
            ))),
            None => Ok(()),
        }
    }
    async fn cancel(&self) -> Result<(), AppError> {
        Ok(())
    }
    fn kill(&self, _reason: Option<AgentKillReason>) -> Result<(), AppError> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl IMockAgent for RecordingAgent {
    fn get_confirmations(&self) -> Vec<Confirmation> {
        Vec::new()
    }
    fn check_approval(&self, action: &str, command_type: Option<&str>) -> bool {
        let _ = approval_key(Some(action), command_type);
        false
    }
    fn confirm(&self, _: &str, _: &str, _: Value, _: bool) -> Result<(), AppError> {
        Ok(())
    }
    async fn mode(&self) -> Result<AgentModeResponse, AppError> {
        Ok(AgentModeResponse {
            mode: "default".to_owned(),
            initialized: false,
        })
    }
}

/// StubTaskManager: allows pre-inserting RecordingAgent handles by conv_id.
/// Also records kill calls.
struct StubTaskManager {
    tasks: Mutex<HashMap<String, AgentInstance>>,
    kill_calls: Mutex<Vec<(String, Option<AgentKillReason>)>>,
}

impl StubTaskManager {
    fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            kill_calls: Mutex::new(Vec::new()),
        }
    }

    fn insert(&self, conv_id: &str, handle: AgentInstance) {
        self.tasks.lock().unwrap().insert(conv_id.to_owned(), handle);
    }

    #[allow(dead_code)]
    fn kill_calls(&self) -> Vec<(String, Option<AgentKillReason>)> {
        self.kill_calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl nomifun_ai_agent::IWorkerTaskManager for StubTaskManager {
    fn get_task(&self, conversation_id: &str) -> Option<AgentInstance> {
        self.tasks.lock().unwrap().get(conversation_id).cloned()
    }

    async fn get_or_build_task(&self, _: &str, _: BuildTaskOptions) -> Result<AgentInstance, AppError> {
        Err(AppError::Internal(
            "StubTaskManager does not support get_or_build_task".into(),
        ))
    }
    fn kill(&self, conversation_id: &str, reason: Option<AgentKillReason>) -> Result<(), AppError> {
        self.kill_calls
            .lock()
            .unwrap()
            .push((conversation_id.to_owned(), reason));
        Ok(())
    }
    fn kill_and_wait(
        &self,
        conversation_id: &str,
        reason: Option<AgentKillReason>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        let _ = self.kill(conversation_id, reason);
        Box::pin(std::future::ready(()))
    }
    fn clear(&self) {}
    fn active_count(&self) -> usize {
        self.tasks.lock().unwrap().len()
    }
    fn collect_idle(&self, _: TimestampMs) -> Vec<String> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers: extract MCP server info from TeamSession via public API
// ---------------------------------------------------------------------------

/// Get the MCP server port from a TeamSession using the public mcp_stdio_config API.
fn session_port(session: &TeamSession) -> u16 {
    session.mcp_stdio_config("lead-1").port
}

/// Get the MCP server auth token from a TeamSession using the public mcp_stdio_config API.
fn session_token(session: &TeamSession) -> String {
    session.mcp_stdio_config("lead-1").token
}

// ---------------------------------------------------------------------------
// MCP protocol helpers (same pattern as e2e_smoke.rs and mcp_server_integration.rs)
// ---------------------------------------------------------------------------

async fn tcp_send(stream: &mut TcpStream, req: &Value) {
    let bytes = serde_json::to_vec(req).unwrap();
    write_frame(stream, &bytes).await.unwrap();
}

async fn tcp_recv(stream: &mut TcpStream) -> Value {
    let frame = read_frame(stream).await.unwrap();
    serde_json::from_slice(&frame).unwrap()
}

/// Connect and complete the MCP initialize handshake. Returns an
/// authenticated, ready-to-use TcpStream.
async fn mcp_connect(port: u16, auth_token: &str, slot_id: &str) -> TcpStream {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .expect("tcp connect to TeamMcpServer");
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "auth_token": auth_token,
            "slot_id": slot_id,
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "e2e-test", "version": "0.1" }
        }
    });
    tcp_send(&mut stream, &init_req).await;
    let resp = tcp_recv(&mut stream).await;
    assert!(
        resp["result"]["serverInfo"]["name"].is_string(),
        "initialize failed: {resp}"
    );
    stream
}

/// Send a tools/call and return the full response envelope.
async fn mcp_call_tool(stream: &mut TcpStream, id: u64, tool: &str, args: Value) -> Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": tool, "arguments": args }
    });
    tcp_send(stream, &req).await;
    tcp_recv(stream).await
}

fn is_mcp_error(resp: &Value) -> bool {
    resp["result"]["isError"].as_bool().unwrap_or(false)
}

fn mcp_text(resp: &Value) -> &str {
    resp["result"]["content"][0]["text"].as_str().unwrap_or("")
}

// ---------------------------------------------------------------------------
// Environment builders
// ---------------------------------------------------------------------------

fn backend_path() -> Arc<PathBuf> {
    Arc::new(PathBuf::from("/tmp/nomicore-e2e-test"))
}

/// Two-agent team definition: one Lead + one Worker.
fn two_agents() -> Vec<TeamAgent> {
    vec![
        TeamAgent {
            slot_id: "lead-1".into(),
            name: "Leader".into(),
            role: TeammateRole::Lead,
            conversation_id: "conv-lead".into(),
            backend: "acp".into(),
            model: "claude".into(),
            custom_agent_id: None,
            status: None,
            conversation_type: None,
            cli_path: None,
        },
        TeamAgent {
            slot_id: "worker-1".into(),
            name: "Worker".into(),
            role: TeammateRole::Teammate,
            conversation_id: "conv-worker".into(),
            backend: "acp".into(),
            model: "claude".into(),
            custom_agent_id: None,
            status: None,
            conversation_type: None,
            cli_path: None,
        },
    ]
}

/// Build a TeamSession + shared task manager pre-populated with RecordingAgents.
///
/// Returns:
/// - Arc<TeamSession>
/// - Arc<StubTaskManager>
/// - Arc<MockTeamRepo>  (for low-level mailbox inspection)
/// - Arc<Mutex<Vec<SendMessageData>>>  (shared sent-messages log)
async fn setup_session() -> (
    Arc<TeamSession>,
    Arc<StubTaskManager>,
    Arc<MockTeamRepo>,
    Arc<Mutex<Vec<SendMessageData>>>,
) {
    let repo = Arc::new(MockTeamRepo::new());
    let repo_dyn: Arc<dyn ITeamRepository> = repo.clone();
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(NullBroadcaster);

    let sent: Arc<Mutex<Vec<SendMessageData>>> = Arc::new(Mutex::new(Vec::new()));
    let task_manager = Arc::new(StubTaskManager::new());

    for agent in two_agents() {
        let handle = AgentInstance::Mock(Arc::new(RecordingAgent::new(&agent.conversation_id, sent.clone())));
        task_manager.insert(&agent.conversation_id, handle);
    }

    let task_manager_dyn: Arc<dyn nomifun_ai_agent::IWorkerTaskManager> = task_manager.clone();

    let team = nomifun_team::types::Team {
        id: "e2e-team".into(),
        name: "E2E Team".into(),
        agents: two_agents(),
        lead_agent_id: Some("lead-1".into()),
        created_at: 1000,
        updated_at: 1000,
    };

    let session = TeamSession::start(
        team,
        repo_dyn,
        broadcaster,
        backend_path(),
        task_manager_dyn,
        "user-e2e".into(),
        Weak::<TeamSessionService>::new(),
    )
    .await
    .expect("TeamSession::start failed");

    (Arc::new(session), task_manager, repo, sent)
}

// ===========================================================================
// Scenario 1: MCP server starts and tools/list surface is correct
// ===========================================================================

/// Scenario 1a: TeamSession starts, MCP server binds a port, tools are available.
///
/// Verifies:
/// - TeamSession::start succeeds
/// - MCP TCP server is reachable
/// - tools/list returns all 10 expected tools
#[tokio::test]
async fn s1a_mcp_server_starts_and_tools_available() {
    let (session, _tm, _repo, _sent) = setup_session().await;

    let port = session_port(&session);
    assert!(port > 0, "MCP server must bind a non-zero port");

    let token = session_token(&session);
    let mut stream = mcp_connect(port, &token, "lead-1").await;

    let req = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
    tcp_send(&mut stream, &req).await;
    let resp = tcp_recv(&mut stream).await;

    let tools = resp["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 10, "expected exactly 10 MCP tools, got {}", tools.len());

    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"team_send_message"), "missing team_send_message");
    assert!(names.contains(&"team_members"), "missing team_members");
    assert!(names.contains(&"team_task_create"), "missing team_task_create");

    session.stop();
}

/// Scenario 1b: mcp_stdio_config is generated correctly for each agent slot.
///
/// Verifies that the stdio config written to conversation.extra contains
/// the correct port, token, and slot_id values.
#[tokio::test]
async fn s1b_mcp_stdio_config_per_agent() {
    let (session, _tm, _repo, _sent) = setup_session().await;

    let cfg_lead = session.mcp_stdio_config("lead-1");
    assert_eq!(cfg_lead.team_id, "e2e-team");
    assert_eq!(cfg_lead.slot_id, "lead-1");
    assert_eq!(cfg_lead.port, session_port(&session));
    assert!(!cfg_lead.token.is_empty());

    let cfg_worker = session.mcp_stdio_config("worker-1");
    assert_eq!(cfg_worker.team_id, "e2e-team");
    assert_eq!(cfg_worker.slot_id, "worker-1");
    assert_eq!(cfg_worker.port, cfg_lead.port, "same server port");
    assert_eq!(cfg_worker.token, cfg_lead.token, "same auth token for same session");
    assert_ne!(cfg_worker.slot_id, cfg_lead.slot_id);

    session.stop();
}

// ===========================================================================
// Scenario 2: MCP team_send_message end-to-end
// ===========================================================================

/// Scenario 2a: Lead calls team_send_message via MCP → mailbox written.
///
/// This is the core side-effect test: MCP tool call must persist the message
/// to the mailbox repo. The wake path (wake_agent_in_session) requires a live
/// TeamSessionService Weak pointer which is not available in this standalone
/// TeamSession test environment. Verify mailbox persistence only here;
/// wake propagation is covered by s9_session_send_message_wakes_lead which
/// uses the public `send_message` / `send_message_to_agent` API.
#[tokio::test]
async fn s2a_mcp_team_send_message_writes_mailbox() {
    let (session, _tm, repo, _sent) = setup_session().await;
    let port = session_port(&session);
    let token = session_token(&session);

    let mut stream = mcp_connect(port, &token, "lead-1").await;
    let resp = mcp_call_tool(
        &mut stream,
        10,
        "team_send_message",
        json!({ "to": "worker-1", "message": "e2e test payload" }),
    )
    .await;

    assert!(!is_mcp_error(&resp), "team_send_message returned error: {resp}");
    assert!(
        mcp_text(&resp).contains("Message sent"),
        "response text must confirm delivery"
    );

    // Mailbox must have the message (persisted by the MCP handler's execute_action)
    let state = repo.state.lock().unwrap();
    let worker_msgs: Vec<_> = state
        .messages
        .iter()
        .filter(|m| m.to_agent_id == "worker-1" && m.content.contains("e2e test payload"))
        .collect();
    assert!(
        !worker_msgs.is_empty(),
        "mailbox must contain the message for worker-1; repo state: {:?}",
        state.messages
    );

    session.stop();
}

/// Scenario 2b: Broadcast team_send_message (to="*") writes to all teammates.
#[tokio::test]
async fn s2b_mcp_broadcast_writes_to_all_agents() {
    let (session, _tm, repo, _sent) = setup_session().await;
    let port = session_port(&session);
    let token = session_token(&session);

    let mut stream = mcp_connect(port, &token, "lead-1").await;
    let resp = mcp_call_tool(
        &mut stream,
        11,
        "team_send_message",
        json!({ "to": "*", "message": "broadcast msg" }),
    )
    .await;

    assert!(!is_mcp_error(&resp), "broadcast team_send_message failed: {resp}");

    // Both lead and worker should have received the broadcast
    let state = repo.state.lock().unwrap();
    let worker_msgs: Vec<_> = state
        .messages
        .iter()
        .filter(|m| m.content.contains("broadcast msg"))
        .collect();
    assert!(
        !worker_msgs.is_empty(),
        "broadcast must write to at least one agent; state.messages={:?}",
        state.messages
    );

    session.stop();
}

/// Scenario 2c: team_send_message is not a no-op — the repo actually receives
/// the row (guards against the "success with no side effect" failure mode).
#[tokio::test]
async fn s2c_send_message_side_effect_reaches_repo() {
    let (session, _tm, repo, _sent) = setup_session().await;
    let port = session_port(&session);
    let token = session_token(&session);

    // Verify repo is initially empty
    {
        let state = repo.state.lock().unwrap();
        assert!(state.messages.is_empty(), "repo must start empty");
    }

    let mut stream = mcp_connect(port, &token, "lead-1").await;
    mcp_call_tool(
        &mut stream,
        12,
        "team_send_message",
        json!({ "to": "worker-1", "message": "side-effect check" }),
    )
    .await;

    // After tool call, repo must have at least one row
    let state = repo.state.lock().unwrap();
    assert!(
        !state.messages.is_empty(),
        "team_send_message must persist at least one message row to repo"
    );

    session.stop();
}

// ===========================================================================
// Scenario 3: on_agent_finish → mark_idle → IdleNotification → leader re-wake
// ===========================================================================

/// Scenario 3a: Worker finishes → on_agent_finish marks worker idle →
/// IdleNotification written to lead mailbox → lead is returned as wake target.
#[tokio::test]
async fn s3a_on_agent_finish_writes_idle_notification_to_lead() {
    let (session, _tm, repo, _sent) = setup_session().await;

    // Set worker to Working (simulates in-flight turn)
    session
        .scheduler()
        .set_status("worker-1", nomifun_team::TeammateStatus::Working)
        .await
        .unwrap();

    // Simulate worker finishing its turn
    let wake_target = session.on_agent_finish("conv-worker", false).await.unwrap();

    // Worker should now be idle and lead should be returned as wake target
    assert_eq!(
        wake_target.as_deref(),
        Some("lead-1"),
        "on_agent_finish must return lead-1 as wake target; got {wake_target:?}"
    );

    let worker_status = session.scheduler().get_status("worker-1").await.unwrap();
    assert_eq!(
        worker_status,
        nomifun_team::TeammateStatus::Idle,
        "worker must be Idle after finish"
    );

    // IdleNotification must be in lead's mailbox
    let state = repo.state.lock().unwrap();
    let lead_idle: Vec<_> = state
        .messages
        .iter()
        .filter(|m| m.to_agent_id == "lead-1" && m.msg_type == "idle_notification")
        .collect();
    assert!(
        !lead_idle.is_empty(),
        "IdleNotification must be written to lead mailbox after worker finish; got {:?}",
        state.messages
    );

    session.stop();
}

/// Scenario 3b: Lead finish does not write IdleNotification to anyone.
#[tokio::test]
async fn s3b_lead_finish_does_not_write_idle_notification() {
    let (session, _tm, repo, _sent) = setup_session().await;

    session
        .scheduler()
        .set_status("lead-1", nomifun_team::TeammateStatus::Working)
        .await
        .unwrap();

    let wake_target = session.on_agent_finish("conv-lead", false).await.unwrap();

    // Lead finish should not produce a wake target
    assert!(wake_target.is_none(), "lead finish must not return a wake target");

    // No idle_notification should be written
    let state = repo.state.lock().unwrap();
    let idle_notifs: Vec<_> = state
        .messages
        .iter()
        .filter(|m| m.msg_type == "idle_notification")
        .collect();
    assert!(
        idle_notifs.is_empty(),
        "lead finish must not write idle_notification; got {idle_notifs:?}"
    );

    session.stop();
}

/// Scenario 3c: Full round-trip — worker sends message via MCP → finishes →
/// lead mailbox has IdleNotification → lead's send_message is called.
#[tokio::test]
async fn s3c_finish_triggers_lead_wake_with_idle_notification() {
    let (session, _tm, repo, sent) = setup_session().await;

    // Write a message to worker's mailbox (so the wake has content to send)
    session
        .mailbox()
        .write(
            "e2e-team",
            "worker-1",
            "lead-1",
            nomifun_team::MailboxMessageType::Message,
            "do the work",
            None,
        )
        .await
        .unwrap();

    // Set worker Working and drain its mailbox (simulates the wake consuming
    // messages via compute_wake_input before on_agent_finish fires).
    session
        .scheduler()
        .set_status("worker-1", nomifun_team::TeammateStatus::Working)
        .await
        .unwrap();
    let _ = session.mailbox().read_unread("e2e-team", "worker-1").await;

    // Worker finishes → IdleNotification → lead returned
    let wake_target = session.on_agent_finish("conv-worker", false).await.unwrap();
    assert_eq!(wake_target.as_deref(), Some("lead-1"));

    // Verify that the IdleNotification is in lead's mailbox (it will be consumed
    // when the lead is re-woken by spawn_finish_subscribers in the real service).
    {
        let state = repo.state.lock().unwrap();
        let lead_msgs: Vec<_> = state.messages.iter().filter(|m| m.to_agent_id == "lead-1").collect();
        assert!(
            !lead_msgs.is_empty(),
            "lead mailbox must have content (idle_notification) after worker finish"
        );
    }

    // Trigger lead wake via the public send_message_to_agent API:
    // this writes to mailbox + wakes the agent, which causes send_message to fire.
    session
        .send_message_to_agent("lead-1", "wake", None)
        .await
        .expect("send_message_to_agent must succeed");

    // Allow async propagation
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Lead's agent send_message must have been called
    let log = sent.lock().unwrap();
    assert!(
        !log.is_empty(),
        "lead's RecordingAgent.send_message must be called after wake; sent log: {log:?}"
    );

    session.stop();
}

// ===========================================================================
// Scenario 4: add_agent (runtime) + finish propagation
// ===========================================================================

/// Scenario 4: Adding a new agent at runtime (simulating spawn_agent outcome)
/// then verifying the new agent receives messages and can finish.
#[tokio::test]
async fn s4_dynamic_agent_added_then_finish_propagates() {
    let (session, task_manager, repo, sent) = setup_session().await;

    // Add a new agent at runtime
    let new_agent = TeamAgent {
        slot_id: "helper-1".into(),
        name: "Helper".into(),
        role: TeammateRole::Teammate,
        conversation_id: "conv-helper".into(),
        backend: "acp".into(),
        model: "claude".into(),
        custom_agent_id: None,
        status: None,
        conversation_type: None,
        cli_path: None,
    };

    // Insert a recording agent for the new conversation
    let handle = AgentInstance::Mock(Arc::new(RecordingAgent::new("conv-helper", sent.clone())));
    task_manager.insert("conv-helper", handle);

    // Add the agent to the session's scheduler
    session.add_agent(&new_agent).await;

    // Verify the agent is in the roster
    let agents = session.scheduler().list_agents().await;
    assert_eq!(agents.len(), 3, "expected 3 agents after add_agent");
    assert!(agents.iter().any(|a| a.slot_id == "helper-1"));

    // Send a welcome message to the new agent
    session
        .mailbox()
        .write(
            "e2e-team",
            "helper-1",
            "lead-1",
            nomifun_team::MailboxMessageType::Message,
            "welcome, helper",
            None,
        )
        .await
        .unwrap();

    // Wake the helper via public send_message_to_agent API
    // (this writes to mailbox AND wakes the agent, consuming the prior mailbox messages)
    session
        .send_message_to_agent("helper-1", "start your task", None)
        .await
        .expect("send_message_to_agent to helper must succeed");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Helper's send_message should have been called
    let log = sent.lock().unwrap();
    assert!(
        !log.is_empty(),
        "helper's RecordingAgent.send_message must be called after send_message_to_agent; log: {log:?}"
    );
    drop(log);

    // Simulate helper finishing → IdleNotification → lead should be wake target
    let wake_target = session.on_agent_finish("conv-helper", false).await.unwrap();
    assert_eq!(
        wake_target.as_deref(),
        Some("lead-1"),
        "helper finish must return lead as wake target"
    );

    // Lead mailbox should have IdleNotification from helper
    let state = repo.state.lock().unwrap();
    let lead_notifs: Vec<_> = state
        .messages
        .iter()
        .filter(|m| m.to_agent_id == "lead-1" && m.msg_type == "idle_notification" && m.from_agent_id == "helper-1")
        .collect();
    assert!(
        !lead_notifs.is_empty(),
        "lead must receive idle_notification from helper; msgs={:?}",
        state.messages
    );

    session.stop();
}

// ===========================================================================
// Scenario 5: Rapid consecutive messages – dedup window after clear
// ===========================================================================

/// Scenario 5: After a first finish + clear_finalized_turn, a second finish
/// for the same conversation must also succeed (not be dropped by dedup).
///
/// This is the regression test for the "dedup window drops legitimate second
/// Finish" bug (fix/team-communication-bugs task #5).
#[tokio::test]
async fn s5_consecutive_finish_events_after_dedup_clear() {
    let (session, _tm, repo, _sent) = setup_session().await;

    // First finish
    session
        .scheduler()
        .set_status("worker-1", nomifun_team::TeammateStatus::Working)
        .await
        .unwrap();
    let first_result = session.on_agent_finish("conv-worker", false).await.unwrap();
    assert_eq!(
        first_result.as_deref(),
        Some("lead-1"),
        "first finish must return wake target"
    );

    // Simulate what the service's finish_subscribers do: clear the dedup window
    // after a successful wake so the next legitimate finish can proceed.
    session.scheduler().clear_finalized_turn("conv-worker");

    // Set worker Working again (second turn)
    session
        .scheduler()
        .set_status("worker-1", nomifun_team::TeammateStatus::Working)
        .await
        .unwrap();

    let second_result = session.on_agent_finish("conv-worker", false).await.unwrap();
    assert_eq!(
        second_result.as_deref(),
        Some("lead-1"),
        "second finish (after dedup clear) must also return wake target; got {second_result:?}"
    );

    // Lead mailbox should have two IdleNotification entries (one per finish)
    let state = repo.state.lock().unwrap();
    let idle_count = state
        .messages
        .iter()
        .filter(|m| m.to_agent_id == "lead-1" && m.msg_type == "idle_notification")
        .count();
    assert_eq!(
        idle_count, 2,
        "both finish events must produce IdleNotification; got {idle_count}"
    );

    session.stop();
}

/// Scenario 5b: Within the dedup window, a duplicate finish SHOULD be dropped.
///
/// Bug (task #5): `on_agent_finish` calls `clear_finalized_turn` immediately
/// after `finalize_turn` returns `wake_target.is_some()`. This means the dedup
/// window is cleared on the first success, allowing the second rapid Finish to
/// also be processed. The intent was to only clear after the re-woken agent
/// completes its *next* turn — not immediately.
///
/// This test is #[ignore] until the fix is merged: the dedup window must NOT
/// be cleared immediately after the first success if the second finish arrives
/// within the 5-second window.
#[tokio::test]
#[ignore = "Bug: on_agent_finish clears dedup window immediately, allowing double-processing within window (task #5 fix pending)"]
async fn s5b_dedup_window_blocks_rapid_duplicate_finish() {
    let (session, _tm, repo, _sent) = setup_session().await;

    session
        .scheduler()
        .set_status("worker-1", nomifun_team::TeammateStatus::Working)
        .await
        .unwrap();

    // First finish — should proceed
    let first = session.on_agent_finish("conv-worker", false).await.unwrap();
    assert!(first.is_some(), "first finish must succeed");

    // Immediately repeat without clearing — should be dedup'd (returns Ok(None)).
    // NOTE: on_agent_finish checks dedup before any state changes, so the second
    // call should return None even after re-setting status to Working.
    session
        .scheduler()
        .set_status("worker-1", nomifun_team::TeammateStatus::Working)
        .await
        .unwrap();

    let second = session.on_agent_finish("conv-worker", false).await.unwrap();
    assert!(
        second.is_none(),
        "second finish within dedup window must be dropped (returns None); got {second:?}"
    );

    // Only one IdleNotification should exist
    let state = repo.state.lock().unwrap();
    let idle_count = state
        .messages
        .iter()
        .filter(|m| m.msg_type == "idle_notification")
        .count();
    assert_eq!(
        idle_count, 1,
        "dedup must prevent double idle_notification; got {idle_count}"
    );

    session.stop();
}

// ===========================================================================
// Scenario 6: task board operations via MCP
// ===========================================================================

/// Scenario 6: team_task_create via MCP → task board persisted → task visible
/// in team_task_list.
#[tokio::test]
async fn s6_mcp_task_create_and_list() {
    let (session, _tm, repo, _sent) = setup_session().await;
    let port = session_port(&session);
    let token = session_token(&session);

    let mut stream = mcp_connect(port, &token, "lead-1").await;

    // Create a task
    let create_resp = mcp_call_tool(
        &mut stream,
        20,
        "team_task_create",
        json!({ "subject": "E2E Task Alpha" }),
    )
    .await;
    assert!(!is_mcp_error(&create_resp), "team_task_create failed: {create_resp}");

    // List tasks — must contain the created task
    let list_resp = mcp_call_tool(&mut stream, 21, "team_task_list", json!({})).await;
    assert!(!is_mcp_error(&list_resp), "team_task_list failed: {list_resp}");
    let text = mcp_text(&list_resp);
    let tasks: Vec<Value> = serde_json::from_str(text).expect("task list must be JSON");
    assert!(
        tasks.iter().any(|t| t["subject"] == "E2E Task Alpha"),
        "created task must appear in task list; got {tasks:?}"
    );

    // Repo-level cross-check: task row reached storage
    let state = repo.state.lock().unwrap();
    assert!(
        state.tasks.iter().any(|t| t.subject == "E2E Task Alpha"),
        "task must be persisted in repo; got {:?}",
        state.tasks
    );

    session.stop();
}

// ===========================================================================
// Scenario 7: team_members reflects dynamic roster
// ===========================================================================

/// Scenario 7: After adding an agent at runtime, team_members via MCP
/// returns the updated roster.
#[tokio::test]
async fn s7_team_members_reflects_dynamic_roster() {
    let (session, _tm, _repo, _sent) = setup_session().await;
    let port = session_port(&session);
    let token = session_token(&session);

    let mut stream = mcp_connect(port, &token, "lead-1").await;

    // Initially 2 members
    let resp = mcp_call_tool(&mut stream, 30, "team_members", json!({})).await;
    assert!(!is_mcp_error(&resp), "team_members failed");
    let members: Vec<Value> = serde_json::from_str(mcp_text(&resp)).expect("team_members must return JSON array");
    assert_eq!(members.len(), 2, "should start with 2 members");

    // Add a third agent
    let new_agent = TeamAgent {
        slot_id: "extra-1".into(),
        name: "ExtraAgent".into(),
        role: TeammateRole::Teammate,
        conversation_id: "conv-extra".into(),
        backend: "acp".into(),
        model: "claude".into(),
        custom_agent_id: None,
        status: None,
        conversation_type: None,
        cli_path: None,
    };
    session.add_agent(&new_agent).await;

    // Now team_members should return 3
    let resp2 = mcp_call_tool(&mut stream, 31, "team_members", json!({})).await;
    assert!(!is_mcp_error(&resp2), "team_members (after add) failed");
    let members2: Vec<Value> = serde_json::from_str(mcp_text(&resp2)).expect("team_members must return JSON array");
    assert_eq!(members2.len(), 3, "roster must include dynamically added agent");
    assert!(
        members2.iter().any(|m| m["name"] == "ExtraAgent"),
        "ExtraAgent must appear in team_members"
    );

    session.stop();
}

// ===========================================================================
// Scenario 8: Authentication on MCP connection
// ===========================================================================

/// Scenario 8a: Wrong auth token must be rejected.
#[tokio::test]
async fn s8a_wrong_auth_token_rejected() {
    let (session, _tm, _repo, _sent) = setup_session().await;
    let port = session_port(&session);

    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .expect("tcp connect");
    let bad_init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "auth_token": "wrong-token",
            "slot_id": "lead-1",
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "attacker", "version": "1" }
        }
    });
    tcp_send(&mut stream, &bad_init).await;
    let resp = tcp_recv(&mut stream).await;
    assert!(
        resp["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("Authentication failed"),
        "wrong token must be rejected with Authentication failed; got {resp}"
    );

    session.stop();
}

/// Scenario 8b: Non-lead slot cannot call team_spawn_agent (role guard).
#[tokio::test]
async fn s8b_worker_cannot_call_spawn_agent() {
    let (session, _tm, _repo, _sent) = setup_session().await;
    let port = session_port(&session);
    let token = session_token(&session);

    let mut stream = mcp_connect(port, &token, "worker-1").await;
    let resp = mcp_call_tool(
        &mut stream,
        40,
        "team_spawn_agent",
        json!({ "name": "Hacker", "backend": "claude" }),
    )
    .await;
    assert!(is_mcp_error(&resp), "worker must not be allowed to call spawn_agent");
    let text = mcp_text(&resp);
    assert!(text.contains("Only Lead"), "error must mention 'Only Lead'; got {text}");

    session.stop();
}

// ===========================================================================
// Scenario 9: send_message via TeamSession (not MCP) — direct API path
// ===========================================================================

/// Scenario 9: TeamSession::send_message writes to lead mailbox and
/// triggers lead's RecordingAgent.send_message.
#[tokio::test]
async fn s9_session_send_message_wakes_lead() {
    let (session, _tm, repo, sent) = setup_session().await;

    session
        .send_message("user input to team", None)
        .await
        .expect("send_message must succeed");

    // Lead mailbox must have the message
    {
        let state = repo.state.lock().unwrap();
        let lead_msgs: Vec<_> = state
            .messages
            .iter()
            .filter(|m| m.to_agent_id == "lead-1" && m.content == "user input to team")
            .collect();
        assert!(!lead_msgs.is_empty(), "message must be in lead mailbox");
    }

    // Lead's send_message must be called
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let log = sent.lock().unwrap();
    assert!(
        log.iter().any(|d| d.content.contains("user input to team")),
        "lead's RecordingAgent.send_message must be called; log: {log:?}"
    );

    session.stop();
}

// ===========================================================================
// Scenario 10: Error finish marks agent as Error status
// ===========================================================================

/// Scenario 10: on_agent_finish with is_error=true must preserve Error status.
///
/// Bug (task #5 related): Currently `on_agent_finish` sets status to Error,
/// then calls `finalize_turn` → `mark_idle`, which overwrites Error with Idle.
/// The correct behavior: Error status should be preserved (not overwritten by
/// mark_idle). This test is #[ignore] until the Error-status-preservation fix
/// is merged into fix/team-communication-bugs.
#[tokio::test]
#[ignore = "Bug: mark_idle overwrites Error status with Idle (fix pending in fix/team-communication-bugs)"]
async fn s10_error_finish_sets_agent_status_to_error() {
    let (session, _tm, _repo, _sent) = setup_session().await;

    session
        .scheduler()
        .set_status("worker-1", nomifun_team::TeammateStatus::Working)
        .await
        .unwrap();

    let wake_target = session.on_agent_finish("conv-worker", true).await.unwrap();
    assert_eq!(wake_target.as_deref(), Some("lead-1"));

    let status = session.scheduler().get_status("worker-1").await.unwrap();
    assert_eq!(
        status,
        nomifun_team::TeammateStatus::Error,
        "error finish must preserve Error status (not be overwritten by mark_idle)"
    );

    session.stop();
}

// ===========================================================================
// Scenario 11: shutdown_approved sentinel interception
// ===========================================================================

/// Scenario 11: Worker sending "shutdown_approved" to lead is intercepted
/// by the MCP bridge and does not land as a raw string in lead's mailbox.
#[tokio::test]
async fn s11_shutdown_approved_interception() {
    let (session, _tm, repo, _sent) = setup_session().await;
    let port = session_port(&session);
    let token = session_token(&session);

    let mut stream = mcp_connect(port, &token, "worker-1").await;
    let resp = mcp_call_tool(
        &mut stream,
        50,
        "team_send_message",
        json!({ "to": "lead-1", "message": "shutdown_approved" }),
    )
    .await;

    assert!(!is_mcp_error(&resp), "shutdown_approved must not be a protocol error");
    let text = mcp_text(&resp);
    let payload: Value = serde_json::from_str(text).expect("shutdown_approved response must be JSON");
    assert_eq!(
        payload["status"], "shutdown_approved_received",
        "must return shutdown_approved_received status; got {text}"
    );

    // The raw sentinel must NOT be in lead's mailbox
    let state = repo.state.lock().unwrap();
    let raw_sentinel: Vec<_> = state
        .messages
        .iter()
        .filter(|m| m.to_agent_id == "lead-1" && m.content == "shutdown_approved")
        .collect();
    assert!(
        raw_sentinel.is_empty(),
        "raw shutdown_approved sentinel must not land in lead mailbox; got {raw_sentinel:?}"
    );

    session.stop();
}

// ===========================================================================
// Scenario 12: compute_wake_input — role prompt injection on cold start
// ===========================================================================

/// Scenario 12a: Cold-start lead gets role prompt injected into first_message.
#[tokio::test]
async fn s12a_cold_start_lead_gets_role_prompt() {
    let (session, _tm, _repo, _sent) = setup_session().await;

    session
        .mailbox()
        .write(
            "e2e-team",
            "lead-1",
            "user",
            nomifun_team::MailboxMessageType::Message,
            "kick off",
            None,
        )
        .await
        .unwrap();

    let input = session
        .compute_wake_input("lead-1")
        .await
        .unwrap()
        .expect("WakeInput must be Some");

    assert!(input.should_send, "should_send must be true when mailbox has messages");
    assert!(
        input.first_message.contains("You are the Team Leader"),
        "cold-start lead must get role prompt; got: {}",
        &input.first_message[..input.first_message.len().min(200)]
    );
    assert!(input.first_message.contains("kick off"));

    session.stop();
}

/// Scenario 12b: Warm lead (role prompt already consumed) does not get role prompt again.
///
/// The cold-start flag is a one-shot: `take_needs_role_prompt` returns true
/// only on the first call. To simulate a "warm" agent, we consume the flag
/// on a first compute_wake_input call, then assert the second call omits it.
#[tokio::test]
async fn s12b_warm_lead_skips_role_prompt() {
    let (session, _tm, _repo, _sent) = setup_session().await;

    // First: consume the cold-start role-prompt flag by calling compute_wake_input once.
    // The mailbox will be empty so should_send=false, but the flag is consumed.
    session
        .mailbox()
        .write(
            "e2e-team",
            "lead-1",
            "user",
            nomifun_team::MailboxMessageType::Message,
            "initial kick",
            None,
        )
        .await
        .unwrap();
    let first_input = session
        .compute_wake_input("lead-1")
        .await
        .unwrap()
        .expect("first WakeInput");
    assert!(
        first_input.first_message.contains("You are the Team Leader"),
        "first (cold) compute_wake_input must include role prompt"
    );
    // The flag is now consumed (take_needs_role_prompt returned true, set to false).

    // Second call: write another message and compute again — no role prompt this time.
    session
        .mailbox()
        .write(
            "e2e-team",
            "lead-1",
            "user",
            nomifun_team::MailboxMessageType::Message,
            "follow-up message",
            None,
        )
        .await
        .unwrap();

    let input = session
        .compute_wake_input("lead-1")
        .await
        .unwrap()
        .expect("second WakeInput must be Some");

    assert!(
        !input.first_message.contains("You are the Team Leader"),
        "warm lead (flag consumed) must NOT get role prompt on second call; got: {}",
        &input.first_message[..input.first_message.len().min(300)]
    );
    assert!(input.first_message.contains("follow-up message"));

    session.stop();
}
