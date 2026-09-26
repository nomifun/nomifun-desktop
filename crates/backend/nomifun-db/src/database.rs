use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use fs2::FileExt;
use sqlx::migrate::Migrator;
use sqlx::pool::PoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use sqlx::{Row, Sqlite, SqlitePool};
use tracing::{info, warn};

use crate::error::DbError;

/// Maximum number of connections in the pool.
const MAX_CONNECTIONS: u32 = 5;

/// SQLite busy timeout in milliseconds.
const BUSY_TIMEOUT_MS: u64 = 5000;

static DB_MIGRATOR: Migrator = sqlx::migrate!();
const CANONICAL_BASELINE_MIGRATION_VERSION: i64 = 1;
const RETIRED_PLUGIN_BASELINE_SHA384: &str =
    "25497335d0bd8ce542d6422ba07ecf7c0ce7186032f1fbe5e3ad408b0aeaf166d8b887c3919ae27102bb91f130a6bd3c";
const CANONICAL_BASELINE_SQL: &str = include_str!("../migrations/001_canonical_baseline.sql");

const RETIRED_PLUGIN_TABLES: &[&str] = &[
    "plugin_artifacts",
    "plugin_build_operation_lineage",
    "plugin_candidate_test_receipts",
    "plugin_catalog_publications",
    "plugin_credential_binding_mutations",
    "plugin_credential_bindings",
    "plugin_deletion_intents",
    "plugin_dependency_mutation_commits",
    "plugin_dependency_mutation_intents",
    "plugin_kv",
    "plugin_library_state",
    "plugin_mount_credential_bindings",
    "plugin_mount_kv",
    "plugin_mount_revisions",
    "plugin_mounts",
    "plugin_product_documents",
    "plugin_products",
    "plugin_projects",
    "plugin_publish_authorizations",
    "plugin_ready_candidates",
    "plugin_release_artifacts",
    "plugin_releases",
    "plugin_service_test_receipts",
    "plugin_source_mutation_commits",
    "plugin_source_mutation_intents",
    "plugin_surface_sessions",
    "product_operations",
    "javascript_runtime_selection",
];

/// Wraps a SQLite connection pool with lifecycle management.
#[derive(Clone, Debug)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Returns a reference to the underlying connection pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Closes all connections in the pool.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Create a transactionally consistent SQLite snapshot at `destination`.
    ///
    /// This reads through SQLite rather than copying the main file, so
    /// committed pages still resident in WAL are included. The caller is
    /// responsible for placing the snapshot in a broader bundle manifest with
    /// the dataset generation and checksums for non-database files.
    pub async fn snapshot_into(&self, destination: &Path) -> Result<(), DbError> {
        if destination.exists() {
            return Err(DbError::Conflict(format!(
                "snapshot destination already exists: {}",
                destination.display()
            )));
        }
        if let Some(parent) = destination.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|error| {
                DbError::Init(format!(
                    "failed to create snapshot directory {}: {error}",
                    parent.display()
                ))
            })?;
        }
        let destination_text = destination.to_str().ok_or_else(|| {
            DbError::SafetyBackup(format!(
                "snapshot destination is not valid UTF-8: {}",
                destination.display()
            ))
        })?;
        sqlx::query("VACUUM main INTO ?")
            .bind(destination_text)
            .execute(&self.pool)
            .await
            .map_err(|error| {
                DbError::SafetyBackup(format!(
                    "could not create WAL-safe SQLite snapshot {}: {error}",
                    destination.display()
                ))
            })?;
        validate_sqlite_snapshot(destination).await
    }
}

pub(crate) async fn validate_sqlite_snapshot(path: &Path) -> Result<(), DbError> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .read_only(true)
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS));
    let pool = PoolOptions::<Sqlite>::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(DbError::Query)?;
    let result = async {
        validate_quick_check(&pool).await?;
        validate_restorable_database_contract(&pool).await
    }
    .await;
    pool.close().await;
    result
}

/// Open an existing v3 database for an offline snapshot without running
/// migrations, recovery, or quarantine/rebuild logic against the source.
///
/// Backup is a preservation operation: an unsupported or invalid source must
/// fail closed instead of being transformed before it is captured.
pub async fn open_database_for_backup(path: &Path) -> Result<Database, DbError> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS));
    let pool = PoolOptions::<Sqlite>::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(DbError::Query)?;
    let validation = async {
        validate_quick_check(&pool).await?;
        validate_restorable_database_contract(&pool).await
    }
    .await;
    if let Err(error) = validation {
        pool.close().await;
        return Err(error);
    }
    Ok(Database { pool })
}

async fn validate_restorable_database_contract(pool: &SqlitePool) -> Result<(), DbError> {
    validate_current_migration_lineage(pool).await?;
    crate::id_schema_contract::validate_id_schema_contract(pool).await?;
    crate::id_schema_contract::validate_id_data_contract(pool).await?;

    let identities = sqlx::query("SELECT singleton_key, owner_user_id FROM installation_identity")
        .fetch_all(pool)
        .await
        .map_err(DbError::Query)?;
    if identities.len() != 1 {
        return Err(DbError::Init(format!(
            "backup installation_identity must contain exactly one row, found {}",
            identities.len()
        )));
    }
    let key: String = identities[0]
        .try_get("singleton_key")
        .map_err(DbError::Query)?;
    let owner_user_id: String = identities[0]
        .try_get("owner_user_id")
        .map_err(DbError::Query)?;
    if key != "installation" {
        return Err(DbError::Init(
            "backup installation_identity contains an invalid singleton key".into(),
        ));
    }
    nomifun_common::UserId::parse(owner_user_id.clone()).map_err(|error| {
        DbError::Init(format!(
            "backup installation owner ID is not canonical: {owner_user_id}: {error}"
        ))
    })?;
    let owner_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE user_id = ?")
        .bind(&owner_user_id)
        .fetch_one(pool)
        .await
        .map_err(DbError::Query)?;
    if owner_rows != 1 {
        return Err(DbError::Init(format!(
            "backup installation identity references missing owner user {owner_user_id}"
        )));
    }
    Ok(())
}

