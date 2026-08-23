//! Task-owner migrations: 041 introduces the tagged union; 042 retires the
//! historical Workshop/global branch without rewriting canonical task history.

use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000001";
const PROJECT_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000002";
const NODE_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000003";
const LEGACY_TASK_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000004";
const CREATIVE_TASK_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000005";
const INPUT_IMAGE_ID: &str = "0190f5fe-7c00-7a00-8abc-00000000000a";
const INPUT_VIDEO_ID: &str = "0190f5fe-7c00-7a00-8abc-00000000000b";
const UNKNOWN_INPUT_TASK_ID: &str = "0190f5fe-7c00-7a00-8abc-00000000000c";
const INCOMPLETE_INPUT_TASK_ID: &str = "0190f5fe-7c00-7a00-8abc-00000000000f";

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

async fn seed_pre_041(pool: &sqlx::SqlitePool) {
    migrate_to(pool, 40).await;
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
    seed_pre_041(&pool).await;

    migrate_to(&pool, 41).await;

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
    seed_pre_041(&pool).await;
    migrate_to(&pool, 41).await;

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

#[tokio::test]
async fn migration_042_drops_legacy_rows_and_canvas_ownership_column() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    seed_pre_041(&pool).await;

    migrate_to(&pool, 42).await;

    let legacy_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM creation_tasks WHERE creation_task_id = ?",
    )
    .bind(LEGACY_TASK_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(legacy_count, 0, "retired unowned tasks must not survive 042");

    let canonical: (String, String, String) = sqlx::query_as(
        "SELECT project_id, node_id, request_fingerprint \
         FROM creation_tasks WHERE creation_task_id = ?",
    )
    .bind(CREATIVE_TASK_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(canonical.0, PROJECT_ID);
    assert_eq!(canonical.1, NODE_ID);
    assert!(!canonical.2.is_empty());

    let canvas_column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('creation_tasks') WHERE name = 'canvas_id'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(canvas_column_count, 0);

    let unowned = sqlx::query(
        "INSERT INTO creation_tasks \
            (creation_task_id, provider_id, model, capability, params, status, submitted_at, \
             request_fingerprint) \
         VALUES (?, ?, 'retired', 't2i', '{}', 'queued', 40, '{}')",
    )
    .bind("0190f5fe-7c00-7a00-8abc-000000000009")
    .bind(PROVIDER_ID)
    .execute(&pool)
    .await;
    assert!(unowned.is_err(), "042 must reject ownerless task writes");
}

#[tokio::test]
async fn migration_044_recovers_only_provable_ordered_input_bindings() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    seed_pre_041(&pool).await;
    migrate_to(&pool, 43).await;

    for (asset_id, kind) in [(INPUT_IMAGE_ID, "image"), (INPUT_VIDEO_ID, "video")] {
        sqlx::query(
            "INSERT INTO workshop_assets \
                (asset_id, kind, title, tags, bytes, in_library, created_at, updated_at) \
             VALUES (?, ?, 'Migration input', '[]', 0, 1, 1, 1)",
        )
        .bind(asset_id)
        .bind(kind)
        .execute(&pool)
        .await
        .unwrap();
    }
    let fingerprint = serde_json::json!({
        "owner": {
            "kind": "canvas_node",
            "project_id": PROJECT_ID,
            "node_id": NODE_ID
        },
        "inputs": [
            {"asset_id": INPUT_VIDEO_ID, "role": "video"},
            {"asset_id": INPUT_IMAGE_ID, "role": "first_frame"}
        ]
    });
    sqlx::query(
        "UPDATE creation_tasks SET request_fingerprint = ? WHERE creation_task_id = ?",
    )
    .bind(fingerprint.to_string())
    .bind(CREATIVE_TASK_ID)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO creation_tasks \
            (creation_task_id, project_id, node_id, provider_id, model, capability, params, \
             status, submitted_at, request_fingerprint) \
         VALUES (?, ?, ?, ?, 'legacy-incomplete-inputs', 'i2i', '{}', 'queued', 31, ?)",
    )
    .bind(INCOMPLETE_INPUT_TASK_ID)
    .bind(PROJECT_ID)
    .bind(NODE_ID)
    .bind(PROVIDER_ID)
    .bind(
        serde_json::json!({
            "inputs": [{"asset_id": INPUT_IMAGE_ID}]
        })
        .to_string(),
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO creation_tasks \
            (creation_task_id, project_id, node_id, provider_id, model, capability, params, \
             status, submitted_at, request_fingerprint) \
         VALUES (?, ?, ?, ?, 'legacy-unknown-inputs', 't2i', '{}', 'queued', 30, ?)",
    )
    .bind(UNKNOWN_INPUT_TASK_ID)
    .bind(PROJECT_ID)
    .bind(NODE_ID)
    .bind(PROVIDER_ID)
    .bind(serde_json::json!({"owner": {"kind": "canvas_node"}}).to_string())
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 44).await;

    let recovered: String = sqlx::query_scalar(
        "SELECT input_bindings FROM creation_tasks WHERE creation_task_id = ?",
    )
    .bind(CREATIVE_TASK_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&recovered).unwrap(),
        serde_json::json!([
            {"asset_id": INPUT_VIDEO_ID, "kind": "video", "role": "video"},
            {"asset_id": INPUT_IMAGE_ID, "kind": "image", "role": "first_frame"}
        ])
    );
    let unknown: Option<String> = sqlx::query_scalar(
        "SELECT input_bindings FROM creation_tasks WHERE creation_task_id = ?",
    )
    .bind(UNKNOWN_INPUT_TASK_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(unknown.is_none(), "missing legacy inputs must remain explicitly unprovable");
    let incomplete: Option<String> = sqlx::query_scalar(
        "SELECT input_bindings FROM creation_tasks WHERE creation_task_id = ?",
    )
    .bind(INCOMPLETE_INPUT_TASK_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        incomplete.is_none(),
        "legacy inputs missing a required role must remain explicitly unprovable"
    );
}

