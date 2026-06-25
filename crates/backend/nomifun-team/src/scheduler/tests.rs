use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use nomifun_ai_agent::AgentStreamEvent;
use nomifun_ai_agent::protocol::events::{FinishEventData, TextEventData};
use nomifun_api_types::WebSocketMessage;
use nomifun_realtime::EventBroadcaster;
use tokio::sync::broadcast;

use super::*;
use crate::crash_detection::CrashReason;
use crate::mailbox::Mailbox;
use crate::task_board::TaskBoard;
use crate::test_utils::MockTeamRepo;
use crate::types::{MailboxMessageType, TeammateRole, TeammateStatus};

// -----------------------------------------------------------------
// normalize_name — §15.1 contract
// -----------------------------------------------------------------

#[test]
fn normalize_name_trims_outer_whitespace() {
    assert_eq!(normalize_name("  Alice  "), "alice");
    assert_eq!(normalize_name("\tBob\n"), "bob");
}

#[test]
fn normalize_name_lowercases_ascii_and_unicode() {
    assert_eq!(normalize_name("ALICE"), "alice");
    assert_eq!(normalize_name("Crème"), "crème");
}

#[test]
fn normalize_name_filters_control_characters() {
    // Null + bell in the middle + outer whitespace.
    assert_eq!(normalize_name("  Ali\x00ce\x07 "), "alice");
}

#[test]
fn normalize_name_collides_on_case_and_whitespace() {
    // Conflict-detection invariant: two inputs that only differ by
    // surrounding whitespace / case must normalize to the same string.
    assert_eq!(normalize_name("  Leader  "), normalize_name("leader"));
}

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

fn make_team_agents() -> Vec<TeamAgent> {
    vec![
        make_agent("lead-1", "Lead", TeammateRole::Lead),
        make_agent("worker-1", "Worker1", TeammateRole::Teammate),
        make_agent("worker-2", "Worker2", TeammateRole::Teammate),
    ]
}

fn make_manager(agents: &[TeamAgent]) -> (TeammateManager, Arc<RecordingBroadcaster>) {
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), agents, mailbox, task_board, broadcaster.clone());
    (mgr, broadcaster)
}

// -- Status management ---------------------------------------------------

#[tokio::test]
async fn initial_status_is_idle() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    for agent in &agents {
        let status = mgr.get_status(&agent.slot_id).await.unwrap();
        assert_eq!(status, TeammateStatus::Idle);
    }
}

#[tokio::test]
async fn set_status_updates_and_broadcasts() {
    let agents = make_team_agents();
    let (mgr, bc) = make_manager(&agents);

    mgr.set_status("worker-1", TeammateStatus::Working).await.unwrap();

    assert_eq!(mgr.get_status("worker-1").await.unwrap(), TeammateStatus::Working);

    let events = bc.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name, "team.agent.status");
    assert_eq!(events[0].data["slot_id"], "worker-1");
    assert_eq!(events[0].data["status"], "working");
}

#[tokio::test]
async fn set_status_nonexistent_agent_fails() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    let result = mgr.set_status("ghost", TeammateStatus::Working).await;
    assert!(matches!(result, Err(TeamError::AgentNotFound(_))));
}

// -- Wake / try_wake -----------------------------------------------------

#[tokio::test]
async fn try_wake_idle_agent_returns_payload() {
    let agents = make_team_agents();
    let (mgr, bc) = make_manager(&agents);

    let payload = mgr.try_wake("worker-1").await.unwrap();
    assert!(payload.is_some());

    let p = payload.unwrap();
    assert_eq!(p.agent.slot_id, "worker-1");
    assert!(p.tasks.is_empty());
    assert!(p.unread_messages.is_empty());

    assert_eq!(mgr.get_status("worker-1").await.unwrap(), TeammateStatus::Working);

    let status_events: Vec<_> = bc
        .events()
        .into_iter()
        .filter(|e| e.name == "team.agent.status")
        .collect();
    assert_eq!(status_events.len(), 1);
    assert_eq!(status_events[0].data["status"], "working");
}

#[tokio::test]
async fn try_wake_non_idle_agent_returns_none() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    mgr.set_status("worker-1", TeammateStatus::Working).await.unwrap();

    let payload = mgr.try_wake("worker-1").await.unwrap();
    assert!(payload.is_none());
}

#[tokio::test]
async fn try_wake_nonexistent_agent_fails() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    let result = mgr.try_wake("ghost").await;
    assert!(matches!(result, Err(TeamError::AgentNotFound(_))));
}

// -- Anti-deadloop: Lead idle after turn ----------------------------------

#[tokio::test]
async fn lead_mark_idle_does_not_wake_self() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    mgr.set_status("lead-1", TeammateStatus::Working).await.unwrap();
    let wake_target = mgr.mark_idle("lead-1", None).await.unwrap();
    assert!(wake_target.is_none());
}

// -- Anti-deadloop: All teammates idle → wake leader ---------------------

#[tokio::test]
async fn all_teammates_idle_signals_wake_leader() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    mgr.set_status("worker-1", TeammateStatus::Working).await.unwrap();
    mgr.set_status("worker-2", TeammateStatus::Working).await.unwrap();

    let result = mgr.mark_idle("worker-1", None).await.unwrap();
    assert!(result.is_none(), "not all teammates idle yet");

    let result = mgr.mark_idle("worker-2", None).await.unwrap();
    assert_eq!(result.as_deref(), Some("lead-1"));
}

#[tokio::test]
async fn partial_teammates_idle_does_not_wake_leader() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    mgr.set_status("worker-1", TeammateStatus::Working).await.unwrap();
    mgr.set_status("worker-2", TeammateStatus::Working).await.unwrap();

    let result = mgr.mark_idle("worker-1", None).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn leader_not_woken_if_already_working() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    mgr.set_status("lead-1", TeammateStatus::Working).await.unwrap();
    mgr.set_status("worker-1", TeammateStatus::Working).await.unwrap();

    let result = mgr.mark_idle("worker-1", None).await.unwrap();
    assert!(result.is_none());
}

// -- Solo team (lead only, no teammates) ---------------------------------

#[tokio::test]
async fn solo_team_no_teammates_no_wake_signal() {
    let agents = vec![make_agent("lead-1", "Lead", TeammateRole::Lead)];
    let (mgr, _) = make_manager(&agents);

    mgr.set_status("lead-1", TeammateStatus::Working).await.unwrap();
    let result = mgr.mark_idle("lead-1", None).await.unwrap();
    assert!(result.is_none());
}

