use nomifun_db::{MigrationLineageStatus, inspect_supported_migration_lineage};
use serde_json::Value;
use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const OWNER_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000001";
const CONVERSATION_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000002";
const PRESET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000003";

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

#[tokio::test]
async fn migration_095_converges_retired_snapshot_capability_buckets() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 94).await;
    sqlx::query(
        "INSERT INTO users \
            (user_id, username, password_hash, created_at, updated_at) \
         VALUES (?, 'snapshot-capability-migration', 'hash', 1, 1)",
    )
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();

    let snapshot = serde_json::json!({
        "preset_id": PRESET_ID,
        "preset_revision": 4,
        "preset_name": "Agent",
        "instructions": "",
        "included_skills": [],
        "excluded_auto_skills": [],
        "enabled_capabilities": ["already.enabled", "shared"],
        "initial_capabilities": ["initial.only", "shared"],
        "on_demand_capabilities": ["on-demand.only", "shared"],
        "required_resource_kinds": [],
        "knowledge_policy": {
            "enabled": false,
            "writeback": false,
            "grounded": false
        },
        "warnings": []
    })
    .to_string();
    sqlx::query(
        "INSERT INTO conversations \
            (conversation_id, user_id, name, type, preset_id, preset_revision, \
             agent_snapshot, created_at, updated_at) \
         VALUES (?, ?, 'Conversation', 'nomi', ?, 4, ?, 10, 20)",
    )
    .bind(CONVERSATION_ID)
    .bind(OWNER_ID)
    .bind(PRESET_ID)
    .bind(snapshot)
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 95).await;

    let persisted: String =
        sqlx::query_scalar("SELECT agent_snapshot FROM conversations WHERE conversation_id = ?")
            .bind(CONVERSATION_ID)
            .fetch_one(&pool)
            .await
            .unwrap();
    let persisted: Value = serde_json::from_str(&persisted).unwrap();
    assert!(persisted.get("initial_capabilities").is_none());
    assert!(persisted.get("on_demand_capabilities").is_none());
    assert_eq!(
        persisted["enabled_capabilities"],
        serde_json::json!([
            "already.enabled",
            "initial.only",
            "on-demand.only",
            "shared"
        ])
    );
    assert_eq!(persisted["preset_id"], PRESET_ID);
    let updated_at: i64 =
        sqlx::query_scalar("SELECT updated_at FROM conversations WHERE conversation_id = ?")
            .bind(CONVERSATION_ID)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(updated_at, 20);
    assert_eq!(
        inspect_supported_migration_lineage(&pool).await.unwrap(),
        MigrationLineageStatus::Current
    );
}

#[tokio::test]
async fn migration_095_rejects_non_string_retired_capabilities_without_mutation() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 94).await;
    sqlx::query(
        "INSERT INTO users \
            (user_id, username, password_hash, created_at, updated_at) \
         VALUES (?, 'invalid-snapshot-capability', 'hash', 1, 1)",
    )
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();
    let snapshot = serde_json::json!({
        "preset_id": PRESET_ID,
        "preset_revision": 4,
        "preset_name": "Agent",
        "initial_capabilities": [{"unexpected": true}],
        "on_demand_capabilities": []
    })
    .to_string();
    sqlx::query(
        "INSERT INTO conversations \
            (conversation_id, user_id, name, type, preset_id, preset_revision, \
             agent_snapshot, created_at, updated_at) \
         VALUES (?, ?, 'Conversation', 'nomi', ?, 4, ?, 10, 20)",
    )
    .bind(CONVERSATION_ID)
    .bind(OWNER_ID)
    .bind(PRESET_ID)
    .bind(&snapshot)
    .execute(&pool)
    .await
    .unwrap();

    let mut connection = pool.acquire().await.unwrap();
    let migration = MIGRATOR
        .iter()
        .find(|migration| migration.version == 95)
        .unwrap();
    assert!(connection.apply(migration).await.is_err());
    drop(connection);

    let persisted: String =
        sqlx::query_scalar("SELECT agent_snapshot FROM conversations WHERE conversation_id = ?")
            .bind(CONVERSATION_ID)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(persisted, snapshot);
    let applied: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = 95")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(applied, 0);
}
