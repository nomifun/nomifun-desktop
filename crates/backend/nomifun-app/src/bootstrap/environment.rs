//! Bootstrap layers shared by non-MCP subcommands.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use fs2::FileExt;
use nomifun_db::sqlx::pool::PoolOptions;
use nomifun_db::sqlx::sqlite::SqliteConnectOptions;
use nomifun_db::sqlx::{Row, Sqlite, SqlitePool};
use tracing::{info, warn};

use crate::{AppConfig, config::load_or_create_storage_generation};
use nomifun_db::Database;

use crate::cli::Cli;

use super::builtin_skills::materialize_builtin_skills;
use super::server_lock::{BootServerLockAuthority, ServerLock, acquire_server_lock};
use super::tracing_init::{LogGuards, init_tracing};
use super::work_dir::resolve_work_dir;

/// Resolved environment needed by all non-MCP subcommands.
pub struct ServerEnvironment {
    /// Must be held alive for the process lifetime to flush log buffers.
    pub _log_guard: LogGuards,
    /// Exclusive per-data-dir lock; held for the process lifetime so a second
    /// backend on the same (shared-by-default) data dir fails fast instead of
    /// double-running cron/channels against the same database.
    pub _server_lock: Arc<ServerLock>,
    /// The data directory can itself be selected as another dataset's work
    /// root, so its work-root lock is separate from (and held alongside) the
    /// server lock.
    pub _data_root_work_lock: WorkRootLock,
    /// When work_dir differs from data_dir, retain its second work-root lock.
    pub _external_work_root_lock: Option<WorkRootLock>,
    pub config: AppConfig,
}

#[derive(Debug)]
pub struct WorkRootLock {
    _file: File,
    canonical_root: PathBuf,
}

const WORK_ROOT_LOCK_FILE: &str = ".nomifun-work-root.lock";

