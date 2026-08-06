//! Migration 025 (drop `writeback_mode`, move the write-back disposition to
//! `manual`/`auto`) over pre-migration data shapes, plus the second, independent
//! `preset_knowledge_policy` value domain that a bindings-only migration would
//! leave rejecting the new vocabulary forever.

use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const BINDING_STAGED: &str = "0190f5fe-7c00-7a00-8abc-0123456789d1";
const BINDING_DIRECT: &str = "0190f5fe-7c00-7a00-8abc-0123456789d2";
const BINDING_FRESH: &str = "0190f5fe-7c00-7a00-8abc-0123456789d3";
const PRESET_CONSERVATIVE: &str = "0190f5fe-7c00-7a00-8abc-0123456789e1";
const PRESET_AGGRESSIVE: &str = "0190f5fe-7c00-7a00-8abc-0123456789e2";
const PRESET_UNSET: &str = "0190f5fe-7c00-7a00-8abc-0123456789e3";

/// Apply migrations up to (and including) `max_version`, skipping versions
/// already recorded in `_sqlx_migrations` so repeated calls are incremental.
async fn migrate_to(pool: &sqlx::SqlitePool, max_version: i64) {
    let mut conn = pool.acquire().await.unwrap();
    conn.ensure_migrations_table().await.unwrap();
    let applied: std::collections::BTreeSet<i64> = conn
        .list_applied_migrations()
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.version)
        .collect();
    for m in MIGRATOR.iter() {
        if m.version <= max_version && !applied.contains(&m.version) {
            conn.apply(m).await.unwrap();
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
    for (preset, mode, eagerness) in [
        (PRESET_CONSERVATIVE, "staged", Some("conservative")),
        (PRESET_AGGRESSIVE, "direct", Some("aggressive")),
        // `eagerness` is nullable — NULL means "unspecified, inherit the mount".
        (PRESET_UNSET, "inherit", None),
    ] {
        sqlx::query(
            "INSERT INTO preset_knowledge_policy \
                (preset_id, enabled, mode, writeback, eagerness, grounded) \
             VALUES (?, 1, ?, 1, ?, 0)",
        )
        .bind(preset)
        .bind(mode)
        .bind(eagerness)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn columns(pool: &sqlx::SqlitePool, table: &str) -> Vec<String> {
    sqlx::query_scalar(&format!("SELECT name FROM pragma_table_info('{table}')"))
        .fetch_all(pool)
        .await
        .unwrap()
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

    assert!(
        !columns(&pool, "knowledge_bindings").await.iter().any(|c| c == "writeback_mode"),
        "writeback_mode has one legal value left once staging is gone, so the column goes"
    );
    assert_eq!(
        binding_eagerness(&pool, BINDING_STAGED).await,
        "manual",
        "conservative was the restrained side, and manual is its successor"
    );
    assert_eq!(
        binding_eagerness(&pool, BINDING_DIRECT).await,
        "auto",
        "aggressive mapped to the self-directed side"
    );
}

#[tokio::test]
async fn migration_keeps_the_ddl_default_in_step_with_the_new_check() {
    let pool = memory_pool().await;
    seed_pre_025(&pool).await;
    migrate_to(&pool, 25).await;

    // sqlite_conversation inserts bindings without naming the disposition, so a
    // narrowed CHECK over a stale DEFAULT would break every conversation open.
    sqlx::query(
        "INSERT INTO knowledge_bindings \
            (knowledge_binding_id, target_kind, target_workpath, updated_at) \
         VALUES (?, 'workpath', '/fresh', 1)",
    )
    .bind(BINDING_FRESH)
    .execute(&pool)
    .await
    .expect("an insert that relies on the DDL default must still satisfy the new CHECK");
    assert_eq!(binding_eagerness(&pool, BINDING_FRESH).await, "manual");
}

#[tokio::test]
async fn migration_rejects_the_retired_vocabulary() {
    let pool = memory_pool().await;
    seed_pre_025(&pool).await;
    migrate_to(&pool, 25).await;

    for stale in ["conservative", "aggressive", "staged", "direct"] {
        let err = sqlx::query(
            "INSERT INTO knowledge_bindings \
                (knowledge_binding_id, target_kind, target_workpath, writeback_eagerness, updated_at) \
             VALUES ('0190f5fe-7c00-7a00-8abc-0123456789f9', 'workpath', '/stale', ?, 1)",
        )
        .bind(stale)
        .execute(&pool)
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("CHECK"),
            "the retired value {stale} must be refused by the CHECK, got: {err}"
        );
    }
}

#[tokio::test]
async fn migration_fixes_the_second_disposition_domain_on_preset_policy() {
    let pool = memory_pool().await;
    seed_pre_025(&pool).await;

    migrate_to(&pool, 25).await;

    let cols = columns(&pool, "preset_knowledge_policy").await;
    assert!(
        !cols.iter().any(|c| c == "mode"),
        "the preset policy's mode carried inherit|staged|direct and loses all meaning"
    );
    assert!(cols.iter().any(|c| c == "eagerness"));

    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT preset_id, eagerness FROM preset_knowledge_policy ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            (PRESET_CONSERVATIVE.to_owned(), Some("manual".to_owned())),
            (PRESET_AGGRESSIVE.to_owned(), Some("auto".to_owned())),
            // NULL must stay NULL: "unspecified" is not the same as "manual".
            (PRESET_UNSET.to_owned(), None),
        ]
    );

    let err = sqlx::query(
        "UPDATE preset_knowledge_policy SET eagerness = 'conservative' WHERE preset_id = ?",
    )
    .bind(PRESET_UNSET)
    .execute(&pool)
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("CHECK"),
        "the preset policy's own CHECK must refuse the retired vocabulary too, got: {err}"
    );
}

#[tokio::test]
async fn fresh_database_passes_the_schema_contract_without_the_placement_column() {
    // init_database_memory runs ALL migrations plus the id schema contract,
    // which validates the four partial unique indexes on every open — a careless
    // table rebuild instead of DROP COLUMN would fail here.
    let db = nomifun_db::init_database_memory().await.unwrap();
    let cols = columns(db.pool(), "knowledge_bindings").await;
    assert!(!cols.iter().any(|c| c == "writeback_mode"));
    let default_disposition: String = sqlx::query_scalar(
        "SELECT dflt_value FROM pragma_table_info('knowledge_bindings') \
         WHERE name = 'writeback_eagerness'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(default_disposition, "'manual'");
}
