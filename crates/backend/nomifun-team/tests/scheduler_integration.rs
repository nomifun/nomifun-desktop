mod common;

use std::sync::Arc;

use common::MockTeamRepo;
use nomifun_api_types::WebSocketMessage;
use nomifun_realtime::EventBroadcaster;
use nomifun_team::{
    Mailbox, MailboxMessageType, SchedulerAction, TaskBoard, TeamAgent, TeammateManager, TeammateRole, TeammateStatus,
    WAKE_TIMEOUT_MS,
};

// ---------------------------------------------------------------------------
// Test infrastructure
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

    fn events_by_name(&self, name: &str) -> Vec<WebSocketMessage<serde_json::Value>> {
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

struct TestHarness {
    mgr: TeammateManager,
    mailbox: Arc<Mailbox>,
    task_board: Arc<TaskBoard>,
    broadcaster: Arc<RecordingBroadcaster>,
}

fn setup_team(agents: &[TeamAgent]) -> TestHarness {
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new(
        "team-1".into(),
        agents,
        mailbox.clone(),
        task_board.clone(),
        broadcaster.clone(),
    );
    TestHarness {
        mgr,
        mailbox,
        task_board,
        broadcaster,
    }
}

// ===========================================================================
// Test-plan §7: Agent 调度引擎
// ===========================================================================

// -- AW-1: Wake idle Agent → status idle→working, payload has tasks+unread --

#[tokio::test]
async fn aw1_wake_idle_agent_transitions_to_working_with_payload() {
    let agents = vec![
        make_agent("lead", "Lead", TeammateRole::Lead),
        make_agent("w1", "Worker", TeammateRole::Teammate),
    ];
    let h = setup_team(&agents);

    h.task_board
        .create_task("team-1", "Task A", None, Some("w1"), &[])
        .await
        .unwrap();

    h.mailbox
        .write("team-1", "w1", "lead", MailboxMessageType::Message, "Do it", None)
        .await
        .unwrap();

    let payload = h.mgr.try_wake("w1").await.unwrap();
    assert!(payload.is_some());

    let p = payload.unwrap();
    assert_eq!(p.agent.slot_id, "w1");
    assert_eq!(p.tasks.len(), 1);
    assert_eq!(p.tasks[0].subject, "Task A");
    assert_eq!(p.unread_messages.len(), 1);
    assert_eq!(p.unread_messages[0].content, "Do it");

    assert_eq!(h.mgr.get_status("w1").await.unwrap(), TeammateStatus::Working);
}

// -- AW-2: Wake → agent completes → finalize_turn → actions executed --------

#[tokio::test]
async fn aw2_wake_complete_finalize_executes_actions() {
    let agents = vec![
        make_agent("lead", "Lead", TeammateRole::Lead),
        make_agent("w1", "Worker1", TeammateRole::Teammate),
        make_agent("w2", "Worker2", TeammateRole::Teammate),
    ];
    let h = setup_team(&agents);

    h.mgr.set_status("w1", TeammateStatus::Working).await.unwrap();
    h.mgr.set_status("w2", TeammateStatus::Working).await.unwrap();

    let actions = vec![
        SchedulerAction::TaskCreate {
            subject: "Write tests".into(),
            description: Some("Unit tests for API".into()),
            owner: Some("w1".into()),
            blocked_by: vec![],
        },
        SchedulerAction::SendMessage {
            to: "w2".into(),
            message: "Please review when done".into(),
        },
    ];

    h.mgr.finalize_turn("w1", &actions).await.unwrap();

    let tasks = h.task_board.list_tasks("team-1").await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].subject, "Write tests");

    let w2_msgs = h.mailbox.read_unread("team-1", "w2").await.unwrap();
    assert_eq!(w2_msgs.len(), 1);
    assert_eq!(w2_msgs[0].content, "Please review when done");

    assert_eq!(h.mgr.get_status("w1").await.unwrap(), TeammateStatus::Idle);
}

// -- AW-3: WAKE_TIMEOUT_MS constant is 60000 --------------------------------

#[tokio::test]
async fn aw3_wake_timeout_constant() {
    assert_eq!(WAKE_TIMEOUT_MS, 60_000);
}

// -- AW-4: Wake non-idle agent → skip (no duplicate wake) -------------------

