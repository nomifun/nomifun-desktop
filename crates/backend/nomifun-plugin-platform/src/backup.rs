//! Safe filesystem format for the two Unified Plugin Core transfers.
//!
//! A Plugin Package is the canonical package tree accepted by the Artifact
//! Store. A Plugin Backup adds exactly one current DataRoot generation and
//! non-secret host metadata. This module does not create another lifecycle or
//! installation state machine.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use nomifun_agent_contracts::{
    DigestHex, PLUGIN_MANIFEST_PATH, PluginArtifact, PluginArtifactFile, PluginId,
    PluginManifest, canonical_json_bytes, digest_bytes, digest_payload,
};
use serde::de::{DeserializeOwned, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Number, Value};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;

const BACKUP_SCHEMA: &str = "nomifun.plugin-backup/v1";
const BACKUP_MANIFEST: &str = "nomifun.plugin-backup.json";
const PACKAGE_DIRECTORY: &str = "package";
const DATA_DIRECTORY: &str = "data";
const DATA_SQLITE: &str = "data/data.sqlite";
const DATA_FILES_DIRECTORY: &str = "data/files";
const CONFIG_FILE: &str = "config.json";
const GRANTS_FILE: &str = "grants.json";
const CREDENTIAL_SLOTS_FILE: &str = "credential-slots.json";
const SQLITE_HEADER: &[u8] = b"SQLite format 3\0";
const MAX_PATH_BYTES: usize = 1_024;
const MAX_COMPONENT_BYTES: usize = 255;
const STAGING_PREFIX: &str = ".nomifun-plugin-transfer-";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluginTransferLimits {
    pub max_files: usize,
    pub max_single_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_zip_bytes: u64,
    pub max_json_bytes: u64,
}

impl Default for PluginTransferLimits {
    fn default() -> Self {
        Self {
            max_files: 32_768,
            max_single_file_bytes: 512 * 1024 * 1024,
            max_total_bytes: 2 * 1024 * 1024 * 1024,
            max_zip_bytes: 1024 * 1024 * 1024,
            max_json_bytes: 16 * 1024 * 1024,
        }
    }
}

impl PluginTransferLimits {
    fn validate(self) -> PluginTransferResult<Self> {
        if self.max_files < 2
            || self.max_single_file_bytes == 0
            || self.max_total_bytes < self.max_single_file_bytes
            || self.max_zip_bytes == 0
            || self.max_json_bytes == 0
        {
            return Err(PluginTransferError::InvalidInput(
                "Plugin transfer limits are invalid".into(),
            ));
        }
        Ok(self)
    }
}

