//! Black-box integration tests for `TaskBoard` service.
//!
//! Exercises the service layer against a real SQLite database.
//!
//! Covers test-plan items:
//! - TK-1..TK-4 (create tasks: no deps, single dep, multi-dep, nonexistent dep)
//! - TU-1..TU-5 (update status, description, owner, nonexistent)
//! - CU-1..CU-4 (check_unblocks: single, multiple, partial, no downstream)
//! - TT-1..TT-3 (list tasks, empty, with deps)
//! - DC-4 (blockedBy/blocks bidirectional consistency)

use std::sync::Arc;

use nomifun_db::{ITeamRepository, SqliteTeamRepository, init_database_memory};
use nomifun_team::{TaskBoard, TaskStatus, TaskUpdate};

async fn setup() -> (TaskBoard, nomifun_db::Database) {
    let db = init_database_memory().await.unwrap();
    // Seed the parent user + team: `team_tasks.team_id` now carries a FK
    // (CASCADE) to `teams(id)` (spec §4.2), so the team must exist before any
    // task can be inserted.
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
         VALUES ('user_1', 'tester', 'hash', 0, 0)",
    )
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO teams (id, user_id, name, workspace, workspace_mode, agents_version, created_at, updated_at) \
         VALUES ('t1', 'user_1', 'Test Team', '/tmp/ws', 'shared', '1.0.0', 0, 0)",
    )
    .execute(db.pool())
    .await
    .unwrap();
    let repo = Arc::new(SqliteTeamRepository::new(db.pool().clone())) as Arc<dyn ITeamRepository>;
    (TaskBoard::new(repo), db)
}

// -- TK: Create tasks ---------------------------------------------------------

#[tokio::test]
async fn tk1_create_task_no_dependencies() {
    let (board, _db) = setup().await;
    let task = board
        .create_task("t1", "Implement feature", None, None, &[])
        .await
        .unwrap();
    assert_eq!(task.subject, "Implement feature");
    assert_eq!(task.status, TaskStatus::Pending);
    assert!(task.blocked_by.is_empty());
    assert!(task.blocks.is_empty());
}

#[tokio::test]
async fn tk2_create_task_with_single_dependency() {
    let (board, _db) = setup().await;
    let task_a = board.create_task("t1", "Task A", None, None, &[]).await.unwrap();
    let task_b = board
        .create_task("t1", "Task B", None, None, std::slice::from_ref(&task_a.id))
        .await
        .unwrap();
    assert_eq!(task_b.blocked_by, vec![task_a.id.clone()]);

    let tasks = board.list_tasks("t1").await.unwrap();
    let a = tasks.iter().find(|t| t.id == task_a.id).unwrap();
    assert_eq!(a.blocks, vec![task_b.id]);
}

#[tokio::test]
async fn tk3_create_task_with_multiple_dependencies() {
    let (board, _db) = setup().await;
    let a = board.create_task("t1", "A", None, None, &[]).await.unwrap();
    let b = board.create_task("t1", "B", None, None, &[]).await.unwrap();
    let c = board
        .create_task("t1", "C", None, None, &[a.id.clone(), b.id.clone()])
        .await
        .unwrap();
    assert_eq!(c.blocked_by.len(), 2);

    let tasks = board.list_tasks("t1").await.unwrap();
    let a_updated = tasks.iter().find(|t| t.id == a.id).unwrap();
    let b_updated = tasks.iter().find(|t| t.id == b.id).unwrap();
    assert!(a_updated.blocks.contains(&c.id));
    assert!(b_updated.blocks.contains(&c.id));
}

#[tokio::test]
async fn tk4_create_task_nonexistent_dependency_fails() {
    let (board, _db) = setup().await;
    let result = board.create_task("t1", "X", None, None, &["nonexistent".into()]).await;
    assert!(result.is_err());
}

// -- TU: Update tasks ---------------------------------------------------------

