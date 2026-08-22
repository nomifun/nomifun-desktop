//! Migration 043 removes the discarded file-backed canvas index while
//! preserving asset rows and canonicalising their provenance.

use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const CANVAS_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000001";
const NODE_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000002";
const ASSET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000003";
const INTERNAL_ASSET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000004";
const PROJECT_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000005";
const PROJECT_NODE_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000006";

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

async fn seed_retired_canvas_data(pool: &sqlx::SqlitePool) {
    migrate_to(pool, 42).await;
    sqlx::query(
        "INSERT INTO workshop_canvases \
            (canvas_id, title, node_count, created_at, updated_at) \
         VALUES (?, 'Retired Canvas', 1, 1, 1)",
    )
    .bind(CANVAS_ID)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO workshop_assets \
            (asset_id, kind, title, tags, in_library, origin, created_at, updated_at) \
         VALUES (?, 'image', 'Preserved Asset', '[]', 1, ?, 1, 1)",
    )
    .bind(ASSET_ID)
    .bind(
        serde_json::json!({
            "prompt": "legacy prompt",
            "canvas_id": CANVAS_ID,
            "node_id": NODE_ID
        })
        .to_string(),
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO workshop_assets \
            (asset_id, kind, title, tags, in_library, origin, created_at, updated_at) \
         VALUES (?, 'text', 'Internal Asset', '[]', 0, ?, 1, 1)",
    )
    .bind(INTERNAL_ASSET_ID)
    .bind(
        serde_json::json!({
            "canvas_id": CANVAS_ID,
            "node_id": NODE_ID
        })
        .to_string(),
    )
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn migration_drops_canvas_index_and_preserves_assets_without_legacy_owners() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    seed_retired_canvas_data(&pool).await;

    migrate_to(&pool, 43).await;

    let canvas_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'workshop_canvases'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(canvas_table_count, 0);

    let origin: Option<String> =
        sqlx::query_scalar("SELECT origin FROM workshop_assets WHERE asset_id = ?")
            .bind(ASSET_ID)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(origin.as_deref().unwrap()).unwrap(),
        serde_json::json!({"prompt": "legacy prompt"})
    );
    let internal_origin: Option<String> =
        sqlx::query_scalar("SELECT origin FROM workshop_assets WHERE asset_id = ?")
            .bind(INTERNAL_ASSET_ID)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(internal_origin.is_none());
}

#[tokio::test]
async fn migration_rejects_retired_canvas_provenance_and_accepts_canonical_project_owners() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 43).await;

    let retired = sqlx::query(
        "INSERT INTO workshop_assets \
            (asset_id, kind, title, tags, in_library, origin, created_at, updated_at) \
         VALUES (?, 'image', 'Retired Origin', '[]', 1, ?, 1, 1)",
    )
    .bind(ASSET_ID)
    .bind(serde_json::json!({"canvas_id": CANVAS_ID, "node_id": NODE_ID}).to_string())
    .execute(&pool)
    .await;
    assert!(retired.is_err(), "canvas_id must be rejected after migration 043");

    sqlx::query(
        "INSERT INTO creative_studio_projects \
            (project_id, title, revision, node_count, connection_count, document_json, \
             created_at, updated_at) \
         VALUES (?, 'Canonical Project', 1, 0, 0, ?, 1, 1)",
    )
    .bind(PROJECT_ID)
    .bind(
        serde_json::json!({
            "schema": "nomifun.creative-studio/v1",
            "projectId": PROJECT_ID,
            "nodes": []
        })
        .to_string(),
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO workshop_assets \
            (asset_id, kind, title, tags, in_library, origin, created_at, updated_at) \
         VALUES (?, 'image', 'Canonical Origin', '[]', 1, ?, 1, 1)",
    )
    .bind(ASSET_ID)
    .bind(
        serde_json::json!({
            "project_id": PROJECT_ID,
            "node_id": PROJECT_NODE_ID
        })
        .to_string(),
    )
    .execute(&pool)
    .await
    .unwrap();

    let orphan_node = sqlx::query(
        "UPDATE workshop_assets SET origin = ? WHERE asset_id = ?",
    )
    .bind(serde_json::json!({"node_id": PROJECT_NODE_ID}).to_string())
    .bind(ASSET_ID)
    .execute(&pool)
    .await;
    assert!(orphan_node.is_err(), "node_id must not exist without project_id");
}
