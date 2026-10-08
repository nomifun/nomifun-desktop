use nomifun_db::{init_database, validate_current_migration_lineage};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

static MIGRATOR: Migrator = sqlx::migrate!();
const NOMI: &str = "0190f5fe-7c00-7a00-8000-000000000114";
const RETIRED_COLUMNS: &[&str] = &[
    "yolo_id", "agent_capabilities", "auth_methods", "config_options",
    "available_modes", "available_models", "available_commands",
];

async fn previous_database(path: &std::path::Path) -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(
        SqliteConnectOptions::new().filename(path).create_if_missing(true)
    ).await.unwrap();
    sqlx::raw_sql("CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY, description TEXT NOT NULL, installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, success BOOLEAN NOT NULL, checksum BLOB NOT NULL, execution_time BIGINT NOT NULL)")
        .execute(&pool).await.unwrap();
    for migration in MIGRATOR.iter().take(2) {
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO _sqlx_migrations(version,description,success,checksum,execution_time) VALUES (?,?,1,?,0)")
            .bind(migration.version).bind(migration.description.as_ref()).bind(migration.checksum.as_ref())
            .execute(&pool).await.unwrap();
    }
    sqlx::query("UPDATE agent_metadata SET name='Saved name',enabled=0,behavior_policy='{\"supports_side_question\":true}',agent_capabilities='{\"stale\":true}',available_models='[\"retired\"]' WHERE agent_id=?")
        .bind(NOMI).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO client_preferences(key,value,updated_at) VALUES ('cache-migration-probe','preserved',1)")
        .execute(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn removes_discovery_cache_preserves_metadata_and_known_lineage_on_restart() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("metadata.sqlite");
    let previous = previous_database(&path).await;
    let before: Vec<(String, String)> = sqlx::query_as("SELECT name,sql FROM sqlite_schema WHERE type='table' AND name LIKE 'agent_%' AND name<>'agent_metadata' ORDER BY name")
        .fetch_all(&previous).await.unwrap();
    previous.close().await;

    for _ in 0..2 {
        let database = init_database(&path).await.unwrap();
        let pool = database.pool();
        validate_current_migration_lineage(pool).await.unwrap();
        let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('agent_metadata')")
            .fetch_all(pool).await.unwrap();
        assert!(RETIRED_COLUMNS.iter().all(|name| !columns.iter().any(|column| column == name)));
        let metadata: (String, bool, String, String) = sqlx::query_as("SELECT name,enabled,behavior_policy,source_key FROM agent_metadata WHERE agent_id=?")
            .bind(NOMI).fetch_one(pool).await.unwrap();
        assert_eq!(metadata, ("Saved name".into(), false, "{\"supports_side_question\":true}".into(), "agent_builtin_nomi".into()));
        assert_eq!(sqlx::query_scalar::<_, String>("SELECT value FROM client_preferences WHERE key='cache-migration-probe'")
            .fetch_one(pool).await.unwrap(), "preserved");
        let after: Vec<(String, String)> = sqlx::query_as("SELECT name,sql FROM sqlite_schema WHERE type='table' AND name LIKE 'agent_%' AND name<>'agent_metadata' ORDER BY name")
            .fetch_all(pool).await.unwrap();
        assert_eq!(after, before, "retiring discovery metadata must not change canonical Agent tables");
        let receipts: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool).await.unwrap();
        assert_eq!(receipts, MIGRATOR.iter().map(|migration| (migration.version, migration.checksum.to_vec())).collect::<Vec<_>>());
        database.close().await;
    }
}

#[tokio::test]
async fn invalid_lineage_cannot_authorize_discovery_cache_removal() {
    for mutation in [
        "UPDATE _sqlx_migrations SET checksum=X'00' WHERE version=1",
        "DELETE FROM _sqlx_migrations WHERE version=1",
        "UPDATE _sqlx_migrations SET success=0 WHERE version=2",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("unknown.sqlite");
        let previous = previous_database(&path).await;
        sqlx::query(mutation).execute(&previous).await.unwrap();
        assert!(init_database(&path).await.is_err(), "invalid lineage must fail closed: {mutation}");
        assert_eq!(sqlx::query_scalar::<_, String>("SELECT agent_capabilities FROM agent_metadata WHERE agent_id=?")
            .bind(NOMI).fetch_one(&previous).await.unwrap(), "{\"stale\":true}");
        previous.close().await;
    }
}
