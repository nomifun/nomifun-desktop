//! Offline backup and restore commands for `nomicore`.
//!
//! These commands intentionally do not boot the HTTP server. Backup acquires
//! the normal per-data-dir server lock and then the resolved work-root lock
//! before opening SQLite, matching server startup order. This excludes both a
//! live backend on the same dataset and another dataset sharing an external
//! work root. Restore never opens the destination database and refuses to
//! overwrite existing files.

use std::fs;
#[cfg(windows)]
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
#[cfg(windows)]
use std::time::Duration;

use anyhow::{Context, Result, bail};
use nomifun_db::backup_bundle::{
    BackupObjectGraph, BackupSource, create_backup_bundle_with_sources, restore_backup_data_dir,
    validate_backup_source_roots, verify_backup_bundle,
};
#[cfg(windows)]
use nomifun_db::backup_bundle::BackupError;

use crate::cli::Cli;
use crate::config::{
    DATA_ENCRYPTION_KEY_FILE, load_or_create_storage_generation,
    validate_existing_data_encryption_key,
};

/// Create a complete offline bundle from the resolved data/work directories.
pub async fn run_backup(cli: &Cli, output: PathBuf) -> Result<ExitCode> {
    // Resolution may perform the one-time, receipt-bound legacy work-dir
    // repair, so acquire the same data-dir authority as server boot first.
    let data_server_lock =
        crate::bootstrap::acquire_offline_server_lock(&cli.data_dir)?;
    let data_dir = data_server_lock.protected_data_dir().to_path_buf();
    let data_root_work_lock =
        crate::bootstrap::acquire_work_root_lock(&data_dir)?;
    nomifun_common::factory_reset::require_data_root_not_owned_as_external_work(
        &data_dir,
    )?;
    let requested_work_dir =
        crate::bootstrap::resolve_work_dir(cli.work_dir.clone(), &data_dir)?;
    let _external_work_root_lock =
        crate::bootstrap::acquire_distinct_work_root_lock(
            &data_root_work_lock,
            &requested_work_dir,
        )?;
    let work_dir = _external_work_root_lock
        .as_ref()
        .map(|lock| lock.protected_root().to_path_buf())
        .unwrap_or_else(|| data_root_work_lock.protected_root().to_path_buf());
    let manifest =
        create_offline_backup_locked(&data_dir, &work_dir, &output).await?;
    println!(
        "backup created: {} ({} bytes)",
        output.display(),
        manifest.files.iter().map(|file| file.bytes).sum::<u64>()
    );
    Ok(ExitCode::SUCCESS)
}

