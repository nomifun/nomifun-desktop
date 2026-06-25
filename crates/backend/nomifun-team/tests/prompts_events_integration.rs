mod common;

use std::sync::Arc;

use common::MockTeamRepo;
use nomifun_api_types::{
    TeamAgentRemovedPayload, TeamAgentRenamedPayload, TeamAgentSpawnedPayload, TeamAgentStatusPayload, WebSocketMessage,
};
use nomifun_realtime::EventBroadcaster;
use nomifun_team::events::TeamEventEmitter;
use nomifun_team::prompts::{build_lead_prompt, build_teammate_prompt, build_wake_payload};
use nomifun_team::types::{
    MailboxMessage, MailboxMessageType, TaskStatus, TeamAgent, TeamTask, TeammateRole, TeammateStatus,
};
use nomifun_team::{Mailbox, TaskBoard, TeammateManager};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

struct RecordingBroadcaster {
    events: std::sync::Mutex<Vec<WebSocketMessage<serde_json::Value>>>,
}

impl RecordingBroadcaster {
    fn new() -> Self {
        Self {
            events: std::sync::Mutex::new(vec![]),
        }
    }

    fn events(&self) -> Vec<WebSocketMessage<serde_json::Value>> {
        self.events.lock().unwrap().clone()
    }
}

impl EventBroadcaster for RecordingBroadcaster {
    fn broadcast(&self, event: WebSocketMessage<serde_json::Value>) {
        self.events.lock().unwrap().push(event);
    }
}

fn make_agent(slot_id: &str, name: &str, role: TeammateRole) -> TeamAgent {
    TeamAgent {
        slot_id: slot_id.into(),
        name: name.into(),
        role,
        conversation_id: format!("conv-{slot_id}"),
        backend: "acp".into(),
        model: "claude".into(),
        custom_agent_id: None,
        status: None,
        conversation_type: None,
        cli_path: None,
    }
}

// ===========================================================================
// Test-plan §9: Prompt Templates
// ===========================================================================

fn default_agent_types() -> Vec<(String, String)> {
    vec![
        ("claude".into(), "Claude".into()),
        ("codex".into(), "Codex".into()),
        ("gemini".into(), "Gemini".into()),
    ]
}

// -- LP-1: Lead prompt contains member list ----------------------------------

#[test]
fn lp1_lead_prompt_contains_member_list() {
    let members = vec![
        make_agent("lead-1", "Lead", TeammateRole::Lead),
        make_agent("w1", "Alice", TeammateRole::Teammate),
        make_agent("w2", "Bob", TeammateRole::Teammate),
    ];
    let types = default_agent_types();
    let prompt = build_lead_prompt("Alpha", &members, &types);

    // Nomi bullet format: `- {name} ({backend}, status: {status})`
    assert!(prompt.contains("- Lead ("), "lead name missing");
    assert!(prompt.contains("- Alice ("), "teammate Alice missing");
    assert!(prompt.contains("- Bob ("), "teammate Bob missing");
}

// -- LP-2: Lead prompt contains tool descriptions ----------------------------

#[test]
fn lp2_lead_prompt_contains_tool_descriptions() {
    let prompt = build_lead_prompt("Beta", &[], &default_agent_types());

    // Nomi lead prompt references the `team_*` coordination tools that the
    // leader must use; the MCP layer enumerates them with arguments, so the
    // prompt mentions each tool at least once.
    let expected_tools = [
        "team_send_message",
        "team_spawn_agent",
        "team_task_create",
        "team_task_list",
        "team_members",
        "team_rename_agent",
        "team_shutdown_agent",
        "team_list_models",
    ];
    for tool in expected_tools {
        assert!(prompt.contains(tool), "missing tool: {tool}");
    }
}

// -- LP-3: Lead prompt contains task management guidance ---------------------

#[test]
fn lp3_lead_prompt_contains_task_management_guidance() {
    let prompt = build_lead_prompt("Gamma", &[], &default_agent_types());

    assert!(
        prompt.contains("Break the work into tasks"),
        "missing decompose guidance"
    );
    assert!(prompt.contains("Assign tasks"), "missing assign guidance");
    assert!(prompt.contains("dependency"), "missing dependency guidance");
    assert!(
        prompt.contains("When teammates report back"),
        "missing teammate result-review guidance"
    );
}

// -- TP-1: Teammate prompt contains execution guidance -----------------------

