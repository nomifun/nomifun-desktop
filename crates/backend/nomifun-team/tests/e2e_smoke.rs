//! End-to-end smoke tests for the team subsystem.
//!
//! **Purpose:** guard against the "agent claims a tool works but it is an
//! empty shell" failure mode by exercising each user-visible capability
//! through its real wiring (TCP MCP server, mailbox, task board, scheduler)
//! and asserting the observable side effect — not just a success return
//! code.
//!
//! **Scenario status:**
//! - scenario 1 (create team → lead → MCP tools available) — `todo!()`,
//!   `#[ignore]`. Unblocks when `spawn_agent` + MCP wiring lands.
//! - scenario 2 (`team_spawn_agent` creates a real session) — `todo!()`,
//!   `#[ignore]`. Unblocks when `spawn_agent` is implemented
//!   (see W5-D29a-* modules).
//! - scenario 3 (shutdown full protocol) — `todo!()`, `#[ignore]`. Unblocks
//!   when shutdown_agent / shutdown_approved mailbox wiring lands.
//! - scenario 4 (crash → testament → leader wake) — `todo!()`, `#[ignore]`.
//!   Unblocks when the crash handler is wired into the stream pipeline.
//! - scenario 5 (MCP tool execution is not a no-op) — **runs now**. Uses
//!   only pieces that already exist (mailbox + task board + TeamMcpServer)
//!   and is the first real e2e guard.
//!
//! All ignored scenarios must stay compiling so the scaffold itself never
//! rots between waves.

mod common;

use std::sync::Arc;

use common::MockTeamRepo;
use nomifun_api_types::WebSocketMessage;
use nomifun_realtime::EventBroadcaster;
use nomifun_team::mcp::protocol::{read_frame, write_frame};
use nomifun_team::{Mailbox, TaskBoard, TeamAgent, TeamMcpServer, TeammateManager, TeammateRole};
use serde_json::{Value, json};
use tokio::net::TcpStream;

// ---------------------------------------------------------------------------
// Shared helpers — local to this file to avoid touching common/mod.rs and
// keep the scaffold self-contained. If more tests start sharing these,
// promote to common/e2e_helpers.rs.
// ---------------------------------------------------------------------------

struct NullBroadcaster;
impl EventBroadcaster for NullBroadcaster {
    fn broadcast(&self, _msg: WebSocketMessage<Value>) {}
}

/// Concrete handle returned by [`setup_team_with_lead`]. Holds every piece
/// a smoke test might need to assert a side effect.
struct SmokeEnv {
    server: TeamMcpServer,
    mailbox: Arc<Mailbox>,
    task_board: Arc<TaskBoard>,
    repo: Arc<MockTeamRepo>,
    #[allow(dead_code)]
    scheduler: Arc<TeammateManager>,
    team_id: String,
    lead_slot_id: String,
    worker_slot_id: String,
    auth_token: String,
}

/// Build a 2-agent team (lead + worker) wired through a real
/// `TeamMcpServer` listening on a random port, against an in-memory mock
/// team repo. Does not spin up a `TeamSessionService`, ACP agents, or
/// backends — those are exercised in the scenarios that need them.
async fn setup_team_with_lead() -> SmokeEnv {
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo.clone()));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(NullBroadcaster);

    let team_id = "smoke-team".to_string();
    let lead_slot_id = "lead-1".to_string();
    let worker_slot_id = "worker-1".to_string();
    let agents = vec![
        TeamAgent {
            slot_id: lead_slot_id.clone(),
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
            slot_id: worker_slot_id.clone(),
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
    ];
    let scheduler = Arc::new(TeammateManager::new(
        team_id.clone(),
        &agents,
        mailbox.clone(),
        task_board.clone(),
        broadcaster.clone(),
    ));

    let auth_token = "smoke-token".to_string();
    let server = TeamMcpServer::start(
        auth_token.clone(),
        scheduler.clone(),
        team_id.clone(),
        broadcaster,
        std::sync::Weak::new(),
    )
    .await
    .unwrap();

    SmokeEnv {
        server,
        mailbox,
        task_board,
        repo,
        scheduler,
        team_id,
        lead_slot_id,
        worker_slot_id,
        auth_token,
    }
}

