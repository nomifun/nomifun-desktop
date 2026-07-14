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
const STARTUP_FILE_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(200),
    Duration::from_millis(400),
    Duration::from_millis(800),
];

static DB_MIGRATOR: Migrator = sqlx::migrate!();

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
}

/// Initialize a file-backed SQLite database.
///
/// Creates the database file and parent directories if they don't exist,
/// configures pragmas (foreign_keys, busy_timeout, journal_mode=WAL),
/// runs migrations, and ensures the system default user exists.
///
/// If initialization fails on an existing file:
/// - A database produced by the pre-baseline migration chain (the squashed
///   001–021 history) is renamed to `*.pre-baseline.bak` and rebuilt from
///   scratch (see [`rebuild_pre_baseline_database`]).
/// - Explicit corruption-like failures attempt recovery by backing up the
///   corrupted file and creating a fresh database.
/// - Everything else (other migration errors, lock contention) fails fast.
pub async fn init_database(path: &Path) -> Result<Database, DbError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| DbError::Init(format!("Failed to create database directory: {e}")))?;
    }

    match try_init_file(path).await {
        Ok(db) => Ok(db),
        Err(e) if path.exists() && is_pre_baseline_migration_error(&e) => {
            // Guardrail (this whole salvage path is pre-launch convenience — see
            // the NOTE at `rebuild_pre_baseline_database`). The rebuild WIPES the
            // database (renames it aside, starts empty), so it must NEVER run
            // against a database that still holds a real user credential: refuse,
            // preserve the file in place, and fail fast so a genuine login is
            // never silently reset to a "needs setup" state. A disposable
            // no-credential DB (fresh/dev) is still rebuilt so startup recovers.
            if database_has_real_credential(path).await {
                return Err(DbError::Init(format!(
                    "refusing to rebuild a database that still holds a real user credential \
                     (pre-baseline migration mismatch); preserved {} in place. Original error: {e}",
                    path.display()
                )));
            }
            rebuild_pre_baseline_database(path, e).await
        }
        Err(e) if path.exists() && should_attempt_recovery(&e) => {
            warn!("Database initialization failed, attempting recovery: {e}");
            recover_and_retry(path, e).await
        }
        Err(e) => Err(e),
    }
}

/// Initialize an in-memory SQLite database (for testing).
///
/// Uses a single connection to ensure all queries share the same in-memory database.
/// Note: WAL journal mode is not available for in-memory databases.
pub async fn init_database_memory() -> Result<Database, DbError> {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .map_err(|e| DbError::Init(format!("Invalid memory connection string: {e}")))?
        .foreign_keys(true)
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS));

    let pool = PoolOptions::<Sqlite>::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .map_err(DbError::Query)?;

    // In-memory DBs are not shared across processes, so no advisory lock is
    // needed (and there is no on-disk path we could create one against).
    run_migrations(&pool).await?;
    ensure_system_user(&pool).await?;

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
            warn!("Could not acquire database startup lock {}: {e}", lock_path.display());
            None
        }
    };

    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))
        .journal_mode(SqliteJournalMode::Wal);

    let pool = PoolOptions::<Sqlite>::new()
        .max_connections(MAX_CONNECTIONS)
        .connect_with(opts)
        .await
        .map_err(DbError::Query)?;

    let setup = async {
        backup_before_unified_execution_migration(&pool, path).await?;
        backup_before_cron_agent_only_migration(&pool, path).await?;
        run_migrations(&pool).await?;
        ensure_system_user(&pool).await
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SafetySnapshotKind {
    UnifiedAgentExecution,
    CronAgentOnly,
}

impl SafetySnapshotKind {
    const fn migration_label(self) -> &'static str {
        match self {
            Self::UnifiedAgentExecution => "037",
            Self::CronAgentOnly => "042",
        }
    }

    const fn file_suffix(self) -> &'static str {
        match self {
            Self::UnifiedAgentExecution => "pre-037-unified-agent-execution",
            Self::CronAgentOnly => "pre-042-cron-agent-only",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SafetySnapshotDisposition {
    Created,
    Reused,
}

impl SafetySnapshotDisposition {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Reused => "reused",
        }
    }
}

/// Create or validate one fixed, adjacent, WAL-safe recovery snapshot.
///
/// Every destructive migration uses this one filesystem/SQLite skeleton. The
/// migration-specific validator proves that an existing file is the promised
/// recovery point rather than merely accepting any syntactically valid SQLite
/// database at the fixed path.
async fn ensure_safety_snapshot(
    pool: &SqlitePool,
    db_path: &Path,
    kind: SafetySnapshotKind,
) -> Result<(PathBuf, SafetySnapshotDisposition), DbError> {
    let backup_path = safety_snapshot_path(db_path, kind);
    if backup_path.exists() {
        validate_safety_snapshot(&backup_path, kind)
            .await
            .map_err(|error| {
                DbError::SafetyBackup(format!(
                    "refusing migration {}: existing safety backup {} is invalid: {error}",
                    kind.migration_label(),
                    backup_path.display(),
                ))
            })?;
        return Ok((backup_path, SafetySnapshotDisposition::Reused));
    }

    // VACUUM INTO reads through SQLite, including committed pages that are
    // still resident in the WAL. Copying only the main file is not a recovery
    // snapshot and is deliberately forbidden here.
    sqlx::query("VACUUM main INTO ?")
        .bind(backup_path.to_string_lossy().as_ref())
        .execute(pool)
        .await
        .map_err(|error| {
            DbError::SafetyBackup(format!(
                "refusing migration {}: could not create WAL-safe SQLite snapshot {}: {error}",
                kind.migration_label(),
                backup_path.display(),
            ))
        })?;

    validate_safety_snapshot(&backup_path, kind)
        .await
        .map_err(|error| {
            DbError::SafetyBackup(format!(
                "refusing migration {}: created safety backup {} but validation failed: {error}",
                kind.migration_label(),
                backup_path.display(),
            ))
        })?;
    Ok((backup_path, SafetySnapshotDisposition::Created))
}

fn safety_snapshot_path(db_path: &Path, kind: SafetySnapshotKind) -> PathBuf {
    let file_name = db_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("nomifun-backend.db");
    db_path.with_file_name(format!("{file_name}.{}.bak", kind.file_suffix()))
}