// -- Agent lifecycle (add/remove/rename) ---------------------------------

#[tokio::test]
async fn add_agent_broadcasts_spawned_event() {
    let agents = make_team_agents();
    let (mgr, bc) = make_manager(&agents);

    let new_agent = make_agent("worker-3", "Worker3", TeammateRole::Teammate);
    mgr.add_agent(&new_agent).await;

    let all = mgr.list_agents().await;
    assert_eq!(all.len(), 4);

    let spawned_events: Vec<_> = bc
        .events()
        .into_iter()
        .filter(|e| e.name == "team.agent.spawned")
        .collect();
    assert_eq!(spawned_events.len(), 1);
}

#[tokio::test]
async fn remove_agent_broadcasts_removed_event() {
    let agents = make_team_agents();
    let (mgr, bc) = make_manager(&agents);

    let conv_id = mgr.remove_agent("worker-2").await.unwrap();
    assert_eq!(conv_id.as_deref(), Some("conv-worker-2"));

    let all = mgr.list_agents().await;
    assert_eq!(all.len(), 2);
    assert!(
        all.iter().all(|a| a.slot_id != "worker-2"),
        "list_agents must not contain the removed slot_id"
    );
    assert!(
        all.iter().any(|a| a.slot_id == "lead-1") && all.iter().any(|a| a.slot_id == "worker-1"),
        "list_agents must retain every slot that was not removed"
    );

    assert!(
        matches!(
            mgr.get_agent("worker-2").await,
            Err(TeamError::AgentNotFound(ref s)) if s == "worker-2"
        ),
        "get_agent on a removed slot must return AgentNotFound"
    );

    let removed_events: Vec<_> = bc
        .events()
        .into_iter()
        .filter(|e| e.name == "team.agent.removed")
        .collect();
    assert_eq!(removed_events.len(), 1);
    assert_eq!(removed_events[0].data["slot_id"], "worker-2");
}

#[tokio::test]
async fn notify_shutdown_acknowledged_broadcasts_shutdown_event() {
    let agents = make_team_agents();
    let (mgr, bc) = make_manager(&agents);

    mgr.notify_shutdown_acknowledged("worker-2");

    let shutdown_events: Vec<_> = bc
        .events()
        .into_iter()
        .filter(|e| e.name == "team.agent.shutdown")
        .collect();
    assert_eq!(shutdown_events.len(), 1);
    assert_eq!(shutdown_events[0].data["slot_id"], "worker-2");
    assert_eq!(shutdown_events[0].data["team_id"], mgr.team_id);
}

#[tokio::test]
async fn notify_shutdown_acknowledged_does_not_remove_slot() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);
    let before = mgr.list_agents().await.len();

    mgr.notify_shutdown_acknowledged("worker-2");

    assert_eq!(mgr.list_agents().await.len(), before);
}

#[tokio::test]
async fn remove_nonexistent_agent_fails() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    let result = mgr.remove_agent("ghost").await;
    assert!(matches!(result, Err(TeamError::AgentNotFound(_))));
}

#[tokio::test]
async fn remove_agent_clears_wake_lock_timeout_and_finalize_dedup() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);
    let conv_id = "conv-worker-2"; // matches make_agent("worker-2")

    // Populate all three state stores for worker-2.
    assert!(mgr.acquire_wake_lock("worker-2"));
    let handle = tokio::spawn(async { tokio::time::sleep(Duration::from_secs(999)).await });
    mgr.wake_timeouts.insert("worker-2".into(), handle);
    assert!(mgr.begin_finalize(conv_id));

    mgr.remove_agent("worker-2").await.unwrap();

    assert!(
        !mgr.active_wakes.contains("worker-2"),
        "active_wakes must not retain a removed slot"
    );
    assert!(
        mgr.wake_timeouts.get("worker-2").is_none(),
        "wake_timeouts must not retain a removed slot"
    );
    assert!(
        mgr.finalized_turns.get(conv_id).is_none(),
        "finalized_turns must not retain the removed slot's conversation_id"
    );
}

#[tokio::test]
async fn remove_agent_twice_second_call_is_noop_and_no_extra_broadcast() {
    let agents = make_team_agents();
    let (mgr, bc) = make_manager(&agents);

    mgr.remove_agent("worker-2").await.unwrap();
    let second = mgr.remove_agent("worker-2").await;
    assert!(
        matches!(second, Err(TeamError::AgentNotFound(ref s)) if s == "worker-2"),
        "second remove of the same slot must fail with AgentNotFound"
    );

    let removed_events: Vec<_> = bc
        .events()
        .into_iter()
        .filter(|e| e.name == "team.agent.removed")
        .collect();
    assert_eq!(
        removed_events.len(),
        1,
        "failed second remove must not emit another team.agent.removed"
    );
}

#[tokio::test]
async fn remove_agent_clear_state_is_idempotent() {
    // clear_agent_state tolerates missing entries — calling it on a slot
    // that never populated any of the three stores is a no-op, not a panic.
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    mgr.remove_agent("worker-1").await.unwrap();

    assert!(!mgr.active_wakes.contains("worker-1"));
    assert!(mgr.wake_timeouts.get("worker-1").is_none());
    assert!(mgr.finalized_turns.get("conv-worker-1").is_none());
}

#[tokio::test]
async fn rename_agent_broadcasts_renamed_event() {
    let agents = make_team_agents();
    let (mgr, bc) = make_manager(&agents);

    mgr.rename_agent("worker-1", "Renamed Worker").await.unwrap();

    let agent = mgr.get_agent("worker-1").await.unwrap();
    assert_eq!(agent.name, "Renamed Worker");

    let renamed_events: Vec<_> = bc
        .events()
        .into_iter()
        .filter(|e| e.name == "team.agent.renamed")
        .collect();
    assert_eq!(renamed_events.len(), 1);
    assert_eq!(renamed_events[0].data["name"], "Renamed Worker");
}

#[tokio::test]
async fn rename_agent_rejects_duplicate_name() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    // "Worker2" already exists (from make_team_agents). Renaming worker-1
    // to " Worker2 " should collide after normalization.
    let err = mgr
        .rename_agent("worker-1", " Worker2 ")
        .await
        .expect_err("duplicate name must be rejected");
    assert!(
        matches!(&err, TeamError::DuplicateAgentName(_)),
        "expected DuplicateAgentName, got {err:?}"
    );

    // Original name must be preserved.
    let agent = mgr.get_agent("worker-1").await.unwrap();
    assert_eq!(agent.name, "Worker1");
}

