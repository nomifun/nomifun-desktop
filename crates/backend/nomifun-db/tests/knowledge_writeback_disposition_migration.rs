//! Migration 025 knowledge-binding disposition contract.

use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const BINDING_STAGED: &str = "0190f5fe-7c00-7a00-8abc-0123456789d1";
const BINDING_DIRECT: &str = "0190f5fe-7c00-7a00-8abc-0123456789d2";
const BINDING_FRESH: &str = "0190f5fe-7c00-7a00-8abc-0123456789d3";

async fn migrate_to(pool: &sqlx::SqlitePool, max_version: i64) {
    let mut conn = pool.acquire().await.unwrap();
    conn.ensure_migrations_table().await.unwrap();
    let applied = conn
        .list_applied_migrations()
        .await
        .unwrap()
        .into_iter()
        .map(|migration| migration.version)
        .collect::<std::collections::BTreeSet<_>>();
    for migration in MIGRATOR.iter() {
        if migration.version <= max_version && !applied.contains(&migration.version) {
            conn.apply(migration).await.unwrap();
        }
    }
}

async fn seed_pre_025(pool: &sqlx::SqlitePool) {
    migrate_to(pool, 24).await;
    for (id, workpath, mode, eagerness) in [
        (BINDING_STAGED, "/a", "staged", "conservative"),
        (BINDING_DIRECT, "/b", "direct", "aggressive"),
    ] {
        sqlx::query(
            "INSERT INTO knowledge_bindings \
                (knowledge_binding_id, target_kind, target_workpath, enabled, writeback, \
                 writeback_mode, writeback_eagerness, updated_at) \
             VALUES (?, 'workpath', ?, 1, 1, ?, ?, 1)",
        )
        .bind(id)
        .bind(workpath)
        .bind(mode)
        .bind(eagerness)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn binding_eagerness(pool: &sqlx::SqlitePool, id: &str) -> String {
    sqlx::query_scalar(
        "SELECT writeback_eagerness FROM knowledge_bindings WHERE knowledge_binding_id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn memory_pool() -> sqlx::SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap()
}

#[tokio::test]
async fn migration_maps_legacy_dispositions_and_drops_the_placement_column() {
    let pool = memory_pool().await;
    seed_pre_025(&pool).await;
    migrate_to(&pool, 25).await;

    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('knowledge_bindings')")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(!columns.iter().any(|column| column == "writeback_mode"));
    assert_eq!(binding_eagerness(&pool, BINDING_STAGED).await, "manual");
    assert_eq!(binding_eagerness(&pool, BINDING_DIRECT).await, "auto");
}

#[tokio::test]
async fn migration_keeps_the_ddl_default_in_step_with_the_new_check() {
    let pool = memory_pool().await;
    seed_pre_025(&pool).await;
    migrate_to(&pool, 25).await;

    sqlx::query(
        "INSERT INTO knowledge_bindings \
            (knowledge_binding_id, target_kind, target_workpath, updated_at) \
         VALUES (?, 'workpath', '/fresh', 1)",
    )
    .bind(BINDING_FRESH)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(binding_eagerness(&pool, BINDING_FRESH).await, "manual");
}

#[tokio::test]
async fn migration_rejects_the_retired_vocabulary() {
    let pool = memory_pool().await;
    seed_pre_025(&pool).await;
    migrate_to(&pool, 25).await;

    for stale in ["conservative", "aggressive", "staged", "direct"] {
        let error = sqlx::query(
            "INSERT INTO knowledge_bindings \
                (knowledge_binding_id, target_kind, target_workpath, writeback_eagerness, updated_at) \
             VALUES ('0190f5fe-7c00-7a00-8abc-0123456789f9', 'workpath', '/stale', ?, 1)",
        )
        .bind(stale)
        .execute(&pool)
        .await
        .unwrap_err();
        assert!(error.to_string().contains("CHECK"));
    }
}

#[tokio::test]
async fn fresh_database_passes_the_schema_contract_without_the_placement_column() {
    let db = nomifun_db::init_database_memory().await.unwrap();
    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('knowledge_bindings')")
            .fetch_all(db.pool())
            .await
            .unwrap();
    assert!(!columns.iter().any(|column| column == "writeback_mode"));
}