async fn migration_window_is_open(
    pool: &SqlitePool,
    required_version: i64,
    target_version: i64,
) -> Result<bool, DbError> {
    if !sqlite_table_exists(pool, "_sqlx_migrations").await? {
        return Ok(false);
    }
    let (has_required, has_target): (i64, i64) = sqlx::query_as(
        "SELECT \
            EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version = ? AND success = 1), \
            EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version = ? AND success = 1)",
    )
    .bind(required_version)
    .bind(target_version)
    .fetch_one(pool)
    .await
    .map_err(DbError::Query)?;
    Ok(has_required == 1 && has_target == 0)
}

async fn sqlite_table_exists(pool: &SqlitePool, table: &str) -> Result<bool, DbError> {
    let exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?)",
    )
    .bind(table)
    .fetch_one(pool)
    .await
    .map_err(DbError::Query)?;
    Ok(exists == 1)
}

async fn table_has_columns(
    pool: &SqlitePool,
    table: &str,
    required: &[&str],
) -> Result<bool, DbError> {
    if !sqlite_table_exists(pool, table).await? {
        return Ok(false);
    }
    // `table` is an internal constant at every call site. Quote it anyway so
    // this helper remains correct if a future table name contains punctuation.
    let quoted = table.replace('"', "\"\"");
    let query = format!("PRAGMA table_info(\"{quoted}\")");
    let rows = sqlx::query(&query)
        .fetch_all(pool)
        .await
        .map_err(DbError::Query)?;
    Ok(required.iter().all(|required_column| {
        rows.iter().any(|row| {
            row.try_get::<String, _>("name")
                .is_ok_and(|name| name == *required_column)
        })
    }))
}

async fn require_table_columns(
    pool: &SqlitePool,
    table: &str,
    required: &[&str],
) -> Result<(), DbError> {
    if table_has_columns(pool, table, required).await? {
        Ok(())
    } else {
        Err(DbError::Init(format!(
            "required recovery schema is missing {table}({})",
            required.join(", ")
        )))
    }
}

/// Create the one forward-only safety snapshot required before migration 037.
async fn backup_before_unified_execution_migration(
    pool: &SqlitePool,
    db_path: &Path,
) -> Result<(), DbError> {
    if !migration_window_is_open(pool, 18, 37).await? {
        return Ok(());
    }

    let (backup_path, disposition) = ensure_safety_snapshot(
        pool,
        db_path,
        SafetySnapshotKind::UnifiedAgentExecution,
    )
    .await?;
    info!(
        backup = %backup_path.display(),
        snapshot_state = disposition.as_str(),
        "Prepared pre-037 database safety snapshot"
    );
    Ok(())
}

#[cfg(test)]
fn unified_execution_backup_path(db_path: &Path) -> PathBuf {
    safety_snapshot_path(db_path, SafetySnapshotKind::UnifiedAgentExecution)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CronRetirementCounts {
    jobs: i64,
    runs: i64,
}

async fn cron_retirement_counts(
    pool: &SqlitePool,
) -> Result<Option<CronRetirementCounts>, DbError> {
    if !table_has_columns(pool, "cron_jobs", &["id", "target_kind"]).await? {
        return Ok(None);
    }
    let jobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cron_jobs WHERE target_kind = 'terminal'",
    )
    .fetch_one(pool)
    .await
    .map_err(DbError::Query)?;
    if jobs == 0 {
        return Ok(None);
    }

    // Once destructive data is present, prove that this is a complete recovery
    // point rather than a database containing only enough columns to satisfy
    // the eligibility query. These columns are common to every 037-041 schema;
    // migration 038's later `user_id` is intentionally not required because a
    // valid 037 recovery point derives it when migrations are replayed.
    require_table_columns(
        pool,
        "cron_jobs",
        &[
            "id",
            "name",
            "enabled",
            "schedule_kind",
            "schedule_value",
            "schedule_tz",
            "schedule_description",
            "payload_message",
            "execution_mode",
            "agent_config",
            "conversation_id",
            "conversation_title",
            "agent_type",
            "created_by",
            "skill_content",
            "description",
            "target_kind",
            "terminal_mode",
            "terminal_session_id",
            "terminal_command",
            "terminal_args",
            "terminal_script",
            "created_at",
            "updated_at",
            "next_run_at",
            "last_run_at",
            "last_status",
            "last_error",
            "run_count",
            "retry_count",
            "max_retries",
            "preset_id",
            "preset_revision",
            "preset_snapshot",
        ],
    )
    .await?;
    require_table_columns(
        pool,
        "cron_job_runs",
        &["id", "job_id", "executed_at_ms", "status", "created_at_ms"],
    )
    .await?;
    require_table_columns(pool, "conversations", &["id", "user_id", "cron_job_id"])
        .await?;
    require_table_columns(
        pool,
        "conversation_artifacts",
        &["conversation_id", "cron_job_id"],
    )
    .await?;
    require_table_columns(pool, "terminal_sessions", &["id"]).await?;
    require_table_columns(pool, "users", &["id"]).await?;
    let runs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cron_job_runs run \
         JOIN cron_jobs job ON job.id = run.job_id \
         WHERE job.target_kind = 'terminal'",
    )
    .fetch_one(pool)
    .await
    .map_err(DbError::Query)?;
    Ok(Some(CronRetirementCounts { jobs, runs }))
}

/// Snapshot terminal cron history before migration 042 removes it.
///
/// The gate intentionally opens at migration 037, not 041: a user can have
/// stopped on any version from 037 through 041, and startup will apply every
/// remaining migration in one call after this preflight.
async fn backup_before_cron_agent_only_migration(
    pool: &SqlitePool,
    db_path: &Path,
) -> Result<(), DbError> {
    if !migration_window_is_open(pool, 37, 42).await? {
        return Ok(());
    }
    let Some(counts) = cron_retirement_counts(pool).await.map_err(|error| {
        DbError::SafetyBackup(format!(
            "refusing migration 042: could not inspect terminal cron retirement data: {error}"
        ))
    })? else {
        return Ok(());
    };

    let (backup_path, disposition) =
        ensure_safety_snapshot(pool, db_path, SafetySnapshotKind::CronAgentOnly).await?;
    info!(
        backup = %backup_path.display(),
        snapshot_state = disposition.as_str(),
        terminal_job_count = counts.jobs,
        terminal_run_count = counts.runs,
        "Prepared pre-042 database safety snapshot for retired terminal cron data"
    );
    Ok(())
}

