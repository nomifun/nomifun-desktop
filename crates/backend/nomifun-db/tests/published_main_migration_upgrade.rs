use std::path::Path;

use nomifun_db::{MigrationLineageStatus, init_database, inspect_supported_migration_lineage};
use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

async fn open(path: &Path, read_only: bool) -> SqlitePool {
    SqlitePoolOptions::new().max_connections(1).connect_with(
        SqliteConnectOptions::new().filename(path).create_if_missing(!read_only).read_only(read_only),
    ).await.unwrap()
}

async fn released_main(path: &Path, head: i64) -> (SqlitePool, String) {
    let pool = open(path, false).await;
    let mut connection = pool.acquire().await.unwrap();
    connection.ensure_migrations_table().await.unwrap();
    for migration in MIGRATOR.iter().filter(|migration| migration.version <= 58) {
        connection.apply(migration).await.unwrap();
    }
    for (published, canonical) in [(59, 73), (60, 74)] {
        if published <= head {
            let mut migration = MIGRATOR.iter().find(|migration| migration.version == canonical).unwrap().clone();
            migration.version = published;
            connection.apply(&migration).await.unwrap();
        }
    }
    drop(connection);
    let owner = nomifun_common::UserId::new().into_string();
    sqlx::query("INSERT INTO users (user_id, username, password_hash, created_at, updated_at) VALUES (?, 'admin', '', 1, 1)")
        .bind(&owner).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO installation_identity (singleton_key, owner_user_id) VALUES ('installation', ?)")
        .bind(owner).execute(&pool).await.unwrap();
    let asset = nomifun_common::WorkshopAssetId::new().into_string();
    sqlx::query("INSERT INTO workshop_assets (asset_id, kind, title, tags, rel_path, in_library, deleted_at, created_at, updated_at) VALUES (?, 'image', 'pending deletion history', '[]', 'workshop/keep-until-cleanup.png', 0, 15, 1, 15)")
        .bind(&asset).execute(&pool).await.unwrap();
    (pool, asset)
}

async fn ledger(pool: &SqlitePool) -> Vec<(i64, Vec<u8>, String, String)> {
    sqlx::query_as("SELECT version, checksum, description, CAST(installed_on AS TEXT) FROM _sqlx_migrations ORDER BY version")
        .fetch_all(pool).await.unwrap()
}

async fn verify_main_upgrade(head: i64) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("published-main.db");
    let (pool, asset) = released_main(&path, head).await;
    let before = ledger(&pool).await;
    assert_eq!(before.iter().map(|row| row.0).collect::<Vec<_>>(),
        MIGRATOR.iter().filter(|migration| migration.version <= 58)
            .map(|migration| migration.version).chain(59..=head).collect::<Vec<_>>());
    pool.close().await;
    let probe = open(&path, true).await;
    assert_eq!(inspect_supported_migration_lineage(&probe).await.unwrap(), MigrationLineageStatus::UpgradeRequired);
    assert_eq!(ledger(&probe).await, before, "read-only probing must not rewrite a ledger");
    probe.close().await;

    let db = init_database(&path).await.unwrap();
    assert_eq!(inspect_supported_migration_lineage(db.pool()).await.unwrap(), MigrationLineageStatus::Current);
    let after = ledger(db.pool()).await;
    assert_eq!(after.len(), MIGRATOR.iter().count());
    for original in &before {
        let canonical = match original.0 {
            59 => 73,
            60 => 74,
            version => version,
        };
        let moved = after.iter().find(|row| row.0 == canonical).unwrap();
        assert_eq!((&moved.1, &moved.2, &moved.3), (&original.1, &original.2, &original.3));
    }
    let row: (String, String, i64, Option<i64>) = sqlx::query_as("SELECT title, rel_path, deleted_at, content_deleted_at FROM workshop_assets WHERE asset_id = ?")
        .bind(&asset).fetch_one(db.pool()).await.unwrap();
    assert_eq!(row, ("pending deletion history".into(), "workshop/keep-until-cleanup.png".into(), 15, None));
    assert!(sqlx::query("UPDATE workshop_assets SET deleted_at = NULL WHERE asset_id = ?").bind(&asset).execute(db.pool()).await.is_err());
    for table in ["remote_bindings", "nomi_agent_presets", "plugin_artifacts", "plugin_products"] {
        let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name=?")
            .bind(table).fetch_one(db.pool()).await.unwrap();
        assert_eq!(exists, 1, "refactor table {table} must exist after main upgrade");
    }
    db.close().await;
    let reopened = init_database(&path).await.unwrap();
    assert_eq!(ledger(reopened.pool()).await, after, "reopening must not repeat reconciliation");
    reopened.close().await;
}