#[test]
fn tp1_teammate_prompt_contains_execution_guidance() {
    let agent = make_agent("w1", "Worker1", TeammateRole::Teammate);
    let prompt = build_teammate_prompt(&agent, "Alpha");

    assert!(prompt.contains("execute tasks"), "missing execution guidance");
    assert!(prompt.contains("team_send_message"), "missing communication tool");
    assert!(prompt.contains("team_task_update"), "missing task update tool");
    assert!(prompt.contains("shutdown_request"), "missing shutdown protocol");
    assert!(prompt.contains("shutdown_approved"), "missing shutdown_approved");
}

// -- TP-2: Teammate prompt contains team name --------------------------------

#[test]
fn tp2_teammate_prompt_contains_team_name() {
    let agent = make_agent("w1", "Worker1", TeammateRole::Teammate);
    let prompt = build_teammate_prompt(&agent, "Project Falcon");

    assert!(prompt.contains("\"Project Falcon\""));
}

// -- WP-1: Wake payload includes unread messages -----------------------------

#[test]
fn wp1_wake_payload_includes_unread_messages() {
    let agent = make_agent("lead-1", "Lead", TeammateRole::Lead);
    let messages = vec![
        MailboxMessage {
            id: 1,
            team_id: "t1".into(),
            to_agent_id: "lead-1".into(),
            from_agent_id: "w1".into(),
            msg_type: MailboxMessageType::Message,
            content: "Feature X is done".into(),
            summary: None,
            files: None,
            read: false,
            created_at: 0,
        },
        MailboxMessage {
            id: 2,
            team_id: "t1".into(),
            to_agent_id: "lead-1".into(),
            from_agent_id: "w2".into(),
            msg_type: MailboxMessageType::IdleNotification,
            content: "idle".into(),
            summary: Some("Finished task Y".into()),
            files: None,
            read: false,
            created_at: 0,
        },
    ];
    let payload = build_wake_payload(&agent, &[], &messages);

    assert!(payload.contains("Feature X is done"));
    assert!(payload.contains("`w1`"));
    assert!(payload.contains("[message]"));
    assert!(payload.contains("`w2`"));
    assert!(payload.contains("[idle_notification]"));
    assert!(payload.contains("Summary: Finished task Y"));
}

// -- WP-2: Wake payload includes current task list ---------------------------

#[test]
fn wp2_wake_payload_includes_task_list() {
    let agent = make_agent("lead-1", "Lead", TeammateRole::Lead);
    let tasks = vec![
        TeamTask {
            id: "aaaaaaaa-1111-2222-3333-444444444444".into(),
            team_id: "t1".into(),
            subject: "Implement auth".into(),
            description: None,
            status: TaskStatus::InProgress,
            owner: Some("w1".into()),
            blocked_by: vec![],
            blocks: vec![],
            metadata: None,
            created_at: 0,
            updated_at: 0,
        },
        TeamTask {
            id: "bbbbbbbb-1111-2222-3333-444444444444".into(),
            team_id: "t1".into(),
            subject: "Write tests".into(),
            description: None,
            status: TaskStatus::Pending,
            owner: Some("w2".into()),
            blocked_by: vec!["aaaaaaaa-1111-2222-3333-444444444444".into()],
            blocks: vec![],
            metadata: None,
            created_at: 0,
            updated_at: 0,
        },
    ];
    let payload = build_wake_payload(&agent, &tasks, &[]);

    assert!(payload.contains("Current Task Board"));
    assert!(payload.contains("Implement auth"));
    assert!(payload.contains("in_progress"));
    assert!(payload.contains("Write tests"));
    assert!(payload.contains("pending"));
    assert!(payload.contains("w1"));
    assert!(payload.contains("w2"));
    assert!(
        payload.contains("aaaaaaaa-1111-2222-3333-444444444444"),
        "blocker task ID should appear in blocked_by column"
    );
}

// -- WP-3: Wake payload with no messages and no tasks builds normally --------

#[test]
fn wp3_wake_payload_empty_builds_normally() {
    let agent = make_agent("w1", "Worker1", TeammateRole::Teammate);
    let payload = build_wake_payload(&agent, &[], &[]);

    assert!(payload.contains("No new messages"));
    assert!(payload.contains("No tasks on the board"));
    assert!(payload.contains("**Worker1**"));
    assert!(payload.contains("teammate"));
}

// ===========================================================================
// Test-plan §8: WebSocket Event Broadcasting
// ===========================================================================

// -- WE-1: Agent status change event -----------------------------------------

