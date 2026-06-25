//! Black-box integration tests for `ITeamRepository`.
//!
//! Tests exercise the repository trait interface without knowledge of
//! the underlying SQLite implementation details.
//!
//! Reworked for the primary-key redesign (spec §5.4/§5.5): the `teams.agents`
//! JSON array is now the `team_agents` table; `team_tasks.blocked_by`/`blocks`
//! JSON arrays are now the `team_task_deps` edge table; `mailbox.id` is an i64
//! autoincrement key. `delete_team` relies on FK CASCADE (no
//! `delete_mailbox_by_team` / `delete_tasks_by_team` helpers).

use std::sync::Arc;

use nomifun_common::now_ms;
use nomifun_db::models::{MailboxMessageRow, TeamAgentRow, TeamRow, TeamTaskRow};
use nomifun_db::{
    DbError, ITeamRepository, SqliteTeamRepository, UpdateTaskParams, UpdateTeamParams, init_database_memory,
};

/// Builds a repo over an in-memory DB seeded with a conversation so the
/// `team_agents.conversation_id` FK (CASCADE) holds.
///
/// `system_default_user` (and the 20 built-in agents) are already seeded by
/// `init_database_memory` via `ensure_system_user`, so we only add the slot
/// conversation that `make_agent` references.
async fn repo() -> (Arc<dyn ITeamRepository>, nomifun_db::Database) {
    let db = init_database_memory().await.unwrap();
    let r = Arc::new(SqliteTeamRepository::new(db.pool().clone()));
    sqlx::query(
        "INSERT INTO conversations (id, user_id, name, type, created_at, updated_at) \
         VALUES (1, 'system_default_user', 'Slot Conv', 'normal', 0, 0)",
    )
    .execute(db.pool())
    .await
    .unwrap();
    (r as Arc<dyn ITeamRepository>, db)
}

fn make_team(id: &str, name: &str) -> TeamRow {
    let now = now_ms();
    TeamRow {
        id: id.into(),
        user_id: "system_default_user".into(),
        name: name.into(),
        workspace: String::new(),
        workspace_mode: "shared".into(),
        lead_agent_id: Some("a1".into()),
        session_mode: None,
        agents_version: "1.0.1".into(),
        created_at: now,
        updated_at: now,
    }
}

fn make_agent(slot_id: &str, team_id: &str, name: &str, sort_order: i64) -> TeamAgentRow {
    TeamAgentRow {
        slot_id: slot_id.into(),
        team_id: team_id.into(),
        name: name.into(),
        role: "teammate".into(),
        conversation_id: Some(1),
        backend: "claude".into(),
        model: String::new(),
        custom_agent_id: None,
        status: None,
        conversation_type: None,
        cli_path: None,
        sort_order,
    }
}

fn make_mailbox_msg(team_id: &str, to: &str, from: &str, msg_type: &str) -> MailboxMessageRow {
    MailboxMessageRow {
        id: 0, // ignored on insert (INTEGER PRIMARY KEY AUTOINCREMENT)
        team_id: team_id.into(),
        to_agent_id: to.into(),
        from_agent_id: from.into(),
        msg_type: msg_type.into(),
        content: "content".into(),
        summary: None,
        files: None,
        read: false,
        created_at: now_ms(),
    }
}

fn make_task(id: &str, team_id: &str, subject: &str) -> TeamTaskRow {
    let now = now_ms();
    TeamTaskRow {
        id: id.into(),
        team_id: team_id.into(),
        subject: subject.into(),
        description: None,
        status: "pending".into(),
        owner: None,
        metadata: None,
        created_at: now,
        updated_at: now,
    }
}

// ── Team CRUD Tests ──────────────────────────────────────────────────

#[tokio::test]
async fn create_and_get_team() {
    let (repo, _db) = repo().await;
    let team = make_team("t1", "Team Alpha");
    repo.create_team(&team).await.unwrap();

    let fetched = repo.get_team("t1").await.unwrap().expect("team exists");
    assert_eq!(fetched.id, "t1");
    assert_eq!(fetched.name, "Team Alpha");
    assert_eq!(fetched.lead_agent_id, Some("a1".into()));
}

#[tokio::test]
async fn get_nonexistent_team_returns_none() {
    let (repo, _db) = repo().await;
    let result = repo.get_team("nonexistent").await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn list_teams_empty() {
    let (repo, _db) = repo().await;
    let teams = repo.list_teams().await.unwrap();
    assert!(teams.is_empty());
}

#[tokio::test]
async fn list_teams_multiple() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Alpha")).await.unwrap();
    repo.create_team(&make_team("t2", "Beta")).await.unwrap();

    let teams = repo.list_teams().await.unwrap();
    assert_eq!(teams.len(), 2);
    assert_eq!(teams[0].id, "t1");
    assert_eq!(teams[1].id, "t2");
}

