//! Black-box integration tests for `Mailbox` service.
//!
//! Exercises the service layer against a real SQLite database.
//!
//! Covers test-plan items:
//! - MW-1..MW-3 (write messages: text, idle_notification, shutdown_request)
//! - MR-1..MR-4 (atomic read + mark, re-read empty, no messages)
//! - MH-1..MH-3 (history query with/without limit)
//! - MD-1..MD-2 (delete by team, isolation)

use std::sync::Arc;

use nomifun_db::{ITeamRepository, SqliteTeamRepository, init_database_memory};
use nomifun_team::{Mailbox, MailboxMessageType};

/// Returns the `Mailbox` service plus the repo (so cascade-delete tests can
/// drop a team) and the owning `Database`. Seeds the `users` row and the two
/// teams (`t1`, `t2`) the tests write into — the `mailbox.team_id` FK
/// (CASCADE, spec §5.4) now requires the parent team to exist.
async fn setup() -> (Mailbox, Arc<dyn ITeamRepository>, nomifun_db::Database) {
    let db = init_database_memory().await.unwrap();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
         VALUES ('user_1', 'tester', 'hash', 0, 0)",
    )
    .execute(db.pool())
    .await
    .unwrap();
    for team_id in ["t1", "t2"] {
        sqlx::query(
            "INSERT INTO teams (id, user_id, name, workspace, workspace_mode, agents_version, created_at, updated_at) \
             VALUES (?, 'user_1', 'Test Team', '/tmp/ws', 'shared', '1.0.0', 0, 0)",
        )
        .bind(team_id)
        .execute(db.pool())
        .await
        .unwrap();
    }
    let repo = Arc::new(SqliteTeamRepository::new(db.pool().clone())) as Arc<dyn ITeamRepository>;
    (Mailbox::new(repo.clone()), repo, db)
}

// -- MW: Write messages -------------------------------------------------------

#[tokio::test]
async fn mw1_write_text_message() {
    let (mailbox, _repo, _db) = setup().await;
    let msg = mailbox
        .write("t1", "a1", "user", MailboxMessageType::Message, "hello", None)
        .await
        .unwrap();
    assert_eq!(msg.msg_type, MailboxMessageType::Message);
    assert_eq!(msg.content, "hello");
    assert!(!msg.read);
    // `mailbox.id` is now an autoincrement i64 (spec §2 row 19) returned by
    // `write_message` via `last_insert_rowid()`.
    assert!(msg.id > 0);
}

#[tokio::test]
async fn mw2_write_idle_notification_with_summary() {
    let (mailbox, _repo, _db) = setup().await;
    let msg = mailbox
        .write(
            "t1",
            "lead",
            "a1",
            MailboxMessageType::IdleNotification,
            "done",
            Some("Task finished"),
        )
        .await
        .unwrap();
    assert_eq!(msg.msg_type, MailboxMessageType::IdleNotification);
    assert_eq!(msg.summary.as_deref(), Some("Task finished"));
}

#[tokio::test]
async fn mw3_write_shutdown_request() {
    let (mailbox, _repo, _db) = setup().await;
    let msg = mailbox
        .write(
            "t1",
            "a1",
            "lead",
            MailboxMessageType::ShutdownRequest,
            "cleanup done",
            None,
        )
        .await
        .unwrap();
    assert_eq!(msg.msg_type, MailboxMessageType::ShutdownRequest);
}

// -- MR: Atomic read + mark ---------------------------------------------------

#[tokio::test]
async fn mr1_read_unread_returns_all_and_marks() {
    let (mailbox, _repo, _db) = setup().await;
    for i in 0..3 {
        mailbox
            .write(
                "t1",
                "a1",
                "user",
                MailboxMessageType::Message,
                &format!("msg-{i}"),
                None,
            )
            .await
            .unwrap();
    }
    let unread = mailbox.read_unread("t1", "a1").await.unwrap();
    assert_eq!(unread.len(), 3);
}