#[tokio::test]
async fn migration_044_enforces_exact_standalone_owner_and_input_shape() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    seed_pre_041(&pool).await;
    migrate_to(&pool, 44).await;

    let standalone_id = "0190f5fe-7c00-7a00-8abc-00000000000d";
    sqlx::query(
        "INSERT INTO creation_tasks \
            (creation_task_id, project_id, workbench_kind, provider_id, model, capability, \
             params, input_bindings, status, submitted_at, request_fingerprint) \
         VALUES (?, ?, 'image', ?, 'model', 't2i', '{}', '[]', 'queued', 1, '{}')",
    )
    .bind(standalone_id)
    .bind(PROJECT_ID)
    .bind(PROVIDER_ID)
    .execute(&pool)
    .await
    .unwrap();

    let mixed = sqlx::query(
        "INSERT INTO creation_tasks \
            (creation_task_id, project_id, workbench_kind, node_id, provider_id, model, capability, \
             params, input_bindings, status, submitted_at, request_fingerprint) \
         VALUES (?, ?, 'video', ?, ?, 'model', 't2v', '{}', '[]', 'queued', 1, '{}')",
    )
    .bind("0190f5fe-7c00-7a00-8abc-00000000000e")
    .bind(PROJECT_ID)
    .bind(NODE_ID)
    .bind(PROVIDER_ID)
    .execute(&pool)
    .await;
    assert!(mixed.is_err(), "standalone and canvas-node fields must be exclusive");

    let malformed_inputs = sqlx::query(
        "UPDATE creation_tasks SET input_bindings = ? WHERE creation_task_id = ?",
    )
    .bind(serde_json::json!([{
        "asset_id": INPUT_IMAGE_ID,
        "kind": "image",
        "role": "reference",
        "guessed": true
    }]).to_string())
    .bind(standalone_id)
    .execute(&pool)
    .await;
    assert!(malformed_inputs.is_err(), "input bindings reject unknown fields");

    for (label, payload) in [
        (
            "explicit null kind",
            serde_json::json!([{
                "asset_id": INPUT_IMAGE_ID,
                "kind": null,
                "role": "reference"
            }])
            .to_string(),
        ),
        (
            "missing role",
            serde_json::json!([{
                "asset_id": INPUT_IMAGE_ID,
                "kind": "image"
            }])
            .to_string(),
        ),
        (
            "duplicate kind",
            format!(
                r#"[{{"asset_id":"{INPUT_IMAGE_ID}","kind":"image","kind":"video","role":"reference"}}]"#
            ),
        ),
    ] {
        let update = sqlx::query(
            "UPDATE creation_tasks SET input_bindings = ? WHERE creation_task_id = ?",
        )
        .bind(&payload)
        .bind(standalone_id)
        .execute(&pool)
        .await;
        assert!(update.is_err(), "update trigger must reject {label}");
    }

    let incomplete_insert = sqlx::query(
        "INSERT INTO creation_tasks \
            (creation_task_id, project_id, workbench_kind, provider_id, model, capability, \
             params, input_bindings, status, submitted_at, request_fingerprint) \
         VALUES (?, ?, 'image', ?, 'model', 'i2i', '{}', ?, 'queued', 1, '{}')",
    )
    .bind("0190f5fe-7c00-7a00-8abc-000000000010")
    .bind(PROJECT_ID)
    .bind(PROVIDER_ID)
    .bind(
        serde_json::json!([{
            "asset_id": INPUT_IMAGE_ID,
            "kind": "image"
        }])
        .to_string(),
    )
    .execute(&pool)
    .await;
    assert!(
        incomplete_insert.is_err(),
        "insert trigger must reject a missing required role"
    );
}