/// Require the complete migration lineage shipped with this build.
///
/// Backup and restore artifacts must already be current; they are preservation
/// boundaries and must not be mutated as part of validation.
pub async fn validate_current_migration_lineage(pool: &SqlitePool) -> Result<(), DbError> {
    if !validate_known_migration_lineage_prefix(pool).await? {
        let expected = DB_MIGRATOR.iter().count();
        let observed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(pool)
            .await
            .map_err(DbError::Query)?;
        return Err(DbError::Init(format!(
            "database migration lineage contains {observed} rows but this binary requires exactly {expected}",
        )));
    }
    Ok(())
}

/// Validate an exact checksum-matching prefix and report whether it is current.
/// Startup may migrate a known prefix in place; backup validation still calls
/// [`validate_current_migration_lineage`] and requires the complete lineage.
pub async fn validate_known_migration_lineage_prefix(
    pool: &SqlitePool,
) -> Result<bool, DbError> {
    let expected = DB_MIGRATOR.iter().collect::<Vec<_>>();
    if expected
        .first()
        .is_none_or(|migration| migration.version != CANONICAL_BASELINE_MIGRATION_VERSION)
        || expected.iter().enumerate().any(|(index, migration)| migration.version != (index + 1) as i64)
    {
        return Err(DbError::Init(
            "database lineage must be the contiguous canonical forward migration chain".into(),
        ));
    }

    let rows =
        sqlx::query("SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .map_err(DbError::Query)?;
    if rows.is_empty() || rows.len() > expected.len() {
        return Err(DbError::Init(format!(
            "database migration lineage contains {} rows but this binary recognizes at most {}",
            rows.len(),
            expected.len(),
        )));
    }

    for (row, expected) in rows.iter().zip(expected.iter()) {
        let version: i64 = row.try_get("version").map_err(DbError::Query)?;
        let success: bool = row.try_get("success").map_err(DbError::Query)?;
        let checksum: Vec<u8> = row.try_get("checksum").map_err(DbError::Query)?;
        if version != expected.version
            || !success
            || checksum.as_slice() != expected.checksum.as_ref()
        {
            return Err(DbError::Init(format!(
                "database migration lineage does not match embedded migration {}",
                expected.version
            )));
        }
    }
    Ok(rows.len() == expected.len())
}

/// Return true only for the exact retired N1/M1 canonical baseline that may
/// undergo the one-time Unified Plugin clean-start. No other checksum, partial
/// lineage, or hand-edited schema is authorized by this boundary.
pub async fn requires_unified_plugin_clean_start(pool: &SqlitePool) -> Result<bool, DbError> {
    let rows = sqlx::query(
        "SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .map_err(DbError::Query)?;
    if rows.len() != 1 {
        return Ok(false);
    }
    let version: i64 = rows[0].try_get("version").map_err(DbError::Query)?;
    let success: bool = rows[0].try_get("success").map_err(DbError::Query)?;
    let checksum: Vec<u8> = rows[0].try_get("checksum").map_err(DbError::Query)?;
    Ok(version == CANONICAL_BASELINE_MIGRATION_VERSION
        && success
        && hex::encode(checksum) == RETIRED_PLUGIN_BASELINE_SHA384)
}

/// Initialize a file-backed SQLite database.
///
/// Creates the database file and parent directories if they don't exist,
/// configures the busy timeout and WAL journal mode, runs migrations, and
/// ensures the canonical installation owner exists. Migration-lineage errors
/// fail fast; the app bootstrap owns any explicit dataset reset.
pub async fn init_database(path: &Path) -> Result<Database, DbError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| DbError::Init(format!("Failed to create database directory: {e}")))?;
    }

    // The database crate never renames, repairs, migrates, or replaces an
    // existing dataset. The app bootstrap owns the v3 hard-reset lifecycle
    // before any pool is opened; direct callers fail closed on corruption or
    // unsupported lineage.
    try_init_file(path).await
}

/// Initialize an in-memory SQLite database (for testing).
///
/// Uses a single connection to ensure all queries share the same in-memory database.
/// Note: WAL journal mode is not available for in-memory databases.
pub async fn init_database_memory() -> Result<Database, DbError> {
    init_database_memory_inner(None).await
}

/// Initialize an in-memory database with an explicitly supplied canonical
/// installation owner.
///
/// This deterministic variant exists for large integration fixtures that need
/// to thread the same owner through many rows. It never opens an existing
/// dataset and therefore cannot replace or alias a persisted owner.
#[doc(hidden)]
pub async fn init_database_memory_with_owner(
    owner_user_id: nomifun_common::UserId,
) -> Result<Database, DbError> {
    init_database_memory_inner(Some(owner_user_id.into_string())).await
}