#[tokio::test]
async fn published_main_059_converges_without_losing_pending_asset_deletions() {
    verify_main_upgrade(59).await;
}

#[tokio::test]
async fn published_main_060_converges_without_losing_pending_asset_deletions() {
    verify_main_upgrade(60).await;
}

#[tokio::test]
async fn failed_main_adoption_rolls_back_ledger_moves_and_refactor_schema() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("must-rollback.db");
    let (pool, asset) = released_main(&path, 60).await;
    sqlx::query("CREATE TABLE plugin_artifacts (sentinel TEXT NOT NULL)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO plugin_artifacts VALUES ('preserve this conflicting table')").execute(&pool).await.unwrap();
    let before = ledger(&pool).await;
    pool.close().await;
    assert!(init_database(&path).await.is_err());
    let pool = open(&path, true).await;
    assert_eq!(ledger(&pool).await, before);
    let sentinel: String = sqlx::query_scalar("SELECT sentinel FROM plugin_artifacts").fetch_one(&pool).await.unwrap();
    assert_eq!(sentinel, "preserve this conflicting table");
    let partial_schema: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name IN ('remote_bindings', 'nomi_agent_presets')",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(partial_schema, 0, "a failure at migration 067 must also roll back the earlier refactor DDL");
    let title: String = sqlx::query_scalar("SELECT title FROM workshop_assets WHERE asset_id=?").bind(asset).fetch_one(&pool).await.unwrap();
    assert_eq!(title, "pending deletion history");
    pool.close().await;
}

#[tokio::test]
async fn edited_main_checksum_is_rejected_without_ledger_reconciliation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("wrong-checksum.db");
    let (pool, _) = released_main(&path, 60).await;
    sqlx::query("UPDATE _sqlx_migrations SET checksum=zeroblob(48) WHERE version=60").execute(&pool).await.unwrap();
    let before = ledger(&pool).await;
    assert!(inspect_supported_migration_lineage(&pool).await.is_err());
    pool.close().await;
    assert!(init_database(&path).await.is_err());
    let pool = open(&path, true).await;
    assert_eq!(ledger(&pool).await, before);
    pool.close().await;
}

#[tokio::test]
async fn malformed_published_prefixes_fail_before_any_schema_or_ledger_write() {
    for (case, mutation) in [
        ("missing common migration", "DELETE FROM _sqlx_migrations WHERE version=28"),
        ("missing published 059", "DELETE FROM _sqlx_migrations WHERE version=59"),
        ("occupied retired gap", "UPDATE _sqlx_migrations SET version=27 WHERE version=28"),
        ("unknown extra version", "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES (61, 'unknown', 1, zeroblob(48), 0)"),
        ("occupied relocation target", "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES (73, 'occupied', 1, zeroblob(48), 0)"),
        ("failed common migration", "UPDATE _sqlx_migrations SET success=0 WHERE version=58"),
        ("unknown common checksum", "UPDATE _sqlx_migrations SET checksum=zeroblob(48) WHERE version=58"),
        ("unknown published checksum", "UPDATE _sqlx_migrations SET checksum=zeroblob(48) WHERE version=59"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("malformed-main.db");
        let (pool, asset) = released_main(&path, 60).await;
        sqlx::query(mutation).execute(&pool).await.unwrap();
        let before = ledger(&pool).await;
        let before_success: Vec<(i64, bool)> = sqlx::query_as(
            "SELECT version, success FROM _sqlx_migrations ORDER BY version"
        ).fetch_all(&pool).await.unwrap();
        let before_schema: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT name, sql FROM sqlite_schema ORDER BY name"
        ).fetch_all(&pool).await.unwrap();
        assert!(inspect_supported_migration_lineage(&pool).await.is_err(), "{case}");
        assert_eq!(ledger(&pool).await, before, "read-only inspection: {case}");
        pool.close().await;

        assert!(init_database(&path).await.is_err(), "{case}");
        let pool = open(&path, true).await;
        assert_eq!(ledger(&pool).await, before, "ledger: {case}");
        let after_success: Vec<(i64, bool)> = sqlx::query_as(
            "SELECT version, success FROM _sqlx_migrations ORDER BY version"
        ).fetch_all(&pool).await.unwrap();
        assert_eq!(after_success, before_success, "success flags: {case}");
        let after_schema: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT name, sql FROM sqlite_schema ORDER BY name"
        ).fetch_all(&pool).await.unwrap();
        assert_eq!(after_schema, before_schema, "no partial DDL: {case}");
        let row: (String, i64) = sqlx::query_as(
            "SELECT title, deleted_at FROM workshop_assets WHERE asset_id=?"
        ).bind(&asset).fetch_one(&pool).await.unwrap();
        assert_eq!(row, ("pending deletion history".into(), 15), "data: {case}");
        pool.close().await;
    }
}