#[tokio::test]
async fn migration_045_allows_only_terminal_standalone_tombstones() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    seed_pre_041(&pool).await;
    migrate_to(&pool, 44).await;

    let terminal_id = "0190f5fe-7c00-7a00-8abc-000000000020";
    let live_id = "0190f5fe-7c00-7a00-8abc-000000000021";
    for (task_id, status) in [(terminal_id, "failed"), (live_id, "queued")] {
        sqlx::query(
            "INSERT INTO creation_tasks \
                (creation_task_id, project_id, workbench_kind, provider_id, model, capability, \
                 params, input_bindings, status, submitted_at, finished_at, request_fingerprint) \
             VALUES (?, ?, 'video', ?, 'model', 't2v', '{}', '[]', ?, 10, \
                     CASE WHEN ? = 'failed' THEN 11 ELSE NULL END, '{}')",
        )
        .bind(task_id)
        .bind(PROJECT_ID)
        .bind(PROVIDER_ID)
        .bind(status)
        .bind(status)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "UPDATE creation_tasks SET status = 'failed', finished_at = 11 \
         WHERE creation_task_id = ?",
    )
    .bind(CREATIVE_TASK_ID)
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 45).await;
    let preexisting_tombstones: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM creation_tasks WHERE deleted_at IS NOT NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        preexisting_tombstones, 0,
        "045 must not infer tombstones for existing task history"
    );
    sqlx::query("UPDATE creation_tasks SET deleted_at = 12 WHERE creation_task_id = ?")
        .bind(terminal_id)
        .execute(&pool)
        .await
        .unwrap();
    for (label, task_id, deleted_at) in [
        ("before submit", terminal_id, 9_i64),
        ("live standalone", live_id, 12_i64),
        ("canvas owner", CREATIVE_TASK_ID, 12_i64),
    ] {
        assert!(
            sqlx::query("UPDATE creation_tasks SET deleted_at = ? WHERE creation_task_id = ?")
                .bind(deleted_at)
                .bind(task_id)
                .execute(&pool)
                .await
                .is_err(),
            "045 must reject {label} tombstone"
        );
    }
    for live_status in ["queued", "running"] {
        assert!(
            sqlx::query("UPDATE creation_tasks SET status = ? WHERE creation_task_id = ?")
                .bind(live_status)
                .bind(terminal_id)
                .execute(&pool)
                .await
                .is_err(),
            "a tombstoned terminal task cannot be resurrected as {live_status}"
        );
    }
    let index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema \
         WHERE type = 'index' AND name = 'idx_creation_tasks_workbench_owner_deleted_page'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(index_count, 1);
}