#[tokio::test]
async fn aw4_wake_non_idle_agent_skipped() {
    let agents = vec![make_agent("w1", "Worker", TeammateRole::Teammate)];
    let h = setup_team(&agents);

    h.mgr.set_status("w1", TeammateStatus::Working).await.unwrap();

    let payload = h.mgr.try_wake("w1").await.unwrap();
    assert!(payload.is_none());
}

// -- DL-1: Lead completes turn → stays idle, no auto-wake -------------------

#[tokio::test]
async fn dl1_lead_finalize_stays_idle_no_auto_wake() {
    let agents = vec![
        make_agent("lead", "Lead", TeammateRole::Lead),
        make_agent("w1", "Worker", TeammateRole::Teammate),
    ];
    let h = setup_team(&agents);

    h.mgr.set_status("lead", TeammateStatus::Working).await.unwrap();

    let actions = vec![SchedulerAction::SendMessage {
        to: "w1".into(),
        message: "Go work".into(),
    }];

    let wake_signal = h.mgr.finalize_turn("lead", &actions).await.unwrap();
    assert!(wake_signal.is_none());

    assert_eq!(h.mgr.get_status("lead").await.unwrap(), TeammateStatus::Idle);
}

// -- DL-2: All teammates idle → wake leader ---------------------------------

#[tokio::test]
async fn dl2_all_teammates_idle_wakes_leader() {
    let agents = vec![
        make_agent("lead", "Lead", TeammateRole::Lead),
        make_agent("w1", "Worker1", TeammateRole::Teammate),
        make_agent("w2", "Worker2", TeammateRole::Teammate),
    ];
    let h = setup_team(&agents);

    h.mgr.set_status("w1", TeammateStatus::Working).await.unwrap();
    h.mgr.set_status("w2", TeammateStatus::Working).await.unwrap();

    let wake1 = h.mgr.finalize_turn("w1", &[]).await.unwrap();
    assert!(wake1.is_none());

    let wake2 = h.mgr.finalize_turn("w2", &[]).await.unwrap();
    assert_eq!(wake2.as_deref(), Some("lead"));
}

// -- DL-3: Partial teammates idle → leader NOT woken ------------------------

#[tokio::test]
async fn dl3_partial_teammates_idle_no_leader_wake() {
    let agents = vec![
        make_agent("lead", "Lead", TeammateRole::Lead),
        make_agent("w1", "Worker1", TeammateRole::Teammate),
        make_agent("w2", "Worker2", TeammateRole::Teammate),
    ];
    let h = setup_team(&agents);

    h.mgr.set_status("w1", TeammateStatus::Working).await.unwrap();
    h.mgr.set_status("w2", TeammateStatus::Working).await.unwrap();

    let wake = h.mgr.finalize_turn("w1", &[]).await.unwrap();
    assert!(wake.is_none());
}

// -- AE-1: send_message action → mailbox + wake target ----------------------

#[tokio::test]
async fn ae1_send_message_action_writes_mailbox() {
    let agents = vec![
        make_agent("lead", "Lead", TeammateRole::Lead),
        make_agent("w1", "Worker", TeammateRole::Teammate),
    ];
    let h = setup_team(&agents);

    h.mgr
        .execute_action(
            "lead",
            &SchedulerAction::SendMessage {
                to: "w1".into(),
                message: "Hello worker".into(),
            },
        )
        .await
        .unwrap();

    let msgs = h.mailbox.read_unread("team-1", "w1").await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "Hello worker");
    assert_eq!(msgs[0].from_agent_id, "lead");
}

// -- AE-2: task_create action → task created --------------------------------

#[tokio::test]
async fn ae2_task_create_action() {
    let agents = vec![make_agent("lead", "Lead", TeammateRole::Lead)];
    let h = setup_team(&agents);

    h.mgr
        .execute_action(
            "lead",
            &SchedulerAction::TaskCreate {
                subject: "Build feature".into(),
                description: Some("With tests".into()),
                owner: None,
                blocked_by: vec![],
            },
        )
        .await
        .unwrap();

    let tasks = h.task_board.list_tasks("team-1").await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].subject, "Build feature");
}

// -- AE-3: idle_notification → agent marked idle + check all-idle -----------

