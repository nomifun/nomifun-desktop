use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use nomifun_agent_contracts::{
    DigestHex, MiniAppBackupId, MiniAppBackupSourceState, MiniAppId,
    MiniAppReleaseArtifactV1, MiniAppSourceBundle, MiniAppWholeAppBackupMetadataV1,
    MINIAPP_M1_SCHEMA_VERSION, MINIAPP_RELEASE_PROFILE_VERSION,
    MINIAPP_WHOLE_APP_BACKUP_VERSION, canonical_json_bytes, digest_bytes,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::{MINIAPP_SOURCE_STORE_FORMAT_VERSION, MiniAppMigrationLedger};

const METADATA_FILE: &str = "metadata.json";
const PRODUCT_FILE: &str = "product.json";
const PROJECT_FILE: &str = "project.json";
const CONFIG_FILE: &str = "config.json";
const CREDENTIAL_SLOTS_FILE: &str = "credential-slots.json";
const RELEASES_DIRECTORY: &str = "releases";
const ARTIFACT_FILE: &str = "artifact.json";
const MANIFEST_FILE: &str = "manifest.json";
const SOURCE_DIRECTORY: &str = "source";
const SOURCE_SNAPSHOT_FILE: &str = "snapshot.json";
const DEPENDENCY_LOCK_FILE: &str = "dependency-lock.json";
const STORAGE_DIRECTORY: &str = "storage";
const KV_FILE: &str = "kv.json";
const FILES_DIRECTORY: &str = "files";
const PRIVATE_DATABASE_FILE: &str = "private.sqlite";
const MIGRATION_LEDGER_FILE: &str = "migration-ledger.json";
const STAGING_PREFIX: &str = ".nomifun-miniapp-backup-staging-";

const MAX_PATH_BYTES: usize = 1_024;
const MAX_COMPONENT_BYTES: usize = 255;
const MAX_RELEASE_SLOTS: usize = 64;
const MAX_SECTION_FILES: usize = 8_192;
const MAX_BACKUP_FILES: usize = 32_768;
const MAX_BACKUP_DIRECTORIES: usize = 32_768;
const MAX_JSON_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DEPENDENCY_LOCK_BYTES: u64 = 8 * 1024 * 1024;
const MAX_KV_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SINGLE_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_RELEASE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_STORAGE_FILES_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_PRIVATE_DATABASE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_BACKUP_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppBackupFile {
    pub relative_path: String,
    pub bytes: Vec<u8>,
}

impl MiniAppBackupFile {
    pub fn new(relative_path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            relative_path: relative_path.into(),
            bytes: bytes.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MiniAppBackupRelease {
    pub artifact: MiniAppReleaseArtifactV1,
    pub manifest_bytes: Vec<u8>,
    pub files: Vec<MiniAppBackupFile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppBackupSource {
    pub source: MiniAppSourceBundle,
    pub dependency_lock: Vec<u8>,
    pub files: Vec<MiniAppBackupFile>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MiniAppBackupStorage {
    pub kv: Vec<Value>,
    pub files: Vec<MiniAppBackupFile>,
    pub private_database: Option<Vec<u8>>,
    pub migration_ledger: Option<MiniAppMigrationLedger>,
}

pub struct MiniAppWholeAppBackupExport<'a> {
    pub backup_id: MiniAppBackupId,
    pub source_miniapp_id: MiniAppId,
    pub owner_quiescent: bool,
    pub created_at_ms: i64,
    pub product: &'a Value,
    pub project: &'a Value,
    pub config: &'a Value,
    pub credential_slots: &'a Value,
    pub releases: &'a BTreeMap<String, MiniAppBackupRelease>,
    pub source: Option<&'a MiniAppBackupSource>,
    pub storage: &'a MiniAppBackupStorage,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MiniAppWholeAppBackupImport {
    pub metadata: MiniAppWholeAppBackupMetadataV1,
    pub product: Value,
    pub project: Value,
    pub config: Value,
    pub credential_slots: Value,
    pub releases: BTreeMap<String, MiniAppBackupRelease>,
    pub source: Option<MiniAppBackupSource>,
    pub storage: MiniAppBackupStorage,
}

#[derive(Debug, Error)]
pub enum MiniAppWholeAppBackupError {
    #[error("MiniApp Whole-App Backup input is invalid: {0}")]
    InvalidInput(String),
    #[error("MiniApp Whole-App Backup path is invalid: {path} ({reason})")]
    InvalidPath { path: String, reason: String },
    #[error("MiniApp Whole-App Backup destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("MiniApp Whole-App Backup inventory is invalid: {0}")]
    InvalidInventory(String),
    #[error("MiniApp Whole-App Backup digest or content was modified: {0}")]
    Tampered(String),
    #[error("MiniApp Whole-App Backup exceeds a size limit: {0}")]
    Oversize(String),
    #[error("MiniApp Whole-App Backup filesystem operation failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("MiniApp Whole-App Backup canonical serialization failed: {0}")]
    Canonical(String),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MiniAppWholeAppBackupFilesystem;

impl MiniAppWholeAppBackupFilesystem {
    pub fn new() -> Self {
        Self
    }

    pub fn export(
        &self,
        request: MiniAppWholeAppBackupExport<'_>,
        destination: impl AsRef<Path>,
    ) -> Result<MiniAppWholeAppBackupMetadataV1, MiniAppWholeAppBackupError> {
        let prepared = prepare_export(request)?;
        let destination = destination.as_ref();
        ensure_normal_destination(destination)?;
        let parent = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        ensure_regular_directory(parent)?;
        ensure_regular_directory_chain(parent)?;
        match fs::symlink_metadata(destination) {
            Ok(_) => {
                return Err(MiniAppWholeAppBackupError::DestinationExists(
                    destination.to_path_buf(),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(destination, error)),
        }

        let mut staging = StagingGuard::allocate(parent)?;
        write_backup_tree(staging.path(), &prepared)?;
        let observed = scan_tree(staging.path())?;
        let expected = expected_inventory(
            &prepared.releases,
            prepared.source.as_ref(),
            &prepared.storage,
        );
        require_exact_inventory(&observed, &expected)?;
        sync_tree_directories(staging.path())?;
        match fs::rename(staging.path(), destination) {
            Ok(()) => {}
            Err(error)
                if error.kind() == io::ErrorKind::AlreadyExists
                    || fs::symlink_metadata(destination).is_ok() =>
            {
                return Err(MiniAppWholeAppBackupError::DestinationExists(
                    destination.to_path_buf(),
                ));
            }
            Err(error) => return Err(io_error(destination, error)),
        }
        let _ = sync_directory_if_supported(parent);
        staging.disarm();
        Ok(prepared.metadata)
    }

    pub fn import(
        &self,
        root: impl AsRef<Path>,
    ) -> Result<MiniAppWholeAppBackupImport, MiniAppWholeAppBackupError> {
        import_backup(root.as_ref())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupFileRecord {
    normalized_relative_path: String,
    digest: DigestHex,
    size_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupSourceSnapshotRecord {
    format_version: String,
    source: MiniAppSourceBundle,
    files: Vec<BackupFileRecord>,
}

#[derive(Serialize)]
struct SourceSnapshotDigestInput<'a> {
    format_version: &'a str,
    files: &'a [BackupFileRecord],
}

#[derive(Serialize)]
struct ProductMetadataDigestInput<'a> {
    product: &'a Value,
    project: &'a Value,
    credential_slots: &'a Value,
}

#[derive(Serialize)]
struct ReleaseInventoryEntry<'a> {
    slot: &'a str,
    artifact: &'a MiniAppReleaseArtifactV1,
}

struct PreparedBackup {
    metadata: MiniAppWholeAppBackupMetadataV1,
    product: Value,
    project: Value,
    config: Value,
    credential_slots: Value,
    releases: BTreeMap<String, MiniAppBackupRelease>,
    source: Option<MiniAppBackupSource>,
    storage: MiniAppBackupStorage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpectedInventory {
    files: BTreeSet<String>,
    directories: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ObservedTree {
    files: BTreeSet<String>,
    directories: BTreeSet<String>,
}

fn prepare_export(
    request: MiniAppWholeAppBackupExport<'_>,
) -> Result<PreparedBackup, MiniAppWholeAppBackupError> {
    validate_object(request.product, PRODUCT_FILE)?;
    validate_object(request.project, PROJECT_FILE)?;
    validate_object(request.config, CONFIG_FILE)?;
    let credential_slot_keys = credential_slot_keys(request.credential_slots)?;
    enforce_canonical_size(request.product, PRODUCT_FILE, MAX_JSON_BYTES)?;
    enforce_canonical_size(request.project, PROJECT_FILE, MAX_JSON_BYTES)?;
    enforce_canonical_size(request.config, CONFIG_FILE, MAX_JSON_BYTES)?;
    enforce_canonical_size(
        request.credential_slots,
        CREDENTIAL_SLOTS_FILE,
        MAX_JSON_BYTES,
    )?;

    if request.releases.len() > MAX_RELEASE_SLOTS {
        return Err(MiniAppWholeAppBackupError::Oversize(format!(
            "Release slot count {} exceeds {}",
            request.releases.len(),
            MAX_RELEASE_SLOTS
        )));
    }
    let mut releases = BTreeMap::new();
    for (slot, release) in request.releases {
        validate_release_slot(slot)?;
        releases.insert(slot.clone(), prepare_release(release.clone())?);
    }
    let source = request.source.cloned().map(prepare_source).transpose()?;
    let storage = prepare_storage(request.storage.clone(), &request.source_miniapp_id)?;
    let metadata = calculate_metadata(
        request.backup_id,
        request.source_miniapp_id,
        request.owner_quiescent,
        request.created_at_ms,
        request.product,
        request.project,
        request.config,
        request.credential_slots,
        credential_slot_keys,
        &releases,
        source.as_ref(),
        &storage,
    )?;

    Ok(PreparedBackup {
        metadata,
        product: request.product.clone(),
        project: request.project.clone(),
        config: request.config.clone(),
        credential_slots: request.credential_slots.clone(),
        releases,
        source,
        storage,
    })
}

fn prepare_release(
    release: MiniAppBackupRelease,
) -> Result<MiniAppBackupRelease, MiniAppWholeAppBackupError> {
    release
        .artifact
        .validate()
        .map_err(|error| MiniAppWholeAppBackupError::InvalidInput(error.to_string()))?;
    let expected_manifest = canonical_bytes(&release.artifact.manifest)?;
    if release.manifest_bytes != expected_manifest {
        return Err(MiniAppWholeAppBackupError::Tampered(
            "Release manifest bytes are not canonical or do not match artifact.json".into(),
        ));
    }
    let files = prepare_files(
        release.files,
        "Release",
        MAX_SECTION_FILES,
        MAX_RELEASE_BYTES,
        false,
    )?;
    if files.len() != release.artifact.files.len() {
        return Err(MiniAppWholeAppBackupError::Tampered(
            "Release file inventory is incomplete".into(),
        ));
    }
    for (actual, expected) in files.iter().zip(&release.artifact.files) {
        if actual.relative_path != expected.normalized_relative_path
            || actual.bytes.len() as u64 != expected.size_bytes
            || digest_bytes(&actual.bytes) != expected.digest
        {
            return Err(MiniAppWholeAppBackupError::Tampered(format!(
                "Release file {} does not match its Artifact inventory",
                expected.normalized_relative_path
            )));
        }
    }
    Ok(MiniAppBackupRelease {
        artifact: release.artifact,
        manifest_bytes: release.manifest_bytes,
        files,
    })
}

fn prepare_source(
    source: MiniAppBackupSource,
) -> Result<MiniAppBackupSource, MiniAppWholeAppBackupError> {
    validate_source_contract(&source.source)?;
    let lock: Value =
        parse_canonical_bytes(&source.dependency_lock, MAX_DEPENDENCY_LOCK_BYTES)?;
    if !lock.is_object() {
        return Err(MiniAppWholeAppBackupError::InvalidInput(
            "source/dependency-lock.json must be a JSON object".into(),
        ));
    }
    if digest_bytes(&source.dependency_lock) != source.source.dependency_lock_digest {
        return Err(MiniAppWholeAppBackupError::Tampered(
            "Source dependency lock digest does not match snapshot.json".into(),
        ));
    }
    let files = prepare_files(
        source.files,
        "Source",
        MAX_SECTION_FILES,
        MAX_SOURCE_BYTES,
        false,
    )?;
    let records = file_records(&files);
    let observed_snapshot_digest = source_snapshot_digest(&records)?;
    if observed_snapshot_digest != source.source.source_snapshot_digest {
        return Err(MiniAppWholeAppBackupError::Tampered(
            "Source file inventory does not match its source snapshot digest".into(),
        ));
    }
    Ok(MiniAppBackupSource {
        source: source.source,
        dependency_lock: source.dependency_lock,
        files,
    })
}

fn prepare_storage(
    storage: MiniAppBackupStorage,
    source_miniapp_id: &MiniAppId,
) -> Result<MiniAppBackupStorage, MiniAppWholeAppBackupError> {
    enforce_canonical_size(&storage.kv, "storage/kv.json", MAX_KV_BYTES)?;
    let files = prepare_files(
        storage.files,
        "Storage",
        MAX_SECTION_FILES,
        MAX_STORAGE_FILES_BYTES,
        true,
    )?;
    if storage.private_database.is_some() != storage.migration_ledger.is_some() {
        return Err(MiniAppWholeAppBackupError::InvalidInput(
            "Private Database bytes and migration ledger must be present or absent together"
                .into(),
        ));
    }
    if let Some(database) = &storage.private_database {
        if database.is_empty() {
            return Err(MiniAppWholeAppBackupError::InvalidInput(
                "storage/private.sqlite must not be empty".into(),
            ));
        }
        enforce_size(
            "storage/private.sqlite",
            database.len() as u64,
            MAX_PRIVATE_DATABASE_BYTES,
        )?;
    }
    if let Some(ledger) = &storage.migration_ledger {
        ledger
            .validate()
            .map_err(|error| MiniAppWholeAppBackupError::InvalidInput(error.to_string()))?;
        if &ledger.miniapp_id != source_miniapp_id {
            return Err(MiniAppWholeAppBackupError::InvalidInput(
                "migration ledger belongs to another MiniApp".into(),
            ));
        }
        enforce_canonical_size(
            ledger,
            "storage/migration-ledger.json",
            MAX_JSON_BYTES,
        )?;
    }
    Ok(MiniAppBackupStorage {
        kv: storage.kv,
        files,
        private_database: storage.private_database,
        migration_ledger: storage.migration_ledger,
    })
}

#[allow(clippy::too_many_arguments)]
fn calculate_metadata(
    backup_id: MiniAppBackupId,
    source_miniapp_id: MiniAppId,
    owner_quiescent: bool,
    created_at_ms: i64,
    product: &Value,
    project: &Value,
    config: &Value,
    credential_slots: &Value,
    credential_slot_keys: BTreeSet<String>,
    releases: &BTreeMap<String, MiniAppBackupRelease>,
    source: Option<&MiniAppBackupSource>,
    storage: &MiniAppBackupStorage,
) -> Result<MiniAppWholeAppBackupMetadataV1, MiniAppWholeAppBackupError> {
    let product_metadata_digest = digest_canonical(&ProductMetadataDigestInput {
        product,
        project,
        credential_slots,
    })?;
    let source_archive_digest = match source {
        Some(source) => digest_canonical(&source_snapshot_record(source)?)?,
        None => digest_bytes(&[]),
    };
    let release_inventory = releases
        .iter()
        .map(|(slot, release)| ReleaseInventoryEntry {
            slot,
            artifact: &release.artifact,
        })
        .collect::<Vec<_>>();
    let release_inventory_digest = digest_canonical(&release_inventory)?;
    let config_digest = digest_canonical(config)?;
    let kv_digest = digest_canonical(&storage.kv)?;
    let files_digest = digest_canonical(&file_records(&storage.files))?;
    let private_database_digest = storage
        .private_database
        .as_deref()
        .map(digest_bytes)
        .unwrap_or_else(|| digest_bytes(&[]));
    let migration_ledger_digest = match &storage.migration_ledger {
        Some(ledger) => digest_canonical(ledger)?,
        None => digest_bytes(&[]),
    };
    let metadata = MiniAppWholeAppBackupMetadataV1 {
        schema_version: MINIAPP_M1_SCHEMA_VERSION.into(),
        backup_version: MINIAPP_WHOLE_APP_BACKUP_VERSION.into(),
        backup_id,
        source_miniapp_id,
        source_state: MiniAppBackupSourceState::Disabled,
        owner_quiescent,
        product_metadata_digest,
        source_archive_digest,
        release_inventory_digest,
        config_digest,
        kv_digest,
        files_digest,
        private_database_digest,
        migration_ledger_digest,
        credential_slot_keys,
        created_at_ms,
    };
    metadata
        .validate()
        .map_err(|error| MiniAppWholeAppBackupError::InvalidInput(error.to_string()))?;
    Ok(metadata)
}

fn write_backup_tree(
    root: &Path,
    prepared: &PreparedBackup,
) -> Result<(), MiniAppWholeAppBackupError> {
    write_new_synced(
        &root.join(PRODUCT_FILE),
        &canonical_bytes(&prepared.product)?,
    )?;
    write_new_synced(
        &root.join(PROJECT_FILE),
        &canonical_bytes(&prepared.project)?,
    )?;
    write_new_synced(
        &root.join(CONFIG_FILE),
        &canonical_bytes(&prepared.config)?,
    )?;
    write_new_synced(
        &root.join(CREDENTIAL_SLOTS_FILE),
        &canonical_bytes(&prepared.credential_slots)?,
    )?;

    let releases_root = root.join(RELEASES_DIRECTORY);
    fs::create_dir(&releases_root).map_err(|error| io_error(&releases_root, error))?;
    for (slot, release) in &prepared.releases {
        let release_root = releases_root.join(slot);
        fs::create_dir(&release_root).map_err(|error| io_error(&release_root, error))?;
        write_new_synced(
            &release_root.join(ARTIFACT_FILE),
            &canonical_bytes(&release.artifact)?,
        )?;
        write_new_synced(&release_root.join(MANIFEST_FILE), &release.manifest_bytes)?;
        write_file_tree(&release_root.join(FILES_DIRECTORY), &release.files)?;
    }

    if let Some(source) = &prepared.source {
        let source_root = root.join(SOURCE_DIRECTORY);
        fs::create_dir(&source_root).map_err(|error| io_error(&source_root, error))?;
        write_new_synced(
            &source_root.join(SOURCE_SNAPSHOT_FILE),
            &canonical_bytes(&source_snapshot_record(source)?)?,
        )?;
        write_new_synced(
            &source_root.join(DEPENDENCY_LOCK_FILE),
            &source.dependency_lock,
        )?;
        write_file_tree(&source_root.join(FILES_DIRECTORY), &source.files)?;
    }

    let storage_root = root.join(STORAGE_DIRECTORY);
    fs::create_dir(&storage_root).map_err(|error| io_error(&storage_root, error))?;
    write_new_synced(
        &storage_root.join(KV_FILE),
        &canonical_bytes(&prepared.storage.kv)?,
    )?;
    write_file_tree(
        &storage_root.join(FILES_DIRECTORY),
        &prepared.storage.files,
    )?;
    if let Some(database) = &prepared.storage.private_database {
        write_new_synced(&storage_root.join(PRIVATE_DATABASE_FILE), database)?;
    }
    if let Some(ledger) = &prepared.storage.migration_ledger {
        write_new_synced(
            &storage_root.join(MIGRATION_LEDGER_FILE),
            &canonical_bytes(ledger)?,
        )?;
    }
    write_new_synced(
        &root.join(METADATA_FILE),
        &canonical_bytes(&prepared.metadata)?,
    )
}

fn write_file_tree(
    root: &Path,
    files: &[MiniAppBackupFile],
) -> Result<(), MiniAppWholeAppBackupError> {
    fs::create_dir(root).map_err(|error| io_error(root, error))?;
    let mut created = BTreeSet::new();
    for file in files {
        let target = join_relative(root, &file.relative_path)?;
        let parent = target.parent().ok_or_else(|| {
            MiniAppWholeAppBackupError::InvalidInventory("backup file has no parent".into())
        })?;
        create_relative_directories(root, parent, &mut created)?;
        write_new_synced(&target, &file.bytes)?;
    }
    Ok(())
}

fn import_backup(root: &Path) -> Result<MiniAppWholeAppBackupImport, MiniAppWholeAppBackupError> {
    ensure_regular_directory(root)?;
    ensure_regular_directory_chain(root)?;
    let observed = scan_tree(root)?;
    let metadata: MiniAppWholeAppBackupMetadataV1 =
        read_canonical(&root.join(METADATA_FILE), MAX_JSON_BYTES)?;
    metadata
        .validate()
        .map_err(|error| MiniAppWholeAppBackupError::Tampered(error.to_string()))?;

    let product: Value = read_canonical(&root.join(PRODUCT_FILE), MAX_JSON_BYTES)?;
    let project: Value = read_canonical(&root.join(PROJECT_FILE), MAX_JSON_BYTES)?;
    let config: Value = read_canonical(&root.join(CONFIG_FILE), MAX_JSON_BYTES)?;
    let credential_slots: Value =
        read_canonical(&root.join(CREDENTIAL_SLOTS_FILE), MAX_JSON_BYTES)?;
    validate_object(&product, PRODUCT_FILE)?;
    validate_object(&project, PROJECT_FILE)?;
    validate_object(&config, CONFIG_FILE)?;
    let slot_keys = credential_slot_keys(&credential_slots)?;

    let mut releases = BTreeMap::new();
    for slot in discover_release_slots(&observed)? {
        let release_root = root.join(RELEASES_DIRECTORY).join(&slot);
        let artifact: MiniAppReleaseArtifactV1 =
            read_canonical(&release_root.join(ARTIFACT_FILE), MAX_JSON_BYTES)?;
        let manifest_bytes =
            read_regular_bounded(&release_root.join(MANIFEST_FILE), MAX_JSON_BYTES)?;
        let mut files = Vec::with_capacity(artifact.files.len());
        for file in &artifact.files {
            let path = join_relative(
                &release_root.join(FILES_DIRECTORY),
                &file.normalized_relative_path,
            )?;
            files.push(MiniAppBackupFile::new(
                file.normalized_relative_path.clone(),
                read_regular_bounded(&path, MAX_SINGLE_FILE_BYTES)?,
            ));
        }
        releases.insert(
            slot,
            prepare_release(MiniAppBackupRelease {
                artifact,
                manifest_bytes,
                files,
            })?,
        );
    }

    let source = import_source(root, &observed)?;
    let kv: Vec<Value> = read_canonical(
        &root.join(STORAGE_DIRECTORY).join(KV_FILE),
        MAX_KV_BYTES,
    )?;
    let storage_files =
        import_storage_files(&root.join(STORAGE_DIRECTORY), &observed)?;
    let storage_root = root.join(STORAGE_DIRECTORY);
    let has_database = observed.files.contains(&format!(
        "{STORAGE_DIRECTORY}/{PRIVATE_DATABASE_FILE}"
    ));
    let has_ledger = observed.files.contains(&format!(
        "{STORAGE_DIRECTORY}/{MIGRATION_LEDGER_FILE}"
    ));
    if has_database != has_ledger {
        return Err(MiniAppWholeAppBackupError::InvalidInventory(
            "Private Database bytes and migration ledger must be present or absent together"
                .into(),
        ));
    }
    let private_database = has_database
        .then(|| {
            read_regular_bounded(
                &storage_root.join(PRIVATE_DATABASE_FILE),
                MAX_PRIVATE_DATABASE_BYTES,
            )
        })
        .transpose()?;
    let migration_ledger = has_ledger
        .then(|| {
            read_canonical::<MiniAppMigrationLedger>(
                &storage_root.join(MIGRATION_LEDGER_FILE),
                MAX_JSON_BYTES,
            )
        })
        .transpose()?;
    let storage = prepare_storage(
        MiniAppBackupStorage {
            kv,
            files: storage_files,
            private_database,
            migration_ledger,
        },
        &metadata.source_miniapp_id,
    )?;

    let calculated = calculate_metadata(
        metadata.backup_id.clone(),
        metadata.source_miniapp_id.clone(),
        metadata.owner_quiescent,
        metadata.created_at_ms,
        &product,
        &project,
        &config,
        &credential_slots,
        slot_keys,
        &releases,
        source.as_ref(),
        &storage,
    )?;
    if calculated != metadata {
        return Err(MiniAppWholeAppBackupError::Tampered(
            "metadata.json does not match the imported payload".into(),
        ));
    }
    let expected = expected_inventory(&releases, source.as_ref(), &storage);
    require_exact_inventory(&observed, &expected)?;

    Ok(MiniAppWholeAppBackupImport {
        metadata,
        product,
        project,
        config,
        credential_slots,
        releases,
        source,
        storage,
    })
}

fn import_source(
    root: &Path,
    observed: &ObservedTree,
) -> Result<Option<MiniAppBackupSource>, MiniAppWholeAppBackupError> {
    let snapshot_path = format!("{SOURCE_DIRECTORY}/{SOURCE_SNAPSHOT_FILE}");
    let has_source = observed.files.contains(&snapshot_path);
    let has_any_source_entry = observed
        .files
        .iter()
        .chain(observed.directories.iter())
        .any(|path| path == SOURCE_DIRECTORY || path.starts_with("source/"));
    if !has_source {
        if has_any_source_entry {
            return Err(MiniAppWholeAppBackupError::InvalidInventory(
                "Source directory exists without source/snapshot.json".into(),
            ));
        }
        return Ok(None);
    }

    let source_root = root.join(SOURCE_DIRECTORY);
    let snapshot: BackupSourceSnapshotRecord =
        read_canonical(&source_root.join(SOURCE_SNAPSHOT_FILE), MAX_JSON_BYTES)?;
    if snapshot.format_version != MINIAPP_SOURCE_STORE_FORMAT_VERSION {
        return Err(MiniAppWholeAppBackupError::InvalidInput(
            "Source snapshot format version is unsupported".into(),
        ));
    }
    validate_file_records(&snapshot.files, "Source", MAX_SOURCE_BYTES, false)?;
    let dependency_lock = read_regular_bounded(
        &source_root.join(DEPENDENCY_LOCK_FILE),
        MAX_DEPENDENCY_LOCK_BYTES,
    )?;
    let mut files = Vec::with_capacity(snapshot.files.len());
    for file in &snapshot.files {
        let path = join_relative(
            &source_root.join(FILES_DIRECTORY),
            &file.normalized_relative_path,
        )?;
        files.push(MiniAppBackupFile::new(
            file.normalized_relative_path.clone(),
            read_regular_bounded(&path, MAX_SINGLE_FILE_BYTES)?,
        ));
    }
    let source = prepare_source(MiniAppBackupSource {
        source: snapshot.source.clone(),
        dependency_lock,
        files,
    })?;
    if source_snapshot_record(&source)? != snapshot {
        return Err(MiniAppWholeAppBackupError::Tampered(
            "source/snapshot.json does not match Source bytes".into(),
        ));
    }
    Ok(Some(source))
}

fn import_storage_files(
    storage_root: &Path,
    observed: &ObservedTree,
) -> Result<Vec<MiniAppBackupFile>, MiniAppWholeAppBackupError> {
    let prefix = format!("{STORAGE_DIRECTORY}/{FILES_DIRECTORY}/");
    let mut files = Vec::new();
    for path in observed.files.iter().filter(|path| path.starts_with(&prefix)) {
        let relative = path.strip_prefix(&prefix).ok_or_else(|| {
            MiniAppWholeAppBackupError::InvalidInventory(
                "Storage file path is outside storage/files".into(),
            )
        })?;
        files.push(MiniAppBackupFile::new(
            relative,
            read_regular_bounded(
                &join_relative(&storage_root.join(FILES_DIRECTORY), relative)?,
                MAX_SINGLE_FILE_BYTES,
            )?,
        ));
    }
    prepare_files(
        files,
        "Storage",
        MAX_SECTION_FILES,
        MAX_STORAGE_FILES_BYTES,
        true,
    )
}

fn discover_release_slots(
    observed: &ObservedTree,
) -> Result<Vec<String>, MiniAppWholeAppBackupError> {
    if !observed.directories.contains(RELEASES_DIRECTORY) {
        return Err(MiniAppWholeAppBackupError::InvalidInventory(
            "releases directory is missing".into(),
        ));
    }
    let prefix = format!("{RELEASES_DIRECTORY}/");
    let mut slots = observed
        .directories
        .iter()
        .filter_map(|path| path.strip_prefix(&prefix))
        .filter(|relative| !relative.is_empty() && !relative.contains('/'))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    slots.sort();
    if slots.len() > MAX_RELEASE_SLOTS {
        return Err(MiniAppWholeAppBackupError::Oversize(format!(
            "Release slot count {} exceeds {}",
            slots.len(),
            MAX_RELEASE_SLOTS
        )));
    }
    for slot in &slots {
        validate_release_slot(slot)?;
    }
    Ok(slots)
}

fn source_snapshot_record(
    source: &MiniAppBackupSource,
) -> Result<BackupSourceSnapshotRecord, MiniAppWholeAppBackupError> {
    let record = BackupSourceSnapshotRecord {
        format_version: MINIAPP_SOURCE_STORE_FORMAT_VERSION.into(),
        source: source.source.clone(),
        files: file_records(&source.files),
    };
    if source_snapshot_digest(&record.files)? != source.source.source_snapshot_digest {
        return Err(MiniAppWholeAppBackupError::Tampered(
            "Source snapshot digest does not match its file inventory".into(),
        ));
    }
    Ok(record)
}

fn source_snapshot_digest(
    files: &[BackupFileRecord],
) -> Result<DigestHex, MiniAppWholeAppBackupError> {
    digest_canonical(&SourceSnapshotDigestInput {
        format_version: MINIAPP_SOURCE_STORE_FORMAT_VERSION,
        files,
    })
}

fn file_records(files: &[MiniAppBackupFile]) -> Vec<BackupFileRecord> {
    files
        .iter()
        .map(|file| BackupFileRecord {
            normalized_relative_path: file.relative_path.clone(),
            digest: digest_bytes(&file.bytes),
            size_bytes: file.bytes.len() as u64,
        })
        .collect()
}

fn prepare_files(
    mut files: Vec<MiniAppBackupFile>,
    label: &str,
    max_count: usize,
    max_total_bytes: u64,
    allow_empty: bool,
) -> Result<Vec<MiniAppBackupFile>, MiniAppWholeAppBackupError> {
    if files.len() > max_count {
        return Err(MiniAppWholeAppBackupError::Oversize(format!(
            "{label} file count {} exceeds {max_count}",
            files.len()
        )));
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let mut collisions = BTreeSet::new();
    let mut previous: Option<&str> = None;
    let mut total = 0u64;
    for file in &files {
        validate_portable_relative_path(&file.relative_path)?;
        if previous.is_some_and(|previous| previous == file.relative_path) {
            return Err(MiniAppWholeAppBackupError::InvalidPath {
                path: file.relative_path.clone(),
                reason: "duplicate path".into(),
            });
        }
        if !collisions.insert(portable_collision_key(&file.relative_path)?) {
            return Err(MiniAppWholeAppBackupError::InvalidPath {
                path: file.relative_path.clone(),
                reason: "path collides under Windows case semantics".into(),
            });
        }
        if !allow_empty && file.bytes.is_empty() {
            return Err(MiniAppWholeAppBackupError::InvalidInput(format!(
                "{label} file {} must not be empty",
                file.relative_path
            )));
        }
        enforce_size(
            &format!("{label} file {}", file.relative_path),
            file.bytes.len() as u64,
            MAX_SINGLE_FILE_BYTES,
        )?;
        total = total.saturating_add(file.bytes.len() as u64);
        if total > max_total_bytes {
            return Err(MiniAppWholeAppBackupError::Oversize(format!(
                "{label} bytes {total} exceed {max_total_bytes}"
            )));
        }
        previous = Some(&file.relative_path);
    }
    Ok(files)
}

fn validate_file_records(
    records: &[BackupFileRecord],
    label: &str,
    max_total_bytes: u64,
    allow_empty: bool,
) -> Result<(), MiniAppWholeAppBackupError> {
    if records.len() > MAX_SECTION_FILES {
        return Err(MiniAppWholeAppBackupError::Oversize(format!(
            "{label} file count {} exceeds {}",
            records.len(),
            MAX_SECTION_FILES
        )));
    }
    let mut previous: Option<&str> = None;
    let mut collisions = BTreeSet::new();
    let mut total = 0u64;
    for record in records {
        validate_portable_relative_path(&record.normalized_relative_path)?;
        validate_digest(record.digest.as_ref(), "file digest")?;
        if previous.is_some_and(|previous| previous >= record.normalized_relative_path.as_str()) {
            return Err(MiniAppWholeAppBackupError::InvalidInventory(format!(
                "{label} file records must be sorted and unique"
            )));
        }
        if !collisions.insert(portable_collision_key(
            &record.normalized_relative_path,
        )?) {
            return Err(MiniAppWholeAppBackupError::InvalidPath {
                path: record.normalized_relative_path.clone(),
                reason: "path collides under Windows case semantics".into(),
            });
        }
        if !allow_empty && record.size_bytes == 0 {
            return Err(MiniAppWholeAppBackupError::InvalidInput(format!(
                "{label} file {} must not be empty",
                record.normalized_relative_path
            )));
        }
        enforce_size(
            &format!("{label} file {}", record.normalized_relative_path),
            record.size_bytes,
            MAX_SINGLE_FILE_BYTES,
        )?;
        total = total.saturating_add(record.size_bytes);
        if total > max_total_bytes {
            return Err(MiniAppWholeAppBackupError::Oversize(format!(
                "{label} bytes {total} exceed {max_total_bytes}"
            )));
        }
        previous = Some(&record.normalized_relative_path);
    }
    Ok(())
}

fn validate_source_contract(
    source: &MiniAppSourceBundle,
) -> Result<(), MiniAppWholeAppBackupError> {
    validate_nonempty(source.project_id.as_ref(), "source.project_id")?;
    validate_nonempty(
        source.source_archive_artifact_id.as_ref(),
        "source.source_archive_artifact_id",
    )?;
    validate_nonempty(
        source.dependency_lock_artifact_id.as_ref(),
        "source.dependency_lock_artifact_id",
    )?;
    validate_digest(
        source.source_snapshot_digest.as_ref(),
        "source.source_snapshot_digest",
    )?;
    validate_digest(
        source.dependency_lock_digest.as_ref(),
        "source.dependency_lock_digest",
    )?;
    if source.build_profile_version.as_ref() != MINIAPP_RELEASE_PROFILE_VERSION {
        return Err(MiniAppWholeAppBackupError::InvalidInput(format!(
            "source.build_profile_version must be {MINIAPP_RELEASE_PROFILE_VERSION}"
        )));
    }
    Ok(())
}

fn validate_object(value: &Value, label: &str) -> Result<(), MiniAppWholeAppBackupError> {
    if value.is_object() {
        Ok(())
    } else {
        Err(MiniAppWholeAppBackupError::InvalidInput(format!(
            "{label} must be a JSON object"
        )))
    }
}

fn credential_slot_keys(
    value: &Value,
) -> Result<BTreeSet<String>, MiniAppWholeAppBackupError> {
    let slots = value.as_array().ok_or_else(|| {
        MiniAppWholeAppBackupError::InvalidInput(
            "credential-slots.json must be a JSON array".into(),
        )
    })?;
    let mut keys = BTreeSet::new();
    for slot in slots {
        let object = slot.as_object().ok_or_else(|| {
            MiniAppWholeAppBackupError::InvalidInput(
                "each credential slot must be a JSON object".into(),
            )
        })?;
        let slot_key = object
            .get("slot_key")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                MiniAppWholeAppBackupError::InvalidInput(
                    "each credential slot requires a string slot_key".into(),
                )
            })?;
        validate_machine_key(slot_key, "credential slot_key")?;
        if !keys.insert(slot_key.to_owned()) {
            return Err(MiniAppWholeAppBackupError::InvalidInput(format!(
                "duplicate credential slot_key: {slot_key}"
            )));
        }
    }
    Ok(keys)
}

fn expected_inventory(
    releases: &BTreeMap<String, MiniAppBackupRelease>,
    source: Option<&MiniAppBackupSource>,
    storage: &MiniAppBackupStorage,
) -> ExpectedInventory {
    let mut files = BTreeSet::from([
        METADATA_FILE.to_owned(),
        PRODUCT_FILE.to_owned(),
        PROJECT_FILE.to_owned(),
        CONFIG_FILE.to_owned(),
        CREDENTIAL_SLOTS_FILE.to_owned(),
        format!("{STORAGE_DIRECTORY}/{KV_FILE}"),
    ]);
    let mut directories = BTreeSet::from([
        RELEASES_DIRECTORY.to_owned(),
        STORAGE_DIRECTORY.to_owned(),
        format!("{STORAGE_DIRECTORY}/{FILES_DIRECTORY}"),
    ]);
    for (slot, release) in releases {
        let root = format!("{RELEASES_DIRECTORY}/{slot}");
        directories.insert(root.clone());
        directories.insert(format!("{root}/{FILES_DIRECTORY}"));
        files.insert(format!("{root}/{ARTIFACT_FILE}"));
        files.insert(format!("{root}/{MANIFEST_FILE}"));
        files.extend(
            release
                .files
                .iter()
                .map(|file| format!("{root}/{FILES_DIRECTORY}/{}", file.relative_path)),
        );
    }
    if let Some(source) = source {
        directories.insert(SOURCE_DIRECTORY.to_owned());
        directories.insert(format!("{SOURCE_DIRECTORY}/{FILES_DIRECTORY}"));
        files.insert(format!("{SOURCE_DIRECTORY}/{SOURCE_SNAPSHOT_FILE}"));
        files.insert(format!("{SOURCE_DIRECTORY}/{DEPENDENCY_LOCK_FILE}"));
        files.extend(source.files.iter().map(|file| {
            format!(
                "{SOURCE_DIRECTORY}/{FILES_DIRECTORY}/{}",
                file.relative_path
            )
        }));
    }
    files.extend(storage.files.iter().map(|file| {
        format!(
            "{STORAGE_DIRECTORY}/{FILES_DIRECTORY}/{}",
            file.relative_path
        )
    }));
    if storage.private_database.is_some() {
        files.insert(format!(
            "{STORAGE_DIRECTORY}/{PRIVATE_DATABASE_FILE}"
        ));
        files.insert(format!(
            "{STORAGE_DIRECTORY}/{MIGRATION_LEDGER_FILE}"
        ));
    }
    for file in &files {
        add_parent_directories(file, &mut directories);
    }
    ExpectedInventory { files, directories }
}

fn add_parent_directories(file: &str, directories: &mut BTreeSet<String>) {
    let mut current = Path::new(file).parent();
    while let Some(parent) = current {
        if parent.as_os_str().is_empty() {
            break;
        }
        directories.insert(path_to_slashes(parent));
        current = parent.parent();
    }
}

fn require_exact_inventory(
    observed: &ObservedTree,
    expected: &ExpectedInventory,
) -> Result<(), MiniAppWholeAppBackupError> {
    if observed.files != expected.files {
        let extra = observed
            .files
            .difference(&expected.files)
            .cloned()
            .collect::<Vec<_>>();
        let missing = expected
            .files
            .difference(&observed.files)
            .cloned()
            .collect::<Vec<_>>();
        return Err(MiniAppWholeAppBackupError::InvalidInventory(format!(
            "file inventory differs; extra={extra:?}, missing={missing:?}"
        )));
    }
    if observed.directories != expected.directories {
        let extra = observed
            .directories
            .difference(&expected.directories)
            .cloned()
            .collect::<Vec<_>>();
        let missing = expected
            .directories
            .difference(&observed.directories)
            .cloned()
            .collect::<Vec<_>>();
        return Err(MiniAppWholeAppBackupError::InvalidInventory(format!(
            "directory inventory differs; extra={extra:?}, missing={missing:?}"
        )));
    }
    Ok(())
}

fn scan_tree(root: &Path) -> Result<ObservedTree, MiniAppWholeAppBackupError> {
    ensure_regular_directory(root)?;
    let canonical_root = fs::canonicalize(root).map_err(|error| io_error(root, error))?;
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let mut collision_keys = BTreeMap::new();
    let mut total_bytes = 0u64;
    scan_directory(
        &canonical_root,
        &canonical_root,
        &mut files,
        &mut directories,
        &mut collision_keys,
        &mut total_bytes,
    )?;
    Ok(ObservedTree { files, directories })
}

fn scan_directory(
    root: &Path,
    current: &Path,
    files: &mut BTreeSet<String>,
    directories: &mut BTreeSet<String>,
    collision_keys: &mut BTreeMap<String, String>,
    total_bytes: &mut u64,
) -> Result<(), MiniAppWholeAppBackupError> {
    for entry in fs::read_dir(current).map_err(|error| io_error(current, error))? {
        let entry = entry.map_err(|error| io_error(current, error))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if is_reparse_or_symlink(&metadata) {
            return Err(MiniAppWholeAppBackupError::InvalidInventory(format!(
                "symlinks, junctions, and reparse points are forbidden: {}",
                path.display()
            )));
        }
        let relative = path.strip_prefix(root).map_err(|_| {
            MiniAppWholeAppBackupError::InvalidInventory(
                "backup entry escaped its root".into(),
            )
        })?;
        let normalized = normalize_filesystem_path(relative)?;
        let collision_key = portable_collision_key(&normalized)?;
        if let Some(previous) = collision_keys.insert(collision_key, normalized.clone()) {
            return Err(MiniAppWholeAppBackupError::InvalidPath {
                path: normalized,
                reason: format!("case collision with {previous}"),
            });
        }
        if metadata.is_dir() {
            directories.insert(normalized);
            if directories.len() > MAX_BACKUP_DIRECTORIES {
                return Err(MiniAppWholeAppBackupError::Oversize(format!(
                    "directory count exceeds {MAX_BACKUP_DIRECTORIES}"
                )));
            }
            scan_directory(
                root,
                &path,
                files,
                directories,
                collision_keys,
                total_bytes,
            )?;
        } else if metadata.is_file() {
            enforce_size(
                &path.display().to_string(),
                metadata.len(),
                MAX_PRIVATE_DATABASE_BYTES,
            )?;
            *total_bytes = total_bytes.saturating_add(metadata.len());
            if *total_bytes > MAX_BACKUP_BYTES {
                return Err(MiniAppWholeAppBackupError::Oversize(format!(
                    "backup bytes exceed {MAX_BACKUP_BYTES}"
                )));
            }
            files.insert(normalized);
            if files.len() > MAX_BACKUP_FILES {
                return Err(MiniAppWholeAppBackupError::Oversize(format!(
                    "file count exceeds {MAX_BACKUP_FILES}"
                )));
            }
        } else {
            return Err(MiniAppWholeAppBackupError::InvalidInventory(format!(
                "special filesystem entries are forbidden: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn read_canonical<T: DeserializeOwned + Serialize>(
    path: &Path,
    limit: u64,
) -> Result<T, MiniAppWholeAppBackupError> {
    let bytes = read_regular_bounded(path, limit)?;
    parse_canonical_bytes(&bytes, limit).map_err(|error| match error {
        MiniAppWholeAppBackupError::InvalidInput(message) => {
            MiniAppWholeAppBackupError::Tampered(format!("{}: {message}", path.display()))
        }
        other => other,
    })
}

fn parse_canonical_bytes<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
    limit: u64,
) -> Result<T, MiniAppWholeAppBackupError> {
    enforce_size("canonical JSON", bytes.len() as u64, limit)?;
    let value: T = serde_json::from_slice(bytes)
        .map_err(|error| MiniAppWholeAppBackupError::InvalidInput(error.to_string()))?;
    if canonical_bytes(&value)? != bytes {
        return Err(MiniAppWholeAppBackupError::InvalidInput(
            "JSON must be byte-for-byte canonical".into(),
        ));
    }
    Ok(value)
}

fn read_regular_bounded(
    path: &Path,
    limit: u64,
) -> Result<Vec<u8>, MiniAppWholeAppBackupError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x00200000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(path)
        .map_err(|error| io_error(path, error))?;
    let metadata = file.metadata().map_err(|error| io_error(path, error))?;
    if is_reparse_or_symlink(&metadata) || !metadata.is_file() {
        return Err(MiniAppWholeAppBackupError::InvalidInventory(format!(
            "expected a regular file: {}",
            path.display()
        )));
    }
    enforce_size(&path.display().to_string(), metadata.len(), limit)?;
    let mut bytes = Vec::new();
    file
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| io_error(path, error))?;
    enforce_size(&path.display().to_string(), bytes.len() as u64, limit)?;
    if bytes.len() as u64 != metadata.len() {
        return Err(MiniAppWholeAppBackupError::Tampered(format!(
            "{} changed while being read",
            path.display()
        )));
    }
    Ok(bytes)
}

fn enforce_canonical_size<T: Serialize>(
    value: &T,
    label: &str,
    limit: u64,
) -> Result<(), MiniAppWholeAppBackupError> {
    enforce_size(label, canonical_bytes(value)?.len() as u64, limit)
}

fn enforce_size(
    label: &str,
    observed: u64,
    limit: u64,
) -> Result<(), MiniAppWholeAppBackupError> {
    if observed > limit {
        Err(MiniAppWholeAppBackupError::Oversize(format!(
            "{label} is {observed} bytes; limit is {limit}"
        )))
    } else {
        Ok(())
    }
}

fn digest_canonical<T: Serialize>(
    value: &T,
) -> Result<DigestHex, MiniAppWholeAppBackupError> {
    Ok(digest_bytes(&canonical_bytes(value)?))
}

fn canonical_bytes<T: Serialize>(
    value: &T,
) -> Result<Vec<u8>, MiniAppWholeAppBackupError> {
    canonical_json_bytes(value)
        .map_err(|error| MiniAppWholeAppBackupError::Canonical(error.to_string()))
}

fn validate_nonempty(value: &str, field: &str) -> Result<(), MiniAppWholeAppBackupError> {
    if value.is_empty() || value.trim() != value {
        Err(MiniAppWholeAppBackupError::InvalidInput(format!(
            "{field} must be non-empty and trimmed"
        )))
    } else {
        Ok(())
    }
}

fn validate_digest(value: &str, field: &str) -> Result<(), MiniAppWholeAppBackupError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(MiniAppWholeAppBackupError::InvalidInput(format!(
            "{field} must be a 64-character lowercase hexadecimal digest"
        )))
    }
}

fn validate_machine_key(value: &str, field: &str) -> Result<(), MiniAppWholeAppBackupError> {
    validate_nonempty(value, field)?;
    if value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        })
    {
        Ok(())
    } else {
        Err(MiniAppWholeAppBackupError::InvalidInput(format!(
            "{field} contains unsupported characters or exceeds 128 bytes"
        )))
    }
}

fn validate_release_slot(slot: &str) -> Result<(), MiniAppWholeAppBackupError> {
    validate_portable_relative_path(slot)?;
    if slot.contains('/') {
        return Err(MiniAppWholeAppBackupError::InvalidPath {
            path: slot.into(),
            reason: "Release slot must be one path component".into(),
        });
    }
    Ok(())
}

fn join_relative(root: &Path, relative: &str) -> Result<PathBuf, MiniAppWholeAppBackupError> {
    validate_portable_relative_path(relative)?;
    let target = relative
        .split('/')
        .fold(root.to_path_buf(), |path, component| path.join(component));
    if !target.starts_with(root) {
        return Err(MiniAppWholeAppBackupError::InvalidInventory(
            "relative path escaped its root".into(),
        ));
    }
    Ok(target)
}

fn normalize_filesystem_path(path: &Path) -> Result<String, MiniAppWholeAppBackupError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(MiniAppWholeAppBackupError::InvalidPath {
            path: path.display().to_string(),
            reason: "path must be a non-empty relative path".into(),
        });
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| {
                    MiniAppWholeAppBackupError::InvalidPath {
                        path: path.display().to_string(),
                        reason: "path must be valid UTF-8".into(),
                    }
                })?;
                components.push(value);
            }
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(MiniAppWholeAppBackupError::InvalidPath {
                    path: path.display().to_string(),
                    reason: "path contains traversal, root, or drive components".into(),
                });
            }
        }
    }
    let normalized = components.join("/");
    validate_portable_relative_path(&normalized)?;
    Ok(normalized)
}