#[tokio::test]
async fn update_team_name() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Old Name")).await.unwrap();

    repo.update_team(
        "t1",
        &UpdateTeamParams {
            name: Some("New Name".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let team = repo.get_team("t1").await.unwrap().unwrap();
    assert_eq!(team.name, "New Name");
}

#[tokio::test]
async fn update_team_lead_agent() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    repo.update_team(
        "t1",
        &UpdateTeamParams {
            lead_agent_id: Some("slot_new".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let team = repo.get_team("t1").await.unwrap().unwrap();
    assert_eq!(team.lead_agent_id.as_deref(), Some("slot_new"));
}

#[tokio::test]
async fn update_nonexistent_team_returns_not_found() {
    let (repo, _db) = repo().await;
    let result = repo
        .update_team(
            "nonexistent",
            &UpdateTeamParams {
                name: Some("X".into()),
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(result, Err(DbError::NotFound(_))));
}

#[tokio::test]
async fn delete_team() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();
    repo.delete_team("t1").await.unwrap();

    let result = repo.get_team("t1").await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn delete_nonexistent_team_returns_not_found() {
    let (repo, _db) = repo().await;
    let result = repo.delete_team("nonexistent").await;
    assert!(matches!(result, Err(DbError::NotFound(_))));
}

// ── Team Agents Tests (was teams.agents JSON array) ──────────────────

#[tokio::test]
async fn create_list_and_order_team_agents() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    repo.create_team_agent(&make_agent("a2", "t1", "Builder", 1)).await.unwrap();
    repo.create_team_agent(&make_agent("a1", "t1", "Lead", 0)).await.unwrap();

    let agents = repo.list_team_agents("t1").await.unwrap();
    assert_eq!(agents.len(), 2);
    // Ordered by sort_order ascending: a1 (0) before a2 (1).
    assert_eq!(agents[0].slot_id, "a1");
    assert_eq!(agents[1].slot_id, "a2");
    assert_eq!(agents[0].conversation_id, Some(1));
}

#[tokio::test]
async fn get_rename_and_remove_team_agent() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();
    repo.create_team_agent(&make_agent("a1", "t1", "Lead", 0)).await.unwrap();

    let one = repo.get_team_agent("a1").await.unwrap().expect("agent exists");
    assert_eq!(one.name, "Lead");

    repo.rename_team_agent("a1", "Architect").await.unwrap();
    let renamed = repo.get_team_agent("a1").await.unwrap().unwrap();
    assert_eq!(renamed.name, "Architect");

    repo.remove_team_agent("a1").await.unwrap();
    assert!(repo.get_team_agent("a1").await.unwrap().is_none());
    assert!(repo.list_team_agents("t1").await.unwrap().is_empty());
}

#[tokio::test]
async fn rename_nonexistent_agent_returns_not_found() {
    let (repo, _db) = repo().await;
    let result = repo.rename_team_agent("nope", "X").await;
    assert!(matches!(result, Err(DbError::NotFound(_))));
}

// ── Mailbox Tests ────────────────────────────────────────────────────

#[tokio::test]
async fn write_message_returns_autoincrement_i64_id() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    let id1 = repo.write_message(&make_mailbox_msg("t1", "a1", "a2", "message")).await.unwrap();
    let id2 = repo.write_message(&make_mailbox_msg("t1", "a1", "a2", "message")).await.unwrap();
    assert!(id1 > 0);
    assert!(id2 > id1);
}

#[tokio::test]
async fn write_and_read_unread_messages() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    for _ in 1..=3 {
        repo.write_message(&make_mailbox_msg("t1", "a1", "a2", "message")).await.unwrap();
    }

    let unread = repo.read_unread_and_mark("t1", "a1").await.unwrap();
    assert_eq!(unread.len(), 3);
    assert!(!unread[0].read); // returned rows reflect pre-mark state
    assert_eq!(unread[0].msg_type, "message");

    let unread2 = repo.read_unread_and_mark("t1", "a1").await.unwrap();
    assert!(unread2.is_empty());
}

#[tokio::test]
async fn peek_and_mark_read_batch_by_id() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    let id1 = repo.write_message(&make_mailbox_msg("t1", "a1", "a2", "message")).await.unwrap();
    let id2 = repo.write_message(&make_mailbox_msg("t1", "a1", "a2", "message")).await.unwrap();

    // mark_read_batch now takes &[i64].
    repo.mark_read_batch(&[id1]).await.unwrap();
    let unread = repo.peek_unread("t1", "a1").await.unwrap();
    assert_eq!(unread.len(), 1);
    assert_eq!(unread[0].id, id2);
}