#[tokio::test]
async fn rename_agent_allows_same_agent_own_name() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    // Renaming worker-1 to its own name (different casing) is not a conflict.
    mgr.rename_agent("worker-1", "WORKER1").await.unwrap();
    let agent = mgr.get_agent("worker-1").await.unwrap();
    assert_eq!(agent.name, "WORKER1");
}

// -- execute_action: SendMessage -----------------------------------------

#[tokio::test]
async fn execute_send_message_writes_to_mailbox() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    let action = SchedulerAction::SendMessage {
        to: "worker-1".into(),
        message: "Do task X".into(),
    };
    mgr.execute_action("lead-1", &action).await.unwrap();

    let unread = mailbox.read_unread("t1", "worker-1").await.unwrap();
    assert_eq!(unread.len(), 1);
    assert_eq!(unread[0].content, "Do task X");
    assert_eq!(unread[0].from_agent_id, "lead-1");
}

#[tokio::test]
async fn execute_broadcast_message_writes_to_all_others() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    let action = SchedulerAction::SendMessage {
        to: "*".into(),
        message: "Attention all".into(),
    };
    mgr.execute_action("lead-1", &action).await.unwrap();

    let u1 = mailbox.read_unread("t1", "worker-1").await.unwrap();
    assert_eq!(u1.len(), 1);
    let u2 = mailbox.read_unread("t1", "worker-2").await.unwrap();
    assert_eq!(u2.len(), 1);
    let u_lead = mailbox.read_unread("t1", "lead-1").await.unwrap();
    assert!(u_lead.is_empty());
}

// -- execute_action: TaskCreate ------------------------------------------

#[tokio::test]
async fn execute_task_create() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox, task_board.clone(), broadcaster);

    let action = SchedulerAction::TaskCreate {
        subject: "Implement feature".into(),
        description: Some("Details here".into()),
        owner: Some("worker-1".into()),
        blocked_by: vec![],
    };
    mgr.execute_action("lead-1", &action).await.unwrap();

    let tasks = task_board.list_tasks("t1").await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].subject, "Implement feature");
    assert_eq!(tasks[0].owner.as_deref(), Some("worker-1"));
}

// -- execute_action: TaskUpdate ------------------------------------------

#[tokio::test]
async fn execute_task_update() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox, task_board.clone(), broadcaster);

    let task = task_board.create_task("t1", "Work", None, None, &[]).await.unwrap();

    let action = SchedulerAction::TaskUpdate {
        task_id: task.id.clone(),
        status: Some("in_progress".into()),
        description: None,
        owner: None,
        blocked_by: None,
    };
    mgr.execute_action("worker-1", &action).await.unwrap();

    let tasks = task_board.list_tasks("t1").await.unwrap();
    assert_eq!(tasks[0].status, crate::types::TaskStatus::InProgress);
}

// -- execute_action: IdleNotification ------------------------------------

#[tokio::test]
async fn execute_idle_notification_writes_to_lead_mailbox() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    mgr.set_status("worker-1", TeammateStatus::Working).await.unwrap();

    let action = SchedulerAction::IdleNotification {
        summary: Some("Task done".into()),
    };
    mgr.execute_action("worker-1", &action).await.unwrap();

    assert_eq!(mgr.get_status("worker-1").await.unwrap(), TeammateStatus::Idle);

    let lead_msgs = mailbox.read_unread("t1", "lead-1").await.unwrap();
    assert_eq!(lead_msgs.len(), 1);
    assert_eq!(lead_msgs[0].msg_type, MailboxMessageType::IdleNotification);
    assert_eq!(lead_msgs[0].from_agent_id, "worker-1");
}

#[tokio::test]
async fn lead_idle_notification_does_not_write_to_self() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    mgr.set_status("lead-1", TeammateStatus::Working).await.unwrap();

    let action = SchedulerAction::IdleNotification {
        summary: Some("Done delegating".into()),
    };
    mgr.execute_action("lead-1", &action).await.unwrap();

    let lead_msgs = mailbox.read_unread("t1", "lead-1").await.unwrap();
    assert!(lead_msgs.is_empty());
}

// -- execute_action: ShutdownAgent ---------------------------------------

#[tokio::test]
async fn execute_shutdown_agent_writes_shutdown_request() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    let action = SchedulerAction::ShutdownAgent {
        slot_id: "worker-1".into(),
        reason: Some("No longer needed".into()),
    };
    mgr.execute_action("lead-1", &action).await.unwrap();

    let msgs = mailbox.read_unread("t1", "worker-1").await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].msg_type, MailboxMessageType::ShutdownRequest);
    assert_eq!(msgs[0].content, "No longer needed");
}

#[tokio::test]
async fn non_lead_cannot_shutdown_agent() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    let action = SchedulerAction::ShutdownAgent {
        slot_id: "worker-2".into(),
        reason: None,
    };
    let result = mgr.execute_action("worker-1", &action).await;
    assert!(matches!(result, Err(TeamError::InvalidRequest(_))));
}

#[tokio::test]
async fn lead_cannot_shutdown_lead() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    let action = SchedulerAction::ShutdownAgent {
        slot_id: "lead-1".into(),
        reason: Some("trying to shutdown self".into()),
    };
    let result = mgr.execute_action("lead-1", &action).await;
    assert!(
        matches!(&result, Err(TeamError::InvalidRequest(msg)) if msg.contains("lead")),
        "lead shutting down lead must be rejected, got {result:?}"
    );

    // No ShutdownRequest message should have been written to the lead's mailbox.
    let lead_msgs = mailbox.read_unread("t1", "lead-1").await.unwrap();
    assert!(lead_msgs.is_empty());
}

#[tokio::test]
async fn lead_can_shutdown_worker() {
    // Positive-path sanity check that the new target-role guard does not
    // regress the normal shutdown flow.
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    let action = SchedulerAction::ShutdownAgent {
        slot_id: "worker-1".into(),
        reason: Some("not needed".into()),
    };
    mgr.execute_action("lead-1", &action).await.unwrap();

    let msgs = mailbox.read_unread("t1", "worker-1").await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].msg_type, MailboxMessageType::ShutdownRequest);
}

// -- execute_action: RenameAgent -----------------------------------------