/// Connect to the MCP server, perform `initialize` as `slot_id`, and
/// return the authenticated stream.
async fn mcp_connect(env: &SmokeEnv, slot_id: &str) -> TcpStream {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", env.server.port()))
        .await
        .expect("tcp connect to TeamMcpServer");

    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "auth_token": env.auth_token,
            "slot_id": slot_id,
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "smoke-test", "version": "1.0" }
        }
    });
    mcp_send(&mut stream, &init_req).await;
    let resp = mcp_recv(&mut stream).await;
    assert!(
        resp["result"]["serverInfo"]["name"].is_string(),
        "initialize failed: {resp}"
    );
    stream
}

/// Send a JSON-RPC `tools/call` and return the raw response envelope.
async fn mcp_call(stream: &mut TcpStream, id: u64, tool: &str, args: Value) -> Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": tool, "arguments": args }
    });
    mcp_send(stream, &req).await;
    mcp_recv(stream).await
}

async fn mcp_send(stream: &mut TcpStream, req: &Value) {
    let bytes = serde_json::to_vec(req).unwrap();
    write_frame(stream, &bytes).await.unwrap();
}

async fn mcp_recv(stream: &mut TcpStream) -> Value {
    let frame = read_frame(stream).await.unwrap();
    serde_json::from_slice(&frame).unwrap()
}

fn is_error_response(resp: &Value) -> bool {
    resp["result"]["isError"].as_bool().unwrap_or(false)
}

/// Assert the given agent's mailbox contains at least one message whose
/// content includes `needle`. Reads the DB-level history so it does not
/// mutate the unread flag (unlike `read_unread`).
async fn assert_mailbox_contains(mailbox: &Mailbox, team_id: &str, agent_id: &str, needle: &str) {
    let history = mailbox
        .get_history(team_id, agent_id, None)
        .await
        .expect("mailbox.get_history");
    assert!(
        history.iter().any(|m| m.content.contains(needle)),
        "expected mailbox[{team_id}/{agent_id}] to contain {needle:?}, got {history:?}"
    );
}

// ===========================================================================
// Scenario 1: create team → lead agent exists → MCP tools available
// ===========================================================================

/// User story: "I create a team; its lead is ready and the MCP surface
/// that lead will drive is actually wired (not an empty shell)."
///
/// Flow:
/// 1. `TeamSessionService::create_team` with a lead + one worker.
/// 2. Assert the returned team has a `lead_agent_id` and two agents.
/// 3. Assert `TeamMcpServer` is started for that team (ensure_session).
/// 4. `tools/list` returns the full 10-tool surface.
/// 5. `team_members` returns both agents.
#[tokio::test]
#[ignore = "unblocks when TeamSessionService e2e wiring is ready (spawn + ensure_session over real DB)"]
async fn smoke_create_team_and_verify_mcp_tools() {
    todo!("scenario 1: fill once spawn_agent / ensure_session end-to-end is merged");
}

// ===========================================================================
// Scenario 2: team_spawn_agent actually creates a new agent session
// ===========================================================================

/// User story: "The lead calls `team_spawn_agent`; a real new agent shows
/// up in the team, has its own conversation row, and has a welcome
/// message in its mailbox — not a success return with no side effect."
///
/// Flow:
/// 1. Create a team with only a lead.
/// 2. Lead calls `team_spawn_agent(name=Helper, role=worker, backend=claude)`.
/// 3. `team_members` includes the new Helper.
/// 4. Conversation repo has a row for the new agent's conversation_id.
/// 5. Helper's mailbox has the welcome / kickoff message.
#[tokio::test]
#[ignore = "unblocks when W5-D29a-* spawn_agent lands"]
async fn smoke_spawn_agent_creates_real_session() {
    todo!("scenario 2: fill once team_spawn_agent persists agent + conversation + welcome mail");
}

