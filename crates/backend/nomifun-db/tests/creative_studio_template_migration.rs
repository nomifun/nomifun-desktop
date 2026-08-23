//! Migration 049 moves the live Creative Studio persistence contract from the
//! retired workflow vocabulary to canonical template names without rewriting
//! the published 040-047 migration lineage.

use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000801";
const TEMPLATE_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000802";
const TEMPLATE_RUN_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000803";
const TEMPLATE_STEP_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000804";
const PROMPT_DRAFT_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000805";
const CREATION_TASK_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000806";
const ASSET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000807";

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

#[tokio::test]
async fn migration_049_preserves_data_and_removes_the_retired_live_schema() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 47).await;

    sqlx::query(
        "INSERT INTO providers \
            (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, \
             enabled, created_at, updated_at) \
         VALUES (?, 'openai', 'Template migration provider', 'https://example.invalid', \
                 'bearer', '', 1, 1, 1)",
    )
    .bind(PROVIDER_ID)
    .execute(&pool)
    .await
    .unwrap();

    let definition = serde_json::json!({
        "id": TEMPLATE_ID,
        "revision": 1
    });
    sqlx::query(
        "INSERT INTO creative_studio_workflows \
            (workflow_id, revision, name, description, category, visibility, \
             definition_json, created_at, updated_at) \
         VALUES (?, 1, 'Migrated template', '', '', 'private', ?, 10, 10)",
    )
    .bind(TEMPLATE_ID)
    .bind(definition.to_string())
    .execute(&pool)
    .await
    .unwrap();

    let aggregate = serde_json::json!({
        "kind": "nomifun.creative-studio.workflow-run",
        "version": 1,
        "revision": 1,
        "workflowSnapshot": definition,
        "request": {
            "id": TEMPLATE_RUN_ID,
            "idempotencyKey": TEMPLATE_RUN_ID,
            "workflowId": TEMPLATE_ID,
            "workflowRevision": 1
        },
        "promptDrafts": [{
            "id": PROMPT_DRAFT_ID,
            "workflowId": TEMPLATE_ID,
            "runRequestId": TEMPLATE_RUN_ID
        }],
        "record": {
            "requestId": TEMPLATE_RUN_ID,
            "workflowId": TEMPLATE_ID,
            "status": "running"
        }
    });
    sqlx::query(
        "INSERT INTO creative_studio_workflow_runs \
            (workflow_run_id, workflow_id, workflow_revision, revision, status, \
             step_ids_json, aggregate_json, created_at, updated_at) \
         VALUES (?, ?, 1, 1, 'running', ?, ?, 20, 20)",
    )
    .bind(TEMPLATE_RUN_ID)
    .bind(TEMPLATE_ID)
    .bind(serde_json::json!([TEMPLATE_STEP_ID]).to_string())
    .bind(aggregate.to_string())
    .execute(&pool)
    .await
    .unwrap();

    let fingerprint = serde_json::json!({
        "owner": {
            "kind": "workflow_step",
            "workflow_id": TEMPLATE_ID,
            "workflow_run_id": TEMPLATE_RUN_ID,
            "workflow_step_id": TEMPLATE_STEP_ID
        },
        "inputs": []
    });
    sqlx::query(
        "INSERT INTO creation_tasks \
            (creation_task_id, workflow_id, workflow_run_id, workflow_step_id, provider_id, \
             model, capability, params, input_bindings, status, submitted_at, request_fingerprint) \
         VALUES (?, ?, ?, ?, ?, 'image-model', 't2i', '{}', '[]', 'running', 20, ?)",
    )
    .bind(CREATION_TASK_ID)
    .bind(TEMPLATE_ID)
    .bind(TEMPLATE_RUN_ID)
    .bind(TEMPLATE_STEP_ID)
    .bind(PROVIDER_ID)
    .bind(fingerprint.to_string())
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO workshop_assets \
            (asset_id, kind, title, tags, in_library, origin, created_at, updated_at) \
         VALUES (?, 'image', 'Migrated result', '[]', 1, ?, 20, 20)",
    )
    .bind(ASSET_ID)
    .bind(
        serde_json::json!({
            "workflow_id": TEMPLATE_ID,
            "workflow_run_id": TEMPLATE_RUN_ID,
            "workflow_step_id": TEMPLATE_STEP_ID,
            "creation_task_id": CREATION_TASK_ID
        })
        .to_string(),
    )
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 49).await;

    let old_tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema \
         WHERE type = 'table' AND name IN (\
             'creative_studio_workflows', 'creative_studio_workflow_runs'\
         )",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(old_tables, 0);

    let template_name: String = sqlx::query_scalar(
        "SELECT name FROM creative_studio_templates WHERE template_id = ?",
    )
    .bind(TEMPLATE_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(template_name, "Migrated template");

    let aggregate_json: String = sqlx::query_scalar(
        "SELECT aggregate_json FROM creative_studio_template_runs \
         WHERE template_run_id = ?",
    )
    .bind(TEMPLATE_RUN_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    let aggregate: serde_json::Value = serde_json::from_str(&aggregate_json).unwrap();
    assert_eq!(
        aggregate["kind"],
        "nomifun.creative-studio.template-run"
    );
    assert_eq!(aggregate["templateSnapshot"]["id"], TEMPLATE_ID);
    assert_eq!(aggregate["request"]["templateId"], TEMPLATE_ID);
    assert_eq!(aggregate["request"]["templateRevision"], 1);
    assert_eq!(aggregate["record"]["templateId"], TEMPLATE_ID);
    assert_eq!(aggregate["promptDrafts"][0]["templateId"], TEMPLATE_ID);
    assert!(
        !aggregate_json.to_ascii_lowercase().contains("workflow"),
        "current aggregate must not retain retired vocabulary"
    );

    let task: (String, String, String, String) = sqlx::query_as(
        "SELECT template_id, template_run_id, template_step_id, request_fingerprint \
         FROM creation_tasks WHERE creation_task_id = ?",
    )
    .bind(CREATION_TASK_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((&task.0, &task.1, &task.2), (&TEMPLATE_ID.into(), &TEMPLATE_RUN_ID.into(), &TEMPLATE_STEP_ID.into()));
    let fingerprint: serde_json::Value = serde_json::from_str(&task.3).unwrap();
    assert_eq!(fingerprint["owner"]["kind"], "template_step");
    assert_eq!(fingerprint["owner"]["template_id"], TEMPLATE_ID);
    assert_eq!(fingerprint["owner"]["template_run_id"], TEMPLATE_RUN_ID);
    assert_eq!(fingerprint["owner"]["template_step_id"], TEMPLATE_STEP_ID);
    assert!(!task.3.to_ascii_lowercase().contains("workflow"));

    let origin_json: String =
        sqlx::query_scalar("SELECT origin FROM workshop_assets WHERE asset_id = ?")
            .bind(ASSET_ID)
            .fetch_one(&pool)
            .await
            .unwrap();
    let origin: serde_json::Value = serde_json::from_str(&origin_json).unwrap();
    assert_eq!(origin["template_id"], TEMPLATE_ID);
    assert_eq!(origin["template_run_id"], TEMPLATE_RUN_ID);
    assert_eq!(origin["template_step_id"], TEMPLATE_STEP_ID);
    assert!(!origin_json.to_ascii_lowercase().contains("workflow"));

    let old_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('creation_tasks') \
         WHERE lower(name) LIKE '%workflow%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(old_columns, 0);

    let old_live_schema: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema \
         WHERE (name LIKE 'creative_studio_%' OR tbl_name IN ('creation_tasks', 'workshop_assets')) \
           AND lower(COALESCE(sql, '')) LIKE '%workflow%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(old_live_schema, 0);

    let rejected = sqlx::query(
        "INSERT INTO workshop_assets \
            (asset_id, kind, title, tags, in_library, origin, created_at, updated_at) \
         VALUES (?, 'image', 'Rejected non-canonical owner', '[]', 1, ?, 30, 30)",
    )
    .bind("0190f5fe-7c00-7a00-8abc-000000000808")
    .bind(
        serde_json::json!({
            "templateId": TEMPLATE_ID,
            "templateRunId": TEMPLATE_RUN_ID,
            "templateStepId": TEMPLATE_STEP_ID
        })
        .to_string(),
    )
    .execute(&pool)
    .await;
    assert!(rejected.is_err(), "camelCase origin keys must fail closed");
}
