use std::collections::BTreeSet;

use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const OWNER_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000011";
const PRESET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000012";

async fn migrate_to(pool: &sqlx::SqlitePool, maximum_version: i64) {
    let mut connection = pool.acquire().await.unwrap();
    connection.ensure_migrations_table().await.unwrap();
    let applied = connection
        .list_applied_migrations()
        .await
        .unwrap()
        .into_iter()
        .map(|migration| migration.version)
        .collect::<BTreeSet<_>>();
    for migration in MIGRATOR.iter() {
        if migration.version <= maximum_version && !applied.contains(&migration.version) {
            connection.apply(migration).await.unwrap();
        }
    }
}

async fn insert_revision(pool: &sqlx::SqlitePool, revision: i64, snapshot: &str) {
    sqlx::query(
        "INSERT INTO nomi_agent_preset_revisions \
            (revision_id, preset_id, revision_no, schema_version, payload_json, \
             revision_digest, created_by, created_at, reason, snapshot_json, \
             contribution_locks_json) \
         VALUES (?, ?, ?, '1.0.0', '{}', ?, ?, ?, 'materialization refresh', ?, '[]')",
    )
    .bind(format!("{PRESET_ID}@{revision}"))
    .bind(PRESET_ID)
    .bind(revision)
    .bind("a".repeat(64))
    .bind(OWNER_ID)
    .bind(revision)
    .bind(snapshot)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn migration_089_allows_a_new_snapshot_for_the_same_semantic_revision_digest() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 87).await;

    sqlx::query(
        "INSERT INTO users \
            (user_id, username, password_hash, created_at, updated_at) \
         VALUES (?, 'agent-materialization-refresh', 'hash', 1, 1)",
    )
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO nomi_agent_presets \
            (preset_id, owner_user_id, source_kind, display_name, current_revision, created_at) \
         VALUES (?, ?, 'user', 'Materialization refresh', 1, 1)",
    )
    .bind(PRESET_ID)
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();
    insert_revision(&pool, 1, r#"{"snapshot":"before"}"#).await;

    migrate_to(&pool, 89).await;
    insert_revision(&pool, 2, r#"{"snapshot":"after"}"#).await;

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM nomi_agent_preset_revisions \
         WHERE preset_id = ? AND revision_digest = ?",
    )
    .bind(PRESET_ID)
    .bind("a".repeat(64))
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 2);
}