#[tokio::test]
async fn mr2_second_read_returns_empty() {
    let (mailbox, _repo, _db) = setup().await;
    mailbox
        .write("t1", "a1", "user", MailboxMessageType::Message, "x", None)
        .await
        .unwrap();
    mailbox.read_unread("t1", "a1").await.unwrap();
    let second = mailbox.read_unread("t1", "a1").await.unwrap();
    assert!(second.is_empty());
}

#[tokio::test]
async fn mr4_no_unread_messages() {
    let (mailbox, _repo, _db) = setup().await;
    let unread = mailbox.read_unread("t1", "a1").await.unwrap();
    assert!(unread.is_empty());
}

// -- MH: History query --------------------------------------------------------

#[tokio::test]
async fn mh1_get_history_no_limit() {
    let (mailbox, _repo, _db) = setup().await;
    for i in 0..5 {
        mailbox
            .write("t1", "a1", "user", MailboxMessageType::Message, &format!("m{i}"), None)
            .await
            .unwrap();
    }
    mailbox.read_unread("t1", "a1").await.unwrap();
    let history = mailbox.get_history("t1", "a1", None).await.unwrap();
    assert_eq!(history.len(), 5);
}

#[tokio::test]
async fn mh2_get_history_with_limit() {
    let (mailbox, _repo, _db) = setup().await;
    for i in 0..10 {
        mailbox
            .write("t1", "a1", "user", MailboxMessageType::Message, &format!("m{i}"), None)
            .await
            .unwrap();
    }
    let history = mailbox.get_history("t1", "a1", Some(5)).await.unwrap();
    assert_eq!(history.len(), 5);
}

#[tokio::test]
async fn mh3_empty_history() {
    let (mailbox, _repo, _db) = setup().await;
    let history = mailbox.get_history("t1", "a1", None).await.unwrap();
    assert!(history.is_empty());
}

// -- MD: Delete team cascades to its mailbox ----------------------------------

#[tokio::test]
async fn md1_delete_team_removes_all_messages() {
    let (mailbox, repo, _db) = setup().await;
    mailbox
        .write("t1", "a1", "user", MailboxMessageType::Message, "x", None)
        .await
        .unwrap();
    mailbox
        .write("t1", "a2", "user", MailboxMessageType::Message, "y", None)
        .await
        .unwrap();
    // The mailbox is purged via FK ON DELETE CASCADE when the team goes away
    // (spec §5.4) — there is no manual delete_mailbox_by_team.
    repo.delete_team("t1").await.unwrap();
    let h1 = mailbox.get_history("t1", "a1", None).await.unwrap();
    let h2 = mailbox.get_history("t1", "a2", None).await.unwrap();
    assert!(h1.is_empty());
    assert!(h2.is_empty());
}

#[tokio::test]
async fn md2_delete_team_does_not_affect_other_teams() {
    let (mailbox, repo, _db) = setup().await;
    mailbox
        .write("t1", "a1", "user", MailboxMessageType::Message, "x", None)
        .await
        .unwrap();
    mailbox
        .write("t2", "a1", "user", MailboxMessageType::Message, "y", None)
        .await
        .unwrap();
    repo.delete_team("t1").await.unwrap();
    let h2 = mailbox.get_history("t2", "a1", None).await.unwrap();
    assert_eq!(h2.len(), 1);
}

// -- Agent scope isolation ----------------------------------------------------

#[tokio::test]
async fn read_unread_scoped_to_target_agent() {
    let (mailbox, _repo, _db) = setup().await;
    mailbox
        .write("t1", "a1", "user", MailboxMessageType::Message, "for-a1", None)
        .await
        .unwrap();
    mailbox
        .write("t1", "a2", "user", MailboxMessageType::Message, "for-a2", None)
        .await
        .unwrap();
    let a1_msgs = mailbox.read_unread("t1", "a1").await.unwrap();
    assert_eq!(a1_msgs.len(), 1);
    assert_eq!(a1_msgs[0].content, "for-a1");
    let a2_msgs = mailbox.read_unread("t1", "a2").await.unwrap();
    assert_eq!(a2_msgs.len(), 1);
    assert_eq!(a2_msgs[0].content, "for-a2");
}