async fn init_database_memory_inner(
    requested_owner_user_id: Option<String>,
) -> Result<Database, DbError> {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .map_err(|e| DbError::Init(format!("Invalid memory connection string: {e}")))?
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))
        .pragma("secure_delete", "ON");

    let pool = PoolOptions::<Sqlite>::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .map_err(DbError::Query)?;

    // In-memory DBs are not shared across processes, so no advisory lock is
    // needed (and there is no on-disk path we could create one against).
    run_migrations(&pool).await?;
    backfill_cs_note_search_text(&pool).await?;
    crate::id_schema_contract::validate_id_schema_contract(&pool).await?;
    ensure_installation_owner(&pool, requested_owner_user_id.as_deref()).await?;
    crate::id_schema_contract::validate_id_data_contract(&pool).await?;

    info!("In-memory database initialized");
    Ok(Database { pool })
}

async fn try_init_file(path: &Path) -> Result<Database, DbError> {
    // Serialize the whole file-backed startup path, not only the sqlx
    // migrator. Opening a fresh SQLite file also runs connection-level PRAGMAs
    // such as WAL setup, which can race before migrations start.
    let lock_path = migrate_lock_path(path);
    let _guard = match MigrateLockGuard::acquire(&lock_path) {
        Ok(guard) => Some(guard),
        Err(e) => {
            // Don't fail startup if flock isn't available (e.g. on some
            // network filesystems) - fall back to SQLite busy-timeout and
            // retry-on-conflict behavior below.
            warn!(
                "Could not acquire database startup lock {}: {e}",
                lock_path.display()
            );
            None
        }
    };

    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))
        .journal_mode(SqliteJournalMode::Wal)
        .pragma("secure_delete", "ON");

    let pool = PoolOptions::<Sqlite>::new()
        .max_connections(MAX_CONNECTIONS)
        .connect_with(opts)
        .await
        .map_err(DbError::Query)?;

    let setup = async {
        run_migrations(&pool).await?;
        backfill_cs_note_search_text(&pool).await?;
        crate::id_schema_contract::validate_id_schema_contract(&pool).await?;
        ensure_installation_owner(&pool, None).await?;
        crate::id_schema_contract::validate_id_data_contract(&pool).await
    }
    .await;
    if let Err(e) = setup {
        // Release every file handle before bubbling up so the caller can
        // rename/backup the database file (Windows refuses to rename files
        // with open handles).
        pool.close().await;
        return Err(e);
    }

    info!("Database initialized at {}", path.display());
    Ok(Database { pool })
}

/// Path of the cross-process advisory lock file used to serialize concurrent
/// migrators on the same database.
///
/// We put it next to the DB file so it lives on the same filesystem (avoids
/// odd flock semantics across mount points) and gets cleaned up alongside the
/// DB if a user resets their data directory.
fn migrate_lock_path(db_path: &Path) -> PathBuf {
    let mut p = db_path.to_path_buf();
    let new_name = match p.file_name().and_then(|s| s.to_str()) {
        Some(name) => format!("{name}.migrate.lock"),
        None => "nomifun.migrate.lock".to_string(),
    };
    p.set_file_name(new_name);
    p
}

/// Populate `cs_notes.search_text` for any note whose folded text is stale, and
/// rebuild the notes full-text index.
///
/// Runs after baseline initialization on every boot. `search_text` has a `''`
/// default and SQL cannot fill it itself: SQLite's `lower()` does not fold CJK
/// full-width forms, so filling it in SQL would fork the normalization
/// semantics away from the single Rust implementation the query path uses, and
/// a mismatch there silently loses exactly the recall this change adds.
///
/// Idempotent and a no-op once every row is current, so the steady-state cost
/// is one scan of a small owner-maintained table.
async fn backfill_cs_note_search_text(pool: &SqlitePool) -> Result<(), DbError> {
    let rewritten = crate::repository::customer_service_search::backfill_note_search_text(pool).await?;
    if rewritten > 0 {
        info!("Rebuilt customer-service note search index for {rewritten} note(s)");
    }
    Ok(())
}

async fn run_migrations(pool: &SqlitePool) -> Result<(), DbError> {
    // File-backed callers hold a cross-process startup lock before opening the
    // SQLite pool. sqlx-sqlite's Migrate impl has no-op
    // lock()/unlock() and the migrator does list_applied -> apply without an
    // outer transaction, so two processes opening the same DB simultaneously
    // (e.g. an auto-update spawning the new version while the old one is
    // still shutting down, or `nomicore doctor` racing the server) can both
    // decide to apply the same version and the slower one's INSERT into
    // `_sqlx_migrations` blows up with `UNIQUE constraint failed:
    // _sqlx_migrations.version`. The outer startup lock also covers connection
    // setup before migration execution.
    let mut conn = pool.acquire().await.map_err(DbError::Query)?;
    let secure_delete: i64 = sqlx::query_scalar("PRAGMA secure_delete")
        .fetch_one(&mut *conn)
        .await
        .map_err(DbError::Query)?;
    if secure_delete != 1 {
        return Err(DbError::Init(
            "SQLite secure_delete must be enabled before database migrations".into(),
        ));
    }
    clean_start_unified_plugin_schema(&mut conn).await?;
    run_migrations_with_retry(&mut conn).await?;
    // Always truncate committed migration WAL frames. Besides keeping startup
    // deterministic, this retries the only safety-critical step if a previous
    // post-032 startup was interrupted after the schema commit.
    truncate_wal(&mut conn).await?;
    validate_quick_check_on_connection(&mut conn).await
}