fn validate_portable_relative_path(
    path: &str,
) -> Result<(), MiniAppWholeAppBackupError> {
    if path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || path.trim() != path
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path.contains(':')
    {
        return Err(MiniAppWholeAppBackupError::InvalidPath {
            path: path.into(),
            reason: "path must be normalized, relative, and slash-separated".into(),
        });
    }
    for component in path.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.len() > MAX_COMPONENT_BYTES
            || component.ends_with(['.', ' '])
            || is_windows_reserved_name(component)
            || component.chars().any(is_combining_mark)
        {
            return Err(MiniAppWholeAppBackupError::InvalidPath {
                path: path.into(),
                reason: "path is unstable under Windows case or Unicode semantics".into(),
            });
        }
    }
    Ok(())
}

fn portable_collision_key(
    path: &str,
) -> Result<String, MiniAppWholeAppBackupError> {
    validate_portable_relative_path(path)?;
    Ok(path.chars().flat_map(char::to_lowercase).collect())
}

fn is_windows_reserved_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn is_combining_mark(character: char) -> bool {
    matches!(
        character as u32,
        0x0300..=0x036f
            | 0x1ab0..=0x1aff
            | 0x1dc0..=0x1dff
            | 0x20d0..=0x20ff
            | 0xfe20..=0xfe2f
    )
}