#[derive(Clone, Debug)]
pub struct PluginPackageExport<'a> {
    pub artifact_digest: &'a DigestHex,
    pub package_root: &'a Path,
    pub include_source: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportedPluginPackage {
    pub artifact: PluginArtifact,
    /// Canonical package-relative bytes suitable for
    /// `PluginArtifactStore::import_files`.
    pub files: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginGrantMetadata {
    pub permission: String,
    pub granted: bool,
}

#[derive(Clone, Debug)]
pub struct PluginBackupExport<'a> {
    pub plugin_id: &'a PluginId,
    pub artifact_digest: &'a DigestHex,
    pub generation: &'a str,
    pub package_root: &'a Path,
    /// The exact current generation directory containing `data.sqlite` and
    /// `files/`. Cache, Preview, staging, and previous generations are not
    /// accepted here.
    pub generation_root: &'a Path,
    pub config: &'a Value,
    pub grants: &'a [PluginGrantMetadata],
    pub credential_slots: &'a BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportedPluginBackup {
    pub plugin_id: PluginId,
    pub artifact_digest: DigestHex,
    pub generation: String,
    pub package: ImportedPluginPackage,
    /// SQLite snapshot suitable for a newly staged DataRoot generation.
    pub data_sqlite: Vec<u8>,
    /// Files-relative bytes suitable for a newly staged DataRoot `files/`.
    pub files: BTreeMap<String, Vec<u8>>,
    pub config: Value,
    pub grants: Vec<PluginGrantMetadata>,
    pub credential_slots: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginBackupDescriptor {
    pub plugin_id: PluginId,
    pub artifact_digest: DigestHex,
    pub generation: String,
    pub bundle_digest: DigestHex,
}

pub type PluginTransferResult<T> = Result<T, PluginTransferError>;

#[derive(Debug, Error)]
pub enum PluginTransferError {
    #[error("invalid Plugin transfer input: {0}")]
    InvalidInput(String),
    #[error("unsafe Plugin transfer path {path}: {reason}")]
    UnsafePath { path: String, reason: String },
    #[error("unsupported Plugin transfer entry: {0}")]
    UnsupportedEntry(String),
    #[error("duplicate or Windows-colliding Plugin transfer entry: {0}")]
    DuplicateEntry(String),
    #[error("Plugin transfer exceeds a limit: {0}")]
    LimitExceeded(String),
    #[error("Plugin transfer destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("Plugin transfer was modified or is inconsistent: {0}")]
    Tampered(String),
    #[error("invalid Plugin transfer ZIP: {0}")]
    InvalidZip(String),
    #[error("Plugin transfer I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Plugin transfer JSON failed: {0}")]
    Json(String),
}

#[derive(Clone, Debug)]
pub struct PluginBackupFilesystem {
    staging_root: PathBuf,
    limits: PluginTransferLimits,
}

impl PluginBackupFilesystem {
    pub fn new(staging_root: impl AsRef<Path>) -> PluginTransferResult<Self> {
        Self::with_limits(staging_root, PluginTransferLimits::default())
    }

    pub fn with_limits(
        staging_root: impl AsRef<Path>,
        limits: PluginTransferLimits,
    ) -> PluginTransferResult<Self> {
        let limits = limits.validate()?;
        ensure_real_directory(staging_root.as_ref())?;
        let staging_root = fs::canonicalize(staging_root.as_ref())
            .map_err(|source| io_error(staging_root.as_ref(), source))?;
        Ok(Self {
            staging_root,
            limits,
        })
    }

    pub fn staging_root(&self) -> &Path {
        &self.staging_root
    }

    pub fn export_package_directory(
        &self,
        request: PluginPackageExport<'_>,
        destination: impl AsRef<Path>,
    ) -> PluginTransferResult<PluginArtifact> {
        let package = self.prepare_package_export(request)?;
        atomic_write_directory(
            destination.as_ref(),
            &TreeSnapshot::from_files(package.files.clone()),
        )?;
        Ok(package.artifact)
    }

    pub fn export_package_zip(
        &self,
        request: PluginPackageExport<'_>,
        destination: impl AsRef<Path>,
    ) -> PluginTransferResult<PluginArtifact> {
        let package = self.prepare_package_export(request)?;
        atomic_write_zip(
            destination.as_ref(),
            &TreeSnapshot::from_files(package.files.clone()),
        )?;
        Ok(package.artifact)
    }

    pub fn import_package_directory(
        &self,
        source: impl AsRef<Path>,
    ) -> PluginTransferResult<ImportedPluginPackage> {
        let snapshot = scan_directory(source.as_ref(), self.limits)?;
        validate_package_tree(&snapshot)?;
        let staged = self.stage_snapshot(&snapshot)?;
        package_from_snapshot(staged.snapshot())
    }

    pub fn import_package_zip(
        &self,
        source: impl AsRef<Path>,
    ) -> PluginTransferResult<ImportedPluginPackage> {
        let snapshot = scan_zip(source.as_ref(), self.limits)?;
        validate_package_tree(&snapshot)?;
        let staged = self.stage_snapshot(&snapshot)?;
        package_from_snapshot(staged.snapshot())
    }

    pub fn export_backup_directory(
        &self,
        request: PluginBackupExport<'_>,
        destination: impl AsRef<Path>,
    ) -> PluginTransferResult<PluginBackupDescriptor> {
        let (descriptor, snapshot) = self.prepare_backup_export(request)?;
        atomic_write_directory(destination.as_ref(), &snapshot)?;
        Ok(descriptor)
    }

    pub fn export_backup_zip(
        &self,
        request: PluginBackupExport<'_>,
        destination: impl AsRef<Path>,
    ) -> PluginTransferResult<PluginBackupDescriptor> {
        let (descriptor, snapshot) = self.prepare_backup_export(request)?;
        atomic_write_zip(destination.as_ref(), &snapshot)?;
        Ok(descriptor)
    }

    pub fn import_backup_directory(
        &self,
        source: impl AsRef<Path>,
    ) -> PluginTransferResult<ImportedPluginBackup> {
        let snapshot = scan_directory(source.as_ref(), self.limits)?;
        validate_backup_tree(&snapshot)?;
        let staged = self.stage_snapshot(&snapshot)?;
        backup_from_snapshot(staged.snapshot(), self.limits)
    }

    pub fn import_backup_zip(
        &self,
        source: impl AsRef<Path>,
    ) -> PluginTransferResult<ImportedPluginBackup> {
        let snapshot = scan_zip(source.as_ref(), self.limits)?;
        validate_backup_tree(&snapshot)?;
        let staged = self.stage_snapshot(&snapshot)?;
        backup_from_snapshot(staged.snapshot(), self.limits)
    }

    fn prepare_package_export(
        &self,
        request: PluginPackageExport<'_>,
    ) -> PluginTransferResult<ImportedPluginPackage> {
        validate_digest(request.artifact_digest)?;
        let snapshot = scan_directory(request.package_root, self.limits)?;
        validate_package_tree(&snapshot)?;
        let current = package_from_snapshot(&snapshot)?;
        if &current.artifact.artifact_digest != request.artifact_digest {
            return Err(PluginTransferError::Tampered(
                "package tree does not match active artifact_digest".into(),
            ));
        }
        if request.include_source {
            return Ok(current);
        }
        let filtered = TreeSnapshot::from_files(
            current
                .files
                .into_iter()
                .filter(|(path, _)| !path.starts_with("source/"))
                .collect(),
        );
        package_from_snapshot(&filtered)
    }

    fn prepare_backup_export(
        &self,
        request: PluginBackupExport<'_>,
    ) -> PluginTransferResult<(PluginBackupDescriptor, TreeSnapshot)> {
        validate_plugin_id(request.plugin_id)?;
        validate_digest(request.artifact_digest)?;
        validate_generation(request.generation)?;

        let package_snapshot = scan_directory(request.package_root, self.limits)?;
        validate_package_tree(&package_snapshot)?;
        let package = package_from_snapshot(&package_snapshot)?;
        if &package.artifact.artifact_digest != request.artifact_digest {
            return Err(PluginTransferError::Tampered(
                "backup package does not match active artifact_digest".into(),
            ));
        }

        let data = scan_directory(request.generation_root, self.limits)?;
        validate_generation_tree(&data)?;
        let data_sqlite = data
            .files
            .get("data.sqlite")
            .ok_or_else(|| PluginTransferError::InvalidInput("data.sqlite is required".into()))?;
        validate_sqlite(data_sqlite)?;
        validate_config(request.config, request.credential_slots)?;
        let grants = normalized_grants(request.grants)?;
        validate_credential_slots(request.credential_slots)?;

        let mut files = BTreeMap::new();
        for (path, bytes) in package.files {
            files.insert(format!("{PACKAGE_DIRECTORY}/{path}"), bytes);
        }
        files.insert(DATA_SQLITE.into(), data_sqlite.clone());
        for (path, bytes) in data.files {
            if let Some(relative) = path.strip_prefix("files/") {
                files.insert(format!("{DATA_FILES_DIRECTORY}/{relative}"), bytes);
            }
        }
        files.insert(CONFIG_FILE.into(), canonical_bytes(request.config)?);
        files.insert(GRANTS_FILE.into(), canonical_bytes(&grants)?);
        files.insert(
            CREDENTIAL_SLOTS_FILE.into(),
            canonical_bytes(request.credential_slots)?,
        );

        let records = file_records(&files);
        let mut manifest = PluginBackupManifest {
            schema: BACKUP_SCHEMA.into(),
            plugin_id: request.plugin_id.clone(),
            artifact_digest: request.artifact_digest.clone(),
            generation: request.generation.to_owned(),
            files: records,
            bundle_digest: DigestHex::from(String::new()),
        };
        manifest.bundle_digest = manifest.computed_digest()?;
        let descriptor = PluginBackupDescriptor {
            plugin_id: manifest.plugin_id.clone(),
            artifact_digest: manifest.artifact_digest.clone(),
            generation: manifest.generation.clone(),
            bundle_digest: manifest.bundle_digest.clone(),
        };
        files.insert(BACKUP_MANIFEST.into(), canonical_bytes(&manifest)?);
        let mut snapshot = TreeSnapshot::from_files(files);
        snapshot.directories.insert(DATA_FILES_DIRECTORY.into());
        validate_backup_tree(&snapshot)?;
        Ok((descriptor, snapshot))
    }

    fn stage_snapshot(&self, snapshot: &TreeSnapshot) -> PluginTransferResult<StagedSnapshot> {
        let guard = StagingDirectory::allocate(&self.staging_root)?;
        write_tree(guard.path(), snapshot)?;
        sync_tree(guard.path())?;
        let observed = scan_directory(guard.path(), self.limits)?;
        if observed.files != snapshot.files {
            return Err(PluginTransferError::Tampered(
                "staged transfer differs from captured input".into(),
            ));
        }
        Ok(StagedSnapshot {
            _guard: guard,
            snapshot: observed,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleFileRecord {
    path: String,
    digest: DigestHex,
    size_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginBackupManifest {
    schema: String,
    plugin_id: PluginId,
    artifact_digest: DigestHex,
    generation: String,
    files: Vec<BundleFileRecord>,
    bundle_digest: DigestHex,
}

#[derive(Serialize)]
struct BackupDigestInput<'a> {
    schema: &'a str,
    plugin_id: &'a PluginId,
    artifact_digest: &'a DigestHex,
    generation: &'a str,
    files: &'a [BundleFileRecord],
}

impl PluginBackupManifest {
    fn computed_digest(&self) -> PluginTransferResult<DigestHex> {
        digest_payload(&BackupDigestInput {
            schema: &self.schema,
            plugin_id: &self.plugin_id,
            artifact_digest: &self.artifact_digest,
            generation: &self.generation,
            files: &self.files,
        })
        .map_err(|error| PluginTransferError::Json(error.to_string()))
    }

    fn validate(&self) -> PluginTransferResult<()> {
        if self.schema != BACKUP_SCHEMA {
            return Err(PluginTransferError::InvalidInput(
                "backup schema is not nomifun.plugin-backup/v1".into(),
            ));
        }
        validate_plugin_id(&self.plugin_id)?;
        validate_digest(&self.artifact_digest)?;
        validate_generation(&self.generation)?;
        validate_records(&self.files)?;
        validate_digest(&self.bundle_digest)?;
        if self.computed_digest()? != self.bundle_digest {
            return Err(PluginTransferError::Tampered(
                "backup manifest digest mismatch".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TreeSnapshot {
    files: BTreeMap<String, Vec<u8>>,
    directories: BTreeSet<String>,
}

impl TreeSnapshot {
    fn from_files(files: BTreeMap<String, Vec<u8>>) -> Self {
        let mut directories = BTreeSet::new();
        for path in files.keys() {
            let parts = path.split('/').collect::<Vec<_>>();
            for end in 1..parts.len() {
                directories.insert(parts[..end].join("/"));
            }
        }
        Self { files, directories }
    }
}

struct StagedSnapshot {
    _guard: StagingDirectory,
    snapshot: TreeSnapshot,
}

impl StagedSnapshot {
    fn snapshot(&self) -> &TreeSnapshot {
        &self.snapshot
    }
}

fn package_from_snapshot(snapshot: &TreeSnapshot) -> PluginTransferResult<ImportedPluginPackage> {
    validate_package_tree(snapshot)?;
    let manifest_bytes = snapshot
        .files
        .get(PLUGIN_MANIFEST_PATH)
        .ok_or_else(|| PluginTransferError::InvalidInput("nomifun.plugin.json is required".into()))?;
    let manifest: PluginManifest = strict_json_from_slice(manifest_bytes)?;
    manifest
        .validate()
        .map_err(|error| PluginTransferError::InvalidInput(error.to_string()))?;
    let mut files = snapshot.files.clone();
    files.insert(PLUGIN_MANIFEST_PATH.into(), canonical_bytes(&manifest)?);
    let artifact_files = files
        .iter()
        .map(|(path, bytes)| PluginArtifactFile {
            normalized_relative_path: path.clone(),
            digest: digest_bytes(bytes),
            size_bytes: bytes.len() as u64,
        })
        .collect();
    let artifact = PluginArtifact::new(manifest, artifact_files)
        .map_err(|error| PluginTransferError::InvalidInput(error.to_string()))?;
    Ok(ImportedPluginPackage { artifact, files })
}

fn backup_from_snapshot(
    snapshot: &TreeSnapshot,
    limits: PluginTransferLimits,
) -> PluginTransferResult<ImportedPluginBackup> {
    validate_backup_tree(snapshot)?;
    let manifest: PluginBackupManifest = parse_canonical(
        snapshot
            .files
            .get(BACKUP_MANIFEST)
            .ok_or_else(|| PluginTransferError::InvalidInput("backup manifest is required".into()))?,
        limits.max_json_bytes,
    )?;
    manifest.validate()?;

    let body = snapshot
        .files
        .iter()
        .filter(|(path, _)| path.as_str() != BACKUP_MANIFEST)
        .map(|(path, bytes)| (path.clone(), bytes.clone()))
        .collect::<BTreeMap<_, _>>();
    if file_records(&body) != manifest.files {
        return Err(PluginTransferError::Tampered(
            "backup file inventory or digest mismatch".into(),
        ));
    }

    let package_files = body
        .iter()
        .filter_map(|(path, bytes)| {
            path.strip_prefix("package/")
                .map(|relative| (relative.to_owned(), bytes.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let package = package_from_snapshot(&TreeSnapshot::from_files(package_files))?;
    if package.artifact.artifact_digest != manifest.artifact_digest {
        return Err(PluginTransferError::Tampered(
            "backup package differs from artifact_digest".into(),
        ));
    }

    let data_sqlite = body
        .get(DATA_SQLITE)
        .cloned()
        .ok_or_else(|| PluginTransferError::InvalidInput("backup data.sqlite is required".into()))?;
    validate_sqlite(&data_sqlite)?;
    let files = body
        .iter()
        .filter_map(|(path, bytes)| {
            path.strip_prefix("data/files/")
                .map(|relative| (relative.to_owned(), bytes.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let config: Value = parse_canonical(
        required_file(&body, CONFIG_FILE)?,
        limits.max_json_bytes,
    )?;
    let grants: Vec<PluginGrantMetadata> = parse_canonical(
        required_file(&body, GRANTS_FILE)?,
        limits.max_json_bytes,
    )?;
    let credential_slots: BTreeSet<String> = parse_canonical(
        required_file(&body, CREDENTIAL_SLOTS_FILE)?,
        limits.max_json_bytes,
    )?;
    validate_config(&config, &credential_slots)?;
    let grants = normalized_grants(&grants)?;
    validate_credential_slots(&credential_slots)?;

    Ok(ImportedPluginBackup {
        plugin_id: manifest.plugin_id,
        artifact_digest: manifest.artifact_digest,
        generation: manifest.generation,
        package,
        data_sqlite,
        files,
        config,
        grants,
        credential_slots,
    })
}

fn required_file<'a>(
    files: &'a BTreeMap<String, Vec<u8>>,
    path: &str,
) -> PluginTransferResult<&'a [u8]> {
    files
        .get(path)
        .map(Vec::as_slice)
        .ok_or_else(|| PluginTransferError::InvalidInput(format!("{path} is required")))
}

fn validate_package_tree(snapshot: &TreeSnapshot) -> PluginTransferResult<()> {
    for path in snapshot.files.keys() {
        validate_package_file(path)?;
    }
    for path in &snapshot.directories {
        validate_package_directory(path)?;
    }
    Ok(())
}

fn validate_package_file(path: &str) -> PluginTransferResult<()> {
    if path == PLUGIN_MANIFEST_PATH
        || path == "service/main.mjs"
        || path
            .strip_prefix("ui/")
            .is_some_and(|relative| !relative.is_empty())
        || path
            .strip_prefix("migrations/")
            .is_some_and(|relative| !relative.is_empty() && relative.ends_with(".mjs"))
        || path
            .strip_prefix("source/")
            .is_some_and(|relative| !relative.is_empty())
    {
        Ok(())
    } else {
        Err(PluginTransferError::UnsupportedEntry(path.into()))
    }
}

fn validate_package_directory(path: &str) -> PluginTransferResult<()> {
    if ["ui", "service", "migrations", "source"]
        .into_iter()
        .any(|root| path == root || path.starts_with(&format!("{root}/")))
    {
        Ok(())
    } else {
        Err(PluginTransferError::UnsupportedEntry(path.into()))
    }
}

fn validate_generation_tree(snapshot: &TreeSnapshot) -> PluginTransferResult<()> {
    for path in snapshot.files.keys() {
        if path != "data.sqlite"
            && !path
                .strip_prefix("files/")
                .is_some_and(|relative| !relative.is_empty())
        {
            return Err(PluginTransferError::UnsupportedEntry(path.clone()));
        }
    }
    for path in &snapshot.directories {
        if path != "files" && !path.starts_with("files/") {
            return Err(PluginTransferError::UnsupportedEntry(path.clone()));
        }
    }
    Ok(())
}

fn validate_backup_tree(snapshot: &TreeSnapshot) -> PluginTransferResult<()> {
    for path in snapshot.files.keys() {
        if matches!(
            path.as_str(),
            BACKUP_MANIFEST | DATA_SQLITE | CONFIG_FILE | GRANTS_FILE | CREDENTIAL_SLOTS_FILE
        ) {
            continue;
        }
        if let Some(relative) = path.strip_prefix("package/") {
            validate_package_file(relative)?;
            continue;
        }
        if path
            .strip_prefix("data/files/")
            .is_some_and(|relative| !relative.is_empty())
        {
            continue;
        }
        return Err(PluginTransferError::UnsupportedEntry(path.clone()));
    }
    for path in &snapshot.directories {
        if matches!(path.as_str(), PACKAGE_DIRECTORY | DATA_DIRECTORY | DATA_FILES_DIRECTORY)
            || path.starts_with("data/files/")
        {
            continue;
        }
        if let Some(relative) = path.strip_prefix("package/") {
            validate_package_directory(relative)?;
            continue;
        }
        return Err(PluginTransferError::UnsupportedEntry(path.clone()));
    }
    Ok(())
}

fn validate_config(config: &Value, slots: &BTreeSet<String>) -> PluginTransferResult<()> {
    if !config.is_object() {
        return Err(PluginTransferError::InvalidInput(
            "non-secret config must be a JSON object".into(),
        ));
    }
    reject_secret_fields(config, slots, "config")
}

fn reject_secret_fields(
    value: &Value,
    slots: &BTreeSet<String>,
    path: &str,
) -> PluginTransferResult<()> {
    match value {
        Value::Object(values) => {
            for (key, value) in values {
                let normalized = key
                    .chars()
                    .filter(|character| character.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>();
                let is_slot = slots.iter().any(|slot| slot.eq_ignore_ascii_case(key));
                if is_slot
                    || matches!(
                        normalized.as_str(),
                        "credential" | "credentials" | "credentialid" | "secret" | "secrets"
                            | "password" | "token" | "apikey" | "authorization"
                    )
                {
                    return Err(PluginTransferError::InvalidInput(format!(
                        "{path}.{key} is secret-bearing and cannot enter a Backup"
                    )));
                }
                reject_secret_fields(value, slots, &format!("{path}.{key}"))?;
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                reject_secret_fields(value, slots, &format!("{path}[{index}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn normalized_grants(grants: &[PluginGrantMetadata]) -> PluginTransferResult<Vec<PluginGrantMetadata>> {
    let mut normalized = grants.to_vec();
    normalized.sort();
    for grant in &normalized {
        validate_permission(&grant.permission)?;
    }
    if normalized.windows(2).any(|pair| pair[0].permission == pair[1].permission) {
        return Err(PluginTransferError::InvalidInput(
            "grant permissions must be unique".into(),
        ));
    }
    Ok(normalized)
}

fn validate_permission(value: &str) -> PluginTransferResult<()> {
    let valid = !value.is_empty()
        && value.len() <= 160
        && value.bytes().next().is_some_and(|byte| byte.is_ascii_lowercase())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'/' | b':' | b'_' | b'-')
        });
    if !valid {
        return Err(PluginTransferError::InvalidInput(format!(
            "invalid grant permission {value}"
        )));
    }
    Ok(())
}

fn validate_credential_slots(slots: &BTreeSet<String>) -> PluginTransferResult<()> {
    for slot in slots {
        let valid = !slot.is_empty()
            && slot.len() <= 96
            && slot.bytes().next().is_some_and(|byte| byte.is_ascii_lowercase())
            && slot.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
            });
        if !valid {
            return Err(PluginTransferError::InvalidInput(format!(
                "invalid credential slot {slot}"
            )));
        }
    }
    Ok(())
}

fn validate_plugin_id(plugin_id: &PluginId) -> PluginTransferResult<()> {
    validate_component(plugin_id.as_ref(), "plugin_id")
}

fn validate_generation(generation: &str) -> PluginTransferResult<()> {
    validate_component(generation, "generation")
}

fn validate_component(value: &str, label: &str) -> PluginTransferResult<()> {
    let valid = !value.is_empty()
        && value.len() <= 160
        && value.nfc().collect::<String>() == value
        && value != "."
        && value != ".."
        && !value.ends_with([' ', '.'])
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && !is_windows_reserved_name(value);
    if !valid {
        return Err(PluginTransferError::InvalidInput(format!(
            "invalid {label}: {value}"
        )));
    }
    Ok(())
}

fn validate_digest(digest: &DigestHex) -> PluginTransferResult<()> {
    if digest.as_ref().len() != 64
        || !digest
            .as_ref()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(PluginTransferError::InvalidInput(
            "artifact_digest must be lowercase SHA-256".into(),
        ));
    }
    Ok(())
}

fn validate_sqlite(bytes: &[u8]) -> PluginTransferResult<()> {
    if !bytes.starts_with(SQLITE_HEADER) {
        return Err(PluginTransferError::InvalidInput(
            "data.sqlite is not a SQLite database snapshot".into(),
        ));
    }
    Ok(())
}

fn file_records(files: &BTreeMap<String, Vec<u8>>) -> Vec<BundleFileRecord> {
    files
        .iter()
        .map(|(path, bytes)| BundleFileRecord {
            path: path.clone(),
            digest: digest_bytes(bytes),
            size_bytes: bytes.len() as u64,
        })
        .collect()
}

fn validate_records(records: &[BundleFileRecord]) -> PluginTransferResult<()> {
    let mut previous: Option<&str> = None;
    for record in records {
        normalize_relative_path(Path::new(&record.path))?;
        validate_digest(&record.digest)?;
        if previous.is_some_and(|value| value >= record.path.as_str()) {
            return Err(PluginTransferError::InvalidInput(
                "backup file inventory must be sorted and unique".into(),
            ));
        }
        previous = Some(&record.path);
    }
    Ok(())
}

fn canonical_bytes(value: &impl Serialize) -> PluginTransferResult<Vec<u8>> {
    canonical_json_bytes(value).map_err(|error| PluginTransferError::Json(error.to_string()))
}

fn parse_canonical<T>(bytes: &[u8], maximum: u64) -> PluginTransferResult<T>
where
    T: DeserializeOwned + Serialize,
{
    if bytes.len() as u64 > maximum {
        return Err(PluginTransferError::LimitExceeded(
            "canonical JSON section is too large".into(),
        ));
    }
    let value = strict_json_from_slice(bytes)?;
    if canonical_bytes(&value)? != bytes {
        return Err(PluginTransferError::Tampered(
            "JSON section is not canonical".into(),
        ));
    }
    Ok(value)
}

fn strict_json_from_slice<T: DeserializeOwned>(bytes: &[u8]) -> PluginTransferResult<T> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictJsonValueSeed
        .deserialize(&mut deserializer)
        .map_err(|error| PluginTransferError::Json(error.to_string()))?;
    deserializer
        .end()
        .map_err(|error| PluginTransferError::Json(error.to_string()))?;
    serde_json::from_value(value).map_err(|error| PluginTransferError::Json(error.to_string()))
}

fn scan_directory(root: &Path, limits: PluginTransferLimits) -> PluginTransferResult<TreeSnapshot> {
    let metadata = fs::symlink_metadata(root).map_err(|source| io_error(root, source))?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
        return Err(unsafe_path(root, "source must be a real directory"));
    }
    let canonical_root = fs::canonicalize(root).map_err(|source| io_error(root, source))?;
    let mut snapshot = TreeSnapshot::default();
    let mut collision_keys = HashSet::new();
    let mut total = 0u64;
    for entry in WalkDir::new(&canonical_root).follow_links(false).min_depth(1) {
        let entry = entry.map_err(|error| PluginTransferError::Io {
            path: error
                .path()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| canonical_root.clone()),
            source: error
                .into_io_error()
                .unwrap_or_else(|| io::Error::other("cannot scan Plugin transfer")),
        })?;
        let relative = entry.path().strip_prefix(&canonical_root).map_err(|_| {
            unsafe_path(entry.path(), "entry escaped transfer root")
        })?;
        let normalized = normalize_relative_path(relative)?;
        let collision = windows_collision_key(&normalized)?;
        if !collision_keys.insert(collision) {
            return Err(PluginTransferError::DuplicateEntry(normalized));
        }
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|source| io_error(entry.path(), source))?;
        if metadata_is_link_or_reparse(&metadata) {
            return Err(unsafe_path(entry.path(), "links and reparse points are forbidden"));
        }
        if metadata.is_dir() {
            snapshot.directories.insert(normalized);
            continue;
        }
        if !metadata.is_file() {
            return Err(unsafe_path(entry.path(), "special files are forbidden"));
        }
        if snapshot.files.len() >= limits.max_files {
            return Err(PluginTransferError::LimitExceeded(
                "file count exceeds the transfer limit".into(),
            ));
        }
        if metadata.len() > limits.max_single_file_bytes {
            return Err(PluginTransferError::LimitExceeded(format!(
                "{normalized} exceeds the single-file limit"
            )));
        }
        total = checked_total(total, metadata.len(), limits.max_total_bytes)?;
        let canonical_file = fs::canonicalize(entry.path())
            .map_err(|source| io_error(entry.path(), source))?;
        if !canonical_file.starts_with(&canonical_root) {
            return Err(unsafe_path(entry.path(), "file escaped transfer root"));
        }
        snapshot.files.insert(
            normalized,
            read_bounded(&canonical_file, limits.max_single_file_bytes)?,
        );
    }
    Ok(snapshot)
}

fn scan_zip(path: &Path, limits: PluginTransferLimits) -> PluginTransferResult<TreeSnapshot> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() {
        return Err(unsafe_path(path, "ZIP source must be a regular file"));
    }
    if metadata.len() > limits.max_zip_bytes {
        return Err(PluginTransferError::LimitExceeded(
            "ZIP exceeds the compressed-size limit".into(),
        ));
    }
    let file = File::open(path).map_err(|source| io_error(path, source))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| PluginTransferError::InvalidZip(error.to_string()))?;
    let mut snapshot = TreeSnapshot::default();
    let mut collision_keys = HashSet::new();
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| PluginTransferError::InvalidZip(error.to_string()))?;
        let raw_name = std::str::from_utf8(entry.name_raw()).map_err(|_| {
            PluginTransferError::UnsafePath {
                path: "<non-utf8>".into(),
                reason: "ZIP names must be UTF-8".into(),
            }
        })?;
        let is_directory = entry.is_dir() || raw_name.ends_with('/');
        let name = raw_name.trim_end_matches('/');
        if name.contains('\\') {
            return Err(PluginTransferError::UnsafePath {
                path: name.into(),
                reason: "ZIP paths must use forward slashes".into(),
            });
        }
        let normalized = normalize_relative_path(Path::new(name))?;
        if normalized != name {
            return Err(PluginTransferError::UnsafePath {
                path: name.into(),
                reason: "ZIP path must already be normalized".into(),
            });
        }
        let collision = windows_collision_key(&normalized)?;
        if !collision_keys.insert(collision) {
            return Err(PluginTransferError::DuplicateEntry(normalized));
        }
        if zip_entry_is_special(entry.unix_mode(), is_directory) {
            return Err(PluginTransferError::UnsafePath {
                path: normalized,
                reason: "ZIP links and special entries are forbidden".into(),
            });
        }
        if is_directory {
            snapshot.directories.insert(normalized);
            continue;
        }
        if snapshot.files.len() >= limits.max_files {
            return Err(PluginTransferError::LimitExceeded(
                "file count exceeds the transfer limit".into(),
            ));
        }
        if entry.size() > limits.max_single_file_bytes {
            return Err(PluginTransferError::LimitExceeded(format!(
                "{normalized} exceeds the single-file limit"
            )));
        }
        total = checked_total(total, entry.size(), limits.max_total_bytes)?;
        let mut bytes = Vec::with_capacity(
            usize::try_from(entry.size().min(1024 * 1024)).unwrap_or(1024 * 1024),
        );
        entry
            .by_ref()
            .take(limits.max_single_file_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|source| io_error(path, source))?;
        if bytes.len() as u64 != entry.size() {
            return Err(PluginTransferError::Tampered(format!(
                "ZIP entry {normalized} size mismatch"
            )));
        }
        snapshot.files.insert(normalized, bytes);
    }
    Ok(snapshot)
}

fn zip_entry_is_special(mode: Option<u32>, directory: bool) -> bool {
    let Some(mode) = mode else { return false };
    let kind = mode & 0o170000;
    kind != 0 && kind != 0o100000 && !(directory && kind == 0o040000)
}

fn atomic_write_directory(destination: &Path, snapshot: &TreeSnapshot) -> PluginTransferResult<()> {
    let parent = destination_parent(destination)?;
    require_absent(destination)?;
    let mut staging = StagingDirectory::allocate_with_prefix(parent, STAGING_PREFIX)?;
    write_tree(staging.path(), snapshot)?;
    sync_tree(staging.path())?;
    require_absent(destination)?;
    fs::rename(staging.path(), destination).map_err(|source| {
        if fs::symlink_metadata(destination).is_ok() {
            PluginTransferError::DestinationExists(destination.to_path_buf())
        } else {
            io_error(destination, source)
        }
    })?;
    sync_directory(parent)?;
    staging.disarm();
    Ok(())
}

fn atomic_write_zip(destination: &Path, snapshot: &TreeSnapshot) -> PluginTransferResult<()> {
    let parent = destination_parent(destination)?;
    require_absent(destination)?;
    let staging = parent.join(format!("{STAGING_PREFIX}{}.zip", Uuid::now_v7()));
    let mut guard = StagingFile::create(parent, staging)?;
    {
        let file = guard.take_file()?;
        let mut writer = zip::ZipWriter::new(file);
        let directory_options = SimpleFileOptions::default().unix_permissions(0o040700);
        for directory in &snapshot.directories {
            writer
                .add_directory(format!("{directory}/"), directory_options)
                .map_err(|error| PluginTransferError::InvalidZip(error.to_string()))?;
        }
        let file_options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o100600);
        for (path, bytes) in &snapshot.files {
            writer
                .start_file(path, file_options)
                .map_err(|error| PluginTransferError::InvalidZip(error.to_string()))?;
            writer
                .write_all(bytes)
                .map_err(|source| io_error(guard.path(), source))?;
        }
        let file = writer
            .finish()
            .map_err(|error| PluginTransferError::InvalidZip(error.to_string()))?;
        file.sync_all().map_err(|source| io_error(guard.path(), source))?;
    }
    require_absent(destination)?;
    fs::rename(guard.path(), destination).map_err(|source| {
        if fs::symlink_metadata(destination).is_ok() {
            PluginTransferError::DestinationExists(destination.to_path_buf())
        } else {
            io_error(destination, source)
        }
    })?;
    sync_directory(parent)?;
    guard.disarm();
    Ok(())
}

fn write_tree(root: &Path, snapshot: &TreeSnapshot) -> PluginTransferResult<()> {
    let mut directories = snapshot.directories.iter().collect::<Vec<_>>();
    directories.sort_by_key(|path| path.split('/').count());
    for relative in directories {
        let path = join_normalized(root, relative);
        if !path.exists() {
            fs::create_dir(&path).map_err(|source| io_error(&path, source))?;
        }
    }
    for (relative, bytes) in &snapshot.files {
        let path = join_normalized(root, relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
        }
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|source| io_error(&path, source))?;
        file.write_all(bytes).map_err(|source| io_error(&path, source))?;
        file.sync_all().map_err(|source| io_error(&path, source))?;
    }
    Ok(())
}

fn destination_parent(destination: &Path) -> PluginTransferResult<&Path> {
    let name = destination.file_name().and_then(|name| name.to_str()).ok_or_else(|| {
        PluginTransferError::InvalidInput("destination must have a UTF-8 filename".into())
    })?;
    validate_component(name, "destination")?;
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    require_real_directory(parent)?;
    Ok(parent)
}

fn require_absent(path: &Path) -> PluginTransferResult<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(PluginTransferError::DestinationExists(path.to_path_buf())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

struct StagingDirectory {
    parent: PathBuf,
    path: Option<PathBuf>,
}

impl StagingDirectory {
    fn allocate(parent: &Path) -> PluginTransferResult<Self> {
        Self::allocate_with_prefix(parent, STAGING_PREFIX)
    }

    fn allocate_with_prefix(parent: &Path, prefix: &str) -> PluginTransferResult<Self> {
        require_real_directory(parent)?;
        for _ in 0..16 {
            let path = parent.join(format!("{prefix}{}", Uuid::now_v7()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        parent: parent.to_path_buf(),
                        path: Some(path),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(source) => return Err(io_error(&path, source)),
            }
        }
        Err(PluginTransferError::InvalidInput(
            "could not allocate a unique staging directory".into(),
        ))
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("active staging directory")
    }

    fn disarm(&mut self) {
        self.path = None;
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else { return };
        if path.parent() == Some(self.parent.as_path()) {
            let _ = validate_tree_without_links(&path).and_then(|()| {
                fs::remove_dir_all(&path).map_err(|source| io_error(&path, source))
            });
        }
    }
}

struct StagingFile {
    parent: PathBuf,
    path: Option<PathBuf>,
    file: Option<File>,
}

impl StagingFile {
    fn create(parent: &Path, path: PathBuf) -> PluginTransferResult<Self> {
        if path.parent() != Some(parent) {
            return Err(unsafe_path(&path, "staging file is not an exact child"));
        }
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .read(true)
            .open(&path)
            .map_err(|source| io_error(&path, source))?;
        Ok(Self {
            parent: parent.to_path_buf(),
            path: Some(path),
            file: Some(file),
        })
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("active staging file")
    }

    fn take_file(&mut self) -> PluginTransferResult<File> {
        self.file.take().ok_or_else(|| {
            PluginTransferError::InvalidInput("staging ZIP file was already consumed".into())
        })
    }

    fn disarm(&mut self) {
        self.path = None;
        self.file = None;
    }
}

impl Drop for StagingFile {
    fn drop(&mut self) {
        self.file.take();
        let Some(path) = self.path.take() else { return };
        if path.parent() == Some(self.parent.as_path()) {
            let _ = fs::remove_file(path);
        }
    }
}

fn normalize_relative_path(path: &Path) -> PluginTransferResult<String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(unsafe_path(path, "path must be a non-empty relative path"));
    }
    let mut components = Vec::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(unsafe_path(path, "path contains traversal, root, or drive components"));
        };
        let component = component
            .to_str()
            .ok_or_else(|| unsafe_path(path, "path must be UTF-8"))?;
        if component.is_empty()
            || component.len() > MAX_COMPONENT_BYTES
            || component.nfc().collect::<String>() != component
            || component.ends_with([' ', '.'])
            || component.contains(['\0', '\\', ':'])
            || is_windows_reserved_name(component)
        {
            return Err(unsafe_path(path, "path component is not portable"));
        }
        components.push(component);
    }
    let normalized = components.join("/");
    if normalized.len() > MAX_PATH_BYTES {
        return Err(unsafe_path(path, "path must already be normalized"));
    }
    Ok(normalized)
}

fn windows_collision_key(path: &str) -> PluginTransferResult<String> {
    let normalized = path.nfc().collect::<String>();
    if normalized != path {
        return Err(PluginTransferError::UnsafePath {
            path: path.into(),
            reason: "path must use NFC".into(),
        });
    }
    Ok(normalized.to_lowercase())
}

fn join_normalized(root: &Path, normalized: &str) -> PathBuf {
    normalized
        .split('/')
        .fold(root.to_path_buf(), |path, component| path.join(component))
}

fn checked_total(current: u64, added: u64, limit: u64) -> PluginTransferResult<u64> {
    let total = current
        .checked_add(added)
        .ok_or_else(|| PluginTransferError::LimitExceeded("total size overflow".into()))?;
    if total > limit {
        return Err(PluginTransferError::LimitExceeded(
            "total bytes exceed the transfer limit".into(),
        ));
    }
    Ok(total)
}

fn read_bounded(path: &Path, limit: u64) -> PluginTransferResult<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() {
        return Err(unsafe_path(path, "expected a regular file"));
    }
    if metadata.len() > limit {
        return Err(PluginTransferError::LimitExceeded(format!(
            "{} exceeds the file limit",
            path.display()
        )));
    }
    fs::read(path).map_err(|source| io_error(path, source))
}

fn ensure_real_directory(path: &Path) -> PluginTransferResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
                return Err(unsafe_path(path, "expected a real directory"));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|source| io_error(path, source))?;
            let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
            if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
                return Err(unsafe_path(path, "created path is not a real directory"));
            }
        }
        Err(source) => return Err(io_error(path, source)),
    }
    Ok(())
}