async fn clean_start_unified_plugin_schema(
    conn: &mut sqlx::SqliteConnection,
) -> Result<(), DbError> {
    let migration_table_exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(DbError::Query)?;
    if migration_table_exists == 0 {
        return Ok(());
    }
    let rows = sqlx::query(
        "SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(DbError::Query)?;
    if rows.is_empty() {
        return Ok(());
    }
    let expected = DB_MIGRATOR.iter().collect::<Vec<_>>();
    // A complete checksum-matching prefix is an ordinary older application
    // version and may receive only the pending forward migrations below.
    if rows.len() <= expected.len()
        && rows.iter().zip(expected.iter()).all(|(row, migration)| {
            row.try_get::<i64, _>("version").ok() == Some(migration.version)
                && row.try_get::<bool, _>("success").ok() == Some(true)
                && row
                    .try_get::<Vec<u8>, _>("checksum")
                    .ok()
                    .as_deref()
                    == Some(migration.checksum.as_ref())
        })
    {
        return Ok(());
    }
    if rows.len() != 1 {
        return Err(DbError::Init(
            "database migration lineage is not eligible for Unified Plugin clean-start".into(),
        ));
    }
    let version: i64 = rows[0].try_get("version").map_err(DbError::Query)?;
    let success: bool = rows[0].try_get("success").map_err(DbError::Query)?;
    let checksum: Vec<u8> = rows[0].try_get("checksum").map_err(DbError::Query)?;
    let observed = hex::encode(&checksum);
    let expected = expected
        .first()
        .ok_or_else(|| DbError::Init("canonical database baseline is missing".into()))?;
    if version == expected.version && success && checksum.as_slice() == expected.checksum.as_ref() {
        return Ok(());
    }
    if version != CANONICAL_BASELINE_MIGRATION_VERSION
        || !success
        || observed != RETIRED_PLUGIN_BASELINE_SHA384
    {
        return Err(DbError::Init(
            "database migration lineage is neither current nor the exact retired Plugin baseline"
                .into(),
        ));
    }

    let table_sql = baseline_segment(
        "CREATE TABLE plugin_artifacts (",
        "CREATE TABLE product_agent_selections (",
    )?;
    let index_sql = baseline_segment(
        "CREATE INDEX idx_plugin_artifacts_package_id",
        "CREATE INDEX idx_provider_model_capabilities_task",
    )?;
    let trigger_sql = baseline_segment(
        "CREATE TRIGGER trg_plugin_artifacts_immutable",
        "CREATE TRIGGER trg_requirements_absorb_done_cancelled",
    )?;
    let agent_schema_digest = nomifun_agent_contracts::digest_payload(
        &nomifun_agent_contracts::agent_store_schema_manifest_payload(),
    )
    .map_err(|error| DbError::Init(format!("cannot digest Agent Store schema: {error}")))?;
    let seed_manifest_digest = nomifun_agent_contracts::digest_payload(
        &nomifun_agent_contracts::official_preset_seed_manifest_payload(),
    )
    .map_err(|error| DbError::Init(format!("cannot digest Agent seed manifest: {error}")))?;
    let mut sql = String::from("BEGIN IMMEDIATE;\n");
    // The exact retired baseline checksum proves these names have the expected
    // Plugin-only meaning. The one Agent-store index below indexed only the
    // retired Plugin UI graph; dropping it preserves every Agent row. Foreign
    // keys are deferred by dropping Plugin children in reverse order.
    sql.push_str("DROP INDEX IF EXISTS idx_agent_presets_ui_plugin;\n");
    for table in RETIRED_PLUGIN_TABLES.iter().rev() {
        sql.push_str("DROP TABLE IF EXISTS \"");
        sql.push_str(table);
        sql.push_str("\";\n");
    }
    sql.push_str(table_sql);
    sql.push('\n');
    sql.push_str(index_sql);
    sql.push('\n');
    sql.push_str(trigger_sql);
    sql.push_str("\nUPDATE schema_metadata SET data_generation = ");
    sql.push_str(&nomifun_agent_contracts::AGENT_STORE_DATA_GENERATION.to_string());
    sql.push_str(", migration_head = ");
    sql.push_str(&nomifun_agent_contracts::AGENT_STORE_MIGRATION_HEAD.to_string());
    sql.push_str(", projection_schema_version = ");
    sql.push_str(
        &nomifun_agent_contracts::AGENT_STORE_PROJECTION_SCHEMA_VERSION.to_string(),
    );
    sql.push_str(", seed_manifest_digest = '");
    sql.push_str(seed_manifest_digest.as_ref());
    sql.push_str("', canonical_schema_manifest_digest = '");
    sql.push_str(agent_schema_digest.as_ref());
    sql.push_str("' WHERE singleton_key = 'canonical';");
    sql.push_str("\nUPDATE _sqlx_migrations SET checksum = X'");
    sql.push_str(&hex::encode(expected.checksum.as_ref()));
    sql.push_str("' WHERE version = 1 AND success = 1;\nCOMMIT;");

    sqlx::raw_sql(&sql)
        .execute(&mut *conn)
        .await
        .map_err(DbError::Query)?;
    info!(
        retired_checksum = RETIRED_PLUGIN_BASELINE_SHA384,
        "Unified Plugin Core clean-start replaced only the retired Plugin schema"
    );
    Ok(())
}

fn baseline_segment(start: &str, end: &str) -> Result<&'static str, DbError> {
    let start_offset = CANONICAL_BASELINE_SQL.find(start).ok_or_else(|| {
        DbError::Init(format!("canonical baseline is missing segment start {start}"))
    })?;
    let end_offset = CANONICAL_BASELINE_SQL[start_offset..]
        .find(end)
        .map(|offset| start_offset + offset)
        .ok_or_else(|| DbError::Init(format!("canonical baseline is missing segment end {end}")))?;
    Ok(&CANONICAL_BASELINE_SQL[start_offset..end_offset])
}