#[tokio::test]
async fn we1_agent_status_change_event() {
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let bc = Arc::new(RecordingBroadcaster::new());
    let agents = vec![
        make_agent("lead-1", "Lead", TeammateRole::Lead),
        make_agent("w1", "Worker", TeammateRole::Teammate),
    ];
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox, task_board, bc.clone());

    mgr.set_status("w1", TeammateStatus::Working).await.unwrap();

    let events = bc.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name, "team.agent.status");

    let payload: TeamAgentStatusPayload = serde_json::from_value(events[0].data.clone()).unwrap();
    assert_eq!(payload.team_id, "t1");
    assert_eq!(payload.slot_id, "w1");
    assert_eq!(payload.status, "working");
}

// -- WE-2: Agent spawned event -----------------------------------------------

#[tokio::test]
async fn we2_agent_spawned_event() {
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let bc = Arc::new(RecordingBroadcaster::new());
    let agents = vec![make_agent("lead-1", "Lead", TeammateRole::Lead)];
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox, task_board, bc.clone());

    let new_agent = make_agent("w2", "NewWorker", TeammateRole::Teammate);
    mgr.add_agent(&new_agent).await;

    let spawned: Vec<_> = bc
        .events()
        .into_iter()
        .filter(|e| e.name == "team.agent.spawned")
        .collect();
    assert_eq!(spawned.len(), 1);

    let payload: TeamAgentSpawnedPayload = serde_json::from_value(spawned[0].data.clone()).unwrap();
    assert_eq!(payload.team_id, "t1");
    assert_eq!(payload.agent.slot_id, "w2");
    assert_eq!(payload.agent.name, "NewWorker");
}

// -- WE-3: Agent removed event -----------------------------------------------

#[tokio::test]
async fn we3_agent_removed_event() {
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let bc = Arc::new(RecordingBroadcaster::new());
    let agents = vec![
        make_agent("lead-1", "Lead", TeammateRole::Lead),
        make_agent("w1", "Worker", TeammateRole::Teammate),
    ];
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox, task_board, bc.clone());

    mgr.remove_agent("w1").await.unwrap();

    let removed: Vec<_> = bc
        .events()
        .into_iter()
        .filter(|e| e.name == "team.agent.removed")
        .collect();
    assert_eq!(removed.len(), 1);

    let payload: TeamAgentRemovedPayload = serde_json::from_value(removed[0].data.clone()).unwrap();
    assert_eq!(payload.team_id, "t1");
    assert_eq!(payload.slot_id, "w1");
}

// -- WE-4: Agent renamed event -----------------------------------------------

#[tokio::test]
async fn we4_agent_renamed_event() {
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let bc = Arc::new(RecordingBroadcaster::new());
    let agents = vec![
        make_agent("lead-1", "Lead", TeammateRole::Lead),
        make_agent("w1", "Worker", TeammateRole::Teammate),
    ];
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox, task_board, bc.clone());

    mgr.rename_agent("w1", "SuperWorker").await.unwrap();

    let renamed: Vec<_> = bc
        .events()
        .into_iter()
        .filter(|e| e.name == "team.agent.renamed")
        .collect();
    assert_eq!(renamed.len(), 1);

    let payload: TeamAgentRenamedPayload = serde_json::from_value(renamed[0].data.clone()).unwrap();
    assert_eq!(payload.team_id, "t1");
    assert_eq!(payload.slot_id, "w1");
    assert_eq!(payload.name, "SuperWorker");
}

// -- Direct TeamEventEmitter test (event payloads use typed structs) ----------

#[test]
fn event_emitter_uses_typed_payloads() {
    let bc = Arc::new(RecordingBroadcaster::new());
    let emitter = TeamEventEmitter::new("team-x".into(), bc.clone());

    let agent = make_agent("s1", "A", TeammateRole::Teammate);
    emitter.broadcast_agent_status("s1", TeammateStatus::Thinking);
    emitter.broadcast_agent_spawned(&agent);
    emitter.broadcast_agent_removed("s1");
    emitter.broadcast_agent_renamed("s1", "B");

    let events = bc.events();
    assert_eq!(events.len(), 4);

    let p1: TeamAgentStatusPayload = serde_json::from_value(events[0].data.clone()).unwrap();
    assert_eq!(p1.status, "thinking");

    let p2: TeamAgentSpawnedPayload = serde_json::from_value(events[1].data.clone()).unwrap();
    assert_eq!(p2.agent.slot_id, "s1");

    let p3: TeamAgentRemovedPayload = serde_json::from_value(events[2].data.clone()).unwrap();
    assert_eq!(p3.slot_id, "s1");

    let p4: TeamAgentRenamedPayload = serde_json::from_value(events[3].data.clone()).unwrap();
    assert_eq!(p4.name, "B");
}