#[tokio::test]
async fn migration_047_makes_standalone_installation_owned_without_rewriting_history() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    seed_pre_041(&pool).await;
    migrate_to(&pool, 46).await;

    let old_standalone_id = "0190f5fe-7c00-7a00-8abc-000000000030";
    let old_fingerprint = serde_json::json!({
        "owner": {
            "kind": "standalone_workbench",
            "project_id": PROJECT_ID,
            "workbench_kind": "image"
        },
        "inputs": []
    })
    .to_string();
    sqlx::query(
        "INSERT INTO creation_tasks \
            (creation_task_id, project_id, workbench_kind, provider_id, model, capability, \
             params, input_bindings, status, submitted_at, request_fingerprint) \
         VALUES (?, ?, 'image', ?, 'legacy-image', 't2i', '{}', '[]', 'failed', 10, ?)",
    )
    .bind(old_standalone_id)
    .bind(PROJECT_ID)
    .bind(PROVIDER_ID)
    .bind(&old_fingerprint)
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 47).await;

    let preserved: (Option<String>, String) = sqlx::query_as(
        "SELECT project_id, request_fingerprint \
         FROM creation_tasks WHERE creation_task_id = ?",
    )
    .bind(old_standalone_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(preserved.0.as_deref(), Some(PROJECT_ID));
    assert_eq!(preserved.1, old_fingerprint, "047 must copy raw fingerprint bytes");

    let new_standalone_id = "0190f5fe-7c00-7a00-8abc-000000000031";
    sqlx::query(
        "INSERT INTO creation_tasks \
            (creation_task_id, project_id, workbench_kind, provider_id, model, capability, \
             params, input_bindings, status, submitted_at, request_fingerprint) \
         VALUES (?, NULL, 'image', ?, 'new-image', 't2i', '{}', '[]', 'queued', 20, ?)",
    )
    .bind(new_standalone_id)
    .bind(PROVIDER_ID)
    .bind(r#"{"owner":{"kind":"standalone_workbench","workbench_kind":"image"},"inputs":[]}"#)
    .execute(&pool)
    .await
    .unwrap();

    let new_project_id: Option<String> = sqlx::query_scalar(
        "SELECT project_id FROM creation_tasks WHERE creation_task_id = ?",
    )
    .bind(new_standalone_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(new_project_id.is_none());

    let mixed_owner = sqlx::query(
        "INSERT INTO creation_tasks \
            (creation_task_id, project_id, workbench_kind, node_id, provider_id, model, capability, \
             params, input_bindings, status, submitted_at, request_fingerprint) \
         VALUES (?, NULL, 'image', ?, ?, 'mixed', 't2i', '{}', '[]', 'queued', 30, '{}')",
    )
    .bind("0190f5fe-7c00-7a00-8abc-000000000032")
    .bind(NODE_ID)
    .bind(PROVIDER_ID)
    .execute(&pool)
    .await;
    assert!(mixed_owner.is_err(), "standalone and CanvasNode fields must stay exclusive");

    let index_sql: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema \
         WHERE type = 'index' AND name = 'idx_creation_tasks_workbench_owner_deleted_page'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let normalized = index_sql.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(normalized.contains(
        "ON creation_tasks( workbench_kind, deleted_at, submitted_at DESC, creation_task_id DESC )"
    ));

    for (asset_id, origin) in [
        (
            "0190f5fe-7c00-7a00-8abc-000000000033",
            serde_json::json!({"workbench_kind": "image"}),
        ),
        (
            "0190f5fe-7c00-7a00-8abc-000000000034",
            serde_json::json!({
                "project_id": PROJECT_ID,
                "workbench_kind": "image"
            }),
        ),
    ] {
        sqlx::query(
            "INSERT INTO workshop_assets \
                (asset_id, kind, title, tags, in_library, origin, created_at, updated_at) \
             VALUES (?, 'image', 'standalone origin', '[]', 1, ?, 1, 1)",
        )
        .bind(asset_id)
        .bind(origin.to_string())
        .execute(&pool)
        .await
        .unwrap();
    }

    for (asset_id, origin) in [
        (
            "0190f5fe-7c00-7a00-8abc-000000000035",
            serde_json::json!({
                "canvas_id": PROJECT_ID,
                "node_id": NODE_ID
            }),
        ),
        (
            "0190f5fe-7c00-7a00-8abc-000000000036",
            serde_json::json!({
                "project_id": PROJECT_ID,
                "node_id": NODE_ID
            }),
        ),
    ] {
        sqlx::query(
            "INSERT INTO workshop_assets \
                (asset_id, kind, title, tags, in_library, origin, created_at, updated_at) \
             VALUES (?, 'image', 'Canvas origin', '[]', 1, ?, 1, 1)",
        )
        .bind(asset_id)
        .bind(origin.to_string())
        .execute(&pool)
        .await
        .unwrap();
    }

    for (label, origin) in [
        (
            "canonical and legacy Canvas identifiers",
            serde_json::json!({
                "canvas_id": PROJECT_ID,
                "project_id": PROJECT_ID,
                "node_id": NODE_ID
            }),
        ),
        (
            "standalone with Canvas identifier",
            serde_json::json!({
                "canvas_id": PROJECT_ID,
                "workbench_kind": "image"
            }),
        ),
    ] {
        let result = sqlx::query(
            "INSERT INTO workshop_assets \
                (asset_id, kind, title, tags, in_library, origin, created_at, updated_at) \
             VALUES (?, 'image', ?, '[]', 1, ?, 1, 1)",
        )
        .bind(nomifun_common::generate_id())
        .bind(label)
        .bind(origin.to_string())
        .execute(&pool)
        .await;
        assert!(result.is_err(), "{label} must be rejected");
    }

    for (index_name, json_path) in [
        ("idx_workshop_assets_origin_canvas_id", "$.canvas_id"),
        ("idx_workshop_assets_origin_project_id", "$.project_id"),
    ] {
        let index_sql: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_schema WHERE type = 'index' AND name = ?",
        )
        .bind(index_name)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            index_sql.contains(json_path),
            "{index_name} must index {json_path}"
        );
    }
}
