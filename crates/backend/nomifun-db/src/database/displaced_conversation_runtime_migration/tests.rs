use sqlx::migrate::{Migrate, Migration};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

use super::{RUNTIME_CHECKSUM, canonical_runtime};
use crate::database::{DB_MIGRATOR, run_migrations_with_retry};
use crate::{MigrationLineageStatus, inspect_supported_migration_lineage};

const USER: &str = "0190f5fe-7c00-7a00-8abc-000000000001";
const CONVERSATION: &str = "0190f5fe-7c00-7a00-8abc-000000000002";
const MESSAGE: &str = "0190f5fe-7c00-7a00-8abc-000000000003";
type LedgerRow = (i64, String, String, bool, Vec<u8>, i64);
type SchemaRow = (String, String, String, Option<String>);

async fn ledger(pool: &SqlitePool) -> Vec<LedgerRow> {
    sqlx::query_as(
        "SELECT version, description, CAST(installed_on AS TEXT), success, checksum, execution_time \
         FROM _sqlx_migrations ORDER BY version",
    ).fetch_all(pool).await.unwrap()
}

async fn schema(pool: &SqlitePool) -> Vec<SchemaRow> {
    sqlx::query_as("SELECT type, name, tbl_name, sql FROM sqlite_schema ORDER BY type, name")
        .fetch_all(pool).await.unwrap()
}

async fn evidence(pool: &SqlitePool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT json_object('id', id, 'conversation', conversation_id, 'turn', turn_operation_id, \
            'sequence', sequence, 'event', event_json, 'model', model_operation_id, \
            'claimed', model_claimed, 'created', created_at) FROM conversation_runtime_events \
         UNION ALL \
         SELECT json_object('id', id, 'operation', operation_id, 'conversation', conversation_id, \
            'message', message_id, 'user', user_id, 'kind', kind, 'request', request_payload, \
            'status', status, 'ok', result_ok, 'text', result_text, 'error', result_error, \
            'created', created_at, 'updated', updated_at, 'completed', completed_at) \
         FROM conversation_delivery_receipts ORDER BY 1",
    ).fetch_all(pool).await.unwrap()
}

async fn prefix(maximum: i64) -> SqlitePool {
    let pool = SqlitePoolOptions::new().max_connections(1)
        .connect("sqlite::memory:").await.unwrap();
    let mut conn = pool.acquire().await.unwrap();
    conn.ensure_migrations_table().await.unwrap();
    for migration in DB_MIGRATOR.iter().filter(|migration| migration.version <= maximum) {
        conn.apply(migration).await.unwrap();
    }
    drop(conn);
    pool
}

async fn seed_evidence(pool: &SqlitePool) {
    sqlx::query("INSERT INTO users (user_id, username, password_hash, created_at, updated_at) \
                 VALUES (?, 'runtime-lineage', 'hash', 1, 1)")
        .bind(USER).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO conversations \
        (conversation_id, user_id, name, type, created_at, updated_at) \
        VALUES (?, ?, 'Runtime lineage', 'nomi', 1, 1)")
        .bind(CONVERSATION).bind(USER).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO conversation_delivery_receipts \
        (operation_id, message_id, conversation_id, user_id, kind, request_payload, status, created_at, updated_at) \
        VALUES ('runtime-turn', ?, ?, ?, 'turn', '{}', 'accepted', 1, 1)")
        .bind(MESSAGE).bind(CONVERSATION).bind(USER).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO conversation_runtime_events \
        (id, conversation_id, turn_operation_id, sequence, event_json, model_operation_id, model_claimed, created_at) \
        VALUES (17, ?, 'runtime-turn', 1, json_object('retained', 1), 'model-once', 1, 1)")
        .bind(CONVERSATION).execute(pool).await.unwrap();
}

async fn displaced_pool() -> SqlitePool {
    let pool = prefix(94).await;
    let canonical = canonical_runtime(&DB_MIGRATOR).unwrap();
    assert_eq!(hex::encode(canonical.checksum.as_ref()), RUNTIME_CHECKSUM);
    let displaced = Migration::new(
        95, canonical.description.clone(), canonical.migration_type,
        canonical.sql.clone(), canonical.no_tx,
    );
    let mut conn = pool.acquire().await.unwrap();
    conn.apply(&displaced).await.unwrap();
    drop(conn);
    seed_evidence(&pool).await;
    pool
}

async fn run_startup_migrations(pool: &SqlitePool) -> Result<(), crate::DbError> {
    let mut conn = pool.acquire().await.unwrap();
    run_migrations_with_retry(&mut conn).await
}

#[tokio::test]
async fn exact_local_prefix_moves_only_the_ledger_version_and_preserves_evidence() {
    let pool = displaced_pool().await;
    let before = ledger(&pool).await;
    let retained = evidence(&pool).await;
    assert_eq!(inspect_supported_migration_lineage(&pool).await.unwrap(), MigrationLineageStatus::UpgradeRequired);
    assert_eq!(ledger(&pool).await, before); // Inspection never repairs the ledger.

    run_startup_migrations(&pool).await.unwrap();
    assert_eq!(inspect_supported_migration_lineage(&pool).await.unwrap(), MigrationLineageStatus::Current);
    let after = ledger(&pool).await;
    assert_eq!(&after[..94], &before[..94]);
    let mut expected = before[94].clone();
    expected.0 = 99;
    assert_eq!(after.iter().find(|row| row.0 == 99).unwrap(), &expected);
    assert_eq!(after.iter().map(|row| row.0).collect::<Vec<_>>(),
        DB_MIGRATOR.iter().map(|migration| migration.version).collect::<Vec<_>>());
    assert_eq!(evidence(&pool).await, retained);
    crate::validate_id_schema_contract(&pool).await.unwrap();

    run_startup_migrations(&pool).await.unwrap();
    assert_eq!(ledger(&pool).await, after);
    assert_eq!(evidence(&pool).await, retained);
    pool.close().await;
}