#[cfg(test)]
fn cron_agent_only_backup_path(db_path: &Path) -> PathBuf {
    safety_snapshot_path(db_path, SafetySnapshotKind::CronAgentOnly)
}

async fn validate_safety_snapshot(
    path: &Path,
    kind: SafetySnapshotKind,
) -> Result<(), DbError> {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .read_only(true)
        .foreign_keys(true)
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS));
    let pool = PoolOptions::<Sqlite>::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .map_err(DbError::Query)?;
    let result = async {
        validate_quick_check(&pool).await?;
        match kind {
            SafetySnapshotKind::UnifiedAgentExecution => {
                validate_unified_execution_snapshot(&pool).await
            }
            SafetySnapshotKind::CronAgentOnly => validate_cron_agent_only_snapshot(&pool).await,
        }
    }
    .await;
    pool.close().await;
    result
}

async fn validate_unified_execution_snapshot(pool: &SqlitePool) -> Result<(), DbError> {
    // A syntactically valid SQLite file is not necessarily the recovery point
    // promised to the user. Require the complete legacy execution schema.
    if !migration_window_is_open(pool, 18, 37).await? {
        return Err(DbError::Init(
            "pre-037 snapshot has the wrong migration window".to_owned(),
        ));
    }
    let legacy_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master \
         WHERE type = 'table' AND name IN (\
             'fleets', 'fleet_members', 'orch_workspaces', 'orch_runs', \
             'orch_run_tasks', 'orch_run_task_deps', 'orch_assignments'\
         )",
    )
    .fetch_one(pool)
    .await
    .map_err(DbError::Query)?;
    if legacy_table_count != 7 {
        return Err(DbError::Init(
            "pre-037 snapshot is not a complete legacy Agent Execution database".to_owned(),
        ));
    }
    Ok(())
}

