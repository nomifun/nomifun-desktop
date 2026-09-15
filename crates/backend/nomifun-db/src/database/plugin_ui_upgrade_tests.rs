//! Regression boundary: the exact e617feb2b prefix must remain append-only.
//! Migration-level fixtures exercise the real startup migrator; fresh-file
//! coverage also exercises the public initializer and logical schema contract.

use sha2::{Digest, Sha384};
use sqlx::migrate::Migrate;

use super::*;

const OWNER: &str = "0190f5fe-7c00-7a00-8abc-000000000001";
const PRESET: &str = "0190f5fe-7c00-7a00-8abc-000000000002";
const DEFAULT_BINDING: &str = r#"{"binding_version":0,"selection":null}"#;
type LedgerRow = (i64, String, String, bool, Vec<u8>, i64);

fn assert_upstream_prefix() {
    // SHA-384 over ordered (i64 big-endian version, SQLx SHA-384 SQL checksum)
    // pairs, computed independently from git blobs at e617feb2b. This prevents
    // a fixture built from today's migrator from hiding edits to old SQL.
    let mut digest = Sha384::new();
    let prefix = DB_MIGRATOR
        .iter()
        .filter(|migration| migration.version <= 103);
    let mut versions = Vec::new();
    for migration in prefix {
        versions.push(migration.version);
        digest.update(migration.version.to_be_bytes());
        digest.update(migration.checksum.as_ref());
    }
    assert_eq!(
        versions,
        (1..=103)
            .filter(|v| ![27, 97, 98].contains(v))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        hex::encode(digest.finalize()),
        "7a2d9c3454fab2f7609364a57b9d2b5ff0327640f4d4e777a16acaa85151404faff6ce118e7f772e3c5c15abf3a7c585"
    );
}

async fn open(path: &Path) -> SqlitePool {
    PoolOptions::<Sqlite>::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true)
                .pragma("secure_delete", "ON"),
        )
        .await
        .unwrap()
}

async fn prefix(path: &Path, head: i64) -> SqlitePool {
    assert_upstream_prefix();
    let pool = open(path).await;
    let mut conn = pool.acquire().await.unwrap();
    conn.ensure_migrations_table().await.unwrap();
    for migration in DB_MIGRATOR
        .iter()
        .filter(|migration| migration.version <= head)
    {
        conn.apply(migration).await.unwrap();
    }
    drop(conn);
    pool
}

async fn ledger(pool: &SqlitePool) -> Vec<LedgerRow> {
    sqlx::query_as("SELECT version, description, CAST(installed_on AS TEXT), success, checksum, execution_time FROM _sqlx_migrations ORDER BY version")
        .fetch_all(pool).await.unwrap()
}

async fn seed_old_rows(pool: &SqlitePool) {
    sqlx::query("INSERT INTO nomi_agent_presets
        (id, preset_id, owner_user_id, source_kind, display_name, description, current_revision, created_at, retired_at_ms)
        VALUES (41, ?, ?, 'user', 'Keep this preset', 'Keep this description', NULL, 123, 456)")
        .bind(PRESET).bind(OWNER).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO plugin_surface_sessions
        (id, surface_session_id, plugin_product_id, owner_user_id, generation, capability_digest,
         active_release_id, active_release_digest, active_release_epoch, issued_at_ms)
        VALUES (42, '0190f5fe-7c00-7a00-8abc-000000000003',
         '0190f5fe-7c00-7a00-8abc-000000000004', ?, 7, ?,
         '0190f5fe-7c00-7a00-8abc-000000000005', ?, 8, 321)",
    )
    .bind(OWNER)
    .bind("a".repeat(64))
    .bind("b".repeat(64))
    .execute(pool)
    .await
    .unwrap();
}

async fn old_rows(pool: &SqlitePool) -> (String, String) {
    let preset = sqlx::query_scalar(
        "SELECT json_object('id', id, 'preset', preset_id,
        'owner', owner_user_id, 'source', source_kind, 'name', display_name,
        'description', description, 'revision', current_revision, 'created', created_at,
        'retired', retired_at_ms) FROM nomi_agent_presets WHERE id = 41",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let surface = sqlx::query_scalar(
        "SELECT json_object('id', id, 'session', surface_session_id,
        'product', plugin_product_id, 'owner', owner_user_id, 'generation', generation,
        'capability', capability_digest, 'release', active_release_id,
        'digest', active_release_digest, 'epoch', active_release_epoch, 'issued', issued_at_ms)
        FROM plugin_surface_sessions WHERE id = 42",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    (preset, surface)
}

