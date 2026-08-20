//! Migration 040 preserves pre-union tasks while introducing strict workflow owners.

use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000001";
const PROJECT_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000002";
const NODE_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000003";
const LEGACY_TASK_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000004";
const CREATIVE_TASK_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000005";

async fn migrate_to(pool: &sqlx::SqlitePool, max_version: i64) {
    let mut connection = pool.acquire().await.unwrap();
    connection.ensure_migrations_table().await.unwrap();
    let applied: std::collections::BTreeSet<i64> = connection
        .list_applied_migrations()
        .await
        .unwrap()
        .into_iter()
        .map(|migration| migration.version)
        .collect();
    for migration in MIGRATOR.iter() {
        if migration.version <= max_version && !applied.contains(&migration.version) {
            connection.apply(migration).await.unwrap();
        }
    }
}

async fn seed_pre_040(pool: &sqlx::SqlitePool) {
    migrate_to(pool, 39).await;
    sqlx::query(
        "INSERT INTO providers \
            (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, \
             enabled, created_at, updated_at) \
         VALUES (?, 'openai', 'Migration Provider', 'https://example.invalid', \
                 'bearer', '', 1, 1, 1)",
    )
    .bind(PROVIDER_ID)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO creative_studio_projects \
            (project_id, title, revision, node_count, connection_count, document_json, \
             created_at, updated_at) \
         VALUES (?, 'Migration Project', 1, 0, 0, ?, 1, 1)",
    )
    .bind(PROJECT_ID)
    .bind(
        serde_json::json!({
            "schema": "nomifun.creative-studio/v1",
            "projectId": PROJECT_ID
        })
        .to_string(),
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO creation_tasks \
            (creation_task_id, provider_id, model, capability, params, status, submitted_at) \
         VALUES (?, ?, 'legacy-model', 't2i', '{}', 'queued', 10)",
    )
    .bind(LEGACY_TASK_ID)
    .bind(PROVIDER_ID)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO creation_tasks \
            (creation_task_id, project_id, node_id, provider_id, model, capability, params, \
             status, submitted_at, request_fingerprint) \
         VALUES (?, ?, ?, ?, 'creative-model', 't2i', '{}', 'queued', 20, ?)",
    )
    .bind(CREATIVE_TASK_ID)
    .bind(PROJECT_ID)
    .bind(NODE_ID)
    .bind(PROVIDER_ID)
    .bind(
        serde_json::json!({
            "owner": {
                "kind": "canvas_node",
                "project_id": PROJECT_ID,
                "node_id": NODE_ID
            }
        })
        .to_string(),
    )
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn migration_preserves_legacy_and_canvas_tasks_without_making_them_workflow_owned() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    seed_pre_040(&pool).await;

    migrate_to(&pool, 40).await;

    let legacy: (Option<String>, Option<String>, Option<String>, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT project_id, workflow_id, workflow_run_id, workflow_step_id, \
                    request_fingerprint \
             FROM creation_tasks WHERE creation_task_id = ?",
        )
        .bind(LEGACY_TASK_ID)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(legacy, (None, None, None, None, None));

    let creative: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT project_id, workflow_id, workflow_run_id, workflow_step_id, \
                request_fingerprint \
         FROM creation_tasks WHERE creation_task_id = ?",
    )
    .bind(CREATIVE_TASK_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(creative.0.as_deref(), Some(PROJECT_ID));
    assert!(creative.1.is_none());
    assert!(creative.2.is_none());
    assert!(creative.3.is_none());
    assert!(creative.4.is_some());
}

#[tokio::test]
async fn migration_adds_the_strict_workflow_owner_shape() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    seed_pre_040(&pool).await;
    migrate_to(&pool, 40).await;

    let invalid = sqlx::query(
        "INSERT INTO creation_tasks \
            (creation_task_id, workflow_id, workflow_run_id, provider_id, model, capability, \
             params, status, submitted_at, request_fingerprint) \
         VALUES (?, ?, ?, ?, 'workflow-model', 't2i', '{}', 'queued', 30, '{}')",
    )
    .bind("0190f5fe-7c00-7a00-8abc-000000000006")
    .bind("0190f5fe-7c00-7a00-8abc-000000000007")
    .bind("0190f5fe-7c00-7a00-8abc-000000000008")
    .bind(PROVIDER_ID)
    .execute(&pool)
    .await;
    assert!(invalid.is_err(), "workflow_step_id is mandatory for the branch");
}