#[tokio::test]
async fn ae3_idle_notification_marks_idle_and_notifies_lead() {
    let agents = vec![
        make_agent("lead", "Lead", TeammateRole::Lead),
        make_agent("w1", "Worker", TeammateRole::Teammate),
    ];
    let h = setup_team(&agents);

    h.mgr.set_status("w1", TeammateStatus::Working).await.unwrap();

    let wake_signal = h
        .mgr
        .execute_action(
            "w1",
            &SchedulerAction::IdleNotification {
                summary: Some("Done".into()),
            },
        )
        .await
        .unwrap();

    assert_eq!(h.mgr.get_status("w1").await.unwrap(), TeammateStatus::Idle);

    let lead_msgs = h.mailbox.read_unread("team-1", "lead").await.unwrap();
    assert_eq!(lead_msgs.len(), 1);
    assert_eq!(lead_msgs[0].msg_type, MailboxMessageType::IdleNotification);

    assert_eq!(wake_signal.as_deref(), Some("lead"));
}

// -- AE-4: spawn_agent action → logged (actual creation via TeamSession) ----

#[tokio::test]
async fn ae4_spawn_agent_action_logged() {
    let agents = vec![make_agent("lead", "Lead", TeammateRole::Lead)];
    let h = setup_team(&agents);

    let result = h
        .mgr
        .execute_action(
            "lead",
            &SchedulerAction::SpawnAgent {
                name: "Helper".into(),
                role: "teammate".into(),
                backend: "claude".into(),
            },
        )
        .await;
    assert!(result.is_ok());
}

// ===========================================================================
// Test-plan §8: WebSocket 事件推送
// ===========================================================================

// -- WE-1: Agent status change broadcasts team.agent.status -----------------

#[tokio::test]
async fn we1_status_change_broadcasts_event() {
    let agents = vec![make_agent("w1", "Worker", TeammateRole::Teammate)];
    let h = setup_team(&agents);

    h.mgr.set_status("w1", TeammateStatus::Working).await.unwrap();

    let events = h.broadcaster.events_by_name("team.agent.status");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["team_id"], "team-1");
    assert_eq!(events[0].data["slot_id"], "w1");
    assert_eq!(events[0].data["status"], "working");
}

// -- WE-2: Dynamic agent creation broadcasts team.agent.spawned -------------

#[tokio::test]
async fn we2_add_agent_broadcasts_spawned() {
    let agents = vec![make_agent("lead", "Lead", TeammateRole::Lead)];
    let h = setup_team(&agents);

    let new = make_agent("w1", "Worker", TeammateRole::Teammate);
    h.mgr.add_agent(&new).await;

    let events = h.broadcaster.events_by_name("team.agent.spawned");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["team_id"], "team-1");
    assert!(events[0].data["agent"].is_object());
}

// -- WE-3: Remove agent broadcasts team.agent.removed ----------------------

#[tokio::test]
async fn we3_remove_agent_broadcasts_removed() {
    let agents = vec![
        make_agent("lead", "Lead", TeammateRole::Lead),
        make_agent("w1", "Worker", TeammateRole::Teammate),
    ];
    let h = setup_team(&agents);

    h.mgr.remove_agent("w1").await.unwrap();

    let events = h.broadcaster.events_by_name("team.agent.removed");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["slot_id"], "w1");
}

// -- WE-4: Rename agent broadcasts team.agent.renamed ----------------------

#[tokio::test]
async fn we4_rename_agent_broadcasts_renamed() {
    let agents = vec![make_agent("w1", "Worker", TeammateRole::Teammate)];
    let h = setup_team(&agents);

    h.mgr.rename_agent("w1", "SuperWorker").await.unwrap();

    let events = h.broadcaster.events_by_name("team.agent.renamed");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["slot_id"], "w1");
    assert_eq!(events[0].data["name"], "SuperWorker");
}

// ===========================================================================
// Full workflow: Lead delegates → Workers complete → Leader re-woken
// ===========================================================================