#[tokio::test]
async fn read_unread_no_messages() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    let unread = repo.read_unread_and_mark("t1", "a1").await.unwrap();
    assert!(unread.is_empty());
}

#[tokio::test]
async fn write_idle_notification_with_summary() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    let mut msg = make_mailbox_msg("t1", "a1", "a2", "idle_notification");
    msg.summary = Some("Task completed".into());
    repo.write_message(&msg).await.unwrap();

    let history = repo.get_history("t1", "a1", None).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].msg_type, "idle_notification");
    assert_eq!(history[0].summary.as_deref(), Some("Task completed"));
}

#[tokio::test]
async fn write_shutdown_request() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    repo.write_message(&make_mailbox_msg("t1", "a1", "a2", "shutdown_request")).await.unwrap();

    let history = repo.get_history("t1", "a1", None).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].msg_type, "shutdown_request");
}

#[tokio::test]
async fn get_history_with_limit() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    for _ in 1..=10 {
        repo.write_message(&make_mailbox_msg("t1", "a1", "a2", "message")).await.unwrap();
    }

    let history = repo.get_history("t1", "a1", Some(5)).await.unwrap();
    assert_eq!(history.len(), 5);
}

#[tokio::test]
async fn get_history_no_limit() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    for _ in 1..=3 {
        repo.write_message(&make_mailbox_msg("t1", "a1", "a2", "message")).await.unwrap();
    }

    let history = repo.get_history("t1", "a1", None).await.unwrap();
    assert_eq!(history.len(), 3);
}

#[tokio::test]
async fn get_history_empty() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    let history = repo.get_history("t1", "a1", None).await.unwrap();
    assert!(history.is_empty());
}

#[tokio::test]
async fn get_history_includes_read_messages() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    repo.write_message(&make_mailbox_msg("t1", "a1", "a2", "message")).await.unwrap();
    repo.read_unread_and_mark("t1", "a1").await.unwrap();

    let history = repo.get_history("t1", "a1", None).await.unwrap();
    assert_eq!(history.len(), 1);
    assert!(history[0].read);
}

// ── Task Board Tests ─────────────────────────────────────────────────

#[tokio::test]
async fn create_and_list_tasks() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    repo.create_task(&make_task("tk1", "t1", "Implement feature")).await.unwrap();

    let tasks = repo.list_tasks("t1").await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].subject, "Implement feature");
    assert_eq!(tasks[0].status, "pending");
}

#[tokio::test]
async fn list_tasks_empty() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    let tasks = repo.list_tasks("t1").await.unwrap();
    assert!(tasks.is_empty());
}

#[tokio::test]
async fn find_task_by_id() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    repo.create_task(&make_task("tk1", "t1", "Task")).await.unwrap();

    let found = repo.find_task_by_id("t1", "tk1").await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, "tk1");
}

