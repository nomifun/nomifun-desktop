//! Bootstrap layers shared by non-MCP subcommands.

use std::time::Instant;

use anyhow::Result;
use tracing::{info, warn};

use crate::AppConfig;
use nomifun_db::Database;

use crate::cli::Cli;

use super::builtin_skills::materialize_builtin_skills;
use super::server_lock::{ServerLock, acquire_server_lock};
use super::tracing_init::{LogGuards, init_tracing};
use super::work_dir::resolve_work_dir;

/// Resolved environment needed by all non-MCP subcommands.
pub struct ServerEnvironment {
    /// Must be held alive for the process lifetime to flush log buffers.
    pub _log_guard: LogGuards,
    /// Exclusive per-data-dir lock; held for the process lifetime so a second
    /// backend on the same (shared-by-default) data dir fails fast instead of
    /// double-running cron/channels against the same database.
    pub _server_lock: ServerLock,
    pub config: AppConfig,
}

/// Layer 1: Logging + config resolution.
///
/// Cheap, synchronous, no IO beyond creating the log directory.
/// All subcommands that need logging and config should call this first.
pub fn init_environment(cli: &Cli, merged_path: &str) -> Result<ServerEnvironment> {
    let log_dir = cli.log_dir.clone().unwrap_or_else(|| cli.data_dir.join("logs"));
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

    let work_dir = resolve_work_dir(cli.work_dir.clone(), &cli.data_dir);

    // SAFETY: called before any service initialization; no concurrent reads.
    unsafe {
        std::env::set_var("NOMIFUN_WORK_DIR", &work_dir);
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
        data_dir: cli.data_dir.clone(),
        work_dir,
        app_version: cli.app_version.clone(),
        auth_policy,
        local_trust_secret: None,
    };
    info!(
        "Running with auth policy {:?} — authentication is {}",
        config.auth_policy,
        if config.auth_policy.is_no_auth() { "disabled" } else { "enabled" }
    );

    // Fail fast BEFORE any data-layer work if another backend already owns
    // this data dir (all hosts share one default dir; see server_lock.rs).
    let server_lock = acquire_server_lock(&config.data_dir)?;

    // If a factory reset was armed (marker file present), perform it now — the
    // earliest safe point: we hold the exclusive lock, no DB pool is open, and
    // no background loop has started. `init_data_layer` then recreates a fresh
    // database via the normal migration path. See nomifun_common::factory_reset.
    match nomifun_common::factory_reset::apply_pending_reset(&config.data_dir, &config.work_dir) {
        Ok(true) => info!(target: "boot", "factory reset applied — database and derived data wiped"),
        Ok(false) => {}
        Err(e) => warn!(target: "boot", "factory reset failed: {e}"),
    }

    Ok(ServerEnvironment {
        _log_guard: log_guard,
        _server_lock: server_lock,
        config,
    })
}

/// Layer 2: Materialize builtin skills + initialize the database.
///
/// Requires only `data_dir`. Subcommands that need persistent state
/// (database, skill files) should call this after `init_environment`.
pub async fn init_data_layer(config: &AppConfig) -> Result<Database> {
    let boot = Instant::now();

    materialize_builtin_skills(&config.data_dir).await?;
    info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: builtin skills materialized"
    );

    let db_path = config.database_path();
    info!("Initializing database at {}", db_path.display());
    let database = nomifun_db::init_database(&db_path).await?;
    info!(elapsed_ms = boot.elapsed().as_millis(), "startup: database initialized");

    // One-shot after a data-dir relocation (desktop temp → per-user app-data):
    // rewrite absolute paths stored in the database to the new root. Gated on
    // the `.relocated-from` marker; never fails the boot.
    super::relocation::rewrite_relocated_paths(&database, &config.data_dir).await;

    Ok(database)
}