/// Restore a verified complete bundle into a fresh destination data directory.
pub async fn run_restore(bundle: PathBuf, destination_data_dir: PathBuf) -> Result<ExitCode> {
    let outcome = restore_offline_backup(&bundle, &destination_data_dir).await?;
    println!(
        "backup restored: {} (managed workspaces restored under {}; storage-generation rotated to {})",
        destination_data_dir.display(),
        destination_data_dir.join("conversations").display(),
        outcome.destination_storage_generation
    );
    println!(
        "note: start the restored installation with --data-dir {} (and no old custom --work-dir) \
         unless you intentionally relocate the restored managed workspaces",
        destination_data_dir.display()
    );
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
async fn create_offline_backup(
    data_dir: &Path,
    work_dir: &Path,
    output: &Path,
) -> Result<nomifun_db::backup_bundle::BackupManifest> {
    // Match server boot's lock order. The data lock protects the receipt and
    // database; the work-root lock prevents another dataset that shares this
    // external root from mutating conversations during the snapshot.
    let data_server_lock =
        crate::bootstrap::acquire_offline_server_lock(data_dir)?;
    let data_dir = data_server_lock.protected_data_dir().to_path_buf();
    let data_root_work_lock =
        crate::bootstrap::acquire_work_root_lock(&data_dir)?;
    nomifun_common::factory_reset::require_data_root_not_owned_as_external_work(
        &data_dir,
    )?;
    let _external_work_root_lock =
        crate::bootstrap::acquire_distinct_work_root_lock(
            &data_root_work_lock,
            work_dir,
        )?;
    let work_dir = _external_work_root_lock
        .as_ref()
        .map(|lock| lock.protected_root().to_path_buf())
        .unwrap_or_else(|| data_root_work_lock.protected_root().to_path_buf());
    create_offline_backup_locked(&data_dir, &work_dir, output).await
}

async fn create_offline_backup_locked(
    data_dir: &Path,
    work_dir: &Path,
    output: &Path,
) -> Result<nomifun_db::backup_bundle::BackupManifest> {
    let source = BackupSource::new(data_dir, work_dir);
    validate_backup_source_roots(source)
        .map_err(|error| anyhow::anyhow!("validate backup source roots: {error}"))?;
    let database_path = data_dir.join("nomifun-backend.db");
    ensure_regular_source_file(&database_path, "database")?;

    nomifun_common::factory_reset::ensure_current_v3_work_root_owner(
        data_dir, work_dir,
    )
    .map_err(|error| {
        anyhow::anyhow!(
            "backup requires a finalized v3 dataset bound to the resolved work directory: {error}"
        )
    })?;
    let generation_path = data_dir.join("storage-generation");
    ensure_regular_source_file(&generation_path, "storage generation")?;
    let generation = load_or_create_storage_generation(data_dir)
        .with_context(|| format!("read storage generation in {}", data_dir.display()))?;
    let encryption_key_path = data_dir.join(DATA_ENCRYPTION_KEY_FILE);
    let encryption_key_present = match fs::symlink_metadata(&encryption_key_path) {
        Ok(_) => {
            ensure_regular_source_file(&encryption_key_path, "encryption key")?;
            validate_existing_data_encryption_key(&encryption_key_path).with_context(|| {
                format!(
                    "validate persistent encryption key {}",
                    encryption_key_path.display()
                )
            })?;
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    let database = nomifun_db::open_database_for_backup(&database_path)
        .await
        .with_context(|| format!("open database without mutation {}", database_path.display()))?;
    let encrypted_values = database_contains_encrypted_values(&database).await;
    let encrypted_values = match encrypted_values {
        Ok(value) => value,
        Err(error) => {
            database.close().await;
            return Err(error);
        }
    };
    if !encryption_key_present && encrypted_values {
        database.close().await;
        bail!(
            "database contains encrypted credentials but {} is missing; refusing an unrestorable backup",
            encryption_key_path.display()
        );
    }
    let result = create_backup_bundle_with_sources(
        &database,
        output,
        &generation,
        BackupObjectGraph::full_database(),
        source,
    )
    .await
    .map_err(|error| anyhow::anyhow!("create backup bundle: {error}"));
    database.close().await;
    result
}

fn ensure_regular_source_file(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect {label} {}", path.display()))?;
    if metadata.file_type().is_symlink() || metadata_has_reparse_point(&metadata) || !metadata.is_file() {
        bail!("{label} must be a regular file without symlink/reparse indirection: {}", path.display());
    }
    Ok(())
}

#[cfg(windows)]
fn metadata_has_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_has_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

async fn database_contains_encrypted_values(database: &nomifun_db::Database) -> Result<bool> {
    const PROBES: &[&str] = &[
        "SELECT EXISTS(SELECT 1 FROM providers WHERE api_key_encrypted <> '' LIMIT 1)",
        "SELECT EXISTS(SELECT 1 FROM channel_plugins WHERE config <> '' LIMIT 1)",
        "SELECT EXISTS(SELECT 1 FROM remote_agents WHERE auth_token IS NOT NULL OR device_public_key IS NOT NULL OR device_private_key IS NOT NULL OR device_token IS NOT NULL LIMIT 1)",
        "SELECT EXISTS(SELECT 1 FROM connector_credentials WHERE payload_encrypted <> '' LIMIT 1)",
        "SELECT EXISTS(SELECT 1 FROM oauth_tokens WHERE access_token <> '' OR refresh_token IS NOT NULL LIMIT 1)",
    ];
    for query in PROBES {
        let present: i64 = nomifun_db::sqlx::query_scalar(query)
            .fetch_one(database.pool())
            .await
            .with_context(|| format!("inspect encrypted backup dependency with `{query}`"))?;
        if present != 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn restore_offline_backup(
    bundle: &Path,
    destination_data_dir: &Path,
) -> Result<nomifun_db::backup_bundle::RestoreOutcome> {
    let manifest = verify_backup_bundle(bundle)
        .map_err(|error| anyhow::anyhow!("verify backup bundle: {error}"))?;
    prepare_restore_destination(destination_data_dir)?;

    #[cfg(windows)]
    let outcome = restore_with_windows_atomic_install_retry(destination_data_dir, || {
        restore_backup_data_dir(bundle, destination_data_dir)
    })
    .await
    .with_context(|| {
        format!(
            "restore backup bundle into {}",
            destination_data_dir.display()
        )
    })?;

    #[cfg(not(windows))]
    let outcome = restore_backup_data_dir(bundle, destination_data_dir)
        .await
        .map_err(|error| anyhow::anyhow!("restore backup bundle: {error}"))?;

    debug_assert_eq!(manifest, outcome.manifest);
    Ok(outcome)
}

#[cfg(windows)]
const WINDOWS_RESTORE_RETRY_DELAYS: [Duration; 6] = [
    Duration::from_millis(10),
    Duration::from_millis(25),
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(200),
    Duration::from_millis(400),
];

#[cfg(windows)]
async fn restore_with_windows_atomic_install_retry<T, Restore, RestoreFuture>(
    destination: &Path,
    mut restore: Restore,
) -> Result<T>
where
    Restore: FnMut() -> RestoreFuture,
    RestoreFuture: Future<Output = std::result::Result<T, BackupError>>,
{
    let max_attempts = WINDOWS_RESTORE_RETRY_DELAYS.len() + 1;
    for attempt in 1..=max_attempts {
        let staging_before = restore_staging_paths(destination).with_context(|| {
            format!(
                "inventory restore staging paths before attempt {attempt} for {}",
                destination.display()
            )
        })?;
        match restore().await {
            Ok(outcome) => {
                wait_for_windows_sqlite_handles(destination).await?;
                cleanup_new_restore_staging(destination, &staging_before).await?;
                return Ok(outcome);
            }
            Err(error) => {
                let staging_after = restore_staging_paths(destination).with_context(|| {
                    format!(
                        "inventory restore staging paths after attempt {attempt} for {}",
                        destination.display()
                    )
                })?;
                let new_staging = staging_after
                    .into_iter()
                    .filter(|path| !staging_before.contains(path))
                    .collect::<Vec<_>>();

                if let Err(cleanup_error) =
                    cleanup_restore_staging_paths(destination, &new_staging).await
                {
                    return Err(anyhow::Error::new(error).context(format!(
                        "restore staging cleanup also failed for {}: {cleanup_error:#}",
                        destination.display()
                    )));
                }
                if destination_exists_no_follow(destination)? {
                    return Err(anyhow::Error::new(error).context(format!(
                        "restore failed after the destination appeared; refusing to retry {}",
                        destination.display()
                    )));
                }
                let retryable = matches!(
                    &error,
                    BackupError::Io(error) if is_transient_windows_filesystem_error(error)
                );
                if retryable && attempt < max_attempts {
                    let delay = WINDOWS_RESTORE_RETRY_DELAYS[attempt - 1];
                    tracing::warn!(
                        destination = %destination.display(),
                        attempt,
                        max_attempts,
                        retry_delay_ms = delay.as_millis(),
                        error = %error,
                        "transient Windows restore filesystem failure; retrying after confirmed staging cleanup"
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                }
                return Err(error.into());
            }
        }
    }

    unreachable!("the bounded Windows restore loop always returns")
}

#[cfg(windows)]
fn is_transient_windows_filesystem_error(error: &std::io::Error) -> bool {
    // ERROR_ACCESS_DENIED may be returned for a directory/file operation while
    // a recently closed SQLite WAL/SHM handle or a filesystem filter still has
    // a short-lived reference. ERROR_SHARING_VIOLATION and
    // ERROR_LOCK_VIOLATION are the more explicit forms of the same condition.
    matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

#[cfg(windows)]
fn restore_staging_paths(destination: &Path) -> Result<Vec<PathBuf>> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let prefix = restore_staging_prefix(destination);
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("read restore parent {}", parent.display()));
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry =
            entry.with_context(|| format!("read restore parent entry {}", parent.display()))?;
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

#[cfg(windows)]
fn restore_staging_prefix(destination: &Path) -> String {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("backup");
    format!(".{name}.staging-")
}

#[cfg(windows)]
async fn cleanup_new_restore_staging(destination: &Path, before: &[PathBuf]) -> Result<()> {
    let new_staging = restore_staging_paths(destination)?
        .into_iter()
        .filter(|path| !before.contains(path))
        .collect::<Vec<_>>();
    cleanup_restore_staging_paths(destination, &new_staging).await
}

#[cfg(windows)]
async fn cleanup_restore_staging_paths(destination: &Path, paths: &[PathBuf]) -> Result<()> {
    for path in paths {
        validate_restore_staging_path(destination, &path)?;
        wait_for_windows_staging_handles(path).await?;
        retry_transient_windows_io(|| match fs::remove_dir_all(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        })
        .await
        .with_context(|| format!("remove restore staging directory {}", path.display()))?;
    }
    let remaining = restore_staging_paths(destination)?;
    if let Some(path) = paths.iter().find(|path| remaining.contains(path)) {
        bail!(
            "restore staging directory still exists after cleanup: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(windows)]
fn validate_restore_staging_path(destination: &Path, staging: &Path) -> Result<()> {
    let expected_parent = destination.parent().unwrap_or_else(|| Path::new("."));
    if staging.parent() != Some(expected_parent)
        || !staging
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(&restore_staging_prefix(destination)))
    {
        bail!(
            "refusing to clean an unrelated restore staging path: {}",
            staging.display()
        );
    }
    let metadata = match fs::symlink_metadata(staging) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("inspect restore staging path {}", staging.display()));
        }
    };
    if metadata.file_type().is_symlink()
        || metadata_has_reparse_point(&metadata)
        || !metadata.is_dir()
    {
        bail!(
            "refusing to clean a non-directory or redirected restore staging path: {}",
            staging.display()
        );
    }
    Ok(())
}

#[cfg(windows)]
fn restore_staging_file_paths(staging: &Path) -> Result<Vec<PathBuf>> {
    let mut pending = vec![staging.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .with_context(|| format!("read restore staging directory {}", directory.display()))?;
        for entry in entries {
            let entry = entry
                .with_context(|| format!("read restore staging entry {}", directory.display()))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .with_context(|| format!("inspect restore staging entry {}", path.display()))?;
            if metadata.file_type().is_symlink() || metadata_has_reparse_point(&metadata) {
                bail!(
                    "refusing to clean redirected restore staging entry: {}",
                    path.display()
                );
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                files.push(path);
            } else {
                bail!(
                    "refusing to clean non-regular restore staging entry: {}",
                    path.display()
                );
            }
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(windows)]
async fn wait_for_windows_sqlite_handles(destination: &Path) -> Result<()> {
    for (name, required) in [
        ("nomifun-backend.db", true),
        ("nomifun-backend.db-wal", false),
        ("nomifun-backend.db-shm", false),
    ] {
        let path = destination.join(name);
        if path_exists_no_follow(&path)? {
            retry_transient_windows_io(|| match probe_file_exclusive_open(&path) {
                Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                result => result,
            })
                .await
                .with_context(|| format!("wait for SQLite handle release {}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn probe_file_exclusive_open(path: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .map(drop)
}

#[cfg(windows)]
async fn wait_for_windows_staging_handles(staging: &Path) -> Result<()> {
    for file in restore_staging_file_paths(staging)? {
        retry_transient_windows_io(|| match probe_file_exclusive_open(&file) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => result,
        })
        .await
        .with_context(|| format!("wait for restore staging handle release {}", file.display()))?;
    }
    retry_transient_windows_io(|| probe_directory_exclusive_open(staging))
        .await
        .with_context(|| format!("wait for restore staging directory release {}", staging.display()))?;
    Ok(())
}

#[cfg(windows)]
fn probe_directory_exclusive_open(path: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map(drop)
}

#[cfg(windows)]
async fn retry_transient_windows_io<T>(
    mut operation: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    for delay in WINDOWS_RESTORE_RETRY_DELAYS {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if is_transient_windows_filesystem_error(&error) => {
                tokio::time::sleep(delay).await;
            }
            Err(error) => return Err(error),
        }
    }
    operation()
}

#[cfg(windows)]
fn destination_exists_no_follow(destination: &Path) -> Result<bool> {
    path_exists_no_follow(destination).with_context(|| {
        format!(
            "inspect restore destination without following links {}",
            destination.display()
        )
    })
}

#[cfg(windows)]
fn path_exists_no_follow(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn prepare_restore_destination(destination: &Path) -> Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) => {
        if metadata.file_type().is_symlink()
            || metadata_has_reparse_point(&metadata)
            || !metadata.is_dir()
        {
            bail!(
                "restore destination must be an absent or empty directory: {}",
                destination.display()
            );
        }
        let mut entries = fs::read_dir(destination)
            .with_context(|| format!("read restore destination {}", destination.display()))?;
        if entries.next().is_some() {
            bail!(
                "restore destination must be absent or empty: {}",
                destination.display()
            );
        }
        fs::remove_dir(destination).with_context(|| {
            format!(
                "remove empty restore destination before atomic install {}",
                destination.display()
            )
        })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        create_offline_backup, load_or_create_storage_generation, prepare_restore_destination,
        restore_offline_backup,
    };
    #[cfg(windows)]
    use super::{
        restore_staging_paths, restore_with_windows_atomic_install_retry,
    };
    use nomifun_common::ConversationId;
    use nomifun_db::backup_bundle::verify_backup_bundle;
    #[cfg(windows)]
    use nomifun_db::backup_bundle::BackupError;
    #[cfg(windows)]
    use std::cell::Cell;
    use std::fs;

    fn canonical_tempdir() -> tempfile::TempDir {
        let canonical_temp_root =
            fs::canonicalize(std::env::temp_dir()).unwrap();
        tempfile::Builder::new()
            .prefix("nomifun-backup-test-")
            .tempdir_in(canonical_temp_root)
            .unwrap()
    }

    #[test]
    fn restore_destination_allows_absent_and_empty_directories() {
        let root = canonical_tempdir();
        let absent = root.path().join("absent");
        prepare_restore_destination(&absent).unwrap();
        let empty = root.path().join("empty");
        fs::create_dir(&empty).unwrap();
        prepare_restore_destination(&empty).unwrap();
        assert!(!empty.exists());
    }

    #[test]
    fn restore_destination_rejects_non_empty_directory() {
        let root = canonical_tempdir();
        let dir = root.path().join("non-empty");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("existing"), b"x").unwrap();
        assert!(prepare_restore_destination(&dir).is_err());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_restore_retries_access_denied_after_cleaning_its_staging() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("restored");
        let attempts = Cell::new(0_u32);

        let value = restore_with_windows_atomic_install_retry(&destination, || {
            let attempt = attempts.get() + 1;
            attempts.set(attempt);
            if attempt == 1 {
                let staging = root.path().join(format!(
                    ".restored.staging-{}",
                    uuid::Uuid::now_v7()
                ));
                fs::create_dir(&staging).unwrap();
                fs::write(staging.join("nomifun-backend.db"), b"database").unwrap();
                std::future::ready(Err::<u32, _>(BackupError::Io(
                    std::io::Error::from_raw_os_error(5),
                )))
            } else {
                std::future::ready(Ok(7_u32))
            }
        })
        .await
        .unwrap();

        assert_eq!(value, 7);
        assert_eq!(attempts.get(), 2);
        assert!(restore_staging_paths(&destination).unwrap().is_empty());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_restore_does_not_retry_non_transient_io() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("restored");
        let attempts = Cell::new(0_u32);

        let error = restore_with_windows_atomic_install_retry(&destination, || {
            attempts.set(attempts.get() + 1);
            std::future::ready(Err::<(), _>(BackupError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "not transient",
            ))))
        })
        .await
        .unwrap_err();

        assert_eq!(attempts.get(), 1);
        assert!(format!("{error:#}").contains("not transient"));
        assert!(restore_staging_paths(&destination).unwrap().is_empty());
    }

    #[tokio::test]
    async fn command_roundtrip_preserves_ids_and_rotates_generation() {
        let root = canonical_tempdir();
        let source = root.path().join("source");
        let bundle = root.path().join("bundle");
        let destination = root.path().join("restored");
        fs::create_dir(&source).unwrap();
        let database_path = source.join("nomifun-backend.db");
        let database = nomifun_db::init_database(&database_path).await.unwrap();
        let installation_owner =
            nomifun_db::installation_owner_id(database.pool()).await.unwrap();
        let conversation_id = ConversationId::new().into_string();
        nomifun_db::sqlx::query(
            "INSERT INTO conversations \
             (conversation_id, user_id, name, type, extra, status, created_at, updated_at) \
             VALUES (?, ?, 'backup command', 'nomi', '{}', 'pending', 1, 1)",
        )
        .bind(&conversation_id)
        .bind(&installation_owner)
        .execute(database.pool())
        .await
        .unwrap();
        database.close().await;

        let source_generation = load_or_create_storage_generation(&source).unwrap();
        nomifun_common::factory_reset::write_v3_dataset_receipt_for_work_dir(
            &source,
            &source,
            &source_generation,
        )
        .unwrap();
        fs::write(source.join("encryption_key"), "11".repeat(32)).unwrap();
        fs::create_dir_all(source.join("companion/shared")).unwrap();
        fs::write(source.join("companion/shared/config.json"), "{}").unwrap();
        fs::create_dir_all(source.join("conversations/managed-temp-ws")).unwrap();
        fs::write(
            source.join("conversations/managed-temp-ws/result.txt"),
            "workspace",
        )
        .unwrap();
        fs::create_dir_all(source.join("logs")).unwrap();
        fs::write(source.join("logs/ignored.log"), "runtime log").unwrap();

        let manifest = create_offline_backup(&source, &source, &bundle)
            .await
            .unwrap();
        assert_eq!(manifest.source_storage_generation, source_generation);
        assert_eq!(verify_backup_bundle(&bundle).unwrap(), manifest);

        let outcome = restore_offline_backup(&bundle, &destination).await.unwrap();
        assert_ne!(
            outcome.destination_storage_generation,
            source_generation,
            "restore must rotate the dataset namespace"
        );
        nomifun_common::factory_reset::require_current_v3_dataset_for_work_dir(
            &destination,
            &destination,
        )
        .unwrap();
        nomifun_common::factory_reset::require_v3_work_root_owner(
            &destination,
            &destination,
            &outcome.destination_storage_generation,
        )
        .unwrap();
        let restored = nomifun_db::init_database(&destination.join("nomifun-backend.db"))
            .await
            .unwrap();
        let restored_id: String = nomifun_db::sqlx::query_scalar(
            "SELECT conversation_id FROM conversations WHERE name = 'backup command'",
        )
        .fetch_one(restored.pool())
        .await
        .unwrap();
        assert_eq!(restored_id, conversation_id);
        restored.close().await;
        assert_eq!(
            fs::read_to_string(destination.join("encryption_key")).unwrap(),
            "11".repeat(32)
        );
        assert_eq!(
            fs::read_to_string(destination.join("companion/shared/config.json")).unwrap(),
            "{}"
        );
        assert_eq!(
            fs::read_to_string(
                destination.join("conversations/managed-temp-ws/result.txt")
            )
            .unwrap(),
            "workspace"
        );
        assert!(!destination.join("logs").exists());
    }

    #[tokio::test]
    async fn backup_refuses_a_contended_server_lock() {
        let root = canonical_tempdir();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        let database = nomifun_db::init_database(&source.join("nomifun-backend.db"))
            .await
            .unwrap();
        database.close().await;
        let generation = load_or_create_storage_generation(&source).unwrap();
        nomifun_common::factory_reset::write_v3_dataset_receipt_for_work_dir(
            &source,
            &source,
            &generation,
        )
        .unwrap();
        let _held = crate::bootstrap::acquire_offline_server_lock(&source).unwrap();

        let error = create_offline_backup(&source, &source, &root.path().join("bundle"))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("already in use"));
    }

    #[tokio::test]
    async fn backup_refuses_a_contended_external_work_root() {
        let root = canonical_tempdir();
        let source = root.path().join("source");
        let work = root.path().join("external-work");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&work).unwrap();
        let _held = crate::bootstrap::acquire_work_root_lock(&work).unwrap();

        let error =
            create_offline_backup(&source, &work, &root.path().join("bundle"))
                .await
                .unwrap_err();

        assert!(format!("{error:#}").contains("already in use"));
        assert!(!root.path().join("bundle").exists());
    }

    #[tokio::test]
    async fn backup_refuses_encrypted_rows_without_their_persistent_key() {
        let root = canonical_tempdir();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        let database = nomifun_db::init_database(&source.join("nomifun-backend.db"))
            .await
            .unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO providers \
             (provider_id, platform, name, base_url, api_key_encrypted, models, enabled, capabilities, created_at, updated_at) \
             VALUES ('0190f5fe-7c00-7a00-8abc-012345678901', 'openai', 'encrypted', \
                     'https://example.invalid', 'ciphertext', '[]', 1, '[]', 1, 1)",
        )
        .execute(database.pool())
        .await
        .unwrap();
        database.close().await;
        let generation = load_or_create_storage_generation(&source).unwrap();
        nomifun_common::factory_reset::write_v3_dataset_receipt_for_work_dir(
            &source,
            &source,
            &generation,
        )
        .unwrap();

        let error = create_offline_backup(&source, &source, &root.path().join("bundle"))
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("encryption_key"),
            "unexpected backup rejection: {error:#}"
        );
        assert!(!root.path().join("bundle").exists());
    }
}
