//! Black-box integration tests for `ICronRepository`.
//!
//! Tests exercise the repository trait interface without knowledge of
//! the underlying SQLite implementation details.
//!
//! Covers test-plan items from Phase 12 test-plan:
//! - Section A (CRUD): CJ-1..CJ-12 (data-layer portion)
//! - Section C (Skill): SK-1..SK-7 (data-layer portion)
//! - Section D (Schedule Calculation): SC-1..SC-8 (data-layer portion)
//! - Section H (Cascade Delete): CD-1 (data-layer portion)

use std::sync::Arc;

use nomifun_common::now_ms;
use nomifun_db::models::CronJobRow;
use nomifun_db::{DbError, ICronRepository, SqliteCronRepository, UpdateCronJobParams, init_database_memory};

async fn repo() -> (Arc<dyn ICronRepository>, nomifun_db::Database) {
    let db = init_database_memory().await.unwrap();

    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
         VALUES ('user_1', 'tester', 'hash', 0, 0)",
    )
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO conversations (id, user_id, name, type, created_at, updated_at) \
         VALUES (1, 'user_1', 'Conv 1', 'normal', 0, 0)",
    )
    .execute(db.pool())
    .await
    .unwrap();

    let r = Arc::new(SqliteCronRepository::new(db.pool().clone()));
    (r as Arc<dyn ICronRepository>, db)
}

fn make_job(id: &str) -> CronJobRow {
    let now = now_ms();
    CronJobRow {
        id: id.into(),
        name: "Test Job".into(),
        enabled: true,
        schedule_kind: "every".into(),
        schedule_value: "60000".into(),
        schedule_tz: None,
        schedule_description: Some("Every minute".into()),
        payload_message: "Run report".into(),
        execution_mode: "existing".into(),
        agent_config: None,
        conversation_id: Some(1),
        conversation_title: Some("Conv 1".into()),
        agent_type: "acp".into(),
        created_by: "user".into(),
        skill_content: None,
        description: None,
        created_at: now,
        updated_at: now,
        next_run_at: Some(now + 60_000),
        last_run_at: None,
        last_status: None,
        last_error: None,
        run_count: 0,
        retry_count: 0,
        max_retries: 3,
        target_kind: "agent".into(),
        terminal_mode: None,
        terminal_session_id: None,
        terminal_command: None,
        terminal_args: None,
        terminal_script: None,
    }
}

// ── A. CRUD ──────────────────────────────────────────────────────────

#[tokio::test]
async fn cj1_insert_returns_all_fields() {
    let (r, _db) = repo().await;
    let job = make_job("cron_cj1");
    r.insert(&job).await.unwrap();

    let found = r.get_by_id("cron_cj1").await.unwrap().expect("found");
    assert_eq!(found.id, "cron_cj1");
    assert_eq!(found.name, "Test Job");
    assert!(found.enabled);
    assert_eq!(found.schedule_kind, "every");
    assert_eq!(found.schedule_value, "60000");
    assert_eq!(found.payload_message, "Run report");
    assert_eq!(found.execution_mode, "existing");
    assert_eq!(found.conversation_id, Some(1));
    assert_eq!(found.agent_type, "acp");
    assert_eq!(found.created_by, "user");
    assert_eq!(found.run_count, 0);
    assert_eq!(found.retry_count, 0);
    assert_eq!(found.max_retries, 3);
}

#[tokio::test]
async fn cj2_three_schedule_kinds() {
    let (r, _db) = repo().await;

    let mut at_job = make_job("cron_at");
    at_job.schedule_kind = "at".into();
    at_job.schedule_value = "1700000000000".into();
    r.insert(&at_job).await.unwrap();

    let mut every_job = make_job("cron_every");
    every_job.schedule_kind = "every".into();
    every_job.schedule_value = "60000".into();
    r.insert(&every_job).await.unwrap();

    let mut cron_job = make_job("cron_cron");
    cron_job.schedule_kind = "cron".into();
    cron_job.schedule_value = "0 */5 * * * *".into();
    cron_job.schedule_tz = Some("Asia/Shanghai".into());
    r.insert(&cron_job).await.unwrap();

    let at = r.get_by_id("cron_at").await.unwrap().unwrap();
    assert_eq!(at.schedule_kind, "at");

    let every = r.get_by_id("cron_every").await.unwrap().unwrap();
    assert_eq!(every.schedule_kind, "every");

    let cron = r.get_by_id("cron_cron").await.unwrap().unwrap();
    assert_eq!(cron.schedule_kind, "cron");
    assert_eq!(cron.schedule_tz.as_deref(), Some("Asia/Shanghai"));
}