async fn truncate_wal(conn: &mut sqlx::SqliteConnection) -> Result<(), DbError> {
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_all(&mut *conn)
        .await
        .map_err(DbError::Query)?;
    Ok(())
}

async fn validate_quick_check(pool: &SqlitePool) -> Result<(), DbError> {
    let rows: Vec<String> = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_all(pool)
        .await
        .map_err(DbError::Query)?;
    require_quick_check_ok(rows)
}

async fn validate_quick_check_on_connection(
    conn: &mut sqlx::SqliteConnection,
) -> Result<(), DbError> {
    let rows: Vec<String> = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_all(&mut *conn)
        .await
        .map_err(DbError::Query)?;
    require_quick_check_ok(rows)
}

fn require_quick_check_ok(rows: Vec<String>) -> Result<(), DbError> {
    if rows.len() == 1 && rows[0] == "ok" {
        return Ok(());
    }
    Err(DbError::Init(format!(
        "post-migration SQLite quick_check failed: {}",
        rows.join("; ")
    )))
}

/// Run sqlx migrations with bounded retries for known recoverable failures.
///
/// The advisory file lock above already serialises well-behaved processes, but
/// a `_sqlx_migrations` UNIQUE conflict can still leak through when:
/// - flock() failed (network FS, sandbox restrictions) and we proceeded.
/// - Two processes that both bypassed the lock raced.
///
/// In every UNIQUE-conflict scenario the failing migration's transaction was
/// rolled back, so re-running `sqlx::migrate!().run` is safe: the second
/// pass sees the row that the winner committed, checksum matches (same
/// shipped binary), and the migration is treated as already applied.
async fn run_migrations_with_retry(conn: &mut sqlx::SqliteConnection) -> Result<(), DbError> {
    let mut retried_unique_conflict = false;

    loop {
        match DB_MIGRATOR.run(&mut *conn).await {
            Ok(()) => return Ok(()),
            Err(e) if !retried_unique_conflict && is_migrations_table_unique_conflict(&e) => {
                retried_unique_conflict = true;
                warn!(
                    "Concurrent migrator detected (UNIQUE conflict on _sqlx_migrations); retrying"
                );
            }
            Err(e) => return Err(DbError::Migration(e)),
        }
    }
}

/// Detect the specific "another process inserted this version first" error.
///
/// sqlx wraps the SQLite error inside `MigrateError::Execute(sqlx::Error)`.
/// We match on the textual message rather than the SQLite extended error code
/// because sqlx loses the structured code by the time it bubbles up here.
fn is_migrations_table_unique_conflict(err: &sqlx::migrate::MigrateError) -> bool {
    let msg = err.to_string();
    msg.contains("UNIQUE constraint failed: _sqlx_migrations.version")
}

/// RAII guard that holds an exclusive file lock for the lifetime of the
/// migration run. Drop unlocks and best-effort closes the file handle.
struct MigrateLockGuard {
    file: std::fs::File,
}

impl MigrateLockGuard {
    fn acquire(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        // Blocking lock via fs2 has no async variant. We're inside an async
        // context but startup blocks anyway and the critical section is
        // bounded (single-process migration run), so this is acceptable.
        FileExt::lock_exclusive(&file)?;
        Ok(Self { file })
    }
}