// ===========================================================================
// Scenario 3: shutdown agent — full request/approval protocol
// ===========================================================================

/// User story: "The lead asks a worker to shut down; the worker is
/// notified, approves, actually leaves the team, and the WS event is
/// broadcast so the UI can refresh."
///
/// Flow:
/// 1. Create team, spawn worker.
/// 2. Lead MCP-calls `team_shutdown_agent(slot_id=worker)`.
/// 3. Worker's mailbox receives a `shutdown_request`.
/// 4. Worker replies `shutdown_approved` via `team_send_message`.
/// 5. Worker is removed from the team roster.
/// 6. `team.agent.removed` WebSocket event is broadcast.
#[tokio::test]
#[ignore = "unblocks when shutdown_request/approved round-trip is wired (W5-D30a/b/c/d series)"]
async fn smoke_shutdown_agent_full_protocol() {
    todo!("scenario 3: fill once shutdown round-trip + team.agent.removed event are wired");
}

// ===========================================================================
// Scenario 4: agent crash → testament → leader wake
// ===========================================================================

/// User story: "If a worker crashes mid-task, the lead gets a testament
/// mailbox message and is woken up to react — no silent failure."
///
/// Flow:
/// 1. Create team, spawn worker.
/// 2. Inject an Error stream chunk into the worker's agent manager.
/// 3. Lead's mailbox receives a crash testament.
/// 4. Worker's status transitions to `Error`.
/// 5. Lead is woken (wake_lock acquired / wake payload built).
#[tokio::test]
#[ignore = "unblocks when crash_detection is wired into the stream pipeline with real AcpAgentManager"]
async fn smoke_agent_crash_recovery() {
    todo!("scenario 4: fill once crash detection → testament → wake lead is wired");
}

// ===========================================================================
// Scenario 5: MCP tool execution is not a no-op
// ===========================================================================
//
// This is the anchor scenario that guards against the core failure mode
// the user called out: a tool returning `success` with no observable side
// effect. It only uses pieces that already exist (mailbox + task board +
// TeamMcpServer), so it runs in CI today.

#[tokio::test]
async fn smoke_mcp_tool_execution_not_noop() {
    let env = setup_team_with_lead().await;
    let mut stream = mcp_connect(&env, &env.lead_slot_id).await;

    // --- team_send_message → mailbox side effect -------------------------
    let msg_resp = mcp_call(
        &mut stream,
        10,
        "team_send_message",
        json!({ "to": env.worker_slot_id, "message": "hello worker" }),
    )
    .await;
    assert!(
        !is_error_response(&msg_resp),
        "team_send_message returned error: {msg_resp}"
    );
    // Guard against the exact failure mode: success envelope, nothing written.
    assert_mailbox_contains(&env.mailbox, &env.team_id, &env.worker_slot_id, "hello worker").await;

    // --- team_task_create → task board side effect -----------------------
    let task_resp = mcp_call(
        &mut stream,
        11,
        "team_task_create",
        json!({ "subject": "Smoke test subject" }),
    )
    .await;
    assert!(
        !is_error_response(&task_resp),
        "team_task_create returned error: {task_resp}"
    );
    let tasks = env.task_board.list_tasks(&env.team_id).await.unwrap();
    assert!(
        tasks.iter().any(|t| t.subject == "Smoke test subject"),
        "team_task_create did not persist task, got {tasks:?}"
    );

    // --- repo-level cross-check: mailbox/task rows actually hit storage --
    // Even if the service layer lies, the repo-level mock's state is the
    // ground truth for "did data move through the stack".
    let repo_state = env.repo.state.lock().unwrap();
    assert!(!repo_state.messages.is_empty(), "no mailbox rows reached the repo");
    assert!(!repo_state.tasks.is_empty(), "no task rows reached the repo");
    drop(repo_state);

    env.server.stop();
}
