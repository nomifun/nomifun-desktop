use std::collections::BTreeSet;

use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::Row;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const OWNER_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000001";
const PRESET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000002";

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

#[tokio::test]
async fn migration_065_adds_retirement_tombstone_without_deleting_history() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 64).await;

    sqlx::query(
        "INSERT INTO users \
         (user_id, username, password_hash, created_at, updated_at) \
         VALUES (?, 'agent-preset-retirement', 'hash', 1, 1)",
    )
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO nomi_agent_presets \
         (preset_id, owner_user_id, source_kind, display_name, current_revision, created_at) \
         VALUES (?, ?, 'user', 'Retirement migration', 1, 1)",
    )
    .bind(PRESET_ID)
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO nomi_agent_preset_revisions \
         (revision_id, preset_id, revision_no, schema_version, payload_json, \
          revision_digest, created_by, created_at, reason, snapshot_json, \
          contribution_locks_json) \
         VALUES (?, ?, 1, '1.0.0', '{}', ?, ?, 1, '', '{}', '[]')",
    )
    .bind(format!("{PRESET_ID}@1"))
    .bind(PRESET_ID)
    .bind("a".repeat(64))
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 65).await;

    let latest: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(latest, 65);

    let columns = sqlx::query("PRAGMA table_info(nomi_agent_presets)")
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<Vec<_>>();
    assert!(columns.contains(&"retired_at_ms".to_owned()));
    let active_index: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema \
         WHERE type = 'index' AND name = 'idx_nomi_agent_presets_active_owner'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(active_index.contains("WHERE retired_at_ms IS NULL"));

    let initial: Option<i64> = sqlx::query_scalar(
        "SELECT retired_at_ms FROM nomi_agent_presets WHERE preset_id = ?",
    )
    .bind(PRESET_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(initial, None);

    sqlx::query(
        "UPDATE nomi_agent_presets SET retired_at_ms = 65 WHERE preset_id = ?",
    )
    .bind(PRESET_ID)
    .execute(&pool)
    .await
    .unwrap();
    let revision_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM nomi_agent_preset_revisions WHERE preset_id = ?",
    )
    .bind(PRESET_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(revision_count, 1);

    let negative = sqlx::query(
        "UPDATE nomi_agent_presets SET retired_at_ms = -1 WHERE preset_id = ?",
    )
    .bind(PRESET_ID)
    .execute(&pool)
    .await;
    assert!(negative.is_err(), "negative retirement timestamps must fail");
}