pub(crate) fn acquire_work_root_lock(work_dir: &Path) -> Result<WorkRootLock> {
    let work_metadata = std::fs::symlink_metadata(work_dir)
        .with_context(|| format!("inspect work dir {}", work_dir.display()))?;
    if lifecycle_metadata_is_link_or_reparse(&work_metadata)
        || !work_metadata.is_dir()
    {
        anyhow::bail!(
            "resolved work dir must be a real directory: {}",
            work_dir.display()
        );
    }

    let canonical_work = nomifun_common::paths::canonicalize_simplified(work_dir)
        .with_context(|| format!("canonicalize work dir {}", work_dir.display()))?;

    let path = canonical_work.join(WORK_ROOT_LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to open work-root lock {}", path.display()))?;
    file.try_lock_exclusive().map_err(|error| {
        if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() {
            anyhow::anyhow!(
                "resolved work directory {} is already in use by another NomiFun dataset; \
                 use a separate --work-dir or stop the other backend",
                canonical_work.display()
            )
        } else {
            anyhow::Error::new(error).context(format!(
                "failed to lock work root {} (filesystem without lock support?)",
                canonical_work.display()
            ))
        }
    })?;
    Ok(WorkRootLock {
        _file: file,
        canonical_root: canonical_work,
    })
}

pub(crate) fn acquire_distinct_work_root_lock(
    data_root_lock: &WorkRootLock,
    work_dir: &Path,
) -> Result<Option<WorkRootLock>> {
    let work_metadata = std::fs::symlink_metadata(work_dir)
        .with_context(|| format!("inspect work dir {}", work_dir.display()))?;
    if lifecycle_metadata_is_link_or_reparse(&work_metadata)
        || !work_metadata.is_dir()
    {
        anyhow::bail!(
            "resolved work dir must be a real directory: {}",
            work_dir.display()
        );
    }
    let canonical_work = nomifun_common::paths::canonicalize_simplified(work_dir)
        .with_context(|| format!("canonicalize work dir {}", work_dir.display()))?;
    if canonical_work == data_root_lock.canonical_root {
        return Ok(None);
    }
    acquire_work_root_lock(&canonical_work).map(Some)
}

impl WorkRootLock {
    pub(crate) fn protected_root(&self) -> &Path {
        &self.canonical_root
    }
}

/// Layer 1: canonical data-root resolution + logging + config resolution.
pub fn init_environment(cli: &Cli, merged_path: &str) -> Result<ServerEnvironment> {
    init_environment_inner(cli, merged_path)
}

/// Initialize the current in-process Nomi-core host against the same canonical
/// root used by every other database-owning command.
pub fn init_nomi_core_environment(
    cli: &Cli,
    merged_path: &str,
) -> Result<ServerEnvironment> {
    init_environment_inner(cli, merged_path)
}

fn init_environment_inner(
    cli: &Cli,
    merged_path: &str,
) -> Result<ServerEnvironment> {
    let startup_data_dir =
        super::data_root::normalize_requested_startup_data_root(
            cli.data_dir.clone(),
        );
    let log_dir = cli
        .log_dir
        .clone()
        .unwrap_or_else(|| startup_data_dir.join("logs"));
    // Export the *actual* log dir so `nomifun_system::sysinfo::resolve_log_dir`
    // (which the settings UI reads via GET /api/system/info) reports where logs
    // truly land instead of its own independent default — otherwise the UI shows
    // a Roaming path while logs write under the Local data dir. Mirrors the
    // NOMIFUN_WORK_DIR export below.
    // SAFETY: called at the very start of boot, before any service initialization
    // or env reads; the only reader of NOMIFUN_LOG_DIR is sysinfo, much later.
    unsafe {
        std::env::set_var("NOMIFUN_LOG_DIR", &log_dir);
    }
    let log_guard = init_tracing(&log_dir, cli.log_level.as_deref());

    // Notes recorded before tracing existed (e.g. the desktop shell's data-dir
    // relocation, which runs before this backend is even spawned): surface
    // them into the persistent log now — the earliest recordable point.
    for (level, message) in super::boot_log::drain_boot_notes() {
        match level {
            super::boot_log::BootNoteLevel::Info => info!(target: "boot", "{message}"),
            super::boot_log::BootNoteLevel::Warn => warn!(target: "boot", "{message}"),
        }
    }

    info!(
        path_segments = merged_path.split(if cfg!(windows) { ';' } else { ':' }).count(),
        path_len = merged_path.len(),
        "startup: PATH ready"
    );

    // Take data-dir authority before resolving any pending reset or legacy
    // work-root recovery hint from its control files.
    let server_lock = Arc::new(acquire_server_lock(&startup_data_dir)?);
    let data_dir = server_lock.protected_data_dir().to_path_buf();
    let data_root_work_lock = acquire_work_root_lock(&data_dir)?;
    nomifun_common::factory_reset::require_data_root_not_owned_as_external_work(
        &data_dir,
    )?;
    let requested_work_dir =
        resolve_work_dir(cli.work_dir.clone(), &data_dir)?;
    let external_work_root_lock =
        acquire_distinct_work_root_lock(
            &data_root_work_lock,
            &requested_work_dir,
        )?;
    let work_dir = external_work_root_lock
        .as_ref()
        .map(|lock| lock.protected_root().to_path_buf())
        .unwrap_or_else(|| data_root_work_lock.protected_root().to_path_buf());

    // SAFETY: called before any service initialization; no concurrent reads.
    unsafe {
        std::env::set_var("NOMIFUN_WORK_DIR", &work_dir);
        // Browser helpers in the agent/gateway crates resolve their default
        // browser root from this effective host data dir. This prevents a
        // custom `--data-dir` from silently leaving browser state under the
        // platform-global config directory, outside v3 reset/backup.
        std::env::set_var("NOMIFUN_DATA_DIR", &data_dir);
    }

    // CLI-derived base policy: `--local` / `--insecure-no-auth` ⇒ NoAuth,
    // otherwise JWT Required. The desktop shell overrides this to
    // `TrustLocalToken` (with a per-boot secret) on its own serving path.
    let auth_policy = if cli.local {
        nomifun_auth::AuthPolicy::NoAuth
    } else {
        nomifun_auth::AuthPolicy::Required
    };

    let config = AppConfig {
        host: cli.host.clone(),
        port: cli.port,
        data_dir,
        work_dir,
        work_dir_is_cli_override: cli.work_dir.is_some(),
        app_version: cli.app_version.clone(),
        auth_policy,
        local_trust_secret: None,
    };
    info!(
        "Running with auth policy {:?} — authentication is {}",
        config.auth_policy,
        if config.auth_policy.is_no_auth() { "disabled" } else { "enabled" }
    );

    Ok(ServerEnvironment {
        _log_guard: log_guard,
        _server_lock: server_lock,
        _data_root_work_lock: data_root_work_lock,
        _external_work_root_lock: external_work_root_lock,
        config,
    })
}

#[derive(Debug, PartialEq, Eq)]
enum ExistingV3DatabaseProbe {
    Missing,
    Current,
    /// A pre-v3 dataset without canonical v3 identity tables/columns.
    Legacy(String),
    /// Claimed v3 lineage, schema/data damage, or an incompatible binary.
    /// This is never authority to retire or reset existing user data.
    RequiresRepair(String),
}

#[derive(Debug, PartialEq, Eq)]
enum V3DataLayerState {
    FinalizedCurrent,
    BootstrapRequired,
}

const DATABASE_PROBE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

async fn table_has_column_contract(
    pool: &SqlitePool,
    table: &str,
    column: &str,
    declared_type: &str,
    not_null: bool,
    primary_key: bool,
) -> Result<bool> {
    // The identifiers are fixed below and never derived from external input.
    let sql = format!("PRAGMA table_info(\"{table}\")");
    let rows = nomifun_db::sqlx::query(&sql).fetch_all(pool).await?;
    Ok(rows.iter().any(|row| {
        let Ok(name) = row.try_get::<String, _>("name") else {
            return false;
        };
        let Ok(kind) = row.try_get::<String, _>("type") else {
            return false;
        };
        let Ok(row_not_null) = row.try_get::<i64, _>("notnull") else {
            return false;
        };
        let Ok(row_primary_key) = row.try_get::<i64, _>("pk") else {
            return false;
        };
        name == column
            && kind.eq_ignore_ascii_case(declared_type)
            && (row_not_null != 0) == not_null
            && (row_primary_key != 0) == primary_key
    }))
}

async fn probe_v3_database_pool(pool: &SqlitePool) -> Result<ExistingV3DatabaseProbe> {
    let quick_check: Vec<String> =
        nomifun_db::sqlx::query_scalar("PRAGMA quick_check")
            .fetch_all(pool)
            .await?;
    if quick_check.as_slice() != ["ok"] {
        return Ok(ExistingV3DatabaseProbe::RequiresRepair(
            format!("SQLite quick_check failed: {}", quick_check.join("; ")),
        ));
    }

    let required_tables: Vec<String> = nomifun_db::sqlx::query_scalar(
        "SELECT name FROM sqlite_schema \
         WHERE type = 'table' \
           AND name IN ('_sqlx_migrations', 'users', 'installation_identity', 'agent_metadata') \
         ORDER BY name",
    )
    .fetch_all(pool)
    .await?;
    if required_tables
        != [
            "_sqlx_migrations",
            "agent_metadata",
            "installation_identity",
            "users",
        ]
    {
        let reason = "required v3 identity tables are missing".to_owned();
        // Losing one identity table is damage to an existing v3 dataset,
        // not proof of a legacy database. Canonical columns in either other
        // identity table are sufficient evidence to preserve it for repair.
        let has_v3_identity = required_tables.iter().any(|table| table == "installation_identity")
            || (table_has_column_contract(pool, "users", "id", "INTEGER", false, true).await?
                && table_has_column_contract(pool, "users", "user_id", "TEXT", true, false).await?)
            || (table_has_column_contract(pool, "agent_metadata", "id", "INTEGER", false, true).await?
                && table_has_column_contract(pool, "agent_metadata", "agent_id", "TEXT", true, false).await?);
        return Ok(if has_v3_identity {
            ExistingV3DatabaseProbe::RequiresRepair(reason)
        } else {
            ExistingV3DatabaseProbe::Legacy(reason)
        });
    }

    if let Err(error) = nomifun_db::validate_current_migration_lineage(pool).await {
        return Ok(ExistingV3DatabaseProbe::RequiresRepair(format!(
            "database migration lineage is not the exact canonical baseline: {error}"
        )));
    }
    if let Err(error) = nomifun_db::validate_id_schema_contract(pool).await {
        return Ok(ExistingV3DatabaseProbe::RequiresRepair(format!(
            "database does not satisfy the complete v3 ID schema contract: {error}"
        )));
    }
    if let Err(error) = nomifun_db::validate_id_data_contract(pool).await {
        return Ok(ExistingV3DatabaseProbe::RequiresRepair(format!(
            "database does not satisfy the complete v3 ID data contract: {error}"
        )));
    }
    if let Err(error) = nomifun_agent_session::AgentSessionStore::from_pool(pool.clone()).await {
        return Ok(ExistingV3DatabaseProbe::RequiresRepair(format!(
            "database does not satisfy the canonical Agent Store schema contract: {error}"
        )));
    }

    for (table, column, declared_type, not_null, primary_key) in [
        ("users", "id", "INTEGER", false, true),
        ("users", "user_id", "TEXT", true, false),
        ("installation_identity", "id", "INTEGER", false, true),
        ("installation_identity", "singleton_key", "TEXT", true, false),
        ("installation_identity", "owner_user_id", "TEXT", true, false),
        ("agent_metadata", "id", "INTEGER", false, true),
        ("agent_metadata", "agent_id", "TEXT", true, false),
    ] {
        if !table_has_column_contract(
            pool, table, column, declared_type, not_null, primary_key,
        )
        .await?
        {
            return Ok(ExistingV3DatabaseProbe::RequiresRepair(
                "core database identity columns do not match the v3 schema".into(),
            ));
        }
    }

    let identities: Vec<(String, String)> = nomifun_db::sqlx::query_as(
        "SELECT singleton_key, owner_user_id FROM installation_identity",
    )
    .fetch_all(pool)
    .await?;
    let [(singleton_key, owner_user_id)] = identities.as_slice() else {
        return Ok(ExistingV3DatabaseProbe::RequiresRepair(format!(
            "expected one installation identity row, found {}",
            identities.len()
        )));
    };
    if singleton_key != "installation" {
        return Ok(ExistingV3DatabaseProbe::RequiresRepair(
            "installation identity singleton key is invalid".into(),
        ));
    }
    if nomifun_common::UserId::parse(owner_user_id.clone()).is_err() {
        return Ok(ExistingV3DatabaseProbe::RequiresRepair(
            "installation owner identity is not a canonical UUIDv7".into(),
        ));
    }
    let owner_rows: i64 =
        nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE user_id = ?")
            .bind(owner_user_id)
            .fetch_one(pool)
            .await?;
    if owner_rows != 1 {
        return Ok(ExistingV3DatabaseProbe::RequiresRepair(
            "installation identity does not resolve to exactly one owner".into(),
        ));
    }

    Ok(ExistingV3DatabaseProbe::Current)
}

async fn probe_existing_v3_database(path: &Path) -> Result<ExistingV3DatabaseProbe> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ExistingV3DatabaseProbe::Missing);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("inspect database before v3 probe {}", path.display()));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        anyhow::bail!(
            "database path must be a regular file before v3 probe: {}",
            path.display()
        );
    }

    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .read_only(true)
        .foreign_keys(true)
        .busy_timeout(DATABASE_PROBE_BUSY_TIMEOUT);
    let pool = PoolOptions::<Sqlite>::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .with_context(|| format!("open existing database read-only for v3 probe {}", path.display()))?;
    let probe = probe_v3_database_pool(&pool)
        .await
        .with_context(|| format!("probe existing database v3 identity {}", path.display()));
    pool.close().await;
    probe
}