#[tokio::test]
async fn cj4_get_by_id_existing() {
    let (r, _db) = repo().await;
    r.insert(&make_job("cron_g1")).await.unwrap();
    let found = r.get_by_id("cron_g1").await.unwrap();
    assert!(found.is_some());
}

#[tokio::test]
async fn cj5_get_by_id_nonexistent() {
    let (r, _db) = repo().await;
    let found = r.get_by_id("cron_nonexistent").await.unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn cj6_list_all() {
    let (r, _db) = repo().await;
    r.insert(&make_job("cron_l1")).await.unwrap();
    r.insert(&make_job("cron_l2")).await.unwrap();
    r.insert(&make_job("cron_l3")).await.unwrap();

    let all = r.list_all().await.unwrap();
    assert!(all.len() >= 3);
}

#[tokio::test]
async fn cj7_list_by_conversation() {
    let (r, db) = repo().await;

    sqlx::query(
        "INSERT INTO conversations (id, user_id, name, type, created_at, updated_at) \
         VALUES (2, 'user_1', 'Conv 2', 'normal', 0, 0)",
    )
    .execute(db.pool())
    .await
    .unwrap();

    r.insert(&make_job("cron_fc1")).await.unwrap();
    r.insert(&make_job("cron_fc2")).await.unwrap();

    let mut other = make_job("cron_fc3");
    other.conversation_id = Some(2);
    r.insert(&other).await.unwrap();

    let conv1 = r.list_by_conversation(1).await.unwrap();
    assert_eq!(conv1.len(), 2);

    let conv2 = r.list_by_conversation(2).await.unwrap();
    assert_eq!(conv2.len(), 1);
}

#[tokio::test]
async fn cj8_update_name_and_enabled() {
    let (r, _db) = repo().await;
    r.insert(&make_job("cron_u1")).await.unwrap();

    let params = UpdateCronJobParams {
        name: Some("Renamed".into()),
        enabled: Some(false),
        ..Default::default()
    };
    r.update("cron_u1", &params).await.unwrap();

    let updated = r.get_by_id("cron_u1").await.unwrap().unwrap();
    assert_eq!(updated.name, "Renamed");
    assert!(!updated.enabled);
    assert!(updated.updated_at >= updated.created_at);
}

#[tokio::test]
async fn cj9_update_schedule_type() {
    let (r, _db) = repo().await;
    r.insert(&make_job("cron_s1")).await.unwrap();

    let params = UpdateCronJobParams {
        schedule_kind: Some("cron".into()),
        schedule_value: Some("0 0 9 * * *".into()),
        schedule_tz: Some(Some("UTC".into())),
        next_run_at: Some(Some(9999999)),
        ..Default::default()
    };
    r.update("cron_s1", &params).await.unwrap();

    let updated = r.get_by_id("cron_s1").await.unwrap().unwrap();
    assert_eq!(updated.schedule_kind, "cron");
    assert_eq!(updated.schedule_value, "0 0 9 * * *");
    assert_eq!(updated.schedule_tz.as_deref(), Some("UTC"));
    assert_eq!(updated.next_run_at, Some(9999999));
}

#[tokio::test]
async fn cj10_update_nonexistent() {
    let (r, _db) = repo().await;
    let params = UpdateCronJobParams {
        name: Some("x".into()),
        ..Default::default()
    };
    let err = r.update("cron_nope", &params).await.unwrap_err();
    assert!(matches!(err, DbError::NotFound(_)));
}

#[tokio::test]
async fn cj11_delete() {
    let (r, _db) = repo().await;
    r.insert(&make_job("cron_d1")).await.unwrap();
    r.delete("cron_d1").await.unwrap();

    let found = r.get_by_id("cron_d1").await.unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn cj12_delete_nonexistent() {
    let (r, _db) = repo().await;
    let err = r.delete("cron_nope").await.unwrap_err();
    assert!(matches!(err, DbError::NotFound(_)));
}

// ── List enabled ─────────────────────────────────────────────────────

#[tokio::test]
async fn list_enabled_filters_disabled_jobs() {
    let (r, _db) = repo().await;
    r.insert(&make_job("cron_en1")).await.unwrap();

    let mut disabled = make_job("cron_en2");
    disabled.enabled = false;
    r.insert(&disabled).await.unwrap();

    let enabled = r.list_enabled().await.unwrap();
    assert_eq!(enabled.len(), 1);
    assert_eq!(enabled[0].id, "cron_en1");
}

// ── C. Skill (data layer) ────────────────────────────────────────────

#[tokio::test]
async fn sk1_save_skill_content() {
    let (r, _db) = repo().await;
    r.insert(&make_job("cron_sk1")).await.unwrap();

    let params = UpdateCronJobParams {
        skill_content: Some(Some("---\nname: test\n---\nDo something".into())),
        ..Default::default()
    };
    r.update("cron_sk1", &params).await.unwrap();

    let updated = r.get_by_id("cron_sk1").await.unwrap().unwrap();
    assert!(updated.skill_content.is_some());
    assert!(updated.skill_content.unwrap().contains("Do something"));
}

#[tokio::test]
async fn sk2_has_skill_after_save() {
    let (r, _db) = repo().await;
    let mut job = make_job("cron_sk2");
    job.skill_content = Some("---\nname: s\n---\ncontent".into());
    r.insert(&job).await.unwrap();

    let found = r.get_by_id("cron_sk2").await.unwrap().unwrap();
    assert!(found.skill_content.is_some());
}

#[tokio::test]
async fn sk3_no_skill_by_default() {
    let (r, _db) = repo().await;
    r.insert(&make_job("cron_sk3")).await.unwrap();

    let found = r.get_by_id("cron_sk3").await.unwrap().unwrap();
    assert!(found.skill_content.is_none());
}

#[tokio::test]
async fn sk7_delete_clears_skill() {
    let (r, _db) = repo().await;
    let mut job = make_job("cron_sk7");
    job.skill_content = Some("content".into());
    r.insert(&job).await.unwrap();

    r.delete("cron_sk7").await.unwrap();
    let found = r.get_by_id("cron_sk7").await.unwrap();
    assert!(found.is_none());
}

// ── H. Cascade delete (data layer) ──────────────────────────────────

#[tokio::test]
async fn cd1_delete_by_conversation_removes_all() {
    let (r, _db) = repo().await;
    r.insert(&make_job("cron_cd1")).await.unwrap();
    r.insert(&make_job("cron_cd2")).await.unwrap();

    let deleted = r.delete_by_conversation(1).await.unwrap();
    assert_eq!(deleted, 2);

    let remaining = r.list_all().await.unwrap();
    assert!(remaining.is_empty());
}

#[tokio::test]
async fn delete_by_conversation_no_match_returns_zero() {
    let (r, _db) = repo().await;
    let deleted = r.delete_by_conversation(999).await.unwrap();
    assert_eq!(deleted, 0);
}

// ── Execution state tracking ────────────────────────────────────────

#[tokio::test]
async fn update_execution_state() {
    let (r, _db) = repo().await;
    r.insert(&make_job("cron_ex1")).await.unwrap();

    let now = now_ms();
    let params = UpdateCronJobParams {
        last_run_at: Some(Some(now)),
        last_status: Some(Some("ok".into())),
        run_count: Some(1),
        retry_count: Some(0),
        next_run_at: Some(Some(now + 60_000)),
        ..Default::default()
    };
    r.update("cron_ex1", &params).await.unwrap();

    let updated = r.get_by_id("cron_ex1").await.unwrap().unwrap();
    assert_eq!(updated.last_run_at, Some(now));
    assert_eq!(updated.last_status.as_deref(), Some("ok"));
    assert_eq!(updated.run_count, 1);
    assert_eq!(updated.retry_count, 0);
}

#[tokio::test]
async fn update_error_state() {
    let (r, _db) = repo().await;
    r.insert(&make_job("cron_err1")).await.unwrap();

    let params = UpdateCronJobParams {
        last_status: Some(Some("error".into())),
        last_error: Some(Some("timeout after 30s".into())),
        retry_count: Some(1),
        ..Default::default()
    };
    r.update("cron_err1", &params).await.unwrap();

    let updated = r.get_by_id("cron_err1").await.unwrap().unwrap();
    assert_eq!(updated.last_status.as_deref(), Some("error"));
    assert_eq!(updated.last_error.as_deref(), Some("timeout after 30s"));
    assert_eq!(updated.retry_count, 1);
}

// ── Agent config JSON ───────────────────────────────────────────────

#[tokio::test]
async fn insert_and_retrieve_agent_config() {
    let (r, _db) = repo().await;
    let mut job = make_job("cron_ag1");
    job.agent_config = Some(r#"{"backend":"openai","name":"GPT-4","modelId":"gpt-4","workspace":"/home/user"}"#.into());
    r.insert(&job).await.unwrap();

    let found = r.get_by_id("cron_ag1").await.unwrap().unwrap();
    let config = found.agent_config.unwrap();
    assert!(config.contains("openai"));
    assert!(config.contains("gpt-4"));
}

// ── new_conversation execution mode ─────────────────────────────────

#[tokio::test]
async fn insert_new_conversation_mode() {
    let (r, _db) = repo().await;
    let mut job = make_job("cron_nc1");
    job.execution_mode = "new_conversation".into();
    r.insert(&job).await.unwrap();

    let found = r.get_by_id("cron_nc1").await.unwrap().unwrap();
    assert_eq!(found.execution_mode, "new_conversation");
}
