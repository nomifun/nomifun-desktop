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

    let canonical_work = std::fs::canonicalize(work_dir)
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
    let canonical_work = std::fs::canonicalize(work_dir)
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

    // Take data-dir authority before resolving any pending reset or legacy
    // work-root recovery hint from its control files.
    let server_lock = Arc::new(acquire_server_lock(&cli.data_dir)?);
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
    ConfirmedLegacy(String),
    InvalidOrDamagedV3(String),
}

#[derive(Debug, PartialEq, Eq)]
enum V3DataLayerState {
    FinalizedCurrent,
    BootstrapRequired,
}

const DATABASE_PROBE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Frozen SHA-384 checksums from the first published migration family.
///
/// Every v0.1.1-v0.2.19 release is an exact prefix of this sequence. Published
/// endpoint lengths are 17, 23, 24, 26, 28, 29, 31 and 33; accepting any exact
/// non-empty prefix also covers a process interrupted between two known
/// migrations without accepting an unknown checksum.
const PRE_SQUASH_LEGACY_MIGRATION_CHECKSUMS: &[&str] = &[
    "a0d4ac9cd452a4cc9fcd7c12740fdd645851ab9a8c0aa551018b886b5146c7e0b98bb48467e9918c7d19330f09f0f96a",
    "dfbbba862215d63a84c6ccc21a1ddf69401902cbb6ac3967ce7e791a85b3742e0589b4c34211807d8a45d7b54b77e427",
    "51bfcbd22c580d06ca06104766b2891a5a2be2562bc428dc6f084b6e1e04d5975ac44a1f86d65d52603417058d54272b",
    "d75f022b152992868e0b9162b5ff5bbb4ef261abb307eea5819ef7cfb8f4218bf84b7617bce655f618fed8edba19602e",
    "1518892a66e20a5d92a92446c801593e2a4f90f668c5126c840db2672f90ae2c239fe238d76762ee2fcd9d725e8b3a02",
    "40c75f0665aba0e6d12cc3d0ed248dce484abb65654f1e52816ba631655646f68bb0fa0e1224fa200c882374324a7e7f",
    "525b2dfd296536bbbfec39f145f08911302d664ba07df54d383ea8267d668b288b6f946a939289462d932526f8ec69ec",
    "7662b70402b4a9b9b77a58f27ab714925429ca8856d1934cab118eb30c0ecb8ead533727429c5e923af3a6955e933e5d",
    "8af9d4a0e0935d5288193073e35cd36b0cc540925aabbb6ad232f10c37d30476432c6966862ff946669e40bc21aeeefd",
    "50b12ee4ac4750aa1e49b3db6928f2255bde79770023039d261dbab3d395bd56af9d78d1bb8267ac9705c18242964102",
    "0f5d28e73091b5ce516d6f7a6ddb7666901da371352815254d294460995ef1a55918f529b7e4693e0624373c43b08500",
    "c6cfeed5150de17df409dbb98a08893a09d4ff3ffb3523f013b6fea1c597842c6906c0fb2afb58e1905b21b8fa95edfd",
    "a63bb98f3f853eb9940470324da027cf553c0a499c9e34b1ba67cd672f69f15c6eba39789d6a2b66ed94a835dd5400d7",
    "7008b57c9b3b6e0e7148612f0686d4c41c2fbf34b40f9c50e1b36664eec8a02c15cb19aae0dfb52936e5e4844910fafa",
    "55e5d0861e539e84478c5178d08c5667f915a25ca564549ca0e8326e8ef4a359e3b713ee7fd1e5e48fbd41bab46ef089",
    "e460b894315ff409235d1b1e5702ea11b83d1677ffdaedf03682a1949a99502d9554897ad7373db89c9f7e87569c1408",
    "27a3657fdf5c8d0543dac2bb79e48091ba7cef184a462e5a186e9c671800f46e7d175f8832e49c5eb9b3c2985d4c5468",
    "98ab96189fe554e0efc6b989c26c220c74976ba2578f8a21c31a33aaa0f4a82350320cb8e04a870de511e6f29ad850db",
    "c477b2ee228b71fac675e5c0adf7a672335d829a7acf3d4e01af56bc84625d6cfdaa91ea47e5952f4a6e76e1657a364c",
    "79355b743b24b258f4c7dc308af8acf1ff685fcc287c42ab27b1569122656634cbe0ed28cbf1d45844f43e841a25923c",
    "997565177a1a7cd44672e729ab90cefdf993e767f83d3b95d8b76d96398ab45eaa87e2474bd27a8acd9305d930ed4ad4",
    "1cbf24a5f5dc6a6331f8960c2ec606cf1a05bf214e18f6117f04fc667d2a5ddca1f64c9ab9d8eefb9888b220fd1f980b",
    "1be6491c2ab149708159169c85ce2bd5c51ab89cb7b66ef44d390c7fed17548d2c38339458b4a64a5af3d2c8c661ccbb",
    "a37b2f2e523d55ce71d4f825bd2643b16714b93aef0654fbe2753b93dbe386253ced0a1ada50032b54b5d2ce4cb55751",
    "eda7167e5798069276709eee4d3bd0760b4520faa502dc24db94507ffee9348ad4020447cf2ec199491f158ae4a43bc2",
    "29ff9d3d0c7657208789a9ea90a8fec2d8131d43b6504067a7c559d0600701d256f5432e019fae82180bd669b2ab7d7f",
    "dd76d7da932462fd7b152656348f6459f10516369d423605c2d9183ee5322a7c37490b39521f9a6c8da2713f2399f603",
    "0d7c80ddd4985342fd0b1fca8a5cd987ad91824f088d3d3a2e4ed0bf4eb208ce4eb0918d6578b135b7531ed67d60e231",
    "ba6435582f86db05e52f3a268334d744b9fcda0ff24bf31ae8fd1a44c9e985cb0c09fcb26a79d87218afc33c026dc187",
    "929616123a67bdf1a35fbb76d46c2e407947b3897bd67efff37c66e0d050f1414b6cc81f25c7094a2c2a2d64cf8b8daf",
    "5ada64bb4b00543f24861ad09b88d17d089266a8af3e0f86f182c2d193daab1fe125526d76e0734b208c133c852dbe03",
    "7ca43af181e5f795bba55d4cf091ae8b0c062154bc9e1053de09a97cd36aa636b24786b458247bcf54baa697febab4d6",
    "2d1c654cd1611bf73cdd1e8e0fd1653f8d8e2f191b6ae36b4ccc5095010367b727df9ba0186f8394c0900bafe8787f61",
];

/// Frozen SHA-384 checksums from the squashed v2 ID-contract family.
///
/// v0.2.20-v0.2.30 published each of the four non-empty prefixes.
const POST_SQUASH_LEGACY_MIGRATION_CHECKSUMS: &[&str] = &[
    "19b5f78e628df34a724e03b19698adc2a5396877d56e3bf6bd568f15cf7e0d479a46daf18ec5557050c11b3796b49ed8",
    "c27210d415b2f3cbd1e0835870617b19ad97715371353b51c311043492a0b748f573f2c3bdaa65ca67d9a101cd5d5773",
    "9adcd32066b4b2995db9aab871ef0cfee9b2766173b2cfb9e49c0e9dc542a8ad27cc485cec99e44dbd0c3b451aac19e4",
    "aed5a04a283f8febfb31ca8400ae588cbf85af04985f487fd9a559eb6e37ca1ced9dc0d134e697ca481becac14a577e6",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublishedLegacyLineage {
    PreSquash,
    PostSquash,
}

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

async fn table_has_column_named(
    pool: &SqlitePool,
    table: &str,
    column: &str,
) -> Result<bool> {
    let sql = format!("PRAGMA table_info(\"{table}\")");
    let rows = nomifun_db::sqlx::query(&sql).fetch_all(pool).await?;
    Ok(rows.iter().any(|row| {
        row.try_get::<String, _>("name")
            .is_ok_and(|name| name == column)
    }))
}

async fn table_exists(pool: &SqlitePool, table: &str) -> Result<bool> {
    let count: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = ?",
    )
    .bind(table)
    .fetch_one(pool)
    .await?;
    Ok(count == 1)
}