async fn validate_cron_agent_only_snapshot(pool: &SqlitePool) -> Result<(), DbError> {
    if !migration_window_is_open(pool, 37, 42).await? {
        return Err(DbError::Init(
            "pre-042 snapshot must have migration 037 applied and migration 042 absent"
                .to_owned(),
        ));
    }
    let Some(counts) = cron_retirement_counts(pool).await? else {
        return Err(DbError::Init(
            "pre-042 snapshot does not contain terminal cron data".to_owned(),
        ));
    };
    if counts.jobs <= 0 {
        return Err(DbError::Init(
            "pre-042 snapshot does not contain terminal cron jobs".to_owned(),
        ));
    }
    Ok(())
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

fn retry_startup_file_op<T, F>(operation: &str, path: &Path, mut op: F) -> std::io::Result<T>
where
    F: FnMut() -> std::io::Result<T>,
{
    for (attempt, delay) in STARTUP_FILE_RETRY_DELAYS.iter().enumerate() {
        match op() {
            Ok(value) => return Ok(value),
            Err(e) if is_retryable_startup_file_error(&e) => {
                warn!(
                    operation,
                    path = %path.display(),
                    attempt = attempt + 1,
                    retry_after_ms = delay.as_millis(),
                    raw_os_error = ?e.raw_os_error(),
                    error = %e,
                    "Startup file operation failed; retrying"
                );
                std::thread::sleep(*delay);
            }
            Err(e) => return Err(e),
        }
    }
    op()
}

fn is_retryable_startup_file_error(error: &std::io::Error) -> bool {
    match error.kind() {
        std::io::ErrorKind::Interrupted
        | std::io::ErrorKind::PermissionDenied
        | std::io::ErrorKind::TimedOut
        | std::io::ErrorKind::WouldBlock => true,
        _ => matches!(error.raw_os_error(), Some(5 | 32 | 33)),
    }
}

async fn run_migrations(pool: &SqlitePool) -> Result<(), DbError> {
    // File-backed callers hold a cross-process startup lock before opening the
    // SQLite pool. sqlx-sqlite's Migrate impl has no-op
    // lock()/unlock() and the migrator does list_applied → apply without an
    // outer transaction, so two processes opening the same DB simultaneously
    // (e.g. an auto-update spawning the new version while the old one is
    // still shutting down, or `nomicore doctor` racing the server) can both
    // decide to apply the same version and the slower one's INSERT into
    // `_sqlx_migrations` blows up with `UNIQUE constraint failed:
    // _sqlx_migrations.version`. The outer startup lock also covers
    // connection PRAGMAs before migration execution.
    //
    // Any future table-rebuild migration (CREATE new + INSERT…SELECT + DROP
    // old + ALTER…RENAME) needs two pragmas:
    // - foreign_keys=OFF: prevents DROP TABLE from triggering ON DELETE CASCADE
    // - legacy_alter_table=ON: prevents ALTER TABLE RENAME from rewriting FK
    //   references in other tables (SQLite 3.26+ rewrites them by default)
    // Both must be set outside a transaction (sqlx wraps each migration in
    // one), so they are applied here for every migration run.
    let mut conn = pool.acquire().await.map_err(DbError::Query)?;
    sqlx::query("PRAGMA foreign_keys = OFF; PRAGMA legacy_alter_table = ON")
        .execute(&mut *conn)
        .await
        .map_err(DbError::Query)?;

    let migration_result = run_migrations_with_retry(&mut conn).await;

    let pragma_result = sqlx::query("PRAGMA foreign_keys = ON; PRAGMA legacy_alter_table = OFF")
        .execute(&mut *conn)
        .await
        .map_err(DbError::Query);

    migration_result?;
    pragma_result?;

    if let Some(violation) = sqlx::query("PRAGMA foreign_key_check")
        .fetch_optional(&mut *conn)
        .await
        .map_err(DbError::Query)?
    {
        let table: String = violation.try_get("table").unwrap_or_else(|_| "<unknown>".into());
        let parent: String = violation.try_get("parent").unwrap_or_else(|_| "<unknown>".into());
        let row_id: Option<i64> = violation.try_get("rowid").ok();
        return Err(DbError::Init(format!(
            "post-migration foreign_key_check failed: table={table}, rowid={row_id:?}, parent={parent}"
        )));
    }
    validate_quick_check_on_connection(&mut conn).await
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

/// Run sqlx migrations with one retry on `_sqlx_migrations` UNIQUE conflict.
///
/// The advisory file lock above already serialises well-behaved processes,
/// but a UNIQUE conflict can still leak through when:
/// - flock() failed (network FS, sandbox restrictions) and we proceeded.
/// - Two processes that both bypassed the lock raced.
///
/// In every UNIQUE-conflict scenario the failing migration's transaction was
/// rolled back, so re-running `sqlx::migrate!().run` is safe: the second
/// pass sees the row that the winner committed, checksum matches (same
/// shipped binary), and the migration is treated as already applied.
async fn run_migrations_with_retry(conn: &mut sqlx::SqliteConnection) -> Result<(), DbError> {
    match DB_MIGRATOR.run(&mut *conn).await {
        Ok(()) => Ok(()),
        Err(e) if is_migrations_table_unique_conflict(&e) => {
            warn!("Concurrent migrator detected (UNIQUE conflict on _sqlx_migrations); retrying");
            DB_MIGRATOR.run(&mut *conn).await.map_err(DbError::Migration)
        }
        Err(e) => Err(DbError::Migration(e)),
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
        // Blocking lock — fs2 has no async variant. We're inside an async
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

/// Ensure the system default user exists.
///
/// Uses INSERT OR IGNORE so it is safe to call on every startup.
/// The system user has an empty password hash, which signals "needs setup".
/// Username defaults to `admin` — matches the legacy web-host login flow so
/// users upgrading from pre-M6 builds keep the same login username.
async fn ensure_system_user(pool: &SqlitePool) -> Result<(), DbError> {
    let now = nomifun_common::now_ms();
    sqlx::query(
        "INSERT OR IGNORE INTO users (id, username, password_hash, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind("system_default_user")
    .bind("admin")
    .bind("")
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Pre-baseline bootstrap salvage
//
// NOTE: pre-launch convenience; remove before release, restoring fail-fast.
//
// The 2026-06-12 clean-baseline refactor squashed migrations 001–021 into a
// single 001_baseline.sql, resetting the migration chain. Any dev database
// created before the squash fails sqlx validation: version 1's checksum no
// longer matches, and applied versions 2–21 are missing from the resolved
// set. The system has not shipped and every dev database is disposable, so
// instead of making each machine delete the file by hand we rename the old
// database (plus its -wal/-shm sidecars) to `*.pre-baseline.bak` and rebuild
// an empty database from the baseline.
// ---------------------------------------------------------------------------

/// Classify migration failures caused by a database whose `_sqlx_migrations`
/// history does not line up with the shipped (squashed) migration set.
fn is_pre_baseline_migration_error(err: &DbError) -> bool {
    use sqlx::migrate::MigrateError;
    matches!(
        err,
        DbError::Migration(
            MigrateError::VersionMismatch(_)
                | MigrateError::VersionMissing(_)
                | MigrateError::VersionTooOld(_, _)
                | MigrateError::VersionTooNew(_, _)
        )
    )
}

/// `{file_name}.pre-baseline.bak`, with a numeric suffix when a previous
/// backup already occupies the name.
fn pre_baseline_backup_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("nomifun-backend.db");
    let base = path.with_file_name(format!("{file_name}.pre-baseline.bak"));
    if !base.exists() {
        return base;
    }
    for n in 1..10_000 {
        let candidate = path.with_file_name(format!("{file_name}.pre-baseline.bak.{n}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    // Practically unreachable; fall back to a timestamped name.
    path.with_file_name(format!(
        "{file_name}.pre-baseline.bak.{}",
        nomifun_common::now_ms()
    ))
}

/// `{file_name}{suffix}` next to `path` (SQLite sidecars append to the full
/// file name: `nomifun-backend.db-wal`, `nomifun-backend.db-shm`).
fn sibling_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map(|s| s.to_os_string()).unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

/// Best-effort check: does the (migration-mismatched) database at `path` still
/// contain a user with a REAL (non-empty) password?
///
/// Gates the destructive pre-baseline rebuild so it never wipes a DB holding a
/// real credential. Opens its own connection WITHOUT running migrations and
/// only runs a `SELECT COUNT`, so it works against the old schema. Uses a
/// read-write open (not read-only) so a WAL-mode database is always fully
/// readable — a false "no credential" here would let the rebuild wipe a real
/// login, which is exactly what we must prevent. The caller has already closed
/// its own pool (see `try_init_file`), so this does not contend with it.
///
/// Any open/query failure returns `false` (allow rebuild); the rebuild only
/// RENAMES the file aside (never deletes), so an unreadable DB is still kept.
async fn database_has_real_credential(path: &Path) -> bool {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS));
    let pool = match PoolOptions::<Sqlite>::new().max_connections(1).connect_with(opts).await {
        Ok(p) => p,
        Err(e) => {
            warn!("pre-baseline guard: could not open {} to probe credentials: {e}", path.display());
            return false;
        }
    };
    let probe: Result<(i64,), _> =
        sqlx::query_as("SELECT COUNT(*) FROM users WHERE password_hash != '' AND password_hash IS NOT NULL")
            .fetch_one(&pool)
            .await;
    pool.close().await;
    match probe {
        Ok((count,)) => count > 0,
        Err(e) => {
            warn!("pre-baseline guard: credential probe failed on {}: {e}", path.display());
            false
        }
    }
}

async fn rebuild_pre_baseline_database(path: &Path, original_error: DbError) -> Result<Database, DbError> {
    let backup = pre_baseline_backup_path(path);
    info!(
        db = %path.display(),
        backup = %backup.display(),
        original_error = %original_error,
        "Database predates the squashed 001_baseline migration chain; \
         renaming it aside and rebuilding an empty database \
         (pre-launch behavior — dev databases are disposable)"
    );

    retry_startup_file_op("rename pre-baseline database", path, || {
        std::fs::rename(path, &backup)
    })
    .map_err(|e| {
        DbError::Init(format!(
            "Pre-baseline rebuild failed: could not rename old database to {}: {e}. \
             Original error: {original_error}",
            backup.display()
        ))
    })?;

    // Move the WAL/SHM sidecars alongside the renamed database so the new
    // file does not start life next to a stale journal.
    for suffix in ["-wal", "-shm"] {
        let src = sibling_with_suffix(path, suffix);
        if !src.exists() {
            continue;
        }
        let dst = sibling_with_suffix(&backup, suffix);
        if let Err(rename_err) =
            retry_startup_file_op("rename pre-baseline sidecar", &src, || std::fs::rename(&src, &dst))
        {
            warn!(
                sidecar = %src.display(),
                error = %rename_err,
                "Could not rename pre-baseline sidecar; deleting it instead"
            );
            retry_startup_file_op("remove pre-baseline sidecar", &src, || std::fs::remove_file(&src)).map_err(
                |e| {
                    DbError::Init(format!(
                        "Pre-baseline rebuild failed: could not move or delete sidecar {}: {e}. \
                         Original error: {original_error}",
                        src.display()
                    ))
                },
            )?;
        }
    }

    match try_init_file(path).await {
        Ok(db) => {
            info!(
                backup = %backup.display(),
                "Rebuilt empty database from baseline; old database preserved at backup path"
            );
            Ok(db)
        }
        Err(retry_err) => Err(DbError::Init(format!(
            "Pre-baseline rebuild failed after renaming old database to {}: {retry_err}. \
             Original error: {original_error}",
            backup.display()
        ))),
    }
}

async fn recover_and_retry(path: &Path, original_error: DbError) -> Result<Database, DbError> {
    let backup_path = format!("{}.backup.{}", path.display(), nomifun_common::now_ms());
    warn!("Backing up corrupted database to: {backup_path}");

    std::fs::rename(path, &backup_path).map_err(|e| {
        DbError::Init(format!(
            "Recovery failed: could not backup corrupted database: {e}. \
             Original error: {original_error}"
        ))
    })?;

    match try_init_file(path).await {
        Ok(db) => {
            warn!("Database recovered. Backup at: {backup_path}");
            Ok(db)
        }
        Err(retry_err) => Err(DbError::Init(format!(
            "Recovery failed after backup: {retry_err}. Original error: {original_error}"
        ))),
    }
}

fn should_attempt_recovery(err: &DbError) -> bool {
    match err {
        DbError::Migration(_) => false,
        DbError::NotFound(_) | DbError::Conflict(_) | DbError::SafetyBackup(_) => false,
        DbError::Query(_) | DbError::Init(_) => is_corruption_like_error(err),
    }
}

fn is_corruption_like_error(err: &DbError) -> bool {
    let message = err.to_string().to_ascii_lowercase();

    [
        "sqlite_corrupt",
        "database disk image is malformed",
        "file is not a database",
        "sqlite_notadb",
        "malformed database schema",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    fn migrator_through(version: i64) -> Migrator {
        Migrator {
            migrations: Cow::Owned(
                DB_MIGRATOR
                    .iter()
                    .filter(|migration| migration.version <= version)
                    .cloned()
                    .collect(),
            ),
            ignore_missing: false,
            locking: false,
            no_tx: false,
        }
    }

    async fn file_database_through(path: &Path, version: i64) -> SqlitePool {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        migrator_through(version).run(&pool).await.unwrap();
        pool
    }

    async fn seed_terminal_cron_before_037(pool: &SqlitePool) {
        sqlx::query(
            "INSERT OR IGNORE INTO users \
             (id, username, password_hash, created_at, updated_at) \
             VALUES ('system_default_user', 'admin', '', 1, 1)",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO cron_jobs (\
                 id, name, enabled, schedule_kind, schedule_value, \
                 payload_message, execution_mode, agent_type, created_by, \
                 target_kind, terminal_mode, terminal_command, terminal_args, \
                 terminal_script, created_at, updated_at\
             ) VALUES (\
                 'cron-terminal-snapshot', 'terminal recovery proof', 1, \
                 'every', '60000', 'terminal work', 'existing', 'nomi', 'user', \
                 'terminal', 'new_terminal', '/bin/sh', '[]', 'echo legacy', 1, 1\
             )",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO cron_job_runs \
             (id, job_id, executed_at_ms, status, created_at_ms) \
             VALUES ('run-terminal-snapshot', 'cron-terminal-snapshot', 2, 'ok', 2)",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn migrate_pool_through(pool: &SqlitePool, version: i64) {
        sqlx::query("PRAGMA foreign_keys = OFF; PRAGMA legacy_alter_table = ON")
            .execute(pool)
            .await
            .unwrap();
        let result = migrator_through(version).run(pool).await;
        sqlx::query("PRAGMA foreign_keys = ON; PRAGMA legacy_alter_table = OFF")
            .execute(pool)
            .await
            .unwrap();
        result.unwrap();
    }

    async fn file_database_with_terminal_cron_through(
        path: &Path,
        version: i64,
    ) -> SqlitePool {
        assert!((36..=41).contains(&version));
        let pool = file_database_through(path, 36).await;
        seed_terminal_cron_before_037(&pool).await;
        if version > 36 {
            migrate_pool_through(&pool, version).await;
        }
        pool
    }

    async fn open_read_only_file(path: &Path) -> SqlitePool {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(false)
                    .read_only(true),
            )
            .await
            .unwrap()
    }

    async fn read_backup_marker(path: &Path) -> Vec<String> {
        let pool = open_read_only_file(path).await;
        let rows = sqlx::query_scalar("SELECT value FROM backup_probe ORDER BY value")
            .fetch_all(&pool)
            .await
            .unwrap();
        pool.close().await;
        rows
    }

    async fn read_cron_retirement_counts(path: &Path) -> CronRetirementCounts {
        let pool = open_read_only_file(path).await;
        let counts = cron_retirement_counts(&pool).await.unwrap().unwrap();
        pool.close().await;
        counts
    }

    #[test]
    fn recovery_skips_migration_version_mismatch() {
        let err = DbError::Migration(sqlx::migrate::MigrateError::VersionMismatch(1));

        assert!(
            !should_attempt_recovery(&err),
            "migration checksum mismatch must not trigger corruption recovery"
        );
    }

    #[test]
    fn recovery_skips_lock_contention_errors() {
        let err = DbError::Init("database is locked".into());

        assert!(
            !should_attempt_recovery(&err),
            "lock contention must not trigger recovery"
        );
    }

    #[test]
    fn recovery_allows_corruption_like_errors() {
        let err = DbError::Init("database disk image is malformed".into());

        assert!(
            should_attempt_recovery(&err),
            "corruption-like failures should trigger recovery"
        );
    }

    #[test]
    fn recovery_never_treats_a_safety_backup_failure_as_source_corruption() {
        let err = DbError::SafetyBackup(
            "fixed snapshot says file is not a database; source is healthy".into(),
        );
        assert!(!should_attempt_recovery(&err));
    }

    #[test]
    fn pre_baseline_detector_matches_version_class_errors() {
        use sqlx::migrate::MigrateError;

        for err in [
            MigrateError::VersionMismatch(1),
            MigrateError::VersionMissing(2),
            MigrateError::VersionTooOld(1, 21),
            MigrateError::VersionTooNew(21, 1),
        ] {
            assert!(
                is_pre_baseline_migration_error(&DbError::Migration(err)),
                "version-class migration errors should trigger pre-baseline rebuild"
            );
        }
    }

    #[test]
    fn pre_baseline_detector_rejects_other_errors() {
        let exec = DbError::Migration(sqlx::migrate::MigrateError::Execute(sqlx::Error::Protocol(
            "boom".to_string(),
        )));
        assert!(
            !is_pre_baseline_migration_error(&exec),
            "execution failures must stay fail-fast"
        );

        let init = DbError::Init("database is locked".into());
        assert!(!is_pre_baseline_migration_error(&init));
    }

    #[test]
    fn pre_baseline_backup_path_appends_numeric_suffix_when_taken() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("nomifun-backend.db");

        let first = pre_baseline_backup_path(&db);
        assert_eq!(first.file_name().unwrap(), "nomifun-backend.db.pre-baseline.bak");

        std::fs::write(&first, b"taken").unwrap();
        let second = pre_baseline_backup_path(&db);
        assert_eq!(second.file_name().unwrap(), "nomifun-backend.db.pre-baseline.bak.1");

        std::fs::write(&second, b"taken").unwrap();
        let third = pre_baseline_backup_path(&db);
        assert_eq!(third.file_name().unwrap(), "nomifun-backend.db.pre-baseline.bak.2");
    }

    #[test]
    fn unified_execution_backup_path_is_fixed_and_adjacent() {
        let db = Path::new("/var/lib/nomifun/nomifun-backend.db");
        let backup = unified_execution_backup_path(db);
        assert_eq!(backup.parent(), db.parent());
        assert_eq!(
            backup.file_name().unwrap(),
            "nomifun-backend.db.pre-037-unified-agent-execution.bak"
        );
    }

    #[test]
    fn cron_agent_only_backup_path_is_fixed_and_adjacent() {
        let db = Path::new("/var/lib/nomifun/nomifun-backend.db");
        let backup = cron_agent_only_backup_path(db);
        assert_eq!(backup.parent(), db.parent());
        assert_eq!(
            backup.file_name().unwrap(),
            "nomifun-backend.db.pre-042-cron-agent-only.bak"
        );
    }

    #[tokio::test]
    async fn unified_execution_backup_contains_committed_wal_pages_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nomifun-backend.db");
        let pool = file_database_through(&path, 36).await;

        sqlx::query("CREATE TABLE backup_probe (value TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        // Flush the schema/history first, then disable auto-checkpoint and put
        // the evidence row in WAL.  The backup must see that committed page;
        // copying only the main database file would miss it.
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("PRAGMA wal_autocheckpoint = 0")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO backup_probe (value) VALUES ('committed-in-wal')")
            .execute(&pool)
            .await
            .unwrap();
        let wal_path = PathBuf::from(format!("{}-wal", path.display()));
        assert!(
            std::fs::metadata(&wal_path).unwrap().len() > 0,
            "the proof row must still have a live WAL sidecar before snapshotting"
        );

        backup_before_unified_execution_migration(&pool, &path)
            .await
            .unwrap();
        let backup = unified_execution_backup_path(&path);
        assert_eq!(
            read_backup_marker(&backup).await,
            vec!["committed-in-wal".to_owned()]
        );

        // A retry validates and reuses the first recovery point. It must not
        // overwrite it with a later source state.
        sqlx::query("INSERT INTO backup_probe (value) VALUES ('after-snapshot')")
            .execute(&pool)
            .await
            .unwrap();
        backup_before_unified_execution_migration(&pool, &path)
            .await
            .unwrap();
        assert_eq!(
            read_backup_marker(&backup).await,
            vec!["committed-in-wal".to_owned()]
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn unified_execution_backup_rejects_an_invalid_existing_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nomifun-backend.db");
        let pool = file_database_through(&path, 36).await;
        let backup = unified_execution_backup_path(&path);

        // This is a valid SQLite database, but not a pre-037 recovery point.
        // Checking only quick_check would incorrectly accept it.
        let unrelated = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&backup)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        sqlx::query("CREATE TABLE unrelated (id INTEGER PRIMARY KEY)")
            .execute(&unrelated)
            .await
            .unwrap();
        unrelated.close().await;

        let error = backup_before_unified_execution_migration(&pool, &path)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("existing safety backup"));
        pool.close().await;
    }

    #[tokio::test]
    async fn cron_agent_only_backup_contains_committed_wal_pages_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nomifun-backend.db");
        let pool = file_database_with_terminal_cron_through(&path, 41).await;

        sqlx::query("CREATE TABLE backup_probe (value TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("PRAGMA wal_autocheckpoint = 0")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO backup_probe (value) VALUES ('cron-committed-in-wal')")
            .execute(&pool)
            .await
            .unwrap();
        let wal_path = PathBuf::from(format!("{}-wal", path.display()));
        assert!(
            std::fs::metadata(&wal_path).unwrap().len() > 0,
            "the cron recovery proof must still be resident in WAL"
        );

        backup_before_cron_agent_only_migration(&pool, &path)
            .await
            .unwrap();
        let backup = cron_agent_only_backup_path(&path);
        assert_eq!(
            read_backup_marker(&backup).await,
            vec!["cron-committed-in-wal".to_owned()]
        );
        assert_eq!(
            read_cron_retirement_counts(&backup).await,
            CronRetirementCounts { jobs: 1, runs: 1 }
        );

        // Retry must validate and reuse the original recovery point rather than
        // replacing it with a later source state.
        sqlx::query("INSERT INTO backup_probe (value) VALUES ('after-cron-snapshot')")
            .execute(&pool)
            .await
            .unwrap();
        backup_before_cron_agent_only_migration(&pool, &path)
            .await
            .unwrap();
        assert_eq!(
            read_backup_marker(&backup).await,
            vec!["cron-committed-in-wal".to_owned()]
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn cron_agent_only_backup_gate_skips_every_ineligible_source() {
        let root = tempfile::tempdir().unwrap();

        // Fresh SQLite file: no migration history.
        let fresh_path = root.path().join("fresh.db");
        let fresh = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&fresh_path)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        backup_before_cron_agent_only_migration(&fresh, &fresh_path)
            .await
            .unwrap();
        assert!(!cron_agent_only_backup_path(&fresh_path).exists());
        fresh.close().await;

        // Pre-037 source, even with terminal cron history.
        let pre_037_path = root.path().join("pre-037.db");
        let pre_037 =
            file_database_with_terminal_cron_through(&pre_037_path, 36).await;
        backup_before_cron_agent_only_migration(&pre_037, &pre_037_path)
            .await
            .unwrap();
        assert!(!cron_agent_only_backup_path(&pre_037_path).exists());
        pre_037.close().await;

        // Migration window is open, but there is no destructive data.
        let no_terminal_path = root.path().join("no-terminal.db");
        let no_terminal = file_database_through(&no_terminal_path, 41).await;
        backup_before_cron_agent_only_migration(&no_terminal, &no_terminal_path)
            .await
            .unwrap();
        assert!(!cron_agent_only_backup_path(&no_terminal_path).exists());
        no_terminal.close().await;

        // Fully migrated source never creates or revalidates a pre-042 file.
        let already_042_path = root.path().join("already-042.db");
        let already_042 = init_database(&already_042_path).await.unwrap();
        backup_before_cron_agent_only_migration(already_042.pool(), &already_042_path)
            .await
            .unwrap();
        assert!(!cron_agent_only_backup_path(&already_042_path).exists());
        already_042.close().await;
    }

    #[tokio::test]
    async fn existing_cron_snapshot_requires_037_present_and_042_absent() {
        for defect in ["missing-037", "already-042"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("nomifun-backend.db");
            let pool = file_database_with_terminal_cron_through(&path, 41).await;
            let backup = cron_agent_only_backup_path(&path);
            sqlx::query("VACUUM main INTO ?")
                .bind(backup.to_string_lossy().as_ref())
                .execute(&pool)
                .await
                .unwrap();

            let backup_pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(
                    SqliteConnectOptions::new()
                        .filename(&backup)
                        .create_if_missing(false),
                )
                .await
                .unwrap();
            match defect {
                "missing-037" => {
                    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 37")
                        .execute(&backup_pool)
                        .await
                        .unwrap();
                }
                "already-042" => {
                    sqlx::query("UPDATE _sqlx_migrations SET version = 42 WHERE version = 41")
                        .execute(&backup_pool)
                        .await
                        .unwrap();
                }
                _ => unreachable!(),
            }
            assert_eq!(
                sqlx::query_scalar::<_, String>("PRAGMA quick_check")
                    .fetch_one(&backup_pool)
                    .await
                    .unwrap(),
                "ok"
            );
            backup_pool.close().await;

            let error = backup_before_cron_agent_only_migration(&pool, &path)
                .await
                .unwrap_err();
            assert!(matches!(error, DbError::SafetyBackup(_)), "{defect}: {error}");
            assert!(error.to_string().contains("existing safety backup"));
            pool.close().await;
        }
    }

    #[tokio::test]
    async fn invalid_existing_cron_snapshot_fails_closed_before_042() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nomifun-backend.db");
        let pool = file_database_with_terminal_cron_through(&path, 41).await;
        let backup = cron_agent_only_backup_path(&path);
        sqlx::query("VACUUM main INTO ?")
            .bind(backup.to_string_lossy().as_ref())
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;

        // Keep the backup valid SQLite with the correct migration window and
        // terminal row, but remove schema required to recover its run history.
        let backup_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&backup)
                    .create_if_missing(false),
            )
            .await
            .unwrap();
        sqlx::query("DROP TABLE cron_job_runs")
            .execute(&backup_pool)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, String>("PRAGMA quick_check")
                .fetch_one(&backup_pool)
                .await
                .unwrap(),
            "ok"
        );
        backup_pool.close().await;

        let error = init_database(&path).await.unwrap_err();
        assert!(matches!(error, DbError::SafetyBackup(_)));
        assert!(error.to_string().contains("cron_job_runs"), "{error}");

        let source = open_read_only_file(&path).await;
        let applied_042: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM _sqlx_migrations \
             WHERE version = 42 AND success = 1)",
        )
        .fetch_one(&source)
        .await
        .unwrap();
        let terminal_jobs: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cron_jobs WHERE target_kind = 'terminal'",
        )
        .fetch_one(&source)
        .await
        .unwrap();
        assert_eq!(applied_042, 0, "migration 042 must not run after backup refusal");
        assert_eq!(terminal_jobs, 1, "source retirement data must remain intact");
        source.close().await;
    }

    #[tokio::test]
    async fn file_startup_from_every_037_through_041_boundary_snapshots_then_applies_042() {
        for source_version in 37..=41 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir
                .path()
                .join(format!("nomifun-through-{source_version}.db"));
            let source =
                file_database_with_terminal_cron_through(&path, source_version).await;
            source.close().await;

            let db = init_database(&path)
                .await
                .unwrap_or_else(|error| panic!("upgrade from {source_version} failed: {error}"));
            let applied_042: i64 = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM _sqlx_migrations \
                 WHERE version = 42 AND success = 1)",
            )
            .fetch_one(db.pool())
            .await
            .unwrap();
            let retired_job: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM cron_jobs WHERE id = 'cron-terminal-snapshot'",
            )
            .fetch_one(db.pool())
            .await
            .unwrap();
            assert_eq!(applied_042, 1, "source version {source_version}");
            assert_eq!(retired_job, 0, "source version {source_version}");
            assert!(
                !table_has_columns(db.pool(), "cron_jobs", &["target_kind"])
                    .await
                    .unwrap(),
                "source version {source_version}: final schema still has target_kind"
            );

            let backup = cron_agent_only_backup_path(&path);
            assert!(backup.exists(), "source version {source_version}");
            validate_safety_snapshot(&backup, SafetySnapshotKind::CronAgentOnly)
                .await
                .unwrap_or_else(|error| {
                    panic!("source version {source_version} backup invalid: {error}")
                });
            assert_eq!(
                read_cron_retirement_counts(&backup).await,
                CronRetirementCounts { jobs: 1, runs: 1 },
                "source version {source_version}"
            );
            let backup_pool = open_read_only_file(&backup).await;
            let backup_version: i64 = sqlx::query_scalar(
                "SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1",
            )
            .fetch_one(&backup_pool)
            .await
            .unwrap();
            assert_eq!(backup_version, source_version);
            backup_pool.close().await;
            db.close().await;
        }
    }

    #[tokio::test]
    async fn corrupt_fixed_backup_fails_closed_without_recovering_the_healthy_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nomifun-backend.db");
        let pool = file_database_through(&path, 36).await;
        sqlx::query("CREATE TABLE source_probe (value TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO source_probe (value) VALUES ('healthy-source')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;

        let fixed_backup = unified_execution_backup_path(&path);
        std::fs::write(&fixed_backup, b"not a sqlite database").unwrap();

        let error = init_database(&path).await.unwrap_err();
        assert!(matches!(error, DbError::SafetyBackup(_)));
        assert!(path.exists(), "the healthy source database stays in place");
        assert_eq!(
            std::fs::read(&fixed_backup).unwrap(),
            b"not a sqlite database",
            "the bad fixed snapshot is not silently replaced"
        );
        let recovery_files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("nomifun-backend.db.backup.")
            })
            .collect();
        assert!(
            recovery_files.is_empty(),
            "backup validation must never enter destructive corruption recovery"
        );

        let source = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(false)
                    .read_only(true),
            )
            .await
            .unwrap();
        let marker: String = sqlx::query_scalar("SELECT value FROM source_probe")
            .fetch_one(&source)
            .await
            .unwrap();
        assert_eq!(marker, "healthy-source");
        let migration_37: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM _sqlx_migrations \
             WHERE version = 37 AND success = 1)",
        )
        .fetch_one(&source)
        .await
        .unwrap();
        assert_eq!(migration_37, 0, "migration 37 was not applied after refusal");
        let unified_schema: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master \
             WHERE type = 'table' AND name = 'agent_executions')",
        )
        .fetch_one(&source)
        .await
        .unwrap();
        assert_eq!(unified_schema, 0, "no empty replacement database was created");
        source.close().await;
    }

    #[tokio::test]
    async fn unified_execution_backup_is_not_recreated_after_migration_037() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nomifun-backend.db");
        let pool = file_database_through(&path, 37).await;
        let backup = unified_execution_backup_path(&path);

        backup_before_unified_execution_migration(&pool, &path)
            .await
            .unwrap();
        assert!(!backup.exists());
        pool.close().await;
    }

    #[tokio::test]
    async fn migration_preserves_fk_references() {
        let db = init_database_memory().await.unwrap();
        let pool = db.pool();

        let fk_table: String = sqlx::query_scalar(
            "SELECT \"table\" FROM pragma_foreign_key_list('messages') WHERE \"from\"='conversation_id'",
        )
        .fetch_one(pool)
        .await
        .unwrap();

        assert_eq!(fk_table, "conversations");
    }

    #[tokio::test]
    async fn database_has_real_credential_distinguishes_empty_and_set_passwords() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("probe.db");

        // Fresh DB: only the seeded system_default_user with an EMPTY password.
        {
            let db = init_database(&path).await.unwrap();
            db.close().await;
        }
        assert!(
            !database_has_real_credential(&path).await,
            "an empty-password seed must not be treated as a real credential"
        );

        // Now write a real password hash and re-probe.
        {
            let db = init_database(&path).await.unwrap();
            sqlx::query("UPDATE users SET password_hash = 'realhash' WHERE id = 'system_default_user'")
                .execute(db.pool())
                .await
                .unwrap();
            db.close().await;
        }
        assert!(
            database_has_real_credential(&path).await,
            "a non-empty password_hash must be detected so the rebuild is refused"
        );
    }

    #[test]
    fn migrations_table_unique_conflict_detected_from_message() {
        // Build the same Execute(sqlx::Error) shape that surfaces when two
        // processes race on `INSERT INTO _sqlx_migrations`. The detector has
        // to match on the textual message because the SQLite extended code
        // is not preserved on the path through MigrateError.
        let inner = sqlx::Error::Protocol("UNIQUE constraint failed: _sqlx_migrations.version".to_string());
        let err = sqlx::migrate::MigrateError::Execute(inner);
        assert!(is_migrations_table_unique_conflict(&err));
    }

    #[test]
    fn migrations_table_unique_conflict_ignores_other_errors() {
        let other = sqlx::migrate::MigrateError::VersionMismatch(1);
        assert!(!is_migrations_table_unique_conflict(&other));

        let unrelated = sqlx::migrate::MigrateError::Execute(sqlx::Error::Protocol(
            "UNIQUE constraint failed: users.username".to_string(),
        ));
        assert!(!is_migrations_table_unique_conflict(&unrelated));
    }

    #[test]
    fn migrate_lock_path_sits_next_to_db() {
        let db = Path::new("/var/lib/nomifun/nomifun-backend.db");
        let lock = migrate_lock_path(db);
        assert_eq!(lock.parent(), db.parent());
        assert_eq!(lock.file_name().unwrap(), "nomifun-backend.db.migrate.lock");
    }

    #[test]
    fn startup_file_retry_handles_windows_transient_lock_errors() {
        for code in [5, 32, 33] {
            let err = std::io::Error::from_raw_os_error(code);
            assert!(
                is_retryable_startup_file_error(&err),
                "Windows startup file error {code} should be retryable"
            );
        }
    }

    #[test]
    fn startup_file_retry_rejects_non_transient_errors() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing file");
        assert!(!is_retryable_startup_file_error(&err));
    }
}