#[tokio::test]
async fn execute_rename_agent() {
    let agents = make_team_agents();
    let (mgr, bc) = make_manager(&agents);

    let action = SchedulerAction::RenameAgent {
        slot_id: "worker-1".into(),
        new_name: "SuperWorker".into(),
    };
    mgr.execute_action("lead-1", &action).await.unwrap();

    let agent = mgr.get_agent("worker-1").await.unwrap();
    assert_eq!(agent.name, "SuperWorker");

    let renamed: Vec<_> = bc
        .events()
        .into_iter()
        .filter(|e| e.name == "team.agent.renamed")
        .collect();
    assert_eq!(renamed.len(), 1);
}

// -- finalize_turn -------------------------------------------------------

#[tokio::test]
async fn finalize_turn_executes_actions_and_marks_idle() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board.clone(), broadcaster);

    mgr.set_status("worker-1", TeammateStatus::Working).await.unwrap();
    mgr.set_status("worker-2", TeammateStatus::Working).await.unwrap();

    let actions = vec![
        SchedulerAction::TaskCreate {
            subject: "Sub-task".into(),
            description: None,
            owner: None,
            blocked_by: vec![],
        },
        SchedulerAction::SendMessage {
            to: "lead-1".into(),
            message: "Done with sub-task".into(),
        },
    ];

    let wake_signal = mgr.finalize_turn("worker-1", &actions).await.unwrap();

    assert_eq!(mgr.get_status("worker-1").await.unwrap(), TeammateStatus::Idle);

    let tasks = task_board.list_tasks("t1").await.unwrap();
    assert_eq!(tasks.len(), 1);

    // Two messages arrive at the lead:
    // 1. the explicit SendMessage from the action list ("Done with sub-task")
    // 2. the IdleNotification that mark_idle now writes automatically
    let lead_msgs = mailbox.read_unread("t1", "lead-1").await.unwrap();
    assert_eq!(lead_msgs.len(), 2);
    assert!(
        lead_msgs
            .iter()
            .any(|m| m.msg_type == MailboxMessageType::Message && m.content == "Done with sub-task")
    );
    assert!(
        lead_msgs
            .iter()
            .any(|m| m.msg_type == MailboxMessageType::IdleNotification)
    );

    assert!(wake_signal.is_none(), "worker-2 still working");
}

#[tokio::test]
async fn finalize_turn_with_idle_notification_skips_double_idle() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox, task_board, broadcaster.clone());

    mgr.set_status("worker-1", TeammateStatus::Working).await.unwrap();

    let actions = vec![SchedulerAction::IdleNotification {
        summary: Some("All done".into()),
    }];

    mgr.finalize_turn("worker-1", &actions).await.unwrap();

    assert_eq!(mgr.get_status("worker-1").await.unwrap(), TeammateStatus::Idle);

    let idle_events: Vec<_> = broadcaster
        .events()
        .into_iter()
        .filter(|e| e.name == "team.agent.status" && e.data["status"] == "idle")
        .collect();
    assert_eq!(idle_events.len(), 1, "idle should be set exactly once");
}

#[tokio::test]
async fn finalize_turn_all_teammates_done_signals_leader_wake() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox, task_board, broadcaster);

    mgr.set_status("worker-1", TeammateStatus::Working).await.unwrap();
    mgr.set_status("worker-2", TeammateStatus::Working).await.unwrap();

    mgr.finalize_turn("worker-1", &[]).await.unwrap();

    let wake_signal = mgr.finalize_turn("worker-2", &[]).await.unwrap();
    assert_eq!(wake_signal.as_deref(), Some("lead-1"));
}

// -- build_wake_payload with unread messages and tasks --------------------

#[tokio::test]
async fn wake_payload_includes_tasks_and_unread() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board.clone(), broadcaster);

    task_board.create_task("t1", "Task A", None, None, &[]).await.unwrap();

    mailbox
        .write(
            "t1",
            "worker-1",
            "lead-1",
            MailboxMessageType::Message,
            "Do task A",
            None,
        )
        .await
        .unwrap();

    let payload = mgr.build_wake_payload("worker-1").await.unwrap();
    assert_eq!(payload.tasks.len(), 1);
    assert_eq!(payload.unread_messages.len(), 1);
    assert_eq!(payload.unread_messages[0].content, "Do task A");
}

// -- D8: mark_idle writes IdleNotification with summary -------------------

#[tokio::test]
async fn mark_idle_with_summary_writes_idle_notification_to_lead() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    mgr.set_status("worker-1", TeammateStatus::Working).await.unwrap();
    mgr.mark_idle("worker-1", Some("sub-task done")).await.unwrap();

    let lead_msgs = mailbox.read_unread("t1", "lead-1").await.unwrap();
    assert_eq!(lead_msgs.len(), 1);
    assert_eq!(lead_msgs[0].msg_type, MailboxMessageType::IdleNotification);
    assert_eq!(lead_msgs[0].from_agent_id, "worker-1");
    assert_eq!(lead_msgs[0].content, "sub-task done");
    assert_eq!(lead_msgs[0].summary.as_deref(), Some("sub-task done"));
}

#[tokio::test]
async fn mark_idle_without_summary_still_writes_fallback_content() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    mgr.set_status("worker-1", TeammateStatus::Working).await.unwrap();
    mgr.mark_idle("worker-1", None).await.unwrap();

    let lead_msgs = mailbox.read_unread("t1", "lead-1").await.unwrap();
    assert_eq!(lead_msgs.len(), 1);
    assert_eq!(lead_msgs[0].content, "idle");
    assert!(lead_msgs[0].summary.is_none());
}

#[tokio::test]
async fn mark_idle_from_lead_does_not_write_notification() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    mgr.set_status("lead-1", TeammateStatus::Working).await.unwrap();
    mgr.mark_idle("lead-1", Some("done")).await.unwrap();

    let lead_msgs = mailbox.read_unread("t1", "lead-1").await.unwrap();
    assert!(lead_msgs.is_empty());
}

#[tokio::test]
async fn mark_idle_broadcasts_status_event() {
    let agents = make_team_agents();
    let (mgr, bc) = make_manager(&agents);

    mgr.set_status("worker-1", TeammateStatus::Working).await.unwrap();
    bc.events.lock().unwrap().clear();

    mgr.mark_idle("worker-1", Some("ok")).await.unwrap();

    let idle_events: Vec<_> = bc
        .events()
        .into_iter()
        .filter(|e| e.name == "team.agent.status" && e.data["status"] == "idle")
        .collect();
    assert_eq!(idle_events.len(), 1);
    assert_eq!(idle_events[0].data["slot_id"], "worker-1");
}