fn ensure_normal_destination(path: &Path) -> Result<(), MiniAppWholeAppBackupError> {
    let name = path.file_name().and_then(|value| value.to_str()).ok_or_else(|| {
        MiniAppWholeAppBackupError::InvalidInput(
            "backup destination must have a UTF-8 final component".into(),
        )
    })?;
    validate_release_slot(name)
}

fn ensure_regular_directory(path: &Path) -> Result<(), MiniAppWholeAppBackupError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
        return Err(MiniAppWholeAppBackupError::InvalidInventory(format!(
            "directory is a symlink, junction, reparse point, or non-directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn ensure_regular_directory_chain(
    path: &Path,
) -> Result<(), MiniAppWholeAppBackupError> {
    // Resolve stable host aliases (for example macOS `/var` -> `/private/var`)
    // before walking ancestors. The caller already rejects a symlink at the
    // owned root itself; checking the canonical chain keeps that boundary
    // without making every temp-directory-backed backup invalid on macOS.
    let canonical = fs::canonicalize(path).map_err(|error| io_error(path, error))?;
    let mut current = Some(canonical.as_path());
    while let Some(candidate) = current {
        ensure_regular_directory(candidate)?;
        current = candidate.parent();
    }
    Ok(())
}

fn create_relative_directories(
    root: &Path,
    parent: &Path,
    created: &mut BTreeSet<PathBuf>,
) -> Result<(), MiniAppWholeAppBackupError> {
    let relative = parent.strip_prefix(root).map_err(|_| {
        MiniAppWholeAppBackupError::InvalidInventory("file parent escaped its root".into())
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(MiniAppWholeAppBackupError::InvalidInventory(
                "file parent contains a non-normal component".into(),
            ));
        };
        current.push(component);
        if created.insert(current.clone()) {
            fs::create_dir(&current).map_err(|error| io_error(&current, error))?;
        }
    }
    Ok(())
}

fn write_new_synced(
    path: &Path,
    bytes: &[u8],
) -> Result<(), MiniAppWholeAppBackupError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| io_error(path, error))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(path, error))
}