async fn assert_new_schema(pool: &SqlitePool) {
    let indexes: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_schema WHERE type = 'index'
        AND name IN ('idx_nomi_agent_presets_ui_plugin', 'idx_plugin_surface_sessions_conversation_id',
                     'idx_installation_role_bindings_provider_mount')")
        .fetch_one(pool).await.unwrap();
    assert_eq!(indexes, 3);
    let bindings: i64 = sqlx::query_scalar("SELECT count(*) FROM installation_role_bindings")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(bindings, 0, "upgrade must not choose a provider implicitly");
}

#[tokio::test]
async fn upstream_prefix_upgrade_preserves_rows_checksums_and_safe_defaults() {
    for head in [96, 103] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("upstream.db");
        let pool = prefix(&path, head).await;
        seed_old_rows(&pool).await;
        let before = ledger(&pool).await;
        let data = old_rows(&pool).await;
        assert_eq!(
            inspect_supported_migration_lineage(&pool).await.unwrap(),
            MigrationLineageStatus::UpgradeRequired
        );
        assert_eq!(ledger(&pool).await, before, "inspection is read-only");

        run_migrations(&pool).await.unwrap();
        assert_eq!(
            inspect_supported_migration_lineage(&pool).await.unwrap(),
            MigrationLineageStatus::Current
        );
        let after = ledger(&pool).await;
        assert_eq!(
            &after[..before.len()],
            before.as_slice(),
            "historical ledger is immutable"
        );
        assert_eq!(old_rows(&pool).await, data);
        let binding: String =
            sqlx::query_scalar("SELECT ui_binding_json FROM nomi_agent_presets WHERE id = 41")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(binding, DEFAULT_BINDING);
        let conversation: Option<String> =
            sqlx::query_scalar("SELECT conversation_id FROM plugin_surface_sessions WHERE id = 42")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(conversation, None);
        assert_new_schema(&pool).await;
        for invalid in [
            "{}",
            r#"{"binding_version":-1,"selection":null}"#,
            r#"{"binding_version":0,"selection":[]}"#,
        ] {
            assert!(
                sqlx::query("UPDATE nomi_agent_presets SET ui_binding_json = ? WHERE id = 41")
                    .bind(invalid)
                    .execute(&pool)
                    .await
                    .is_err()
            );
        }
        assert!(
            sqlx::query(
                "UPDATE plugin_surface_sessions SET conversation_id = 'not-a-uuid' WHERE id = 42"
            )
            .execute(&pool)
            .await
            .is_err()
        );
        pool.close().await;

        let reopened = open(&path).await;
        run_migrations(&reopened).await.unwrap();
        assert_eq!(ledger(&reopened).await, after);
        assert_eq!(old_rows(&reopened).await, data);
        reopened.close().await;
    }
}

#[tokio::test]
async fn fresh_file_reopens_without_reapplying_additive_migrations() {
    assert_upstream_prefix();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fresh.db");
    let db = init_database(&path).await.unwrap();
    assert_new_schema(db.pool()).await;
    let owner: String = sqlx::query_scalar("SELECT owner_user_id FROM installation_identity")
        .fetch_one(db.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO nomi_agent_presets
        (preset_id, owner_user_id, source_kind, display_name, created_at)
        VALUES (?, ?, 'user', 'Fresh preset', 1)",
    )
    .bind(PRESET)
    .bind(&owner)
    .execute(db.pool())
    .await
    .unwrap();
    let before = ledger(db.pool()).await;
    db.close().await;
    let reopened = init_database(&path).await.unwrap();
    assert_eq!(ledger(reopened.pool()).await, before);
    assert_eq!(
        inspect_supported_migration_lineage(reopened.pool())
            .await
            .unwrap(),
        MigrationLineageStatus::Current
    );
    let binding: String =
        sqlx::query_scalar("SELECT ui_binding_json FROM nomi_agent_presets WHERE preset_id = ?")
            .bind(PRESET)
            .fetch_one(reopened.pool())
            .await
            .unwrap();
    assert_eq!(binding, DEFAULT_BINDING);
    reopened.close().await;
}

#[tokio::test]
async fn unknown_historical_checksums_still_refuse_upgrade_without_writes() {
    for version in [60, 80, 95] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("unknown.db");
        let pool = prefix(&path, 103).await;
        seed_old_rows(&pool).await;
        sqlx::query("UPDATE _sqlx_migrations SET checksum = zeroblob(48) WHERE version = ?")
            .bind(version)
            .execute(&pool)
            .await
            .unwrap();
        let before = ledger(&pool).await;
        let data = old_rows(&pool).await;
        let schema: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT name, sql FROM sqlite_schema ORDER BY name")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(inspect_supported_migration_lineage(&pool).await.is_err());
        pool.close().await;
        assert!(init_database(&path).await.is_err());
        let pool = open(&path).await;
        assert_eq!(ledger(&pool).await, before);
        assert_eq!(old_rows(&pool).await, data);
        let after: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT name, sql FROM sqlite_schema ORDER BY name")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(after, schema, "rejection must not apply any suffix DDL");
        pool.close().await;
    }
}