#[cfg(windows)]
fn lifecycle_metadata_is_link_or_reparse(
    metadata: &std::fs::Metadata,
) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn lifecycle_metadata_is_link_or_reparse(
    metadata: &std::fs::Metadata,
) -> bool {
    metadata.file_type().is_symlink()
}

async fn prepare_v3_data_layer(config: &AppConfig) -> Result<V3DataLayerState> {
    // A receipt is also a binding to the resolved work root.  Never silently
    // accept a database that was finalized against another external workspace.
    // An explicit reset is the only operation allowed to change that binding.
    let receipt_status =
        nomifun_common::factory_reset::inspect_v3_dataset_receipt(
            &config.data_dir,
            &config.work_dir,
        )?;
    // A validated immutable plan supersedes the transient request. The
    // request is deliberately removed as soon as the plan is durable, so a
    // crash in that gap must be allowed to reach the filesystem coordinator
    // even while the old receipt still names the previous work root.
    let pending_reset =
        nomifun_common::factory_reset::read_pending_v3_reset(
            &config.data_dir,
            &config.work_dir,
        )?
        .is_some();
    if receipt_status
        == nomifun_common::factory_reset::DatasetReceiptStatus::WorkRootMismatch
        && !pending_reset
        && !config
            .data_dir
            .join(nomifun_common::factory_reset::V3_DATASET_RESET_REQUEST_FILE)
            .exists()
    {
        anyhow::bail!(
            "the v3 dataset receipt is bound to a different resolved work root; \
             refusing to accept the database with the current --work-dir; \
             request an explicit factory reset before changing the work root"
        );
    }

    // The filesystem gate always runs before the read-only SQLite probe, but
    // it is deliberately non-destructive when a database file exists.  The
    // app probe below is the only authority allowed to classify/retire that
    // database. Receipt-valid databases still have to prove a supported
    // exact embedded baseline plus the complete schema/data and installation
    // identity contracts. Historical prefixes are preserved for an explicit
    // reset; startup never mutates them in place.
    match nomifun_common::factory_reset::prepare_v3_dataset(
        &config.data_dir,
        &config.work_dir,
    )? {
        nomifun_common::factory_reset::DatasetPreparation::ResetApplied => {
            info!(
                target: "boot",
                "v3 dataset reset prepared — retired data will not be migrated"
            );
        }
        nomifun_common::factory_reset::DatasetPreparation::Unchanged => {}
    }

    let state = match probe_existing_v3_database(&config.database_path()).await? {
        ExistingV3DatabaseProbe::Missing => V3DataLayerState::BootstrapRequired,
        ExistingV3DatabaseProbe::Current => {
            let reset_pending =
                nomifun_common::factory_reset::read_pending_v3_reset(
                    &config.data_dir,
                    &config.work_dir,
                )?
                .is_some();
            if !reset_pending
                && receipt_status
                    != nomifun_common::factory_reset::DatasetReceiptStatus::Current
            {
                let bootstrap_status =
                    nomifun_common::factory_reset::inspect_v3_dataset_bootstrap_binding(
                        &config.data_dir,
                        &config.work_dir,
                    )?;
                if bootstrap_status
                    == nomifun_common::factory_reset::DatasetReceiptStatus::WorkRootMismatch
                {
                    anyhow::bail!(
                        "the valid v3 database has an unfinished bootstrap binding for a \
                         different resolved work root; refusing to attach the current workspace"
                    );
                }
                if bootstrap_status
                    != nomifun_common::factory_reset::DatasetReceiptStatus::Current
                {
                    anyhow::bail!(
                        "the v3 database passed its identity probe but has neither a matching \
                         finalized receipt nor an unfinished bootstrap binding for this resolved \
                         work root; refusing to guess the workspace identity"
                    );
                }
            }
            let state = if nomifun_common::factory_reset::require_current_v3_dataset_for_work_dir(
                &config.data_dir,
                &config.work_dir,
            )
            .is_ok()
            {
                V3DataLayerState::FinalizedCurrent
            } else {
                // A fresh database may already exist during crash recovery,
                // but the pending reset/receipt hand-off still requires the
                // full server bootstrap before it can be finalized.
                V3DataLayerState::BootstrapRequired
            };
            if state == V3DataLayerState::FinalizedCurrent
                && !config.work_dir_is_cli_override
                && nomifun_common::dir_config::repairable_malformed_work_dir_exists(
                    &config.data_dir,
                )?
            {
                nomifun_common::factory_reset::ensure_current_v3_work_root_owner(
                    &config.data_dir,
                    &config.work_dir,
                )?;
                nomifun_common::dir_config::replace_malformed_work_dir_after_lifecycle_proof(
                    &config.data_dir,
                    &config.work_dir,
                )?;
                warn!(
                    target: "boot",
                    work_dir = %config.work_dir.display(),
                    "repaired a truncated legacy dir-config after the database proved its v3 lineage"
                );
            }
            state
        }
        ExistingV3DatabaseProbe::RequiresRepair(reason) => {
            return Err(nomifun_common::AppError::Conflict(format!(
                "database compatibility validation failed; existing data preserved without automatic retirement: {reason}"
            )).into());
        }
        ExistingV3DatabaseProbe::Legacy(reason) => {
            warn!(
                target: "boot",
                database = %config.database_path().display(),
                reason,
                "database is a legacy dataset without v3 identity; checking retirement authority"
            );
            nomifun_common::factory_reset::retire_non_v3_dataset_after_probe(
                &config.data_dir,
                &config.work_dir,
            )?;
            V3DataLayerState::BootstrapRequired
        }
    };
    Ok(state)
}

