use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::Row;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const OWNER_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000001";
const CONVERSATION_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000002";

async fn migrate_to(pool: &sqlx::SqlitePool, maximum_version: i64) {
    let mut connection = pool.acquire().await.unwrap();
    connection.ensure_migrations_table().await.unwrap();
    let applied = connection
        .list_applied_migrations()
        .await
        .unwrap()
        .into_iter()
        .map(|migration| migration.version)
        .collect::<std::collections::BTreeSet<_>>();
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
async fn migration_061_renames_runtime_snapshot_columns_without_aliases() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 60).await;

    sqlx::query(
        "INSERT INTO users \
            (user_id, username, password_hash, created_at, updated_at) \
         VALUES (?, 'agent-snapshot-migration', 'hash', 1, 1)",
    )
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();
    let snapshot =
        r#"{"preset_id":"0190f5fe-7c00-7a00-8abc-000000000003","preset_revision":4}"#;
    sqlx::query(
        "INSERT INTO conversations \
            (conversation_id, user_id, name, type, preset_snapshot, created_at, updated_at) \
         VALUES (?, ?, 'Conversation', 'nomi', ?, 1, 1)",
    )
    .bind(CONVERSATION_ID)
    .bind(OWNER_ID)
    .bind(snapshot)
    .execute(&pool)
    .await
    .unwrap();

    assert!(
        table_columns(&pool, "conversations")
            .await
            .contains(&"preset_snapshot".to_owned())
    );

    migrate_to(&pool, 61).await;

    for table in [
        "conversations",
        "agent_execution_participants",
        "agent_execution_template_participants",
        "cron_jobs",
    ] {
        let columns = table_columns(&pool, table).await;
        assert!(
            columns.contains(&"agent_snapshot".to_owned()),
            "{table} must expose the canonical agent_snapshot column"
        );
        assert!(
            !columns.contains(&"preset_snapshot".to_owned()),
            "{table} must not retain a preset_snapshot compatibility alias"
        );
    }

    let persisted: String =
        sqlx::query_scalar("SELECT agent_snapshot FROM conversations WHERE conversation_id = ?")
            .bind(CONVERSATION_ID)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(persisted, snapshot);
}