// -- D8: settled set expansion ({Idle, Completed, Error}) -----------------

#[tokio::test]
async fn all_teammates_settled_with_completed_wakes_leader() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    mgr.set_status("worker-1", TeammateStatus::Completed).await.unwrap();
    mgr.set_status("worker-2", TeammateStatus::Working).await.unwrap();

    let result = mgr.mark_idle("worker-2", None).await.unwrap();
    assert_eq!(result.as_deref(), Some("lead-1"));
}

#[tokio::test]
async fn all_teammates_settled_with_error_wakes_leader() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    mgr.set_status("worker-1", TeammateStatus::Error).await.unwrap();
    mgr.set_status("worker-2", TeammateStatus::Working).await.unwrap();

    let result = mgr.mark_idle("worker-2", None).await.unwrap();
    assert_eq!(
        result.as_deref(),
        Some("lead-1"),
        "Error counts as settled — leader should be woken"
    );
}

#[tokio::test]
async fn working_teammate_blocks_leader_wake() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    mgr.set_status("worker-1", TeammateStatus::Working).await.unwrap();
    mgr.set_status("worker-2", TeammateStatus::Working).await.unwrap();

    let result = mgr.mark_idle("worker-1", None).await.unwrap();
    assert!(result.is_none(), "worker-2 still Working blocks wake");
}

#[tokio::test]
async fn thinking_teammate_blocks_leader_wake() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    mgr.set_status("worker-1", TeammateStatus::Thinking).await.unwrap();
    mgr.set_status("worker-2", TeammateStatus::Working).await.unwrap();

    let result = mgr.mark_idle("worker-2", None).await.unwrap();
    assert!(result.is_none(), "Thinking is not settled");
}

#[tokio::test]
async fn tool_use_teammate_blocks_leader_wake() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    mgr.set_status("worker-1", TeammateStatus::ToolUse).await.unwrap();
    mgr.set_status("worker-2", TeammateStatus::Working).await.unwrap();

    let result = mgr.mark_idle("worker-2", None).await.unwrap();
    assert!(result.is_none(), "ToolUse is not settled");
}

// -- D8: acquire_wake_lock / release_wake_lock ----------------------------

#[tokio::test]
async fn wake_lock_first_caller_wins_and_release_is_idempotent() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    assert!(mgr.acquire_wake_lock("worker-1"));
    assert!(
        !mgr.acquire_wake_lock("worker-1"),
        "second acquire must fail while lock is held"
    );

    mgr.release_wake_lock("worker-1");
    assert!(mgr.acquire_wake_lock("worker-1"), "lock is reusable after release");

    mgr.release_wake_lock("worker-1");
    mgr.release_wake_lock("worker-1"); // double release is a no-op
}

#[tokio::test]
async fn wake_lock_is_scoped_per_slot() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    assert!(mgr.acquire_wake_lock("worker-1"));
    assert!(mgr.acquire_wake_lock("worker-2"), "different slot must not be blocked");
}

#[tokio::test]
async fn is_wake_active_reflects_lock_state() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    assert!(!mgr.is_wake_active("worker-1"), "lock not held initially");
    assert!(mgr.acquire_wake_lock("worker-1"));
    assert!(mgr.is_wake_active("worker-1"), "lock is held after acquire");
    mgr.release_wake_lock("worker-1");
    assert!(!mgr.is_wake_active("worker-1"), "lock released after release");
}

// -- W4-D19a: finalize-turn dedup ----------------------------------------

#[tokio::test]
async fn begin_finalize_first_call_returns_true() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    assert!(mgr.begin_finalize("conv-worker-1"));
}

#[tokio::test]
async fn begin_finalize_within_window_returns_false() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    assert!(mgr.begin_finalize("conv-worker-1"));
    assert!(
        !mgr.begin_finalize("conv-worker-1"),
        "second finalize within 5s window must be deduped"
    );
}

#[tokio::test]
async fn clear_finalized_turn_allows_immediate_retry() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    assert!(mgr.begin_finalize("conv-worker-1"));
    mgr.clear_finalized_turn("conv-worker-1");
    assert!(
        mgr.begin_finalize("conv-worker-1"),
        "clearing the dedup entry must let the next finalize proceed"
    );
}

#[tokio::test]
async fn wake_lock_concurrent_acquire_exactly_one_succeeds() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);
    let mgr = Arc::new(mgr);

    let mut handles = vec![];
    for _ in 0..16 {
        let mgr = mgr.clone();
        handles.push(tokio::spawn(async move { mgr.acquire_wake_lock("worker-1") }));
    }

    let mut winners = 0usize;
    for h in handles {
        if h.await.unwrap() {
            winners += 1;
        }
    }
    assert_eq!(winners, 1, "exactly one concurrent acquire should win the lock");
}

// -- W4-D18b: wake_timeouts -------------------------------------------------

#[tokio::test]
async fn clear_wake_timeout_removes_entry() {
    let handle = tokio::spawn(async { tokio::time::sleep(std::time::Duration::from_secs(999)).await });
    let map: DashMap<String, tokio::task::JoinHandle<()>> = DashMap::new();
    map.insert("slot-1".into(), handle);
    // Simulate clear
    if let Some((_, h)) = map.remove("slot-1") {
        h.abort();
    }
    assert!(map.get("slot-1").is_none());
}

#[test]
fn clear_nonexistent_slot_no_panic() {
    let map: DashMap<String, tokio::task::JoinHandle<()>> = DashMap::new();
    // Should not panic
    map.remove("nonexistent");
}

// -- W4-D18b-2: arm_wake_timeout --------------------------------------------

use std::sync::atomic::{AtomicU32, Ordering};

fn counting_handler(counter: Arc<AtomicU32>) -> WakeTimeoutHandler {
    Arc::new(move |_slot_id: String| {
        let c = counter.clone();
        Box::pin(async move {
            c.fetch_add(1, Ordering::SeqCst);
        })
    })
}

async fn wait_for_map_empty(mgr: &TeammateManager, slot_id: &str, ticks: u32) {
    for _ in 0..ticks {
        if mgr.wake_timeouts.get(slot_id).is_none() {
            return;
        }
        tokio::task::yield_now().await;
    }
}