fn install_storage_generation_environment(config: &AppConfig) -> Result<()> {
    // Generate this only after every reset decision has removed the old
    // dataset marker and the caller has committed to bootstrapping/opening the
    // data layer. Browser-local state is outside SQLite, so the value scopes
    // every entity cache key to exactly this post-reset generation.
    let storage_generation = load_and_publish_storage_generation(&config.data_dir)?;
    let receipt_status =
        nomifun_common::factory_reset::inspect_v3_dataset_receipt(
            &config.data_dir,
            &config.work_dir,
        )?;
    if receipt_status
        == nomifun_common::factory_reset::DatasetReceiptStatus::Current
    {
        nomifun_common::factory_reset::ensure_current_v3_work_root_owner(
            &config.data_dir,
            &config.work_dir,
        )?;
    } else {
        nomifun_common::factory_reset::ensure_v3_work_root_binding(
            &config.data_dir,
            &config.work_dir,
            &storage_generation,
        )?;
        nomifun_common::factory_reset::write_v3_dataset_bootstrap_binding(
            &config.data_dir,
            &config.work_dir,
            &storage_generation,
        )?;
    }
    Ok(())
}

fn load_and_publish_storage_generation(data_dir: &Path) -> Result<String> {
    let storage_generation = load_or_create_storage_generation(data_dir)?;
    // SAFETY: host bootstrap is single-threaded before services and routes are
    // published; system-info is the only later reader of this variable.
    unsafe {
        std::env::set_var("NOMIFUN_STORAGE_GENERATION", &storage_generation);
    }
    Ok(storage_generation)
}