fn require_real_directory(path: &Path) -> PluginTransferResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
        return Err(unsafe_path(path, "expected a real directory"));
    }
    Ok(())
}

fn validate_tree_without_links(path: &Path) -> PluginTransferResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if metadata_is_link_or_reparse(&metadata) {
        return Err(unsafe_path(path, "tree contains a link or reparse point"));
    }
    if metadata.is_file() {
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(unsafe_path(path, "tree contains a special entry"));
    }
    for entry in fs::read_dir(path).map_err(|source| io_error(path, source))? {
        let entry = entry.map_err(|source| io_error(path, source))?;
        validate_tree_without_links(&entry.path())?;
    }
    Ok(())
}

fn sync_tree(path: &Path) -> PluginTransferResult<()> {
    validate_tree_without_links(path)?;
    for entry in WalkDir::new(path).follow_links(false).min_depth(1) {
        let entry = entry.map_err(|error| PluginTransferError::Io {
            path: error.path().map(Path::to_path_buf).unwrap_or_else(|| path.into()),
            source: error
                .into_io_error()
                .unwrap_or_else(|| io::Error::other("cannot sync transfer tree")),
        })?;
        if entry.file_type().is_file() {
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(entry.path())
                .and_then(|file| file.sync_all())
                .map_err(|source| io_error(entry.path(), source))?;
        }
    }
    let mut directories = WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_dir())
        .map(|entry| entry.into_path())
        .collect::<Vec<_>>();
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        sync_directory(&directory)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> PluginTransferResult<()> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|source| io_error(path, source))
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> PluginTransferResult<()> {
    match File::open(path).and_then(|file| file.sync_all()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

#[cfg(not(any(unix, windows)))]
fn sync_directory(_path: &Path) -> PluginTransferResult<()> {
    Ok(())
}

#[cfg(windows)]
fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn is_windows_reserved_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON" | "PRN" | "AUX" | "NUL"
            | "COM1" | "COM2" | "COM3" | "COM4" | "COM5" | "COM6" | "COM7" | "COM8" | "COM9"
            | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9"
    )
}

fn unsafe_path(path: &Path, reason: impl Into<String>) -> PluginTransferError {
    PluginTransferError::UnsafePath {
        path: path.display().to_string(),
        reason: reason.into(),
    }
}

fn io_error(path: &Path, source: io::Error) -> PluginTransferError {
    PluginTransferError::Io {
        path: path.to_path_buf(),
        source,
    }
}

struct StrictJsonValueSeed;

impl<'de> DeserializeSeed<'de> for StrictJsonValueSeed {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictJsonValueVisitor)
    }
}

struct StrictJsonValueVisitor;

impl<'de> Visitor<'de> for StrictJsonValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("strict JSON without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(value.into()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        StrictJsonValueSeed.deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(StrictJsonValueSeed)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        let mut keys = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate decoded JSON object key: {key}"
                )));
            }
            values.insert(key, map.next_value_seed(StrictJsonValueSeed)?);
        }
        Ok(Value::Object(values))
    }
}
