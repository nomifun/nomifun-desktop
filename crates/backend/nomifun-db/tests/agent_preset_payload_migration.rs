use std::collections::BTreeSet;

use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::Row;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const OWNER_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000001";
const PRESET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000002";
const REVISION_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000002@1";

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

async fn table_columns(pool: &sqlx::SqlitePool, table: &str) -> Vec<String> {
    sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect()
}

#[tokio::test]
async fn migration_064_renames_agent_preset_revision_payload_without_aliases() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 63).await;

    sqlx::query(
        "INSERT INTO users \
            (user_id, username, password_hash, created_at, updated_at) \
         VALUES (?, 'agent-preset-payload-migration', 'hash', 1, 1)",
    )
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO nomi_agent_presets \
            (preset_id, owner_user_id, source_kind, display_name, current_revision, created_at) \
         VALUES (?, ?, 'user', 'Payload migration', 1, 1)",
    )
    .bind(PRESET_ID)
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();

    let payload = r#"{"schema_version":"1.0.0","system_prompt":"preserve me"}"#;
    let snapshot = r#"{"snapshot":"preserve me"}"#;
    let contribution_locks = r#"[{"contribution_id":"preserve-me"}]"#;
    sqlx::query(
        "INSERT INTO nomi_agent_preset_revisions \
            (revision_id, preset_id, revision_no, schema_version, editor_document_json, \
             revision_digest, created_by, created_at, reason, snapshot_json, \
             contribution_locks_json) \
         VALUES (?, ?, 1, '1.0.0', ?, ?, ?, 1, 'migration test', ?, ?)",
    )
    .bind(REVISION_ID)
    .bind(PRESET_ID)
    .bind(payload)
    .bind("a".repeat(64))
    .bind(OWNER_ID)
    .bind(snapshot)
    .bind(contribution_locks)
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 64).await;

    let columns = table_columns(&pool, "nomi_agent_preset_revisions").await;
    assert!(
        columns.contains(&"payload_json".to_owned()),
        "canonical payload_json column must exist"
    );
    assert!(
        !columns.contains(&"editor_document_json".to_owned()),
        "the retired editor_document_json column must not remain as an alias"
    );

    let persisted: (String, String, String) = sqlx::query_as(
        "SELECT payload_json, snapshot_json, contribution_locks_json \
         FROM nomi_agent_preset_revisions WHERE revision_id = ?",
    )
    .bind(REVISION_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(persisted.0, payload);
    assert_eq!(persisted.1, snapshot);
    assert_eq!(persisted.2, contribution_locks);

    let table_sql: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema \
         WHERE type = 'table' AND name = 'nomi_agent_preset_revisions'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(table_sql.contains("payload_json"));
    assert!(!table_sql.contains("editor_document_json"));
}