impl ServerEnvironment {
    /// Mint authority for startup orphan reconciliation while retaining the
    /// exact OS-level server lock. This proves exclusive database ownership;
    /// it does not prove that descendants of a previous owner have exited.
    pub fn boot_reconciliation_authority(&self) -> BootServerLockAuthority {
        self._server_lock.boot_authority()
    }

    /// Open the existing finalized dataset for the doctor command.
    ///
    /// Doctor may run the destructive pre-open reset gate, but it must never
    /// create/finalize a replacement dataset without the server's complete
    /// service/side-store bootstrap.
    pub async fn init_doctor_data_layer(&self) -> Result<Database> {
        if prepare_v3_data_layer(&self.config).await?
            != V3DataLayerState::FinalizedCurrent
        {
            anyhow::bail!(
                "the v3 dataset requires bootstrap after reset; start NomiFun normally once, then rerun `nomicore doctor`"
            );
        }
        install_storage_generation_environment(&self.config)?;

        let db_path = self.config.database_path();
        info!(
            "Opening validated database for doctor at {}",
            db_path.display()
        );
        let database = nomifun_db::init_database(&db_path).await?;
        Ok(database)
    }
}

/// Layer 2: Materialize builtin skills + initialize the database.
///
/// Requires only `data_dir`. Subcommands that need persistent state
/// (database, skill files) should call this after `init_environment`.
pub async fn init_data_layer(config: &AppConfig) -> Result<Database> {
    let boot = Instant::now();

    let preparation = prepare_v3_data_layer(config).await?;
    if preparation == V3DataLayerState::BootstrapRequired
        && !config.work_dir_is_cli_override
        && nomifun_common::dir_config::replace_malformed_work_dir_after_lifecycle_proof(
            &config.data_dir,
            &config.work_dir,
        )?
    {
        warn!(
            target: "boot",
            work_dir = %config.work_dir.display(),
            "repaired a truncated legacy dir-config after the dataset was proven safe for fresh v3 bootstrap"
        );
    }
    install_storage_generation_environment(config)?;

    materialize_builtin_skills(&config.data_dir).await?;
    info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: builtin skills materialized"
    );

    let db_path = config.database_path();
    info!("Initializing database at {}", db_path.display());
    let database = nomifun_db::init_database(&db_path).await?;
    info!(elapsed_ms = boot.elapsed().as_millis(), "startup: database initialized");

    // One-shot absolute-path rewrite after a data-root relocation (layout
    // migration). Idempotent and never fails the boot; see
    // `bootstrap::relocation`.
    super::relocation::rewrite_relocated_paths(&database, &config.data_dir).await;

    Ok(database)
}