#[tokio::test]
async fn tu1_update_status_pending_to_in_progress() {
    let (board, _db) = setup().await;
    let task = board.create_task("t1", "Work", None, None, &[]).await.unwrap();
    let updated = board
        .update_task(
            "t1",
            &task.id,
            &TaskUpdate {
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.status, TaskStatus::InProgress);
}

#[tokio::test]
async fn tu2_update_status_to_completed_triggers_unblock() {
    let (board, _db) = setup().await;
    let a = board.create_task("t1", "A", None, None, &[]).await.unwrap();
    let b = board
        .create_task("t1", "B", None, None, std::slice::from_ref(&a.id))
        .await
        .unwrap();

    board
        .update_task(
            "t1",
            &a.id,
            &TaskUpdate {
                status: Some(TaskStatus::Completed),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let tasks = board.list_tasks("t1").await.unwrap();
    let b_updated = tasks.iter().find(|t| t.id == b.id).unwrap();
    assert!(b_updated.blocked_by.is_empty());
}

#[tokio::test]
async fn tu3_update_description() {
    let (board, _db) = setup().await;
    let task = board.create_task("t1", "Work", None, None, &[]).await.unwrap();
    let updated = board
        .update_task(
            "t1",
            &task.id,
            &TaskUpdate {
                description: Some("Updated description".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.description.as_deref(), Some("Updated description"));
}

#[tokio::test]
async fn tu4_update_owner() {
    let (board, _db) = setup().await;
    let task = board.create_task("t1", "Work", None, None, &[]).await.unwrap();
    let updated = board
        .update_task(
            "t1",
            &task.id,
            &TaskUpdate {
                owner: Some("agent-2".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.owner.as_deref(), Some("agent-2"));
}

#[tokio::test]
async fn tu5_update_nonexistent_task_fails() {
    let (board, _db) = setup().await;
    let result = board.update_task("t1", "nonexistent", &TaskUpdate::default()).await;
    assert!(result.is_err());
}

// -- CU: Check unblocks ------------------------------------------------------

#[tokio::test]
async fn cu1_complete_unblocks_single_downstream() {
    let (board, _db) = setup().await;
    let a = board.create_task("t1", "A", None, None, &[]).await.unwrap();
    let b = board
        .create_task("t1", "B", None, None, std::slice::from_ref(&a.id))
        .await
        .unwrap();

    board
        .update_task(
            "t1",
            &a.id,
            &TaskUpdate {
                status: Some(TaskStatus::Completed),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let tasks = board.list_tasks("t1").await.unwrap();
    let b_updated = tasks.iter().find(|t| t.id == b.id).unwrap();
    assert!(b_updated.blocked_by.is_empty());
}

#[tokio::test]
async fn cu2_complete_unblocks_multiple_downstream() {
    let (board, _db) = setup().await;
    let a = board.create_task("t1", "A", None, None, &[]).await.unwrap();
    let b = board
        .create_task("t1", "B", None, None, std::slice::from_ref(&a.id))
        .await
        .unwrap();
    let c = board
        .create_task("t1", "C", None, None, std::slice::from_ref(&a.id))
        .await
        .unwrap();

    board
        .update_task(
            "t1",
            &a.id,
            &TaskUpdate {
                status: Some(TaskStatus::Completed),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let tasks = board.list_tasks("t1").await.unwrap();
    let b_updated = tasks.iter().find(|t| t.id == b.id).unwrap();
    let c_updated = tasks.iter().find(|t| t.id == c.id).unwrap();
    assert!(b_updated.blocked_by.is_empty());
    assert!(c_updated.blocked_by.is_empty());
}

#[tokio::test]
async fn cu3_partial_unblock_preserves_other_deps() {
    let (board, _db) = setup().await;
    let a = board.create_task("t1", "A", None, None, &[]).await.unwrap();
    let x = board.create_task("t1", "X", None, None, &[]).await.unwrap();
    let b = board
        .create_task("t1", "B", None, None, &[a.id.clone(), x.id.clone()])
        .await
        .unwrap();

    board
        .update_task(
            "t1",
            &a.id,
            &TaskUpdate {
                status: Some(TaskStatus::Completed),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let tasks = board.list_tasks("t1").await.unwrap();
    let b_updated = tasks.iter().find(|t| t.id == b.id).unwrap();
    assert_eq!(b_updated.blocked_by, vec![x.id]);
}

#[tokio::test]
async fn cu4_complete_no_downstream_is_noop() {
    let (board, _db) = setup().await;
    let task = board.create_task("t1", "Solo", None, None, &[]).await.unwrap();
    let updated = board
        .update_task(
            "t1",
            &task.id,
            &TaskUpdate {
                status: Some(TaskStatus::Completed),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.status, TaskStatus::Completed);
}

// -- TT: List tasks -----------------------------------------------------------

#[tokio::test]
async fn tt1_list_all_tasks() {
    let (board, _db) = setup().await;
    board.create_task("t1", "A", None, None, &[]).await.unwrap();
    board.create_task("t1", "B", None, None, &[]).await.unwrap();
    let tasks = board.list_tasks("t1").await.unwrap();
    assert_eq!(tasks.len(), 2);
}

#[tokio::test]
async fn tt2_list_empty() {
    let (board, _db) = setup().await;
    let tasks = board.list_tasks("t1").await.unwrap();
    assert!(tasks.is_empty());
}

#[tokio::test]
async fn tt3_list_includes_dependency_info() {
    let (board, _db) = setup().await;
    let a = board.create_task("t1", "A", None, None, &[]).await.unwrap();
    let b = board
        .create_task("t1", "B", None, None, std::slice::from_ref(&a.id))
        .await
        .unwrap();
    let tasks = board.list_tasks("t1").await.unwrap();
    let b_found = tasks.iter().find(|t| t.id == b.id).unwrap();
    assert_eq!(b_found.blocked_by, vec![a.id.clone()]);
    let a_found = tasks.iter().find(|t| t.id == a.id).unwrap();
    assert!(a_found.blocks.contains(&b.id));
}

// -- DC-4: Bidirectional consistency ------------------------------------------

#[tokio::test]
async fn dc4_blocked_by_blocks_bidirectional_consistency() {
    let (board, _db) = setup().await;
    let a = board.create_task("t1", "A", None, None, &[]).await.unwrap();
    let b = board
        .create_task("t1", "B", None, None, std::slice::from_ref(&a.id))
        .await
        .unwrap();

    let tasks = board.list_tasks("t1").await.unwrap();
    let a_found = tasks.iter().find(|t| t.id == a.id).unwrap();
    let b_found = tasks.iter().find(|t| t.id == b.id).unwrap();
    assert!(a_found.blocks.contains(&b.id));
    assert!(b_found.blocked_by.contains(&a.id));
}