#[tokio::test]
async fn unknown_failed_gapped_and_target_conflicting_ledgers_are_unchanged() {
    for mutation in [
        "UPDATE _sqlx_migrations SET checksum = zeroblob(48) WHERE version = 95",
        "UPDATE _sqlx_migrations SET checksum = zeroblob(48) WHERE version = 1",
        "UPDATE _sqlx_migrations SET success = 0 WHERE version = 95",
        "UPDATE _sqlx_migrations SET success = 0 WHERE version = 1",
        "DELETE FROM _sqlx_migrations WHERE version = 94",
        "UPDATE _sqlx_migrations SET version = 99 WHERE version = 95",
        "UPDATE _sqlx_migrations SET version = 99 WHERE version = 94",
        "INSERT INTO _sqlx_migrations SELECT 99, description, installed_on, success, checksum, execution_time \
         FROM _sqlx_migrations WHERE version = 95",
        "INSERT INTO _sqlx_migrations SELECT 96, description, installed_on, success, checksum, execution_time \
         FROM _sqlx_migrations WHERE version = 95",
    ] {
        let pool = displaced_pool().await;
        sqlx::query(mutation).execute(&pool).await.unwrap();
        let before = (ledger(&pool).await, schema(&pool).await, evidence(&pool).await);
        assert!(inspect_supported_migration_lineage(&pool).await.is_err(), "{mutation}");
        assert!(run_startup_migrations(&pool).await.is_err(), "{mutation}");
        assert_eq!((ledger(&pool).await, schema(&pool).await, evidence(&pool).await), before, "{mutation}");
        pool.close().await;
    }
}

#[tokio::test]
async fn suffix_failure_rolls_back_the_move_and_all_applied_schema_changes() {
    let pool = displaced_pool().await;
    // Fail late, after upstream 095/096 and effect migrations would have run.
    sqlx::query("CREATE TRIGGER force_suffix_failure BEFORE INSERT ON _sqlx_migrations \
        WHEN NEW.version = 103 BEGIN SELECT RAISE(ABORT, 'forced suffix failure'); END")
        .execute(&pool).await.unwrap();
    let before = (ledger(&pool).await, schema(&pool).await, evidence(&pool).await);
    let error = run_startup_migrations(&pool).await.unwrap_err();
    assert!(error.to_string().contains("forced suffix failure"));
    assert_eq!((ledger(&pool).await, schema(&pool).await, evidence(&pool).await), before);
    assert_eq!(inspect_supported_migration_lineage(&pool).await.unwrap(), MigrationLineageStatus::UpgradeRequired);
    pool.close().await;
}

#[tokio::test]
async fn upstream_prefix_and_fresh_database_keep_canonical_095() {
    for maximum in [0, 95, 96] {
        let pool = prefix(maximum).await;
        let before = ledger(&pool).await;
        run_startup_migrations(&pool).await.unwrap();
        let after = ledger(&pool).await;
        assert_eq!(&after[..before.len()], before.as_slice());
        let upstream = DB_MIGRATOR.iter().find(|migration| migration.version == 95).unwrap();
        assert_eq!(after.iter().find(|row| row.0 == 95).unwrap().4.as_slice(), upstream.checksum.as_ref());
        assert_eq!(inspect_supported_migration_lineage(&pool).await.unwrap(), MigrationLineageStatus::Current);
        pool.close().await;
    }
}

#[tokio::test]
async fn git_extension_preserves_the_miniapp_persistent_owner_codec() {
    let pool = prefix(102).await;
    seed_evidence(&pool).await;
    sqlx::query("UPDATE conversations SET status = 'running', admission_epoch = 1, \
        active_turn_operation_id = 'runtime-turn' WHERE conversation_id = ?")
        .bind(CONVERSATION).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO conversation_hosted_effects \
        (id, user_id, conversation_id, operation_id, turn_operation_id, admission_epoch, \
         owner_domain, capability_id, action_name, input_sha256, state, created_at) \
        VALUES (23, ?, ?, 'hosted-once', 'runtime-turn', 1, 'miniapp', 'hosted-capability', \
                'invoke', ?, 'pending', 2)")
        .bind(USER).bind(CONVERSATION).bind("a".repeat(64)).execute(&pool).await.unwrap();
    let retained = evidence(&pool).await;
    run_startup_migrations(&pool).await.unwrap();
    let row: (i64, String, String, String, Option<String>) = sqlx::query_as(
        "SELECT id, owner_domain, operation_id, state, resource_key FROM conversation_hosted_effects",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(row, (23, "miniapp".into(), "hosted-once".into(), "pending".into(), None));
    assert_eq!(evidence(&pool).await, retained);
    pool.close().await;
}