#[tokio::test]
async fn find_task_by_id_not_found() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    let found = repo.find_task_by_id("t1", "nonexistent").await.unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn update_task_status() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    repo.create_task(&make_task("tk1", "t1", "Task")).await.unwrap();

    repo.update_task(
        "tk1",
        &UpdateTaskParams {
            status: Some("in_progress".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let updated = repo.find_task_by_id("t1", "tk1").await.unwrap().unwrap();
    assert_eq!(updated.status, "in_progress");
}

#[tokio::test]
async fn update_task_description_and_owner() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    repo.create_task(&make_task("tk1", "t1", "Task")).await.unwrap();

    repo.update_task(
        "tk1",
        &UpdateTaskParams {
            description: Some("New description".into()),
            owner: Some("agent-2".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let updated = repo.find_task_by_id("t1", "tk1").await.unwrap().unwrap();
    assert_eq!(updated.description.as_deref(), Some("New description"));
    assert_eq!(updated.owner.as_deref(), Some("agent-2"));
}

#[tokio::test]
async fn update_nonexistent_task_returns_not_found() {
    let (repo, _db) = repo().await;
    let result = repo
        .update_task(
            "nonexistent",
            &UpdateTaskParams {
                status: Some("completed".into()),
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(result, Err(DbError::NotFound(_))));
}

// ── Task Dependency Tests (was blocked_by/blocks JSON arrays) ────────

#[tokio::test]
async fn add_and_remove_task_dep() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();
    // Both task rows must exist for the team_task_deps FK.
    repo.create_task(&make_task("tkA", "t1", "Task A")).await.unwrap();
    repo.create_task(&make_task("tkB", "t1", "Task B")).await.unwrap();

    // tkA blocks tkB.
    repo.add_task_dep("tkA", "tkB").await.unwrap();

    // "what tkA blocks" and "who blocks tkB".
    assert_eq!(repo.list_blocking("tkA").await.unwrap(), vec!["tkB".to_string()]);
    assert_eq!(repo.list_blockers("tkB").await.unwrap(), vec!["tkA".to_string()]);

    // Completing tkA removes the edge.
    repo.remove_task_dep("tkA", "tkB").await.unwrap();
    assert!(repo.list_blockers("tkB").await.unwrap().is_empty());
}

#[tokio::test]
async fn add_task_dep_idempotent() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();
    repo.create_task(&make_task("tkA", "t1", "A")).await.unwrap();
    repo.create_task(&make_task("tkB", "t1", "B")).await.unwrap();

    repo.add_task_dep("tkA", "tkB").await.unwrap();
    repo.add_task_dep("tkA", "tkB").await.unwrap();

    // INSERT OR IGNORE on the composite PK: no duplicate edge.
    assert_eq!(repo.list_blocking("tkA").await.unwrap(), vec!["tkB".to_string()]);
}

#[tokio::test]
async fn multi_dependency_unblock() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();
    repo.create_task(&make_task("tkA", "t1", "A")).await.unwrap();
    repo.create_task(&make_task("tkB", "t1", "B")).await.unwrap();
    repo.create_task(&make_task("tkC", "t1", "C")).await.unwrap();

    // tkA blocks both tkB and tkC.
    repo.add_task_dep("tkA", "tkB").await.unwrap();
    repo.add_task_dep("tkA", "tkC").await.unwrap();

    // Completing A unblocks both.
    repo.remove_task_dep("tkA", "tkB").await.unwrap();
    repo.remove_task_dep("tkA", "tkC").await.unwrap();

    assert!(repo.list_blockers("tkB").await.unwrap().is_empty());
    assert!(repo.list_blockers("tkC").await.unwrap().is_empty());
}

#[tokio::test]
async fn partial_unblock_preserves_other_blockers() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();
    repo.create_task(&make_task("tkA", "t1", "A")).await.unwrap();
    repo.create_task(&make_task("tkX", "t1", "X")).await.unwrap();
    repo.create_task(&make_task("tkB", "t1", "B")).await.unwrap();

    // tkB is blocked by both tkA and tkX.
    repo.add_task_dep("tkA", "tkB").await.unwrap();
    repo.add_task_dep("tkX", "tkB").await.unwrap();

    // Complete A only.
    repo.remove_task_dep("tkA", "tkB").await.unwrap();

    assert_eq!(repo.list_blockers("tkB").await.unwrap(), vec!["tkX".to_string()]);
}

// ── Data Consistency Tests ───────────────────────────────────────────

#[tokio::test]
async fn delete_team_cascades_agents_mailbox_tasks_and_deps() {
    let (repo, db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();

    repo.create_team_agent(&make_agent("a1", "t1", "Lead", 0)).await.unwrap();
    repo.write_message(&make_mailbox_msg("t1", "a1", "a2", "message")).await.unwrap();
    repo.create_task(&make_task("tkA", "t1", "A")).await.unwrap();
    repo.create_task(&make_task("tkB", "t1", "B")).await.unwrap();
    repo.add_task_dep("tkA", "tkB").await.unwrap();

    // Single delete — FK CASCADE handles the rest (no manual cleanup helpers).
    repo.delete_team("t1").await.unwrap();

    assert!(repo.get_team("t1").await.unwrap().is_none());
    assert!(repo.list_team_agents("t1").await.unwrap().is_empty());
    assert!(repo.get_history("t1", "a1", None).await.unwrap().is_empty());
    assert!(repo.list_tasks("t1").await.unwrap().is_empty());

    let deps: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM team_task_deps")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(deps.0, 0);
}

#[tokio::test]
async fn task_dependency_directionality_is_consistent() {
    let (repo, _db) = repo().await;
    repo.create_team(&make_team("t1", "Team")).await.unwrap();
    repo.create_task(&make_task("tkA", "t1", "A")).await.unwrap();
    repo.create_task(&make_task("tkB", "t1", "B")).await.unwrap();

    repo.add_task_dep("tkA", "tkB").await.unwrap();

    // A single directed edge yields both views consistently.
    assert!(
        repo.list_blocking("tkA").await.unwrap().contains(&"tkB".to_string()),
        "tkA should block tkB"
    );
    assert!(
        repo.list_blockers("tkB").await.unwrap().contains(&"tkA".to_string()),
        "tkB should be blocked by tkA"
    );
}