/// Yield repeatedly so a freshly spawned watchdog task gets a chance to
/// reach its `select!` (and arm its sleep) before the test advances time.
async fn let_watchdog_settle() {
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn arm_wake_timeout_fires_handler_after_deadline() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);
    let counter = Arc::new(AtomicU32::new(0));
    let (tx, rx) = broadcast::channel::<AgentStreamEvent>(8);

    mgr.arm_wake_timeout("worker-1", rx, counting_handler(counter.clone()));
    // Keep the sender alive so the channel does not close before the deadline.
    let_watchdog_settle().await;
    // Advance slightly past the deadline — handler must fire exactly once.
    tokio::time::advance(Duration::from_millis(WAKE_TIMEOUT_MS + 500)).await;
    wait_for_map_empty(&mgr, "worker-1", 128).await;

    assert_eq!(counter.load(Ordering::SeqCst), 1, "handler must fire on inactivity");
    assert!(
        mgr.wake_timeouts.get("worker-1").is_none(),
        "map entry must be cleared after watchdog exit"
    );
    drop(tx);
}

#[tokio::test(start_paused = true)]
async fn arm_wake_timeout_activity_resets_deadline() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);
    let counter = Arc::new(AtomicU32::new(0));
    let (tx, rx) = broadcast::channel::<AgentStreamEvent>(8);

    mgr.arm_wake_timeout("worker-1", rx, counting_handler(counter.clone()));
    let_watchdog_settle().await;

    // Just before the first deadline, an activity chunk arrives.
    tokio::time::advance(Duration::from_millis(WAKE_TIMEOUT_MS - 1_000)).await;
    tx.send(AgentStreamEvent::Text(TextEventData { content: "hi".into() }))
        .unwrap();
    // Let the select branch observe the chunk before advancing again.
    tokio::task::yield_now().await;

    // Advance another near-full window — deadline should have been reset,
    // so no timeout has fired yet.
    tokio::time::advance(Duration::from_millis(WAKE_TIMEOUT_MS - 1_000)).await;
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "activity must reset deadline; handler must not have fired"
    );

    // Cross the new deadline — handler fires.
    tokio::time::advance(Duration::from_millis(2_000)).await;
    wait_for_map_empty(&mgr, "worker-1", 128).await;
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    drop(tx);
}

#[tokio::test(start_paused = true)]
async fn arm_wake_timeout_finish_exits_without_firing() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);
    let counter = Arc::new(AtomicU32::new(0));
    let (tx, rx) = broadcast::channel::<AgentStreamEvent>(8);

    mgr.arm_wake_timeout("worker-1", rx, counting_handler(counter.clone()));
    let_watchdog_settle().await;

    tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();
    wait_for_map_empty(&mgr, "worker-1", 128).await;

    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "Finish must not trigger timeout handler"
    );
    assert!(
        mgr.wake_timeouts.get("worker-1").is_none(),
        "map entry must be cleared after Finish"
    );

    // Advance past the would-be deadline to make sure no lingering timer fires.
    tokio::time::advance(Duration::from_millis(WAKE_TIMEOUT_MS * 2)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 0);
    drop(tx);
}

#[tokio::test(start_paused = true)]
async fn arm_wake_timeout_channel_close_exits_without_firing() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);
    let counter = Arc::new(AtomicU32::new(0));
    let (tx, rx) = broadcast::channel::<AgentStreamEvent>(8);

    mgr.arm_wake_timeout("worker-1", rx, counting_handler(counter.clone()));
    let_watchdog_settle().await;
    drop(tx);
    wait_for_map_empty(&mgr, "worker-1", 128).await;

    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "closed channel must not fire inactivity handler"
    );
    assert!(mgr.wake_timeouts.get("worker-1").is_none());
}

#[tokio::test(start_paused = true)]
async fn arm_wake_timeout_replaces_existing_watchdog() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);
    let counter_a = Arc::new(AtomicU32::new(0));
    let counter_b = Arc::new(AtomicU32::new(0));

    let (tx_a, rx_a) = broadcast::channel::<AgentStreamEvent>(8);
    mgr.arm_wake_timeout("worker-1", rx_a, counting_handler(counter_a.clone()));

    // Immediately re-arm — the first watchdog must be aborted.
    let (tx_b, rx_b) = broadcast::channel::<AgentStreamEvent>(8);
    mgr.arm_wake_timeout("worker-1", rx_b, counting_handler(counter_b.clone()));
    let_watchdog_settle().await;

    // Cross the deadline. Only one handler (the second watchdog's) may fire.
    tokio::time::advance(Duration::from_millis(WAKE_TIMEOUT_MS + 500)).await;
    wait_for_map_empty(&mgr, "worker-1", 128).await;

    assert_eq!(counter_a.load(Ordering::SeqCst), 0, "aborted watchdog must not fire");
    assert_eq!(counter_b.load(Ordering::SeqCst), 1, "replacement watchdog must fire");
    drop(tx_a);
    drop(tx_b);
}

// -- W4-D20b1: crash testament formatting -----------------------------------

#[test]
fn crash_testament_contains_reason_keyword() {
    use crate::crash_detection::CrashReason;

    for (reason, keyword) in [
        (CrashReason::ProcessExited, "ProcessExited"),
        (CrashReason::SessionNotFound, "SessionNotFound"),
        (CrashReason::Unknown("segfault".into()), "Unknown — segfault"),
    ] {
        let testament = format_crash_testament("Bob", &reason, None);
        assert!(
            testament.contains(keyword),
            "expected '{}' in testament: {}",
            keyword,
            testament
        );
    }
}

#[test]
fn crash_testament_includes_last_message_when_provided() {
    use crate::crash_detection::CrashReason;

    let testament = format_crash_testament("Alice", &CrashReason::ProcessExited, Some("working on task X"));
    assert!(testament.contains("Last message: working on task X"));
    assert!(testament.contains("ProcessExited"));
    assert!(testament.contains("Alice"));
}

#[test]
fn crash_testament_omits_last_message_when_none() {
    use crate::crash_detection::CrashReason;

    let testament = format_crash_testament("Charlie", &CrashReason::SessionNotFound, None);
    assert!(!testament.contains("Last message"));
    assert!(testament.contains("SessionNotFound"));
    assert!(testament.contains("Charlie"));
}