fn migration_rows_match_exact_prefix(
    rows: &[(i64, i64, String)],
    checksums: &[&str],
) -> bool {
    !rows.is_empty()
        && rows.len() <= checksums.len()
        && rows.iter().enumerate().all(
            |(index, (version, success, checksum))| {
                *version == index as i64 + 1
                    && *success == 1
                    && checksum == checksums[index]
            },
        )
}

async fn has_exact_published_legacy_lineage(
    pool: &SqlitePool,
) -> Result<Option<PublishedLegacyLineage>> {
    let rows: Vec<(i64, i64, String)> = nomifun_db::sqlx::query_as(
        "SELECT version, CAST(success AS INTEGER), lower(hex(checksum)) \
         FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(pool)
    .await?;
    if migration_rows_match_exact_prefix(
        &rows,
        PRE_SQUASH_LEGACY_MIGRATION_CHECKSUMS,
    ) {
        return Ok(Some(PublishedLegacyLineage::PreSquash));
    }
    if migration_rows_match_exact_prefix(
        &rows,
        POST_SQUASH_LEGACY_MIGRATION_CHECKSUMS,
    ) {
        return Ok(Some(PublishedLegacyLineage::PostSquash));
    }
    Ok(None)
}

async fn has_legacy_text_identity_columns(pool: &SqlitePool) -> Result<bool> {
    Ok(table_has_column_contract(
        pool, "users", "id", "TEXT", true, true,
    )
    .await?
        && !table_has_column_named(pool, "users", "user_id").await?
        && table_has_column_contract(
            pool,
            "agent_metadata",
            "id",
            "TEXT",
            true,
            true,
        )
        .await?
        && !table_has_column_named(pool, "agent_metadata", "agent_id").await?)
}

async fn has_exact_legacy_identity_schema(
    pool: &SqlitePool,
    lineage: PublishedLegacyLineage,
) -> Result<bool> {
    if !has_legacy_text_identity_columns(pool).await? {
        return Ok(false);
    }
    match lineage {
        // The original 001_baseline family never had an installation identity
        // table. Its absence is a strong negative discriminator from every v3
        // database, including a damaged v3 database with TEXT columns forged
        // onto the two entity tables.
        PublishedLegacyLineage::PreSquash => {
            Ok(!table_exists(pool, "installation_identity").await?)
        }
        // The squashed 001_id_contract_v2 family introduced a natural-key
        // singleton. v3 instead uses an INTEGER row id plus singleton_key.
        PublishedLegacyLineage::PostSquash => {
            Ok(table_has_column_contract(
                pool,
                "installation_identity",
                "key",
                "TEXT",
                true,
                true,
            )
            .await?
                && !table_has_column_named(
                    pool,
                    "installation_identity",
                    "singleton_key",
                )
                .await?)
        }
    }
}

async fn probe_v3_database_pool(pool: &SqlitePool) -> Result<ExistingV3DatabaseProbe> {
    let quick_check: Vec<String> =
        nomifun_db::sqlx::query_scalar("PRAGMA quick_check")
            .fetch_all(pool)
            .await?;
    if quick_check.as_slice() != ["ok"] {
        return Ok(ExistingV3DatabaseProbe::InvalidOrDamagedV3(
            format!("SQLite quick_check failed: {}", quick_check.join("; ")),
        ));
    }

    let has_migrations_table: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema \
         WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(pool)
    .await?;
    if has_migrations_table == 1
        && let Some(lineage) =
            has_exact_published_legacy_lineage(pool).await?
        && has_exact_legacy_identity_schema(pool, lineage).await?
    {
        let family = match lineage {
            PublishedLegacyLineage::PreSquash => "pre-squash",
            PublishedLegacyLineage::PostSquash => "post-squash",
        };
        return Ok(ExistingV3DatabaseProbe::ConfirmedLegacy(format!(
            "database matches an exact published pre-v3 {family} migration prefix and legacy-only identity schema"
        )));
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
        return Ok(ExistingV3DatabaseProbe::InvalidOrDamagedV3(
            "required v3 identity tables are missing".into(),
        ));
    }

    let migration_status = match nomifun_db::inspect_supported_migration_lineage(pool).await {
        Ok(status) => status,
        Err(error) => {
            return Ok(ExistingV3DatabaseProbe::InvalidOrDamagedV3(format!(
                "database migration lineage is not a supported embedded prefix: {error}"
            )));
        }
    };
    // The full ID registry describes the latest embedded schema. A valid older
    // migration prefix necessarily lacks later tables/columns, so defer the
    // complete contract until init_database applies the missing suffix. The
    // baseline identity checks below still authenticate the dataset before any
    // writable open.
    if migration_status == nomifun_db::MigrationLineageStatus::Current {
        if let Err(error) = nomifun_db::validate_id_schema_contract(pool).await {
            return Ok(ExistingV3DatabaseProbe::InvalidOrDamagedV3(format!(
                "database does not satisfy the complete v3 ID schema contract: {error}"
            )));
        }
        if let Err(error) = nomifun_db::validate_id_data_contract(pool).await {
            return Ok(ExistingV3DatabaseProbe::InvalidOrDamagedV3(format!(
                "database does not satisfy the complete v3 ID data contract: {error}"
            )));
        }
    }

    let schema_matches = table_has_column_contract(pool, "users", "id", "INTEGER", false, true)
        .await?
        && table_has_column_contract(pool, "users", "user_id", "TEXT", true, false).await?
        && table_has_column_contract(
            pool,
            "installation_identity",
            "id",
            "INTEGER",
            false,
            true,
        )
        .await?
        && table_has_column_contract(
            pool,
            "installation_identity",
            "singleton_key",
            "TEXT",
            true,
            false,
        )
        .await?
        && table_has_column_contract(
            pool,
            "installation_identity",
            "owner_user_id",
            "TEXT",
            true,
            false,
        )
        .await?
        && table_has_column_contract(
            pool,
            "agent_metadata",
            "id",
            "INTEGER",
            false,
            true,
        )
        .await?
        && table_has_column_contract(
            pool,
            "agent_metadata",
            "agent_id",
            "TEXT",
            true,
            false,
        )
        .await?;
    if !schema_matches {
        return Ok(ExistingV3DatabaseProbe::InvalidOrDamagedV3(
            "core database identity columns do not match the v3 schema".into(),
        ));
    }

    let identities: Vec<(String, String)> = nomifun_db::sqlx::query_as(
        "SELECT singleton_key, owner_user_id FROM installation_identity",
    )
    .fetch_all(pool)
    .await?;
    let [(singleton_key, owner_user_id)] = identities.as_slice() else {
        return Ok(ExistingV3DatabaseProbe::InvalidOrDamagedV3(format!(
            "expected one installation identity row, found {}",
            identities.len()
        )));
    };
    if singleton_key != "installation" {
        return Ok(ExistingV3DatabaseProbe::InvalidOrDamagedV3(
            "installation identity singleton key is invalid".into(),
        ));
    }
    if nomifun_common::UserId::parse(owner_user_id.clone()).is_err() {
        return Ok(ExistingV3DatabaseProbe::InvalidOrDamagedV3(
            "installation owner identity is not a canonical UUIDv7".into(),
        ));
    }
    let owner_rows: i64 =
        nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE user_id = ?")
            .bind(owner_user_id)
            .fetch_one(pool)
            .await?;
    if owner_rows != 1 {
        return Ok(ExistingV3DatabaseProbe::InvalidOrDamagedV3(
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

fn first_active_v3_lifecycle_artifact(
    data_dir: &Path,
    allow_released_legacy_receipt: bool,
) -> Result<Option<&'static str>> {
    const ARTIFACTS: &[&str] = &[
        nomifun_common::factory_reset::V3_DATASET_RECEIPT_FILE,
        nomifun_common::factory_reset::V3_DATASET_BOOTSTRAP_FILE,
        nomifun_common::factory_reset::V3_DATASET_RESET_REQUEST_FILE,
        nomifun_common::dataset_roots::WORK_ROOT_BINDING_FILE,
    ];
    for artifact in ARTIFACTS {
        if allow_released_legacy_receipt
            && *artifact
                == nomifun_common::factory_reset::V3_DATASET_RECEIPT_FILE
        {
            continue;
        }
        match std::fs::symlink_metadata(data_dir.join(artifact)) {
            Ok(_) => return Ok(Some(artifact)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("inspect active v3 lifecycle artifact {artifact}")
                });
            }
        }
    }

    // Arming creates the reset-control directory before atomically publishing
    // plan.json. A crash in that gap leaves only an orphan directory (and
    // possibly uniquely named temporary files), which carries no destructive
    // authority and must not permanently block a strictly proven legacy reset.
    // Conversely, any plan path is active evidence: a malformed, symlinked or
    // non-regular plan still fails closed when the normal reset reader handles
    // it.
    let reset_dir = data_dir.join(
        nomifun_common::factory_reset::V3_DATASET_RESET_DIR,
    );
    match std::fs::symlink_metadata(&reset_dir) {
        Ok(metadata)
            if lifecycle_metadata_is_link_or_reparse(&metadata)
                || !metadata.is_dir() =>
        {
            return Ok(Some(
                nomifun_common::factory_reset::V3_DATASET_RESET_DIR,
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "inspect active v3 reset control directory {}",
                    reset_dir.display()
                )
            });
        }
    }
    let plan = reset_dir.join(
        nomifun_common::factory_reset::V3_DATASET_RESET_PLAN_FILE,
    );
    match std::fs::symlink_metadata(&plan) {
        Ok(metadata)
            if lifecycle_metadata_is_link_or_reparse(&metadata)
                || !metadata.is_file() =>
        {
            anyhow::bail!(
                "active v3 reset plan is not a regular no-follow file: {}",
                plan.display()
            );
        }
        Ok(_) => Ok(Some(
            nomifun_common::factory_reset::V3_DATASET_RESET_PLAN_FILE,
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| {
            format!("inspect active v3 reset plan {}", plan.display())
        }),
    }
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
    let released_legacy_receipt =
        nomifun_common::factory_reset::legacy_v3_receipt_can_be_retired_after_database_probe(
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
        && !released_legacy_receipt
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
    // database. Receipt-valid databases still have to prove a checksum-exact
    // prefix of the embedded migration lineage plus the baseline installation
    // identity contract. Fully migrated databases additionally prove the
    // complete schema/data contract here; supported prefixes prove it after
    // init_database applies the missing suffix.
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
        ExistingV3DatabaseProbe::ConfirmedLegacy(reason) => {
            if let Some(artifact) =
                first_active_v3_lifecycle_artifact(
                    &config.data_dir,
                    released_legacy_receipt,
                )?
            {
                anyhow::bail!(
                    "database matches a published legacy lineage but active v3 \
                     lifecycle artifact {artifact} is also present; preserving \
                     the ambiguous dataset and refusing automatic reset"
                );
            }
            warn!(
                target: "boot",
                database = %config.database_path().display(),
                reason,
                "strictly confirmed legacy database; retiring it once without migration"
            );
            nomifun_common::factory_reset::retire_non_v3_dataset_after_probe(
                &config.data_dir,
                &config.work_dir,
            )?;
            V3DataLayerState::BootstrapRequired
        }
        ExistingV3DatabaseProbe::InvalidOrDamagedV3(reason) => {
            anyhow::bail!(
                "the existing database is neither a proven current v3 dataset \
                 nor an exact published legacy dataset; preserving it without \
                 mutation: {reason}"
            );
        }
    };
    Ok(state)
}

fn install_storage_generation_environment(config: &AppConfig) -> Result<()> {
    // Generate this only after every reset decision has removed the old
    // dataset marker and the caller has committed to bootstrapping/opening the
    // data layer. Browser-local state is outside SQLite, so the value scopes
    // every entity cache key to exactly this post-reset generation.
    let storage_generation = load_or_create_storage_generation(&config.data_dir)?;
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
    }
    // SAFETY: initialization is still single-threaded and happens before any
    // service or route can read this variable.
    unsafe {
        std::env::set_var("NOMIFUN_STORAGE_GENERATION", &storage_generation);
    }
    if receipt_status
        != nomifun_common::factory_reset::DatasetReceiptStatus::Current
    {
        nomifun_common::factory_reset::write_v3_dataset_bootstrap_binding(
            &config.data_dir,
            &config.work_dir,
            &storage_generation,
        )?;
    }
    Ok(())
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

    Ok(database)
}

/// Commit the filesystem-level v3 dataset only after every required
/// product-owned side store has initialized successfully.
///
/// Keeping this separate from [`init_data_layer`] is deliberate: the main
/// SQLite schema alone is not proof that companion/public-agent/workshop and
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
    use sha2::{Digest, Sha384};

    const V3_BASELINE_SQL: &str =
        include_str!("../../../nomifun-db/migrations/001_v3_baseline.sql");

    fn v3_baseline_checksum() -> Vec<u8> {
        Sha384::digest(V3_BASELINE_SQL.as_bytes()).to_vec()
    }

    fn checksum_bytes(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).unwrap();
                u8::from_str_radix(pair, 16).unwrap()
            })
            .collect()
    }

    async fn create_migrations_table(pool: &SqlitePool) {
        nomifun_db::sqlx::query(
            "CREATE TABLE _sqlx_migrations (\
                version BIGINT PRIMARY KEY, \
                description TEXT NOT NULL, \
                installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, \
                success BOOLEAN NOT NULL, \
                checksum BLOB NOT NULL, \
                execution_time BIGINT NOT NULL\
             )",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn create_legacy_database(
        path: &Path,
        lineage: PublishedLegacyLineage,
        migration_count: usize,
    ) {
        let checksums = match lineage {
            PublishedLegacyLineage::PreSquash => {
                PRE_SQUASH_LEGACY_MIGRATION_CHECKSUMS
            }
            PublishedLegacyLineage::PostSquash => {
                POST_SQUASH_LEGACY_MIGRATION_CHECKSUMS
            }
        };
        assert!(
            (1..=checksums.len()).contains(&migration_count),
            "legacy fixture must use a non-empty known prefix"
        );
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);
        let pool = PoolOptions::<Sqlite>::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        create_migrations_table(&pool).await;
        for (index, checksum) in
            checksums.iter().take(migration_count).enumerate()
        {
            nomifun_db::sqlx::query(
                "INSERT INTO _sqlx_migrations \
                     (version, description, success, checksum, execution_time) \
                 VALUES (?, 'published legacy migration', 1, ?, 0)",
            )
            .bind(index as i64 + 1)
            .bind(checksum_bytes(checksum))
            .execute(&pool)
            .await
            .unwrap();
        }
        match lineage {
            PublishedLegacyLineage::PreSquash => {
                nomifun_db::sqlx::raw_sql(
                    "CREATE TABLE users (id TEXT PRIMARY KEY NOT NULL);\
                     CREATE TABLE agent_metadata (id TEXT PRIMARY KEY NOT NULL);",
                )
                .execute(&pool)
                .await
                .unwrap();
            }
            PublishedLegacyLineage::PostSquash => {
                nomifun_db::sqlx::raw_sql(
                    "CREATE TABLE users (id TEXT PRIMARY KEY NOT NULL);\
                     CREATE TABLE installation_identity (\
                         \"key\" TEXT PRIMARY KEY NOT NULL\
                     );\
                     CREATE TABLE agent_metadata (id TEXT PRIMARY KEY NOT NULL);",
                )
                .execute(&pool)
                .await
                .unwrap();
            }
        }
        pool.close().await;
    }

    async fn create_exact_legacy_database(path: &Path) {
        create_legacy_database(
            path,
            PublishedLegacyLineage::PostSquash,
            POST_SQUASH_LEGACY_MIGRATION_CHECKSUMS.len(),
        )
        .await;
    }

    fn test_config(data_dir: &Path, work_dir: &Path) -> AppConfig {
        AppConfig {
            data_dir: data_dir.to_path_buf(),
            work_dir: work_dir.to_path_buf(),
            ..AppConfig::default()
        }
    }

    fn write_released_v3_receipt_without_binding_flag(
        data_dir: &Path,
        work_dir: &Path,
        generation: &str,
    ) {
        let receipt = serde_json::json!({
            "contract_version": nomifun_common::factory_reset::V3_DATASET_CONTRACT_VERSION,
            "generation": generation,
            "work_root": std::fs::canonicalize(work_dir)
                .unwrap()
                .display()
                .to_string(),
            "installed_at": nomifun_common::now_ms(),
        });
        std::fs::write(
            data_dir.join(
                nomifun_common::factory_reset::V3_DATASET_RECEIPT_FILE,
            ),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn probe_accepts_database_created_from_all_embedded_migrations() {
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
    async fn probe_accepts_supported_migration_prefix_for_incremental_upgrade() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nomifun-backend.db");
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let pool = PoolOptions::<Sqlite>::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        create_migrations_table(&pool).await;
        nomifun_db::sqlx::raw_sql(V3_BASELINE_SQL)
            .execute(&pool)
            .await
            .unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO _sqlx_migrations \
                 (version, description, success, checksum, execution_time) \
             VALUES (1, 'v3 baseline', 1, ?, 0)",
        )
        .bind(v3_baseline_checksum())
        .execute(&pool)
        .await
        .unwrap();
        let owner = nomifun_common::UserId::new();
        nomifun_db::sqlx::query(
            "INSERT INTO users \
                 (user_id, username, password_hash, created_at, updated_at) \
             VALUES (?, 'admin', '', 1, 1)",
        )
        .bind(owner.as_str())
        .execute(&pool)
        .await
        .unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO installation_identity (singleton_key, owner_user_id) \
             VALUES ('installation', ?)",
        )
        .bind(owner.as_str())
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;

        assert_eq!(
            probe_existing_v3_database(&path).await.unwrap(),
            ExistingV3DatabaseProbe::Current
        );

        let upgraded = nomifun_db::init_database(&path).await.unwrap();
        assert_eq!(
            nomifun_db::inspect_supported_migration_lineage(upgraded.pool())
                .await
                .unwrap(),
            nomifun_db::MigrationLineageStatus::Current
        );
        let applied: i64 =
            nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
                .fetch_one(upgraded.pool())
                .await
                .unwrap();
        assert!(applied > 1, "embedded migration suffix must be applied");
        upgraded.close().await;
    }

    #[tokio::test]
    async fn probe_confirms_every_published_legacy_lineage_prefix_and_schema() {
        // Derived from the migration manifests in the signed release tags:
        // v0.1.1-v0.2.19 (pre-squash) and v0.2.20-v0.2.30
        // (post-squash). Tags sharing an endpoint are intentionally grouped.
        const PRE_SQUASH_PUBLISHED_PREFIXES: &[usize] =
            &[17, 23, 24, 26, 28, 29, 31, 33];
        const POST_SQUASH_PUBLISHED_PREFIXES: &[usize] = &[1, 2, 3, 4];

        for (lineage, prefixes, family) in [
            (
                PublishedLegacyLineage::PreSquash,
                PRE_SQUASH_PUBLISHED_PREFIXES,
                "pre-squash",
            ),
            (
                PublishedLegacyLineage::PostSquash,
                POST_SQUASH_PUBLISHED_PREFIXES,
                "post-squash",
            ),
        ] {
            for &migration_count in prefixes {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("nomifun-backend.db");
                create_legacy_database(
                    &path,
                    lineage,
                    migration_count,
                )
                .await;

                assert!(matches!(
                    probe_existing_v3_database(&path).await.unwrap(),
                    ExistingV3DatabaseProbe::ConfirmedLegacy(reason)
                        if reason.contains("published pre-v3")
                            && reason.contains(family)
                ));
            }
        }
    }

    #[tokio::test]
    async fn legacy_checksum_prefix_with_the_wrong_schema_family_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nomifun-backend.db");
        create_legacy_database(
            &path,
            PublishedLegacyLineage::PreSquash,
            PRE_SQUASH_LEGACY_MIGRATION_CHECKSUMS.len(),
        )
        .await;
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(false);
        let pool = PoolOptions::<Sqlite>::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        nomifun_db::sqlx::query(
            "CREATE TABLE installation_identity (\
                 \"key\" TEXT PRIMARY KEY NOT NULL\
             )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;

        assert!(matches!(
            probe_existing_v3_database(&path).await.unwrap(),
            ExistingV3DatabaseProbe::InvalidOrDamagedV3(_)
        ));
    }

    #[tokio::test]
    async fn probe_rejects_unknown_future_migration_lineage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nomifun-backend.db");
        let database = nomifun_db::init_database(&path).await.unwrap();
        let latest: i64 =
            nomifun_db::sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
                .fetch_one(database.pool())
                .await
                .unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO _sqlx_migrations \
                 (version, description, success, checksum, execution_time) \
             VALUES (?, 'unknown future migration', 1, X'00', 0)",
        )
        .bind(latest + 1)
        .execute(database.pool())
        .await
        .unwrap();
        database.close().await;

        assert!(matches!(
            probe_existing_v3_database(&path).await.unwrap(),
            ExistingV3DatabaseProbe::InvalidOrDamagedV3(reason)
                if reason.contains("migration lineage")
        ));
    }

    #[tokio::test]
    async fn finalized_current_database_is_ready_for_doctor() {
        let data = tempfile::tempdir().unwrap();
        let path = data.path().join("nomifun-backend.db");
        let database = nomifun_db::init_database(&path).await.unwrap();
        database.close().await;
        let generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(data.path().join("storage-generation"), &generation).unwrap();
        nomifun_common::factory_reset::write_v3_dataset_receipt(data.path(), &generation)
            .unwrap();

        assert_eq!(
            prepare_v3_data_layer(&test_config(data.path(), data.path()))
                .await
                .unwrap(),
            V3DataLayerState::FinalizedCurrent
        );
        assert!(path.is_file());
        assert!(
            !data
                .path()
                .join(nomifun_common::factory_reset::V3_DATASET_RESET_DIR)
                .exists()
        );
    }

    #[tokio::test]
    async fn probe_rejects_forged_v3_lineage_when_core_schema_is_legacy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nomifun-backend.db");
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let pool = PoolOptions::<Sqlite>::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        create_migrations_table(&pool).await;
        nomifun_db::sqlx::query(
            "INSERT INTO _sqlx_migrations \
                 (version, description, success, checksum, execution_time) \
             VALUES (1, 'v3 baseline', 1, ?, 0)",
        )
        .bind(v3_baseline_checksum())
        .execute(&pool)
        .await
        .unwrap();
        nomifun_db::sqlx::query(
            "CREATE TABLE users (id TEXT PRIMARY KEY, user_id TEXT NOT NULL UNIQUE)",
        )
        .execute(&pool)
        .await
        .unwrap();
        nomifun_db::sqlx::query(
            "CREATE TABLE installation_identity (\
                id TEXT PRIMARY KEY, \
                singleton_key TEXT NOT NULL, \
                owner_user_id TEXT NOT NULL\
             )",
        )
        .execute(&pool)
        .await
        .unwrap();
        nomifun_db::sqlx::query(
            "CREATE TABLE agent_metadata (id TEXT PRIMARY KEY, agent_id TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;

        assert!(matches!(
            probe_existing_v3_database(&path).await.unwrap(),
            ExistingV3DatabaseProbe::InvalidOrDamagedV3(reason)
                if reason.contains("core database identity columns")
        ));
    }

    #[tokio::test]
    async fn probe_rejects_v3_database_with_tampered_baseline_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nomifun-backend.db");
        let database = nomifun_db::init_database(&path).await.unwrap();
        nomifun_db::sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00'")
            .execute(database.pool())
            .await
            .unwrap();
        database.close().await;

        assert!(matches!(
            probe_existing_v3_database(&path).await.unwrap(),
            ExistingV3DatabaseProbe::InvalidOrDamagedV3(reason)
                if reason.contains("migration lineage")
        ));
    }

    #[tokio::test]
    async fn probe_rejects_current_lineage_with_invalid_managed_origin_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nomifun-backend.db");
        let database = nomifun_db::init_database(&path).await.unwrap();
        let mut connection = database.pool().acquire().await.unwrap();
        nomifun_db::sqlx::query("PRAGMA ignore_check_constraints = ON")
            .execute(&mut *connection)
            .await
            .unwrap();
        let asset_id = nomifun_common::WorkshopAssetId::new();
        nomifun_db::sqlx::query(
            "INSERT INTO workshop_assets \
                (asset_id, kind, title, tags, in_library, origin, created_at, updated_at) \
             VALUES (?, 'image', 'corrupt origin', '[]', 1, \
                     '{\"canvas_id\":null}', 1, 1)",
        )
        .bind(asset_id.as_str())
        .execute(&mut *connection)
        .await
        .unwrap();
        nomifun_db::sqlx::query("PRAGMA ignore_check_constraints = OFF")
            .execute(&mut *connection)
            .await
            .unwrap();
        drop(connection);
        database.close().await;

        assert!(matches!(
            probe_existing_v3_database(&path).await.unwrap(),
            ExistingV3DatabaseProbe::InvalidOrDamagedV3(reason)
                if reason.contains("complete v3 ID data contract")
                    && reason.contains("origin.canvas_id")
        ));
    }

    #[tokio::test]
    async fn explicit_reset_overrides_current_receipt_and_retires_managed_side_store() {
        let data = tempfile::tempdir().unwrap();
        let config = test_config(data.path(), data.path());
        let database = nomifun_db::init_database(&config.database_path()).await.unwrap();
        database.close().await;
        let generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(data.path().join("storage-generation"), &generation).unwrap();
        nomifun_common::factory_reset::write_v3_dataset_receipt(data.path(), &generation)
            .unwrap();
        std::fs::create_dir_all(data.path().join("knowledge")).unwrap();
        std::fs::write(
            data.path().join("knowledge/stale-index"),
            b"pre-reset side store",
        )
        .unwrap();
        nomifun_common::factory_reset::request_v3_dataset_reset(
            data.path(),
            data.path(),
        )
        .unwrap();

        assert_eq!(
            prepare_v3_data_layer(&config).await.unwrap(),
            V3DataLayerState::BootstrapRequired
        );
        let plan =
            nomifun_common::factory_reset::read_pending_v3_reset(data.path(), data.path())
                .unwrap()
                .expect("explicit reset must stay pending until all side stores initialize");
        assert_eq!(
            plan.reason,
            nomifun_common::factory_reset::DatasetResetReason::ExplicitFactoryReset
        );
        assert!(!config.database_path().exists());
        assert!(!data.path().join("knowledge").exists());
        assert!(
            data.path()
                .join(plan.retired_dir)
                .join("knowledge/stale-index")
                .is_file()
        );
        assert!(
            !data
                .path()
                .join(nomifun_common::factory_reset::V3_DATASET_RECEIPT_FILE)
                .exists(),
            "the stale pre-reset receipt must be quarantined"
        );
    }

    #[tokio::test]
    async fn finalize_publishes_receipt_only_after_side_store_bootstrap_succeeds() {
        let data = tempfile::tempdir().unwrap();
        let config = test_config(data.path(), data.path());
        std::fs::write(config.database_path(), b"old database").unwrap();
        std::fs::create_dir_all(data.path().join("companion")).unwrap();
        std::fs::write(data.path().join("companion/old-state"), b"old").unwrap();
        nomifun_common::factory_reset::request_v3_dataset_reset(
            data.path(),
            data.path(),
        )
        .unwrap();
        assert_eq!(
            prepare_v3_data_layer(&config).await.unwrap(),
            V3DataLayerState::BootstrapRequired
        );
        install_storage_generation_environment(&config).unwrap();
        let database = nomifun_db::init_database(&config.database_path()).await.unwrap();
        database.close().await;

        let side_store = data.path().join("companion/current-state");
        std::fs::create_dir_all(side_store.parent().unwrap()).unwrap();
        std::fs::write(&side_store, b"current").unwrap();
        assert!(
            !data
                .path()
                .join(nomifun_common::factory_reset::V3_DATASET_RECEIPT_FILE)
                .exists(),
            "database and side-store bootstrap alone must not publish the final receipt"
        );
        assert!(
            data
                .path()
                .join(nomifun_common::factory_reset::V3_DATASET_RESET_DIR)
                .is_dir()
        );

        finalize_data_layer(&config).unwrap();

        assert!(side_store.is_file());
        assert!(
            !data
                .path()
                .join(nomifun_common::factory_reset::V3_DATASET_RESET_DIR)
                .exists()
        );
        nomifun_common::factory_reset::require_current_v3_dataset_for_work_dir(
            data.path(),
            data.path(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn ambiguous_database_with_v3_receipt_is_preserved() {
        let data = tempfile::tempdir().unwrap();
        let path = data.path().join("nomifun-backend.db");
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let pool = PoolOptions::<Sqlite>::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        nomifun_db::sqlx::query("CREATE TABLE legacy_sentinel (value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO legacy_sentinel (value) VALUES ('must-not-migrate')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;

        let generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(data.path().join("storage-generation"), &generation).unwrap();
        nomifun_common::factory_reset::write_v3_dataset_receipt(data.path(), &generation)
            .unwrap();

        let error =
            prepare_v3_data_layer(&test_config(data.path(), data.path()))
                .await
                .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("preserving it without mutation")
        );
        assert!(path.is_file());
        assert!(
            !data
                .path()
                .join(nomifun_common::factory_reset::V3_DATASET_RESET_DIR)
                .exists()
        );

        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(false)
            .read_only(true);
        let preserved = PoolOptions::<Sqlite>::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        let sentinel: String =
            nomifun_db::sqlx::query_scalar("SELECT value FROM legacy_sentinel")
                .fetch_one(&preserved)
                .await
                .unwrap();
        assert_eq!(sentinel, "must-not-migrate");
        preserved.close().await;
    }

    #[tokio::test]
    async fn exact_legacy_database_is_retired_once_without_migration() {
        let data = tempfile::tempdir().unwrap();
        let config = test_config(data.path(), data.path());
        create_exact_legacy_database(&config.database_path()).await;
        // storage-generation predates the v3 contract, so it is legacy data
        // to quarantine rather than independent evidence of a v3 dataset.
        let legacy_generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(
            data.path().join("storage-generation"),
            &legacy_generation,
        )
        .unwrap();

        assert_eq!(
            prepare_v3_data_layer(&config).await.unwrap(),
            V3DataLayerState::BootstrapRequired
        );
        assert!(!config.database_path().exists());
        let plan =
            nomifun_common::factory_reset::read_pending_v3_reset(
                data.path(),
                data.path(),
            )
            .unwrap()
            .expect("confirmed legacy reset remains pending until bootstrap");
        assert_eq!(
            plan.reason,
            nomifun_common::factory_reset::DatasetResetReason::NonV3Dataset
        );
        assert!(
            data.path()
                .join(&plan.retired_dir)
                .join("nomifun-backend.db")
                .is_file()
        );

        install_storage_generation_environment(&config).unwrap();
        let database =
            nomifun_db::init_database(&config.database_path()).await.unwrap();
        database.close().await;
        finalize_data_layer(&config).unwrap();
        let retired_count = std::fs::read_dir(
            data.path()
                .join(nomifun_common::factory_reset::RETIRED_DATASETS_DIR),
        )
        .unwrap()
        .count();

        assert_eq!(
            prepare_v3_data_layer(&config).await.unwrap(),
            V3DataLayerState::FinalizedCurrent
        );
        assert_eq!(
            std::fs::read_dir(
                data.path()
                    .join(nomifun_common::factory_reset::RETIRED_DATASETS_DIR),
            )
            .unwrap()
            .count(),
            retired_count,
            "a finalized v3 generation must not be reset a second time"
        );
    }

    #[tokio::test]
    async fn exact_legacy_rollback_cannot_consume_automatic_retirement_twice() {
        let data = tempfile::tempdir().unwrap();
        let config = test_config(data.path(), data.path());
        create_exact_legacy_database(&config.database_path()).await;
        assert_eq!(
            prepare_v3_data_layer(&config).await.unwrap(),
            V3DataLayerState::BootstrapRequired
        );
        install_storage_generation_environment(&config).unwrap();
        let database =
            nomifun_db::init_database(&config.database_path()).await.unwrap();
        database.close().await;
        finalize_data_layer(&config).unwrap();

        for relative in [
            "nomifun-backend.db",
            "storage-generation",
            nomifun_common::factory_reset::V3_DATASET_RECEIPT_FILE,
            nomifun_common::dataset_roots::WORK_ROOT_OWNER_FILE,
            nomifun_common::dataset_roots::WORK_ROOT_BINDING_FILE,
        ] {
            match std::fs::remove_file(data.path().join(relative)) {
                Ok(()) => {}
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!(
                    "remove simulated rolled-back lifecycle {relative}: {error}"
                ),
            }
        }
        create_exact_legacy_database(&config.database_path()).await;

        let error = prepare_v3_data_layer(&config).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("already consumed its one automatic")
        );
        assert!(
            config.database_path().is_file(),
            "the second exact legacy dataset must be preserved for an explicit user decision"
        );
        assert!(
            !data
                .path()
                .join(
                    nomifun_common::factory_reset::V3_DATASET_RESET_DIR,
                )
                .exists()
        );
    }

    #[tokio::test]
    async fn exact_legacy_nested_work_root_is_retired_once_without_reloading_history() {
        let data = tempfile::tempdir().unwrap();
        let work = data.path().join("chosen-workspace");
        let conversations = work.join("conversations");
        std::fs::create_dir_all(&conversations).unwrap();
        let legacy_sentinel = conversations.join("legacy-sentinel");
        std::fs::write(&legacy_sentinel, b"must-not-migrate").unwrap();
        nomifun_common::dir_config::set_work_dir(data.path(), &work).unwrap();
        let config = test_config(data.path(), &work);
        create_exact_legacy_database(&config.database_path()).await;

        assert_eq!(
            prepare_v3_data_layer(&config).await.unwrap(),
            V3DataLayerState::BootstrapRequired
        );
        assert!(!config.database_path().exists());
        assert!(!legacy_sentinel.exists());
        let plan =
            nomifun_common::factory_reset::read_pending_v3_reset(
                data.path(),
                &work,
            )
            .unwrap()
            .expect("nested legacy reset remains pending until v3 bootstrap");
        let retired_sentinel = work
            .join(&plan.work_retired_dir)
            .join("conversations/legacy-sentinel");
        assert_eq!(
            std::fs::read(&retired_sentinel).unwrap(),
            b"must-not-migrate"
        );

        install_storage_generation_environment(&config).unwrap();
        let database =
            nomifun_db::init_database(&config.database_path()).await.unwrap();
        database.close().await;
        finalize_data_layer(&config).unwrap();
        let data_retired_count = std::fs::read_dir(
            data.path()
                .join(nomifun_common::factory_reset::RETIRED_DATASETS_DIR),
        )
        .unwrap()
        .count();
        let work_retired_count =
            std::fs::read_dir(work.join(".nomifun-retired-datasets"))
                .unwrap()
                .count();

        assert_eq!(
            nomifun_common::factory_reset::prepare_v3_dataset(
                data.path(),
                &work,
            )
            .unwrap(),
            nomifun_common::factory_reset::DatasetPreparation::Unchanged
        );
        assert_eq!(
            prepare_v3_data_layer(&config).await.unwrap(),
            V3DataLayerState::FinalizedCurrent
        );
        assert_eq!(
            std::fs::read_dir(
                data.path()
                    .join(nomifun_common::factory_reset::RETIRED_DATASETS_DIR),
            )
            .unwrap()
            .count(),
            data_retired_count
        );
        assert_eq!(
            std::fs::read_dir(work.join(".nomifun-retired-datasets"))
                .unwrap()
                .count(),
            work_retired_count,
            "the nested historical workspace must not be retired twice"
        );
        assert!(retired_sentinel.is_file());
    }

    #[tokio::test]
    async fn orphan_reset_directory_without_plan_is_not_v3_lineage_evidence() {
        let data = tempfile::tempdir().unwrap();
        let database = data.path().join("nomifun-backend.db");
        create_exact_legacy_database(&database).await;
        let reset_dir = data
            .path()
            .join(nomifun_common::factory_reset::V3_DATASET_RESET_DIR);
        std::fs::create_dir(&reset_dir).unwrap();
        std::fs::write(
            reset_dir.join(".plan.json.tmp-interrupted"),
            b"incomplete",
        )
        .unwrap();

        assert_eq!(
            first_active_v3_lifecycle_artifact(data.path(), false).unwrap(),
            None
        );
        assert!(matches!(
            probe_existing_v3_database(&database).await.unwrap(),
            ExistingV3DatabaseProbe::ConfirmedLegacy(_)
        ));
        assert!(
            database.is_file(),
            "classification alone must not mutate the legacy database"
        );
    }

    #[tokio::test]
    async fn exact_legacy_database_with_v3_lifecycle_evidence_is_preserved() {
        let data = tempfile::tempdir().unwrap();
        let config = test_config(data.path(), data.path());
        create_exact_legacy_database(&config.database_path()).await;
        let generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(data.path().join("storage-generation"), &generation)
            .unwrap();
        nomifun_common::factory_reset::write_v3_dataset_receipt(
            data.path(),
            &generation,
        )
        .unwrap();

        let error = prepare_v3_data_layer(&config).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("active v3 lifecycle artifact")
        );
        assert!(config.database_path().is_file());
        assert!(
            !data
                .path()
                .join(nomifun_common::factory_reset::V3_DATASET_RESET_DIR)
                .exists()
        );
    }

    #[tokio::test]
    async fn data_side_binding_closes_released_receipt_compatibility_window() {
        let data = tempfile::tempdir().unwrap();
        let config = test_config(data.path(), data.path());
        create_exact_legacy_database(&config.database_path()).await;
        let generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(
            data.path().join("storage-generation"),
            &generation,
        )
        .unwrap();
        nomifun_common::factory_reset::write_v3_dataset_bootstrap_binding(
            data.path(),
            data.path(),
            &generation,
        )
        .unwrap();
        std::fs::remove_file(
            data.path().join(
                nomifun_common::factory_reset::V3_DATASET_BOOTSTRAP_FILE,
            ),
        )
        .unwrap();
        write_released_v3_receipt_without_binding_flag(
            data.path(),
            data.path(),
            &generation,
        );

        let error = prepare_v3_data_layer(&config).await.unwrap_err();
        assert!(error.to_string().contains("active v3 lifecycle artifact"));
        assert!(config.database_path().is_file());
        assert!(
            !data
                .path()
                .join(nomifun_common::factory_reset::V3_DATASET_RESET_DIR)
                .exists()
        );
    }

    #[tokio::test]
    async fn released_legacy_receipt_mismatch_resets_once_then_protects_new_v3() {
        let data = tempfile::tempdir().unwrap();
        let old_work = tempfile::tempdir().unwrap();
        let new_work = tempfile::tempdir().unwrap();
        let another_work = tempfile::tempdir().unwrap();
        let config = test_config(data.path(), new_work.path());
        create_exact_legacy_database(&config.database_path()).await;
        let old_generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(
            data.path().join("storage-generation"),
            &old_generation,
        )
        .unwrap();
        write_released_v3_receipt_without_binding_flag(
            data.path(),
            old_work.path(),
            &old_generation,
        );
        std::fs::create_dir_all(
            old_work.path().join("conversations"),
        )
        .unwrap();
        let inactive_old_workspace =
            old_work.path().join("conversations/history");
        std::fs::write(&inactive_old_workspace, b"old").unwrap();

        assert_eq!(
            prepare_v3_data_layer(&config).await.unwrap(),
            V3DataLayerState::BootstrapRequired
        );
        let plan =
            nomifun_common::factory_reset::read_pending_v3_reset(
                data.path(),
                new_work.path(),
            )
            .unwrap()
            .expect("strictly proven legacy dataset should have a reset plan");
        assert_eq!(
            plan.reason,
            nomifun_common::factory_reset::DatasetResetReason::WorkDirChange
        );
        assert!(
            inactive_old_workspace.is_file(),
            "the unlocked root named only by the old receipt must remain untouched and inactive"
        );
        assert!(
            !new_work.path().join("conversations").exists(),
            "a fresh target must not inherit old conversations"
        );
        assert_eq!(
            nomifun_common::dir_config::checked_persisted_work_dir(
                data.path(),
            )
            .unwrap(),
            Some(std::fs::canonicalize(new_work.path()).unwrap())
        );

        install_storage_generation_environment(&config).unwrap();
        let database =
            nomifun_db::init_database(&config.database_path()).await.unwrap();
        database.close().await;
        finalize_data_layer(&config).unwrap();
        let new_generation =
            std::fs::read(data.path().join("storage-generation")).unwrap();
        assert_ne!(new_generation, old_generation.as_bytes());

        let new_workspace = new_work.path().join("conversations");
        std::fs::create_dir_all(&new_workspace).unwrap();
        let new_v3_sentinel = new_workspace.join("new-v3-data");
        std::fs::write(&new_v3_sentinel, b"new").unwrap();
        let receipt: serde_json::Value = serde_json::from_slice(
            &std::fs::read(
                data.path().join(
                    nomifun_common::factory_reset::V3_DATASET_RECEIPT_FILE,
                ),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            receipt
                .get("work_root_binding_required")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        assert_eq!(
            prepare_v3_data_layer(&config).await.unwrap(),
            V3DataLayerState::FinalizedCurrent
        );
        assert!(new_v3_sentinel.is_file());
        assert_eq!(
            std::fs::read(data.path().join("storage-generation")).unwrap(),
            new_generation
        );

        let mismatched_new_config =
            test_config(data.path(), another_work.path());
        let error = prepare_v3_data_layer(&mismatched_new_config)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("different resolved work root"));
        assert!(
            new_v3_sentinel.is_file(),
            "a current receipt must never reopen automatic retirement"
        );
        assert!(config.database_path().is_file());
    }

    #[tokio::test]
    async fn released_external_receipt_can_rebind_to_locked_data_root() {
        let data = tempfile::tempdir().unwrap();
        let old_work = tempfile::tempdir().unwrap();
        let config = test_config(data.path(), data.path());
        create_exact_legacy_database(&config.database_path()).await;
        let old_generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(
            data.path().join("storage-generation"),
            &old_generation,
        )
        .unwrap();
        write_released_v3_receipt_without_binding_flag(
            data.path(),
            old_work.path(),
            &old_generation,
        );
        std::fs::create_dir_all(
            old_work.path().join("conversations"),
        )
        .unwrap();
        let old_external_sentinel =
            old_work.path().join("conversations/history");
        std::fs::write(&old_external_sentinel, b"old").unwrap();

        assert_eq!(
            prepare_v3_data_layer(&config).await.unwrap(),
            V3DataLayerState::BootstrapRequired
        );
        let plan =
            nomifun_common::factory_reset::read_pending_v3_reset(
                data.path(),
                data.path(),
            )
            .unwrap()
            .expect("legacy fallback-to-data reset plan");
        assert_eq!(
            plan.reason,
            nomifun_common::factory_reset::DatasetResetReason::NonV3Dataset
        );
        assert!(old_external_sentinel.is_file());
        assert_eq!(
            nomifun_common::dir_config::checked_persisted_work_dir(
                data.path(),
            )
            .unwrap(),
            Some(std::fs::canonicalize(data.path()).unwrap())
        );

        install_storage_generation_environment(&config).unwrap();
        let database =
            nomifun_db::init_database(&config.database_path()).await.unwrap();
        database.close().await;
        finalize_data_layer(&config).unwrap();
        assert_eq!(
            prepare_v3_data_layer(&config).await.unwrap(),
            V3DataLayerState::FinalizedCurrent
        );
    }

    #[tokio::test]
    async fn released_external_receipt_never_claims_nonempty_data_root_target() {
        let data = tempfile::tempdir().unwrap();
        let old_work = tempfile::tempdir().unwrap();
        let config = test_config(data.path(), data.path());
        create_exact_legacy_database(&config.database_path()).await;
        let old_generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(
            data.path().join("storage-generation"),
            &old_generation,
        )
        .unwrap();
        write_released_v3_receipt_without_binding_flag(
            data.path(),
            old_work.path(),
            &old_generation,
        );
        std::fs::create_dir_all(data.path().join("conversations"))
            .unwrap();
        let unowned_target =
            data.path().join("conversations/do-not-retire");
        std::fs::write(&unowned_target, b"preserve").unwrap();

        let error = prepare_v3_data_layer(&config).await.unwrap_err();
        assert!(error.to_string().contains("already contains conversations"));
        assert!(unowned_target.is_file());
        assert!(config.database_path().is_file());
        assert!(
            !data
                .path()
                .join(nomifun_common::factory_reset::V3_DATASET_RESET_DIR)
                .exists()
        );
    }

    #[tokio::test]
    async fn released_same_root_receipt_does_not_block_strict_legacy_retirement() {
        let data = tempfile::tempdir().unwrap();
        let config = test_config(data.path(), data.path());
        create_exact_legacy_database(&config.database_path()).await;
        let old_generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(
            data.path().join("storage-generation"),
            &old_generation,
        )
        .unwrap();
        write_released_v3_receipt_without_binding_flag(
            data.path(),
            data.path(),
            &old_generation,
        );
        std::fs::create_dir_all(data.path().join("conversations"))
            .unwrap();
        std::fs::write(
            data.path().join("conversations/history"),
            b"old",
        )
        .unwrap();

        assert_eq!(
            prepare_v3_data_layer(&config).await.unwrap(),
            V3DataLayerState::BootstrapRequired
        );
        let plan =
            nomifun_common::factory_reset::read_pending_v3_reset(
                data.path(),
                data.path(),
            )
            .unwrap()
            .expect("strict legacy reset plan");
        assert_eq!(
            plan.reason,
            nomifun_common::factory_reset::DatasetResetReason::NonV3Dataset
        );
        assert!(!config.database_path().exists());
        assert!(!data.path().join("conversations").exists());
        assert!(
            data.path()
                .join(&plan.retired_dir)
                .join("nomifun-backend.db")
                .is_file()
        );
        assert!(
            data.path()
                .join(&plan.retired_dir)
                .join("conversations/history")
                .is_file()
        );
    }

    #[tokio::test]
    async fn released_external_receipt_persists_binding_for_later_boots() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let config = test_config(data.path(), work.path());
        create_exact_legacy_database(&config.database_path()).await;
        let old_generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(
            data.path().join("storage-generation"),
            &old_generation,
        )
        .unwrap();
        write_released_v3_receipt_without_binding_flag(
            data.path(),
            work.path(),
            &old_generation,
        );
        assert!(
            nomifun_common::dir_config::checked_persisted_work_dir(
                data.path(),
            )
            .unwrap()
            .is_none()
        );

        assert_eq!(
            prepare_v3_data_layer(&config).await.unwrap(),
            V3DataLayerState::BootstrapRequired
        );
        assert_eq!(
            nomifun_common::dir_config::checked_persisted_work_dir(
                data.path(),
            )
            .unwrap(),
            Some(std::fs::canonicalize(work.path()).unwrap())
        );
        install_storage_generation_environment(&config).unwrap();
        let database =
            nomifun_db::init_database(&config.database_path()).await.unwrap();
        database.close().await;
        finalize_data_layer(&config).unwrap();

        assert_eq!(
            crate::bootstrap::work_dir::resolve_work_dir(
                None,
                data.path(),
            )
            .unwrap(),
            std::fs::canonicalize(work.path()).unwrap()
        );
        assert_eq!(
            prepare_v3_data_layer(&config).await.unwrap(),
            V3DataLayerState::FinalizedCurrent
        );
    }

    #[tokio::test]
    async fn receiptless_legacy_database_never_claims_nonempty_override_target() {
        let data = tempfile::tempdir().unwrap();
        let override_work = tempfile::tempdir().unwrap();
        let mut config = test_config(data.path(), override_work.path());
        config.work_dir_is_cli_override = true;
        create_exact_legacy_database(&config.database_path()).await;
        std::fs::create_dir_all(
            override_work.path().join("conversations"),
        )
        .unwrap();
        let unowned_target =
            override_work.path().join("conversations/new-data");
        std::fs::write(&unowned_target, b"preserve").unwrap();

        let error = prepare_v3_data_layer(&config).await.unwrap_err();
        assert!(
            error.to_string().contains("already contains")
                || error.to_string().contains("must not already contain")
        );
        assert!(unowned_target.is_file());
        assert!(config.database_path().is_file());
        assert!(
            !data
                .path()
                .join(nomifun_common::factory_reset::V3_DATASET_RESET_DIR)
                .exists()
        );
    }

    #[tokio::test]
    async fn damaged_current_v3_is_never_automatically_retired() {
        let data = tempfile::tempdir().unwrap();
        let config = test_config(data.path(), data.path());
        let database =
            nomifun_db::init_database(&config.database_path()).await.unwrap();
        nomifun_db::sqlx::query(
            "UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = 1",
        )
        .execute(database.pool())
        .await
        .unwrap();
        database.close().await;
        let generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(data.path().join("storage-generation"), &generation)
            .unwrap();
        nomifun_common::factory_reset::write_v3_dataset_receipt(
            data.path(),
            &generation,
        )
        .unwrap();
        std::fs::create_dir_all(data.path().join("knowledge")).unwrap();
        let side_store = data.path().join("knowledge/current-v3");
        std::fs::write(&side_store, b"preserve").unwrap();

        let error = prepare_v3_data_layer(&config).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("preserving it without mutation")
        );
        assert!(config.database_path().is_file());
        assert!(side_store.is_file());
        assert!(
            !data
                .path()
                .join(nomifun_common::factory_reset::V3_DATASET_RESET_DIR)
                .exists()
        );
        assert!(
            data.path()
                .join(nomifun_common::factory_reset::V3_DATASET_RECEIPT_FILE)
                .is_file()
        );
    }

    #[tokio::test]
    async fn valid_v3_database_without_receipt_is_not_retired_before_probe() {
        let data = tempfile::tempdir().unwrap();
        let path = data.path().join("nomifun-backend.db");
        let database = nomifun_db::init_database(&path).await.unwrap();
        database.close().await;
        let generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(data.path().join("storage-generation"), &generation).unwrap();
        nomifun_common::factory_reset::write_v3_dataset_bootstrap_binding(
            data.path(),
            data.path(),
            &generation,
        )
        .unwrap();

        assert_eq!(
            prepare_v3_data_layer(&test_config(data.path(), data.path()))
                .await
                .unwrap(),
            V3DataLayerState::BootstrapRequired
        );
        assert!(path.is_file());
        assert!(
            !data
                .path()
                .join(nomifun_common::factory_reset::V3_DATASET_RESET_DIR)
                .exists()
        );
    }

    #[tokio::test]
    async fn valid_v3_database_without_any_lifecycle_binding_fails_closed() {
        let data = tempfile::tempdir().unwrap();
        let path = data.path().join("nomifun-backend.db");
        let database = nomifun_db::init_database(&path).await.unwrap();
        database.close().await;
        let generation_path = data.path().join("storage-generation");
        if generation_path.exists() {
            std::fs::remove_file(&generation_path).unwrap();
        }

        let error = prepare_v3_data_layer(&test_config(data.path(), data.path()))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("neither a matching finalized receipt"));
        assert!(path.is_file());
        assert!(
            !data
                .path()
                .join(nomifun_common::factory_reset::V3_DATASET_RESET_DIR)
                .exists()
        );
    }

    #[tokio::test]
    async fn work_root_change_plan_resumes_after_request_clear_crash_gap() {
        let data = tempfile::tempdir().unwrap();
        let old_work = tempfile::tempdir().unwrap();
        let new_work = tempfile::tempdir().unwrap();
        let config = test_config(data.path(), new_work.path());
        let database =
            nomifun_db::init_database(&config.database_path()).await.unwrap();
        database.close().await;
        let old_generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(
            data.path().join("storage-generation"),
            &old_generation,
        )
        .unwrap();
        nomifun_common::factory_reset::write_v3_dataset_receipt_for_work_dir(
            data.path(),
            old_work.path(),
            &old_generation,
        )
        .unwrap();
        std::fs::create_dir_all(old_work.path().join("conversations"))
            .unwrap();
        let old_sentinel =
            old_work.path().join("conversations/current-before-change");
        std::fs::write(&old_sentinel, b"old-current").unwrap();

        nomifun_common::factory_reset::request_v3_dataset_reset_for_work_dir(
            data.path(),
            new_work.path(),
        )
        .unwrap();
        let plan =
            nomifun_common::factory_reset::arm_v3_dataset_reset(
                data.path(),
                new_work.path(),
                nomifun_common::factory_reset::DatasetResetReason::WorkDirChange,
            )
            .unwrap();
        assert!(
            !data
                .path()
                .join(
                    nomifun_common::factory_reset::V3_DATASET_RESET_REQUEST_FILE,
                )
                .exists(),
            "the immutable plan must have consumed the transient request"
        );
        assert!(
            config.database_path().is_file(),
            "simulate a crash before the plan applies the old data roots"
        );

        assert_eq!(
            prepare_v3_data_layer(&config).await.unwrap(),
            V3DataLayerState::BootstrapRequired
        );
        let resumed =
            nomifun_common::factory_reset::read_pending_v3_reset(
                data.path(),
                new_work.path(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(resumed.operation_id, plan.operation_id);
        assert_eq!(resumed.generation, plan.generation);
        assert!(!config.database_path().exists());
        assert!(
            old_sentinel.is_file(),
            "the detached old work root is preserved rather than migrated"
        );
        assert!(!new_work.path().join("conversations").exists());
    }

    #[tokio::test]
    async fn finalized_database_rejects_a_different_resolved_work_root() {
        let data = tempfile::tempdir().unwrap();
        let first_work = tempfile::tempdir().unwrap();
        let second_work = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(second_work.path().join("conversations")).unwrap();
        std::fs::write(
            second_work.path().join("conversations/legacy.txt"),
            b"must-not-be-accepted",
        )
        .unwrap();

        let path = data.path().join("nomifun-backend.db");
        let database = nomifun_db::init_database(&path).await.unwrap();
        database.close().await;
        let generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(data.path().join("storage-generation"), &generation).unwrap();
        nomifun_common::factory_reset::write_v3_dataset_receipt_for_work_dir(
            data.path(),
            first_work.path(),
            &generation,
        )
        .unwrap();

        let error = prepare_v3_data_layer(&test_config(data.path(), second_work.path()))
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("bound to a different resolved work root")
        );
        assert!(path.is_file());
        assert!(
            second_work
                .path()
                .join("conversations/legacy.txt")
                .is_file()
        );
        assert!(
            !data
                .path()
                .join(nomifun_common::factory_reset::V3_DATASET_RESET_DIR)
                .exists()
        );
    }

    #[test]
    fn external_work_root_lock_blocks_a_second_dataset_until_drop() {
        let work = tempfile::tempdir().unwrap();

        let first = acquire_work_root_lock(work.path()).unwrap();
        let error = acquire_work_root_lock(work.path())
            .expect_err("second dataset must not share a live external work root");
        assert!(error.to_string().contains("already in use"));

        drop(first);
        acquire_work_root_lock(work.path()).unwrap();
    }

    #[test]
    fn data_dir_as_work_root_also_gets_a_work_root_lock() {
        let data = tempfile::tempdir().unwrap();
        let first = acquire_work_root_lock(data.path()).unwrap();
        let error = acquire_work_root_lock(data.path())
            .expect_err("a second dataset must not reuse the same resolved work root");
        assert!(error.to_string().contains("already in use"));
        drop(first);
    }

    #[test]
    fn missing_work_root_is_not_recreated_by_lock_acquisition() {
        let parent = tempfile::tempdir().unwrap();
        let missing = parent.path().join("deleted-external-work-root");

        let error = acquire_work_root_lock(&missing)
            .expect_err("a missing bound work root must fail closed");

        assert!(error.to_string().contains("inspect work dir"));
        assert!(
            !missing.exists(),
            "locking must never silently recreate a deleted work root"
        );
    }

    #[test]
    fn data_root_lock_blocks_cross_dataset_work_root_alias() {
        let first_data = tempfile::tempdir().unwrap();
        let second_data = tempfile::tempdir().unwrap();
        let second_external_work = tempfile::tempdir().unwrap();

        let first_data_lock =
            acquire_work_root_lock(first_data.path()).unwrap();
        let _first_external_lock = acquire_distinct_work_root_lock(
            &first_data_lock,
            second_data.path(),
        )
        .unwrap()
        .expect("the second data directory is distinct");

        let error = acquire_work_root_lock(second_data.path()).expect_err(
            "a directory used as dataset one work root cannot concurrently become dataset two data root",
        );

        assert!(error.to_string().contains("already in use"));
        assert!(
            acquire_work_root_lock(second_external_work.path()).is_ok(),
            "the conflict is scoped to the aliased root"
        );
    }
}