/// Commit the filesystem-level v3 dataset only after every required
/// product-owned side store has initialized successfully.
///
/// Keeping this separate from [`init_data_layer`] is deliberate: the main
/// SQLite schema alone is not proof that companion/workshop and
/// the other service-owned stores completed their v3 bootstrap. If service
/// assembly fails, the pending reset plan remains durable and the next boot
/// resumes instead of accepting a half-initialized dataset.
pub fn finalize_data_layer(config: &AppConfig) -> Result<()> {
    let storage_generation = load_or_create_storage_generation(&config.data_dir)?;
    nomifun_common::factory_reset::write_v3_dataset_receipt_for_work_dir(
        &config.data_dir,
        &config.work_dir,
        &storage_generation,
    )?;
    nomifun_common::factory_reset::finalize_v3_dataset_reset(
        &config.data_dir,
        &config.work_dir,
    )?;
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn probe_accepts_database_created_from_the_canonical_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nomifun-backend.db");
        let database = nomifun_db::init_database(&path).await.unwrap();
        database.close().await;

        assert_eq!(
            probe_existing_v3_database(&path).await.unwrap(),
            ExistingV3DatabaseProbe::Current
        );
    }

    #[tokio::test]
    async fn probe_rejects_an_edited_canonical_baseline_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nomifun-backend.db");
        let database = nomifun_db::init_database(&path).await.unwrap();
        sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00'")
            .execute(database.pool())
            .await
            .unwrap();
        database.close().await;

        match probe_existing_v3_database(&path).await.unwrap() {
            ExistingV3DatabaseProbe::RequiresRepair(reason) => {
                assert!(reason.contains("migration lineage"), "{reason}");
            }
            other => panic!("edited lineage must fail closed: {other:?}"),
        }
    }

    #[tokio::test]
    async fn probe_rejects_schema_damage_without_retiring_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nomifun-backend.db");
        let database = nomifun_db::init_database(&path).await.unwrap();
        sqlx::query("DROP INDEX idx_agent_events_correlation")
            .execute(database.pool())
            .await
            .unwrap();
        database.close().await;

        match probe_existing_v3_database(&path).await.unwrap() {
            ExistingV3DatabaseProbe::RequiresRepair(reason) => {
                assert!(reason.contains("schema contract"), "{reason}");
            }
            other => panic!("damaged schema must fail closed: {other:?}"),
        }
    }
}