#[tokio::test]
async fn write_crash_testament_delivers_to_lead_mailbox() {
    use crate::crash_detection::CrashReason;

    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    mgr.write_crash_testament("worker-1", "Worker1", &CrashReason::ProcessExited, None)
        .await
        .unwrap();

    let lead_msgs = mailbox.read_unread("t1", "lead-1").await.unwrap();
    assert_eq!(lead_msgs.len(), 1);
    assert_eq!(lead_msgs[0].from_agent_id, "worker-1");
    assert!(lead_msgs[0].content.contains("ProcessExited"));
    assert!(lead_msgs[0].content.contains("Worker1"));
}

#[tokio::test]
async fn write_crash_testament_noop_when_no_lead() {
    use crate::crash_detection::CrashReason;

    // Team with no lead
    let agents = vec![
        make_agent("worker-1", "Worker1", TeammateRole::Teammate),
        make_agent("worker-2", "Worker2", TeammateRole::Teammate),
    ];
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    // Should not panic or error
    mgr.write_crash_testament("worker-1", "Worker1", &CrashReason::SessionNotFound, Some("last words"))
        .await
        .unwrap();

    // No messages delivered to anyone
    let msgs1 = mailbox.read_unread("t1", "worker-1").await.unwrap();
    let msgs2 = mailbox.read_unread("t1", "worker-2").await.unwrap();
    assert!(msgs1.is_empty());
    assert!(msgs2.is_empty());
}

#[tokio::test]
async fn write_crash_testament_noop_when_lead_crashes() {
    use crate::crash_detection::CrashReason;

    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    // Lead crashing should not write to itself
    mgr.write_crash_testament("lead-1", "Lead", &CrashReason::ProcessExited, None)
        .await
        .unwrap();

    let lead_msgs = mailbox.read_unread("t1", "lead-1").await.unwrap();
    assert!(lead_msgs.is_empty());
}

// -- W4-D20b-2: handle_agent_crash -----------------------------------------

#[tokio::test]
async fn handle_agent_crash_marks_slot_as_error() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    mgr.set_status("worker-1", TeammateStatus::Working).await.unwrap();

    let wake_target = mgr
        .handle_agent_crash("worker-1", CrashReason::ProcessExited, None)
        .await
        .unwrap();

    assert_eq!(
        mgr.get_status("worker-1").await.unwrap(),
        TeammateStatus::Error,
        "crashed slot must end in Error (aka Failed)"
    );
    assert_eq!(wake_target, Some("lead-1".to_string()));
}

#[tokio::test]
async fn handle_agent_crash_releases_wake_lock() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    assert!(mgr.acquire_wake_lock("worker-1"));

    mgr.handle_agent_crash("worker-1", CrashReason::SessionNotFound, Some("last words"))
        .await
        .unwrap();

    assert!(
        mgr.acquire_wake_lock("worker-1"),
        "wake lock must be released after crash so the slot is reusable"
    );
}

#[tokio::test]
async fn handle_agent_crash_writes_testament_to_lead() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    mgr.handle_agent_crash("worker-1", CrashReason::Unknown("segfault".into()), Some("cleaning up"))
        .await
        .unwrap();

    let lead_msgs = mailbox.read_unread("t1", "lead-1").await.unwrap();
    assert_eq!(lead_msgs.len(), 1);
    assert_eq!(lead_msgs[0].from_agent_id, "worker-1");
    assert!(lead_msgs[0].content.contains("Worker1"));
    assert!(lead_msgs[0].content.contains("segfault"));
    assert!(lead_msgs[0].content.contains("cleaning up"));
}

#[tokio::test]
async fn handle_agent_crash_returns_lead_slot_for_teammate_crash() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    let wake_target = mgr
        .handle_agent_crash("worker-2", CrashReason::ProcessExited, None)
        .await
        .unwrap();

    assert_eq!(
        wake_target,
        Some("lead-1".to_string()),
        "caller needs the lead slot id to trigger a wake"
    );
}

// -- W4-D20c: handle_agent_crash leader branch -----------------------------

#[tokio::test]
async fn handle_agent_crash_leader_branch_returns_none() {
    // Leader crash has no higher-ranked agent to wake. handle_agent_crash
    // must not self-wake and must not remove the leader slot — downstream
    // session code inspects the leader entry to emit the error event.
    // Local state (status/locks) still gets cleaned so nothing leaks.
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    mgr.set_status("lead-1", TeammateStatus::Working).await.unwrap();
    assert!(mgr.acquire_wake_lock("lead-1"));

    let wake_target = mgr
        .handle_agent_crash("lead-1", CrashReason::ProcessExited, Some("last words"))
        .await
        .unwrap();

    assert_eq!(wake_target, None, "leader crash must not self-wake");
    assert_eq!(mgr.get_status("lead-1").await.unwrap(), TeammateStatus::Error);
    assert!(
        mgr.acquire_wake_lock("lead-1"),
        "lock must be released even for the leader branch"
    );

    // Leader cannot write a testament to itself — the mailbox must be
    // empty, otherwise the leader would read its own death notice on a
    // future resume.
    let lead_msgs = mailbox.read_unread("t1", "lead-1").await.unwrap();
    assert!(
        lead_msgs.is_empty(),
        "leader crash must not produce a self-addressed testament"
    );
}

#[tokio::test]
async fn handle_agent_crash_leader_keeps_agents_list_intact() {
    // Leader crash must not remove slots from the roster — the session
    // layer still needs to enumerate teammates to emit finalization /
    // error events.
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    let before: Vec<String> = mgr.list_agents().await.into_iter().map(|a| a.slot_id).collect();

    mgr.handle_agent_crash("lead-1", CrashReason::SessionNotFound, None)
        .await
        .unwrap();

    let after: Vec<String> = mgr.list_agents().await.into_iter().map(|a| a.slot_id).collect();

    assert_eq!(before, after, "leader crash must preserve the agents list");
}

#[tokio::test]
async fn handle_agent_crash_leader_clears_wake_timeout() {
    // Pending wake timeouts for the leader must be cancelled on crash —
    // the slot will never answer, so a lingering timer only risks a late
    // spurious callback.
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    let handle = tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(999)).await;
    });
    mgr.wake_timeouts.insert("lead-1".into(), handle);

    mgr.handle_agent_crash("lead-1", CrashReason::ProcessExited, None)
        .await
        .unwrap();

    assert!(
        mgr.wake_timeouts.get("lead-1").is_none(),
        "wake timeout entry must be removed after leader crash"
    );
}