#[tokio::test]
async fn full_workflow_lead_delegate_workers_idle_lead_rewake() {
    let agents = vec![
        make_agent("lead", "Lead", TeammateRole::Lead),
        make_agent("w1", "Worker1", TeammateRole::Teammate),
        make_agent("w2", "Worker2", TeammateRole::Teammate),
    ];
    let h = setup_team(&agents);

    // 1. Wake lead with user message
    h.mailbox
        .write("team-1", "lead", "user", MailboxMessageType::Message, "Build X", None)
        .await
        .unwrap();

    let payload = h.mgr.try_wake("lead").await.unwrap().unwrap();
    assert_eq!(payload.unread_messages.len(), 1);

    // 2. Lead delegates work to workers
    let lead_actions = vec![
        SchedulerAction::TaskCreate {
            subject: "Implement X".into(),
            description: None,
            owner: Some("w1".into()),
            blocked_by: vec![],
        },
        SchedulerAction::SendMessage {
            to: "w1".into(),
            message: "Implement X".into(),
        },
        SchedulerAction::SendMessage {
            to: "w2".into(),
            message: "Write tests for X".into(),
        },
        SchedulerAction::IdleNotification {
            summary: Some("Delegated".into()),
        },
    ];

    let wake_signal = h.mgr.finalize_turn("lead", &lead_actions).await.unwrap();
    assert!(wake_signal.is_none(), "lead idle → no auto-wake");

    // 3. Workers start working
    h.mgr.set_status("w1", TeammateStatus::Working).await.unwrap();
    h.mgr.set_status("w2", TeammateStatus::Working).await.unwrap();

    // 4. Worker1 finishes
    let w1_actions = vec![SchedulerAction::IdleNotification {
        summary: Some("Implemented X".into()),
    }];
    let wake = h.mgr.finalize_turn("w1", &w1_actions).await.unwrap();
    assert!(wake.is_none(), "w2 still working");

    // 5. Worker2 finishes → all idle → leader should be woken
    let w2_actions = vec![SchedulerAction::IdleNotification {
        summary: Some("Tests written".into()),
    }];
    let wake = h.mgr.finalize_turn("w2", &w2_actions).await.unwrap();
    assert_eq!(wake.as_deref(), Some("lead"), "all teammates idle → wake leader");

    // 6. Verify lead has idle notifications from both workers
    let lead_msgs = h.mailbox.read_unread("team-1", "lead").await.unwrap();
    assert_eq!(lead_msgs.len(), 2);

    let summaries: Vec<_> = lead_msgs.iter().map(|m| m.content.as_str()).collect();
    assert!(summaries.contains(&"Implemented X"));
    assert!(summaries.contains(&"Tests written"));
}

// ===========================================================================
// Shutdown flow: Lead initiates → target receives shutdown_request
// ===========================================================================

#[tokio::test]
async fn shutdown_flow_lead_sends_request_to_target() {
    let agents = vec![
        make_agent("lead", "Lead", TeammateRole::Lead),
        make_agent("w1", "Worker", TeammateRole::Teammate),
    ];
    let h = setup_team(&agents);

    h.mgr
        .execute_action(
            "lead",
            &SchedulerAction::ShutdownAgent {
                slot_id: "w1".into(),
                reason: Some("No longer needed".into()),
            },
        )
        .await
        .unwrap();

    let msgs = h.mailbox.read_unread("team-1", "w1").await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].msg_type, MailboxMessageType::ShutdownRequest);
    assert_eq!(msgs[0].content, "No longer needed");
}

#[tokio::test]
async fn shutdown_flow_non_lead_rejected() {
    let agents = vec![
        make_agent("lead", "Lead", TeammateRole::Lead),
        make_agent("w1", "Worker1", TeammateRole::Teammate),
        make_agent("w2", "Worker2", TeammateRole::Teammate),
    ];
    let h = setup_team(&agents);

    let result = h
        .mgr
        .execute_action(
            "w1",
            &SchedulerAction::ShutdownAgent {
                slot_id: "w2".into(),
                reason: None,
            },
        )
        .await;
    assert!(result.is_err());
}

// ===========================================================================
// Broadcast message (to="*") sends to all except sender
// ===========================================================================

#[tokio::test]
async fn broadcast_message_sends_to_all_except_sender() {
    let agents = vec![
        make_agent("lead", "Lead", TeammateRole::Lead),
        make_agent("w1", "Worker1", TeammateRole::Teammate),
        make_agent("w2", "Worker2", TeammateRole::Teammate),
    ];
    let h = setup_team(&agents);

    h.mgr
        .execute_action(
            "lead",
            &SchedulerAction::SendMessage {
                to: "*".into(),
                message: "Attention everyone".into(),
            },
        )
        .await
        .unwrap();

    let w1_msgs = h.mailbox.read_unread("team-1", "w1").await.unwrap();
    let w2_msgs = h.mailbox.read_unread("team-1", "w2").await.unwrap();
    let lead_msgs = h.mailbox.read_unread("team-1", "lead").await.unwrap();

    assert_eq!(w1_msgs.len(), 1);
    assert_eq!(w2_msgs.len(), 1);
    assert!(lead_msgs.is_empty());
}