impl Drop for MigrateLockGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// Ensure exactly one canonical installation owner exists.
///
/// The owner is a normal UUIDv7-addressed user entity. The singleton
/// `installation_identity` row is the durable indirection used by
/// repositories and logical-reference checks; restoring a database therefore
/// preserves the same owner ID, while a fresh dataset mints an unrelated one.
async fn ensure_installation_owner(
    pool: &SqlitePool,
    requested_owner_user_id: Option<&str>,
) -> Result<String, DbError> {
    let mut transaction = pool.begin().await.map_err(DbError::Query)?;

    let existing: Option<String> = sqlx::query_scalar(
        "SELECT owner_user_id FROM installation_identity \
         WHERE singleton_key = 'installation'",
    )
    .fetch_optional(&mut *transaction)
    .await
    .map_err(DbError::Query)?;

    let owner_user_id = if let Some(owner_user_id) = existing {
        if let Some(requested_owner_user_id) = requested_owner_user_id
            && requested_owner_user_id != owner_user_id
        {
            return Err(DbError::Init(format!(
                "existing installation owner {owner_user_id} does not match requested test owner {requested_owner_user_id}"
            )));
        }
        nomifun_common::UserId::parse(owner_user_id.clone()).map_err(|error| {
            DbError::Init(format!(
                "installation owner ID is not canonical: {owner_user_id}: {error}"
            ))
        })?;
        let owner_exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE user_id = ?")
            .bind(&owner_user_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(DbError::Query)?;
        if owner_exists != 1 {
            return Err(DbError::Init(format!(
                "installation identity references missing owner user {owner_user_id}"
            )));
        }
        owner_user_id
    } else {
        let identity_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM installation_identity")
            .fetch_one(&mut *transaction)
            .await
            .map_err(DbError::Query)?;
        if identity_rows != 0 {
            return Err(DbError::Init(
                "installation_identity contains an invalid singleton key".to_owned(),
            ));
        }
        let owner_user_id = requested_owner_user_id
            .map(str::to_owned)
            .unwrap_or_else(|| nomifun_common::UserId::new().into_string());
        nomifun_common::UserId::parse(owner_user_id.clone()).map_err(|error| {
            DbError::Init(format!(
                "requested installation owner ID is not canonical: {owner_user_id}: {error}"
            ))
        })?;
        let now = nomifun_common::now_ms();
        sqlx::query(
            "INSERT INTO users (user_id, username, password_hash, created_at, updated_at) \
             VALUES (?, 'admin', '', ?, ?)",
        )
        .bind(&owner_user_id)
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(DbError::Query)?;
        sqlx::query(
            "INSERT INTO installation_identity (singleton_key, owner_user_id) \
             VALUES ('installation', ?)",
        )
        .bind(&owner_user_id)
        .execute(&mut *transaction)
        .await
        .map_err(DbError::Query)?;
        owner_user_id
    };

    transaction.commit().await.map_err(DbError::Query)?;
    Ok(owner_user_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn initialization_enables_secure_delete_on_every_database_connection() {
        let database = init_database_memory().await.unwrap();
        let secure_delete: i64 = sqlx::query_scalar("PRAGMA secure_delete")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(secure_delete, 1);
    }

    #[tokio::test]
    async fn unified_plugin_clean_start_replaces_only_the_exact_retired_plugin_schema() {
        let database = init_database_memory().await.unwrap();
        sqlx::query(
            "INSERT INTO client_preferences (key, value, updated_at) VALUES ('keep-me', 'safe', 1)",
        )
        .execute(database.pool())
        .await
        .unwrap();

        let mut conn = database.pool().acquire().await.unwrap();
        let mut retired = String::from(
            "PRAGMA foreign_keys = OFF;\n\
             DELETE FROM _sqlx_migrations WHERE version >= 2;\n\
             ALTER TABLE provider_model_capabilities DROP COLUMN compaction_threshold_pct;\n\
             ALTER TABLE agent_sessions DROP COLUMN reasoning_effort_v2;\n\
             ALTER TABLE agent_sessions DROP COLUMN reasoning_effort;\n",
        );
        for table in CANONICAL_PLUGIN_TABLES_FOR_TEST.iter().rev() {
            retired.push_str(&format!("DROP TABLE IF EXISTS \"{table}\";\n"));
        }
        for table in RETIRED_PLUGIN_TABLES {
            retired.push_str(&format!(
                "CREATE TABLE IF NOT EXISTS \"{table}\" (id INTEGER PRIMARY KEY);\n"
            ));
        }
        retired.push_str(
            "CREATE INDEX IF NOT EXISTS idx_agent_presets_ui_plugin \
             ON agent_presets(json_extract(display_json, '$.ui_binding.selection.plugin_id'));\n",
        );
        retired.push_str(&format!(
            "UPDATE _sqlx_migrations SET checksum = X'{RETIRED_PLUGIN_BASELINE_SHA384}' WHERE version = 1;"
        ));
        sqlx::raw_sql(&retired).execute(&mut *conn).await.unwrap();
        drop(conn);
        assert!(requires_unified_plugin_clean_start(database.pool()).await.unwrap());

        let mut conn = database.pool().acquire().await.unwrap();
        clean_start_unified_plugin_schema(&mut conn).await.unwrap();
        run_migrations_with_retry(&mut conn).await.unwrap();
        drop(conn);
        validate_current_migration_lineage(database.pool()).await.unwrap();
        crate::id_schema_contract::validate_id_schema_contract(database.pool())
            .await
            .unwrap();

        let plugin_tables: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND (name = 'plugins' OR name LIKE 'plugin_%') ORDER BY name",
        )
        .fetch_all(database.pool())
        .await
        .unwrap();
        assert_eq!(
            plugin_tables,
            [
                "plugin_artifacts",
                "plugin_credential_bindings",
                "plugin_drafts",
                "plugin_grants",
                "plugin_library_state",
                "plugin_mutations",
                "plugins",
            ]
        );
        let kept: String = sqlx::query_scalar(
            "SELECT value FROM client_preferences WHERE key = 'keep-me'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(kept, "safe");
        let retired_index: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_schema \
             WHERE type = 'index' AND name = 'idx_agent_presets_ui_plugin'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(retired_index, 0);
        let metadata: (String, String) = sqlx::query_as(
            "SELECT seed_manifest_digest, canonical_schema_manifest_digest \
             FROM schema_metadata WHERE singleton_key = 'canonical'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(
            metadata.0,
            nomifun_agent_contracts::digest_payload(
                &nomifun_agent_contracts::official_preset_seed_manifest_payload(),
            )
            .unwrap()
            .as_ref()
        );
        assert_eq!(
            metadata.1,
            nomifun_agent_contracts::digest_payload(
                &nomifun_agent_contracts::agent_store_schema_manifest_payload(),
            )
            .unwrap()
            .as_ref()
        );
    }

    const CANONICAL_PLUGIN_TABLES_FOR_TEST: &[&str] = &[
        "plugin_mutations",
        "plugin_library_state",
        "plugin_grants",
        "plugin_credential_bindings",
        "plugin_drafts",
        "plugins",
        "plugin_artifacts",
    ];

    async fn remove_native_pause_fixture_columns(pool: &SqlitePool) {
        sqlx::raw_sql("ALTER TABLE agent_turns DROP COLUMN native_pause_revision; \
            ALTER TABLE agent_turns DROP COLUMN native_pause_json; \
            ALTER TABLE agent_turns DROP COLUMN native_pause_requested_json; \
            ALTER TABLE agent_turns DROP COLUMN native_budget_json;")
            .execute(pool).await.unwrap();
    }

    async fn remove_native_checkpoint_fixture_columns(pool: &SqlitePool) {
        // These tests deliberately reconstruct an older schema in a new
        // temporary database, so remove every later column as well as its
        // migration receipt. Production migration 005 is forward-only.
        remove_native_pause_fixture_columns(pool).await;
        sqlx::raw_sql("ALTER TABLE agent_turns DROP COLUMN native_checkpoint_json; \
            ALTER TABLE agent_turns DROP COLUMN native_checkpoint_digest; \
            ALTER TABLE agent_turns DROP COLUMN native_checkpoint_revision; \
            ALTER TABLE agent_turns DROP COLUMN native_checkpoint_seq; \
            ALTER TABLE agent_turns DROP COLUMN execution_fence; \
            ALTER TABLE agent_turns DROP COLUMN execution_owner; \
            ALTER TABLE agent_turns DROP COLUMN execution_generation; \
            ALTER TABLE agent_turns DROP COLUMN execution_lease_until;")
            .execute(pool).await.unwrap();
    }

    #[tokio::test]
    async fn native_pause_forward_migration_repairs_only_nonterminal_head_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("native-pause-upgrade.db");
        let database = init_database(&path).await.unwrap();
        remove_native_pause_fixture_columns(database.pool()).await;
        sqlx::query("DELETE FROM _sqlx_migrations WHERE version=7").execute(database.pool()).await.unwrap();
        sqlx::query("UPDATE schema_metadata SET migration_head=5, canonical_schema_manifest_digest='6c3ea4f8d5d3e12cbc36ebb48d7d86ef79d1794630c1acbab0e646799de92d96' WHERE singleton_key='canonical'")
            .execute(database.pool()).await.unwrap();
        let cases = ["running", "completed", "failed", "cancelled"];
        let mut sessions = Vec::new();
        for state in cases {
            let session = uuid::Uuid::now_v7().to_string();
            let start = format!("{session}:start");
            let terminal = format!("{session}:terminal");
            sqlx::query("INSERT INTO agent_sessions (agent_session_id,owner_ref_json,state,archived,pinned,agent_binding_json,next_seq,created_at) VALUES (?,'{}','live',0,0,'{}',3,1)")
                .bind(&session).execute(database.pool()).await.unwrap();
            sqlx::query("INSERT INTO agent_events (session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES (?,1,?,'fixture',?,'turn/started',1,'task','{}')")
                .bind(&session).bind(&start).bind(&start).execute(database.pool()).await.unwrap();
            if state != "running" {
                sqlx::query("INSERT INTO agent_events (session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json) VALUES (?,2,?,'fixture',?,?,1,'task',?,'{}')")
                    .bind(&session).bind(&terminal).bind(&terminal).bind(format!("turn/{state}")).bind(&start).execute(database.pool()).await.unwrap();
            }
            sqlx::query("INSERT INTO agent_turns (session_id,turn_id,operation_id,idempotency_key,state,started_event_id,terminal_event_id,accepted_at,started_at,finished_at,execution_owner,execution_fence,native_checkpoint_json) VALUES (?,'task','task','task',?,?,?,1,1,?,'old-owner',4,'{\"progress\":7}')")
                .bind(&session).bind(state).bind(&start).bind((state != "running").then_some(&terminal))
                .bind((state != "running").then_some(2i64)).execute(database.pool()).await.unwrap();
            sqlx::query("INSERT INTO agent_session_heads (session_id,status,active_set_generation,last_seq,unread_count) VALUES (?,'failed',0,2,0)")
                .bind(&session).execute(database.pool()).await.unwrap();
            sessions.push((session,state));
        }
        database.close().await;
        for _ in 0..2 {
            let upgraded = init_database(&path).await.unwrap();
            validate_current_migration_lineage(upgraded.pool()).await.unwrap();
            for (session,state) in &sessions {
                let row: (String,Option<String>,String,i64,String,Option<String>) = sqlx::query_as(
                    "SELECT h.status,h.active_turn_id,t.state,t.execution_fence,t.native_checkpoint_json,t.native_pause_json FROM agent_session_heads h JOIN agent_turns t ON h.session_id=t.session_id WHERE h.session_id=?")
                    .bind(session).fetch_one(upgraded.pool()).await.unwrap();
                assert_eq!(row.2,*state);
                assert_eq!(row.3,4);
                assert_eq!(row.4,"{\"progress\":7}");
                assert!(row.5.is_none());
                if *state == "running" { assert_eq!(row.0,"reconciliation"); assert_eq!(row.1.as_deref(),Some("task")); }
                else { assert_eq!(row.0,"failed"); assert!(row.1.is_none()); }
            }
            let head: i64 = sqlx::query_scalar("SELECT migration_head FROM schema_metadata WHERE singleton_key='canonical'").fetch_one(upgraded.pool()).await.unwrap();
            assert_eq!(head,6);
            upgraded.close().await;
        }
    }

    #[tokio::test]
    async fn native_checkpoint_forward_migration_preserves_existing_user_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("native-checkpoint-upgrade.db");
        let database = init_database(&path).await.unwrap();
        sqlx::query("INSERT INTO client_preferences (key,value,updated_at) VALUES ('checkpoint-upgrade','keep',1)")
            .execute(database.pool()).await.unwrap();
        remove_native_checkpoint_fixture_columns(database.pool()).await;
        sqlx::query("DELETE FROM _sqlx_migrations WHERE version >= 5").execute(database.pool()).await.unwrap();
        sqlx::query("UPDATE schema_metadata SET migration_head = 3, canonical_schema_manifest_digest = '263a05e5d0a8bb3e535b531791fc3828600cd37f47b645288b0b98acb4cd8856' WHERE singleton_key = 'canonical'")
            .execute(database.pool()).await.unwrap();
        database.close().await;
        let upgraded = init_database(&path).await.unwrap();
        validate_current_migration_lineage(upgraded.pool()).await.unwrap();
        let value: String = sqlx::query_scalar("SELECT value FROM client_preferences WHERE key = 'checkpoint-upgrade'")
            .fetch_one(upgraded.pool()).await.unwrap();
        assert_eq!(value, "keep");
        let digest: String = sqlx::query_scalar("SELECT canonical_schema_manifest_digest FROM schema_metadata WHERE singleton_key='canonical'")
            .fetch_one(upgraded.pool()).await.unwrap();
        assert_eq!(digest, nomifun_agent_contracts::digest_payload(&nomifun_agent_contracts::agent_store_schema_manifest_payload()).unwrap().as_ref());
        upgraded.close().await;
    }

    #[tokio::test]
    async fn existing_baseline_upgrades_in_place_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model-context.db");
        let database = init_database(&path).await.unwrap();
        sqlx::query(
            "INSERT INTO client_preferences (key, value, updated_at) VALUES ('preserve-context-test', 'saved', 1)",
        )
        .execute(database.pool())
        .await
        .unwrap();
        remove_native_checkpoint_fixture_columns(database.pool()).await;
        sqlx::query("DELETE FROM _sqlx_migrations WHERE version >= 2")
            .execute(database.pool())
            .await
            .unwrap();
        sqlx::query("ALTER TABLE provider_model_capabilities DROP COLUMN compaction_threshold_pct")
            .execute(database.pool())
            .await
            .unwrap();
        sqlx::query("ALTER TABLE agent_sessions DROP COLUMN reasoning_effort_v2")
            .execute(database.pool())
            .await
            .unwrap();
        sqlx::query("ALTER TABLE agent_sessions DROP COLUMN reasoning_effort")
            .execute(database.pool())
            .await
            .unwrap();
        database.close().await;

        let upgraded = init_database(&path).await.unwrap();
        validate_current_migration_lineage(upgraded.pool()).await.unwrap();
        let retained: String = sqlx::query_scalar(
            "SELECT value FROM client_preferences WHERE key = 'preserve-context-test'",
        )
        .fetch_one(upgraded.pool())
        .await
        .unwrap();
        assert_eq!(retained, "saved");
        upgraded.close().await;

        let reopened = init_database(&path).await.unwrap();
        validate_current_migration_lineage(reopened.pool()).await.unwrap();
        reopened.close().await;
    }

    #[tokio::test]
    async fn extended_reasoning_migration_preserves_existing_session_effort() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reasoning-effort-v2.db");
        let database = init_database(&path).await.unwrap();
        let session_id = "0190f5fe-7c00-7a00-8000-000000000141";
        sqlx::query(
            "INSERT INTO agent_sessions (\
                agent_session_id, owner_ref_json, state, archived, pinned, \
                agent_binding_json, next_seq, created_at, reasoning_effort, reasoning_effort_v2\
             ) VALUES (?, ?, 'live', 0, 0, '{}', 1, 1, 'high', 'high')",
        )
        .bind(session_id)
        .bind(r#"{"principal_kind":"user","principal_id":"reasoning-migration"}"#)
        .execute(database.pool())
        .await
        .unwrap();
        remove_native_checkpoint_fixture_columns(database.pool()).await;
        sqlx::query("DELETE FROM _sqlx_migrations WHERE version >= 4")
            .execute(database.pool())
            .await
            .unwrap();
        sqlx::query("ALTER TABLE agent_sessions DROP COLUMN reasoning_effort_v2")
            .execute(database.pool())
            .await
            .unwrap();
        sqlx::query(
            "UPDATE schema_metadata SET migration_head = 2, \
             canonical_schema_manifest_digest = \
             'd6fcfed0f24fac2e3045e1a920e36b2d6a3751f7adb2d59de363e171e20e6b1f' \
             WHERE singleton_key = 'canonical'",
        )
        .execute(database.pool())
        .await
        .unwrap();
        database.close().await;

        let upgraded = init_database(&path).await.unwrap();
        validate_current_migration_lineage(upgraded.pool()).await.unwrap();
        let effort: Option<String> = sqlx::query_scalar(
            "SELECT reasoning_effort_v2 FROM agent_sessions WHERE agent_session_id = ?",
        )
        .bind(session_id)
        .fetch_one(upgraded.pool())
        .await
        .unwrap();
        assert_eq!(effort.as_deref(), Some("high"));
        upgraded.close().await;
    }

    #[tokio::test]
    async fn public_snapshot_includes_committed_wal_pages_and_refuses_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.db");
        let snapshot = dir.path().join("bundle").join("main.db");
        let database = init_database(&source).await.unwrap();
        sqlx::query(
            "INSERT INTO client_preferences (key, value, updated_at) \
             VALUES ('snapshot_probe', 'committed', ?)",
        )
        .bind(nomifun_common::now_ms())
        .execute(database.pool())
        .await
        .unwrap();
        database.snapshot_into(&snapshot).await.unwrap();
        let options = SqliteConnectOptions::new()
            .filename(&snapshot)
            .create_if_missing(false)
            .read_only(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        let value: String =
            sqlx::query_scalar("SELECT value FROM client_preferences WHERE key = 'snapshot_probe'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(value, "committed");
        pool.close().await;
        assert!(database.snapshot_into(&snapshot).await.is_err());
        database.close().await;
    }
}