#[tokio::test]
async fn handle_agent_crash_clears_wake_timeout() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    // Install a long-running dummy timeout so we can observe that it was
    // cancelled once the crash handler ran.
    let handle = tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(999)).await;
    });
    mgr.wake_timeouts.insert("worker-1".into(), handle);

    mgr.handle_agent_crash("worker-1", CrashReason::ProcessExited, None)
        .await
        .unwrap();

    assert!(
        mgr.wake_timeouts.get("worker-1").is_none(),
        "wake timeout entry must be removed after crash"
    );
}

#[tokio::test]
async fn handle_agent_crash_unknown_slot_errors() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    let result = mgr.handle_agent_crash("ghost", CrashReason::ProcessExited, None).await;

    assert!(matches!(result, Err(TeamError::AgentNotFound(_))));
}

// -- W4-D22: handle_inactivity_timeout -------------------------------------

#[tokio::test]
async fn handle_inactivity_timeout_teammate_marks_error_and_wakes_lead() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    mgr.set_status("worker-1", TeammateStatus::Working).await.unwrap();

    let wake_target = mgr.handle_inactivity_timeout("worker-1").await.unwrap();

    assert_eq!(
        mgr.get_status("worker-1").await.unwrap(),
        TeammateStatus::Error,
        "stuck slot must end in Error"
    );
    assert_eq!(wake_target, Some("lead-1".to_string()));

    let lead_msgs = mailbox.read_unread("t1", "lead-1").await.unwrap();
    assert_eq!(lead_msgs.len(), 1);
    assert_eq!(lead_msgs[0].from_agent_id, "worker-1");
    assert_eq!(lead_msgs[0].msg_type, MailboxMessageType::Message);
    assert!(lead_msgs[0].content.contains("Worker1"));
    assert!(lead_msgs[0].content.contains("timed out"));
}

#[tokio::test]
async fn handle_inactivity_timeout_leader_returns_none_no_mailbox_write() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    mgr.set_status("lead-1", TeammateStatus::Working).await.unwrap();
    assert!(mgr.acquire_wake_lock("lead-1"));

    let wake_target = mgr.handle_inactivity_timeout("lead-1").await.unwrap();

    assert_eq!(wake_target, None, "leader inactivity must not self-wake");
    assert_eq!(mgr.get_status("lead-1").await.unwrap(), TeammateStatus::Error);
    assert!(
        mgr.acquire_wake_lock("lead-1"),
        "lock must be released even when leader stuck"
    );

    let lead_msgs = mailbox.read_unread("t1", "lead-1").await.unwrap();
    assert!(
        lead_msgs.is_empty(),
        "leader must not receive a self-addressed timeout message"
    );
}

#[tokio::test]
async fn handle_inactivity_timeout_releases_wake_lock() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    assert!(mgr.acquire_wake_lock("worker-1"));

    mgr.handle_inactivity_timeout("worker-1").await.unwrap();

    assert!(
        mgr.acquire_wake_lock("worker-1"),
        "wake lock must be released after inactivity timeout"
    );
}

#[tokio::test]
async fn handle_inactivity_timeout_clears_wake_timeout_entry() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    let handle = tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(999)).await;
    });
    mgr.wake_timeouts.insert("worker-1".into(), handle);

    mgr.handle_inactivity_timeout("worker-1").await.unwrap();

    assert!(
        mgr.wake_timeouts.get("worker-1").is_none(),
        "wake timeout entry must be removed after inactivity recovery"
    );
}

#[tokio::test]
async fn handle_inactivity_timeout_unknown_slot_errors() {
    let agents = make_team_agents();
    let (mgr, _) = make_manager(&agents);

    let result = mgr.handle_inactivity_timeout("ghost").await;

    assert!(matches!(result, Err(TeamError::AgentNotFound(_))));
}

#[tokio::test]
async fn handle_inactivity_timeout_no_lead_returns_none() {
    // Team with no lead: a stuck teammate has nowhere to route the
    // diagnostic message. The handler must still clean local state
    // and must not panic or return an error.
    let agents = vec![
        make_agent("worker-1", "Worker1", TeammateRole::Teammate),
        make_agent("worker-2", "Worker2", TeammateRole::Teammate),
    ];
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    let wake_target = mgr.handle_inactivity_timeout("worker-1").await.unwrap();

    assert_eq!(wake_target, None);
    assert_eq!(mgr.get_status("worker-1").await.unwrap(), TeammateStatus::Error);

    let msgs2 = mailbox.read_unread("t1", "worker-2").await.unwrap();
    assert!(msgs2.is_empty());
}

// -- W5-D30b: notify_shutdown_rejected -------------------------------------

#[tokio::test]
async fn notify_shutdown_rejected_delivers_to_lead_mailbox() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    mgr.notify_shutdown_rejected("worker-1", "still working on task X")
        .await
        .unwrap();

    let lead_msgs = mailbox.read_unread("t1", "lead-1").await.unwrap();
    assert_eq!(lead_msgs.len(), 1);
    assert_eq!(lead_msgs[0].from_agent_id, "worker-1");
    assert!(lead_msgs[0].content.contains("Worker1"));
    assert!(lead_msgs[0].content.contains("declined shutdown"));
    assert!(lead_msgs[0].content.contains("still working on task X"));

    // Agent was not removed
    assert!(mgr.get_agent("worker-1").await.is_ok());
}

#[tokio::test]
async fn notify_shutdown_rejected_noop_when_no_lead() {
    let agents = vec![
        make_agent("worker-1", "Worker1", TeammateRole::Teammate),
        make_agent("worker-2", "Worker2", TeammateRole::Teammate),
    ];
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    mgr.notify_shutdown_rejected("worker-1", "busy").await.unwrap();

    let msgs1 = mailbox.read_unread("t1", "worker-1").await.unwrap();
    let msgs2 = mailbox.read_unread("t1", "worker-2").await.unwrap();
    assert!(msgs1.is_empty());
    assert!(msgs2.is_empty());
}

#[tokio::test]
async fn notify_shutdown_rejected_noop_when_sender_is_lead() {
    let agents = make_team_agents();
    let repo = Arc::new(MockTeamRepo::new());
    let mailbox = Arc::new(Mailbox::new(repo.clone()));
    let task_board = Arc::new(TaskBoard::new(repo));
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(RecordingBroadcaster::new());
    let mgr = TeammateManager::new("t1".into(), &agents, mailbox.clone(), task_board, broadcaster);

    mgr.notify_shutdown_rejected("lead-1", "irrelevant").await.unwrap();

    let lead_msgs = mailbox.read_unread("t1", "lead-1").await.unwrap();
    assert!(lead_msgs.is_empty());
}