fn sync_tree_directories(root: &Path) -> Result<(), MiniAppWholeAppBackupError> {
    let mut directories = Vec::new();
    collect_directories(root, &mut directories)?;
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        sync_directory_if_supported(&directory)?;
    }
    Ok(())
}

fn collect_directories(
    root: &Path,
    directories: &mut Vec<PathBuf>,
) -> Result<(), MiniAppWholeAppBackupError> {
    ensure_regular_directory(root)?;
    directories.push(root.to_path_buf());
    for entry in fs::read_dir(root).map_err(|error| io_error(root, error))? {
        let entry = entry.map_err(|error| io_error(root, error))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if is_reparse_or_symlink(&metadata) {
            return Err(MiniAppWholeAppBackupError::InvalidInventory(
                "staged backup contains a symlink or reparse point".into(),
            ));
        }
        if metadata.is_dir() {
            collect_directories(&path, directories)?;
        } else if !metadata.is_file() {
            return Err(MiniAppWholeAppBackupError::InvalidInventory(
                "staged backup contains a special file".into(),
            ));
        }
    }
    Ok(())
}

fn validate_removal_tree(root: &Path) -> Result<(), MiniAppWholeAppBackupError> {
    ensure_regular_directory(root)?;
    for entry in fs::read_dir(root).map_err(|error| io_error(root, error))? {
        let entry = entry.map_err(|error| io_error(root, error))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if is_reparse_or_symlink(&metadata) {
            return Err(MiniAppWholeAppBackupError::InvalidInventory(format!(
                "cleanup target contains a symlink or reparse point: {}",
                path.display()
            )));
        }
        if metadata.is_dir() {
            validate_removal_tree(&path)?;
        } else if !metadata.is_file() {
            return Err(MiniAppWholeAppBackupError::InvalidInventory(format!(
                "cleanup target contains a special file: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_or_symlink(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_or_symlink(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn path_to_slashes(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn sync_directory_if_supported(path: &Path) -> Result<(), MiniAppWholeAppBackupError> {
    #[cfg(unix)]
    {
        match fs::File::open(path).and_then(|file| file.sync_all()) {
            Ok(()) => Ok(()),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
                ) =>
            {
                Ok(())
            }
            Err(error) => Err(io_error(path, error)),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn io_error(path: impl Into<PathBuf>, source: io::Error) -> MiniAppWholeAppBackupError {
    MiniAppWholeAppBackupError::Io {
        path: path.into(),
        source,
    }
}

struct StagingGuard {
    parent: PathBuf,
    path: Option<PathBuf>,
}

impl StagingGuard {
    fn allocate(parent: &Path) -> Result<Self, MiniAppWholeAppBackupError> {
        let path = parent.join(format!("{STAGING_PREFIX}{}", Uuid::now_v7()));
        fs::create_dir(&path).map_err(|error| io_error(&path, error))?;
        Ok(Self {
            parent: parent.to_path_buf(),
            path: Some(path),
        })
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("staging guard must be armed")
    }

    fn disarm(&mut self) {
        self.path = None;
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        if path.parent() != Some(self.parent.as_path()) {
            return;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            return;
        };
        let Some(id) = name.strip_prefix(STAGING_PREFIX) else {
            return;
        };
        if Uuid::parse_str(id).is_err() || validate_removal_tree(&path).is_err() {
            return;
        }
        let _ = fs::remove_dir_all(&path);
        let _ = sync_directory_if_supported(&self.parent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    struct Fixture {
        temp: TempDir,
        product: Value,
        project: Value,
        config: Value,
        credential_slots: Value,
        releases: BTreeMap<String, MiniAppBackupRelease>,
        storage: MiniAppBackupStorage,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                temp: tempfile::tempdir().unwrap(),
                product: json!({
                    "display_name": "Backup fixture",
                    "lifecycle": "disabled",
                    "miniapp_id": "miniapp-source"
                }),
                project: json!({
                    "project_id": "project-source",
                    "source_state": "empty"
                }),
                config: json!({
                    "revision": 1,
                    "values": {}
                }),
                credential_slots: json!([{
                    "display_name": "API key",
                    "kind": "secret_text",
                    "required": false,
                    "slot_key": "api_key"
                }]),
                releases: BTreeMap::new(),
                storage: MiniAppBackupStorage {
                    kv: vec![json!({"key": "theme", "value": "dark"})],
                    files: vec![MiniAppBackupFile::new(
                        "state/current.bin",
                        b"state".to_vec(),
                    )],
                    private_database: None,
                    migration_ledger: None,
                },
            }
        }

        fn destination(&self, name: &str) -> PathBuf {
            self.temp.path().join(name)
        }

        fn request(&self) -> MiniAppWholeAppBackupExport<'_> {
            MiniAppWholeAppBackupExport {
                backup_id: MiniAppBackupId::from("backup-1"),
                source_miniapp_id: MiniAppId::from("miniapp-source"),
                owner_quiescent: true,
                created_at_ms: 1,
                product: &self.product,
                project: &self.project,
                config: &self.config,
                credential_slots: &self.credential_slots,
                releases: &self.releases,
                source: None,
                storage: &self.storage,
            }
        }

        fn export(&self, destination: &Path) -> MiniAppWholeAppBackupMetadataV1 {
            MiniAppWholeAppBackupFilesystem::new()
                .export(self.request(), destination)
                .unwrap()
        }
    }

    #[test]
    fn whole_app_backup_roundtrips_and_never_overwrites() {
        let fixture = Fixture::new();
        let destination = fixture.destination("backup");
        let metadata = fixture.export(&destination);
        let imported = MiniAppWholeAppBackupFilesystem::new()
            .import(&destination)
            .unwrap();

        assert_eq!(imported.metadata, metadata);
        assert_eq!(imported.product, fixture.product);
        assert_eq!(imported.project, fixture.project);
        assert_eq!(imported.config, fixture.config);
        assert_eq!(imported.credential_slots, fixture.credential_slots);
        assert_eq!(imported.releases, fixture.releases);
        assert_eq!(imported.source, None);
        assert_eq!(imported.storage, fixture.storage);
        assert_eq!(
            imported.metadata.credential_slot_keys,
            BTreeSet::from(["api_key".to_owned()])
        );
        assert!(matches!(
            MiniAppWholeAppBackupFilesystem::new().export(
                fixture.request(),
                &destination
            ),
            Err(MiniAppWholeAppBackupError::DestinationExists(_))
        ));
    }

    #[test]
    fn whole_app_backup_rejects_tampered_bytes() {
        let fixture = Fixture::new();
        let destination = fixture.destination("tampered");
        fixture.export(&destination);
        fs::write(
            destination.join("storage/files/state/current.bin"),
            b"changed",
        )
        .unwrap();

        assert!(matches!(
            MiniAppWholeAppBackupFilesystem::new().import(&destination),
            Err(MiniAppWholeAppBackupError::Tampered(_))
        ));
    }

    #[test]
    fn whole_app_backup_rejects_extra_files() {
        let fixture = Fixture::new();
        let destination = fixture.destination("extra");
        fixture.export(&destination);
        fs::write(destination.join("unexpected.txt"), b"extra").unwrap();

        assert!(matches!(
            MiniAppWholeAppBackupFilesystem::new().import(&destination),
            Err(MiniAppWholeAppBackupError::InvalidInventory(_))
        ));
    }

    #[cfg(windows)]
    #[test]
    fn whole_app_backup_rejects_windows_junctions() {
        let fixture = Fixture::new();
        let destination = fixture.destination("junction");
        fixture.export(&destination);
        let outside = fixture.destination("outside");
        fs::create_dir(&outside).unwrap();
        let injected = destination.join("storage/files/linked");
        junction::create(&outside, &injected).unwrap();

        assert!(matches!(
            MiniAppWholeAppBackupFilesystem::new().import(&destination),
            Err(MiniAppWholeAppBackupError::InvalidInventory(_))
        ));
        junction::delete(&injected).unwrap();
    }
}
