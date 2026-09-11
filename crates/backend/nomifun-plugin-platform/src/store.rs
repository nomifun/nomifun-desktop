use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use nomifun_agent_contracts::{
    ArtifactEnvelope, ArtifactFileDigest, ArtifactId, DigestHex, PluginPackageArtifactV1,
    PluginPackageV1Manifest, canonical_json_bytes,
};
use nomifun_common::zip_safe::{self, ZipColonPolicy};
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;
use walkdir::WalkDir;

const ARTIFACTS_DIRECTORY: &str = "artifacts";
const STAGING_DIRECTORY: &str = ".staging";
const PACKAGE_DIRECTORY: &str = "package";
const ARTIFACT_RECORD_FILE: &str = "artifact.json";
const MANIFEST_FILE: &str = "manifest.json";
const ENTRYPOINT_FILE: &str = "main.mjs";
const SOURCE_MAP_FILE: &str = "main.mjs.map";
const RESOURCES_DIRECTORY: &str = "resources";
const COPY_BUFFER_BYTES: usize = 64 * 1024;
const MAX_NORMALIZED_PATH_BYTES: usize = 1024;
const MAX_PATH_COMPONENT_BYTES: usize = 255;
const ARTIFACT_RECORD_BYTES_PER_FILE: u64 = MAX_NORMALIZED_PATH_BYTES as u64 + 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArtifactStoreLimits {
    pub max_file_count: usize,
    pub max_manifest_bytes: u64,
    pub max_single_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_zip_bytes: u64,
}

impl Default for ArtifactStoreLimits {
    fn default() -> Self {
        Self {
            max_file_count: 4_096,
            max_manifest_bytes: 1024 * 1024,
            max_single_file_bytes: 64 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
            max_zip_bytes: 128 * 1024 * 1024,
        }
    }
}

impl ArtifactStoreLimits {
    fn validate(self) -> Result<Self, PluginArtifactStoreError> {
        if self.max_file_count < 2
            || self.max_manifest_bytes == 0
            || self.max_single_file_bytes == 0
            || self.max_total_bytes < self.max_manifest_bytes
            || self.max_total_bytes < self.max_single_file_bytes
            || self.max_zip_bytes == 0
        {
            return Err(PluginArtifactStoreError::InvalidLimits);
        }
        Ok(self)
    }
}

pub trait ImportCancellation: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

#[derive(Debug, Default)]
pub struct NeverCancel;

impl ImportCancellation for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}

#[derive(Debug, Default)]
pub struct CancellationFlag {
    cancelled: AtomicBool,
}

impl CancellationFlag {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl ImportCancellation for CancellationFlag {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredPluginArtifact {
    pub artifact: PluginPackageArtifactV1,
    pub managed_relative_path: String,
    pub artifact_root: PathBuf,
    pub package_root: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArtifactImportResult {
    pub stored: StoredPluginArtifact,
    pub already_present: bool,
}

#[derive(Debug, Error)]
pub enum PluginArtifactStoreError {
    #[error("artifact store limits are invalid")]
    InvalidLimits,
    #[error("artifact import was canceled")]
    Canceled,
    #[error("managed artifact path is unsafe: {path}")]
    UnsafeManagedPath { path: PathBuf },
    #[error("package source is not a regular directory or file: {path}")]
    InvalidSource { path: PathBuf },
    #[error("unsafe package path {path}: {reason}")]
    UnsafePackagePath { path: String, reason: String },
    #[error("unsupported package entry: {path}")]
    UnsupportedEntry { path: String },
    #[error("duplicate or Windows case-colliding package entry: {path}")]
    DuplicateEntry { path: String },
    #[error("package contains too many files ({observed} > {limit})")]
    TooManyFiles { observed: usize, limit: usize },
    #[error("package file is too large: {path} ({observed} > {limit})")]
    FileTooLarge {
        path: String,
        observed: u64,
        limit: u64,
    },
    #[error("package expands beyond the total byte limit ({observed} > {limit})")]
    TotalSizeExceeded { observed: u64, limit: u64 },
    #[error("zip input is too large ({observed} > {limit})")]
    ZipTooLarge { observed: u64, limit: u64 },
    #[error("invalid zip package: {0}")]
    InvalidZip(String),
    #[error("invalid manifest envelope: {0}")]
    InvalidManifest(String),
    #[error("plugin package contract is invalid: {0}")]
    Contract(String),
    #[error("published artifact {digest} is missing or has been modified")]
    PublishedArtifactMismatch { digest: String },
    #[error("filesystem operation failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Clone, Debug)]
pub struct PluginArtifactStore {
    managed_root: PathBuf,
    artifacts_root: PathBuf,
    staging_root: PathBuf,
    limits: ArtifactStoreLimits,
}

impl PluginArtifactStore {
    pub fn new(
        managed_root: impl AsRef<Path>,
        limits: ArtifactStoreLimits,
    ) -> Result<Self, PluginArtifactStoreError> {
        let limits = limits.validate()?;
        let requested_root = managed_root.as_ref();
        ensure_directory_without_symlink(requested_root)?;
        let managed_root = fs::canonicalize(requested_root)
            .map_err(|source| io_error(requested_root, source))?;
        let artifacts_root = managed_root.join(ARTIFACTS_DIRECTORY);
        let staging_root = managed_root.join(STAGING_DIRECTORY);
        ensure_direct_child_directory(&managed_root, &artifacts_root)?;
        ensure_direct_child_directory(&managed_root, &staging_root)?;
        Ok(Self {
            managed_root,
            artifacts_root,
            staging_root,
            limits,
        })
    }

    pub fn managed_root(&self) -> &Path {
        &self.managed_root
    }

    pub fn import_directory(
        &self,
        source: impl AsRef<Path>,
        cancellation: &dyn ImportCancellation,
    ) -> Result<ArtifactImportResult, PluginArtifactStoreError> {
        check_canceled(cancellation)?;
        let source = source.as_ref();
        let source_metadata =
            fs::symlink_metadata(source).map_err(|error| io_error(source, error))?;
        if source_metadata.file_type().is_symlink() || !source_metadata.is_dir() {
            return Err(PluginArtifactStoreError::InvalidSource {
                path: source.to_path_buf(),
            });
        }
        let source_root =
            fs::canonicalize(source).map_err(|error| io_error(source, error))?;
        if source_root.starts_with(&self.managed_root)
            || self.managed_root.starts_with(&source_root)
        {
            return Err(PluginArtifactStoreError::UnsafeManagedPath {
                path: source_root,
            });
        }
        let mut staging = self.create_staging()?;
        let scanned = self.stage_directory(&source_root, staging.path(), cancellation)?;
        check_canceled(cancellation)?;
        let artifact = self.finish_staging(scanned, staging.path())?;
        check_canceled(cancellation)?;
        let result = self.publish_or_reuse(&artifact, staging.path())?;
        if !result.already_present {
            staging.disarm();
        }
        Ok(result)
    }

    pub fn inspect_directory(
        &self,
        source: impl AsRef<Path>,
        cancellation: &dyn ImportCancellation,
    ) -> Result<PluginPackageArtifactV1, PluginArtifactStoreError> {
        check_canceled(cancellation)?;
        let source = source.as_ref();
        let source_metadata =
            fs::symlink_metadata(source).map_err(|error| io_error(source, error))?;
        if source_metadata.file_type().is_symlink() || !source_metadata.is_dir() {
            return Err(PluginArtifactStoreError::InvalidSource {
                path: source.to_path_buf(),
            });
        }
        let source_root =
            fs::canonicalize(source).map_err(|error| io_error(source, error))?;
        if source_root.starts_with(&self.managed_root)
            || self.managed_root.starts_with(&source_root)
        {
            return Err(PluginArtifactStoreError::UnsafeManagedPath {
                path: source_root,
            });
        }
        let staging = self.create_staging()?;
        let scanned = self.stage_directory(&source_root, staging.path(), cancellation)?;
        check_canceled(cancellation)?;
        self.finish_staging(scanned, staging.path())
    }

    pub fn import_zip(
        &self,
        source: impl AsRef<Path>,
        cancellation: &dyn ImportCancellation,
    ) -> Result<ArtifactImportResult, PluginArtifactStoreError> {
        check_canceled(cancellation)?;
        let source = source.as_ref();
        let metadata = fs::symlink_metadata(source).map_err(|error| io_error(source, error))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(PluginArtifactStoreError::InvalidSource {
                path: source.to_path_buf(),
            });
        }
        if metadata.len() > self.limits.max_zip_bytes {
            return Err(PluginArtifactStoreError::ZipTooLarge {
                observed: metadata.len(),
                limit: self.limits.max_zip_bytes,
            });
        }
        let mut staging = self.create_staging()?;
        let scanned = self.stage_zip(source, staging.path(), cancellation)?;
        check_canceled(cancellation)?;
        let artifact = self.finish_staging(scanned, staging.path())?;
        check_canceled(cancellation)?;
        let result = self.publish_or_reuse(&artifact, staging.path())?;
        if !result.already_present {
            staging.disarm();
        }
        Ok(result)
    }

    pub fn inspect_zip(
        &self,
        source: impl AsRef<Path>,
        cancellation: &dyn ImportCancellation,
    ) -> Result<PluginPackageArtifactV1, PluginArtifactStoreError> {
        check_canceled(cancellation)?;
        let source = source.as_ref();
        let metadata = fs::symlink_metadata(source).map_err(|error| io_error(source, error))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(PluginArtifactStoreError::InvalidSource {
                path: source.to_path_buf(),
            });
        }
        if metadata.len() > self.limits.max_zip_bytes {
            return Err(PluginArtifactStoreError::ZipTooLarge {
                observed: metadata.len(),
                limit: self.limits.max_zip_bytes,
            });
        }
        let staging = self.create_staging()?;
        let scanned = self.stage_zip(source, staging.path(), cancellation)?;
        check_canceled(cancellation)?;
        self.finish_staging(scanned, staging.path())
    }

    pub fn load(
        &self,
        artifact_digest: &DigestHex,
    ) -> Result<StoredPluginArtifact, PluginArtifactStoreError> {
        validate_digest_segment(artifact_digest.as_ref())?;
        let artifact_root = self.artifacts_root.join(artifact_digest.as_ref());
        self.verify_published(&artifact_root, Some(artifact_digest))
    }

    fn create_staging(&self) -> Result<StagingGuard, PluginArtifactStoreError> {
        for _ in 0..8 {
            let path = self
                .staging_root
                .join(format!("import-{}", Uuid::now_v7()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    fs::create_dir(path.join(PACKAGE_DIRECTORY))
                        .map_err(|error| io_error(&path, error))?;
                    return Ok(StagingGuard {
                        staging_parent: self.staging_root.clone(),
                        path: Some(path),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error(&path, error)),
            }
        }
        Err(PluginArtifactStoreError::Io {
            path: self.staging_root.clone(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "could not allocate unique artifact staging directory",
            ),
        })
    }

    fn stage_directory(
        &self,
        source_root: &Path,
        staging: &Path,
        cancellation: &dyn ImportCancellation,
    ) -> Result<ScannedPackage, PluginArtifactStoreError> {
        let mut entries = Vec::new();
        let mut all_entries = HashSet::new();
        for entry in WalkDir::new(source_root).follow_links(false).min_depth(1) {
            check_canceled(cancellation)?;
            let entry = entry.map_err(|error| PluginArtifactStoreError::Io {
                path: error
                    .path()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| source_root.to_path_buf()),
                source: error
                    .into_io_error()
                    .unwrap_or_else(|| io::Error::other("cannot walk package source")),
            })?;
            let relative = entry.path().strip_prefix(source_root).map_err(|_| {
                PluginArtifactStoreError::UnsafePackagePath {
                    path: entry.path().display().to_string(),
                    reason: "entry is outside the source root".into(),
                }
            })?;
            let normalized = normalize_relative_path(relative)?;
            let collision_key = windows_collision_key(&normalized)?;
            if !all_entries.insert(collision_key) {
                return Err(PluginArtifactStoreError::DuplicateEntry { path: normalized });
            }
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|error| io_error(entry.path(), error))?;
            if metadata.file_type().is_symlink() {
                return Err(PluginArtifactStoreError::UnsafePackagePath {
                    path: normalized,
                    reason: "symbolic links are forbidden".into(),
                });
            }
            if metadata.is_dir() {
                validate_allowed_directory(&normalized)?;
                continue;
            }
            if !metadata.is_file() {
                return Err(PluginArtifactStoreError::UnsafePackagePath {
                    path: normalized,
                    reason: "only regular files are allowed".into(),
                });
            }
            validate_allowed_file(&normalized)?;
            let canonical = fs::canonicalize(entry.path())
                .map_err(|error| io_error(entry.path(), error))?;
            if !canonical.starts_with(source_root) {
                return Err(PluginArtifactStoreError::UnsafePackagePath {
                    path: normalized,
                    reason: "resolved file escapes the source root".into(),
                });
            }
            entries.push((normalized, canonical, metadata.len()));
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));

        let mut scanner = PackageScanner::new(self.limits, staging, cancellation);
        for (normalized, source, declared_size) in entries {
            scanner.register_file(&normalized)?;
            scanner.check_declared_size(&normalized, declared_size)?;
            let file = File::open(&source).map_err(|error| io_error(&source, error))?;
            scanner.consume_file(&normalized, file)?;
        }
        scanner.finish()
    }

    fn stage_zip(
        &self,
        source: &Path,
        staging: &Path,
        cancellation: &dyn ImportCancellation,
    ) -> Result<ScannedPackage, PluginArtifactStoreError> {
        let file = File::open(source).map_err(|error| io_error(source, error))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|error| PluginArtifactStoreError::InvalidZip(
                error.to_string(),
            ))?;
        if archive.len() > self.limits.max_file_count {
            return Err(PluginArtifactStoreError::TooManyFiles {
                observed: archive.len(),
                limit: self.limits.max_file_count,
            });
        }

        let mut scanner = PackageScanner::new(self.limits, staging, cancellation);
        let mut all_entries = HashSet::new();
        for index in 0..archive.len() {
            check_canceled(cancellation)?;
            let mut entry = archive.by_index(index).map_err(|error| {
                PluginArtifactStoreError::InvalidZip(error.to_string())
            })?;
            let raw_name = entry.name().to_owned();
            if entry.encrypted() || zip_entry_is_special(entry.unix_mode()) {
                return Err(PluginArtifactStoreError::UnsafePackagePath {
                    path: raw_name,
                    reason: "encrypted links and special files are forbidden".into(),
                });
            }
            let safe = zip_safe::safe_zip_entry_path(&raw_name, ZipColonPolicy::RejectAll)
                .ok_or_else(|| PluginArtifactStoreError::UnsafePackagePath {
                    path: raw_name.clone(),
                    reason: "absolute, traversal, backslash, and drive paths are forbidden".into(),
                })?;
            let normalized = normalize_relative_path(&safe)?;
            let collision_key = windows_collision_key(&normalized)?;
            if !all_entries.insert(collision_key) {
                return Err(PluginArtifactStoreError::DuplicateEntry { path: normalized });
            }
            if entry.is_dir() {
                validate_allowed_directory(&normalized)?;
                continue;
            }
            validate_allowed_file(&normalized)?;
            scanner.register_file(&normalized)?;
            scanner.check_declared_size(&normalized, entry.size())?;
            scanner.consume_file(&normalized, &mut entry)?;
        }
        scanner.finish()
    }

    fn finish_staging(
        &self,
        scanned: ScannedPackage,
        staging: &Path,
    ) -> Result<PluginPackageArtifactV1, PluginArtifactStoreError> {
        let envelope: ArtifactEnvelope<PluginPackageV1Manifest> =
            strict_json_from_slice(&scanned.manifest_bytes)?;
        if !envelope
            .verify()
            .map_err(|error| PluginArtifactStoreError::InvalidManifest(error.to_string()))?
        {
            return Err(PluginArtifactStoreError::InvalidManifest(
                "manifest payload digest does not match its payload".into(),
            ));
        }
        envelope
            .payload
            .validate()
            .map_err(|error| PluginArtifactStoreError::Contract(error.to_string()))?;

        let artifact = PluginPackageArtifactV1::new(
            ArtifactId::from(Uuid::now_v7().to_string()),
            envelope.payload.clone(),
            scanned.files,
        )
        .map_err(|error| PluginArtifactStoreError::Contract(error.to_string()))?;
        if artifact.manifest != envelope {
            return Err(PluginArtifactStoreError::InvalidManifest(
                "manifest envelope is not the canonical Plugin Package v1 envelope".into(),
            ));
        }

        let package_root = staging.join(PACKAGE_DIRECTORY);
        write_new_synced(
            &package_root.join(MANIFEST_FILE),
            &canonical_json_bytes(&envelope)
                .map_err(|error| PluginArtifactStoreError::InvalidManifest(error.to_string()))?,
        )?;
        write_new_synced(
            &staging.join(ARTIFACT_RECORD_FILE),
            &canonical_json_bytes(&artifact)
                .map_err(|error| PluginArtifactStoreError::Contract(error.to_string()))?,
        )?;
        sync_tree_directories(staging)?;
        Ok(artifact)
    }

    fn publish_or_reuse(
        &self,
        artifact: &PluginPackageArtifactV1,
        staging: &Path,
    ) -> Result<ArtifactImportResult, PluginArtifactStoreError> {
        let final_root = self.artifacts_root.join(artifact.artifact_digest.as_ref());
        if final_root.exists() {
            return self.reuse_existing(&final_root, artifact);
        }

        match fs::rename(staging, &final_root) {
            Ok(()) => {
                let published = (|| {
                    sync_directory_if_supported(&self.artifacts_root)?;
                    let stored =
                        self.verify_published(&final_root, Some(&artifact.artifact_digest))?;
                    Ok(ArtifactImportResult {
                        stored,
                        already_present: false,
                    })
                })();
                if published.is_err() {
                    remove_owned_published_directory(&self.artifacts_root, &final_root);
                }
                published
            }
            Err(error)
                if error.kind() == io::ErrorKind::AlreadyExists || final_root.exists() =>
            {
                self.reuse_existing(&final_root, artifact)
            }
            Err(error) => Err(io_error(&final_root, error)),
        }
    }

    fn reuse_existing(
        &self,
        final_root: &Path,
        incoming: &PluginPackageArtifactV1,
    ) -> Result<ArtifactImportResult, PluginArtifactStoreError> {
        let stored = self.verify_published(final_root, Some(&incoming.artifact_digest))?;
        if stored.artifact.artifact_digest != incoming.artifact_digest
            || stored.artifact.manifest != incoming.manifest
            || stored.artifact.files != incoming.files
        {
            return Err(PluginArtifactStoreError::PublishedArtifactMismatch {
                digest: incoming.artifact_digest.as_ref().to_owned(),
            });
        }
        Ok(ArtifactImportResult {
            stored,
            already_present: true,
        })
    }

    fn verify_published(
        &self,
        artifact_root: &Path,
        expected_digest: Option<&DigestHex>,
    ) -> Result<StoredPluginArtifact, PluginArtifactStoreError> {
        let mismatch = || PluginArtifactStoreError::PublishedArtifactMismatch {
            digest: expected_digest
                .map(|digest| digest.as_ref().to_owned())
                .unwrap_or_else(|| artifact_root.display().to_string()),
        };
        let root_metadata = fs::symlink_metadata(artifact_root).map_err(|_| mismatch())?;
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err(mismatch());
        }
        if artifact_root.parent() != Some(self.artifacts_root.as_path()) {
            return Err(mismatch());
        }
        verify_artifact_root_inventory(artifact_root).map_err(|_| mismatch())?;

        let record_path = artifact_root.join(ARTIFACT_RECORD_FILE);
        let record_bytes = read_regular_bounded(&record_path, self.artifact_record_limit())
            .map_err(|_| mismatch())?;
        let artifact: PluginPackageArtifactV1 =
            strict_json_from_slice(&record_bytes).map_err(|_| mismatch())?;
        artifact.validate().map_err(|_| mismatch())?;
        if expected_digest.is_some_and(|digest| digest != &artifact.artifact_digest)
            || artifact_root.file_name().and_then(|name| name.to_str())
                != Some(artifact.artifact_digest.as_ref())
        {
            return Err(mismatch());
        }

        let package_root = artifact_root.join(PACKAGE_DIRECTORY);
        let package_metadata = fs::symlink_metadata(&package_root).map_err(|_| mismatch())?;
        if package_metadata.file_type().is_symlink() || !package_metadata.is_dir() {
            return Err(mismatch());
        }
        let manifest_bytes =
            read_regular_bounded(&package_root.join(MANIFEST_FILE), self.limits.max_manifest_bytes)
                .map_err(|_| mismatch())?;
        let manifest: ArtifactEnvelope<PluginPackageV1Manifest> =
            strict_json_from_slice(&manifest_bytes).map_err(|_| mismatch())?;
        if manifest != artifact.manifest {
            return Err(mismatch());
        }

        let observed = inventory_published_files(
            &package_root,
            self.limits,
            &NeverCancel,
        )
        .map_err(|_| mismatch())?;
        if observed != artifact.files {
            return Err(mismatch());
        }
        Ok(StoredPluginArtifact {
            managed_relative_path: format!(
                "{ARTIFACTS_DIRECTORY}/{}",
                artifact.artifact_digest.as_ref()
            ),
            artifact,
            artifact_root: artifact_root.to_path_buf(),
            package_root,
        })
    }

    fn artifact_record_limit(&self) -> u64 {
        self.limits.max_manifest_bytes.saturating_add(
            (self.limits.max_file_count as u64).saturating_mul(ARTIFACT_RECORD_BYTES_PER_FILE),
        )
    }
}

struct PackageScanner<'a> {
    limits: ArtifactStoreLimits,
    staging: &'a Path,
    cancellation: &'a dyn ImportCancellation,
    file_count: usize,
    total_bytes: u64,
    collision_keys: HashSet<String>,
    manifest_bytes: Option<Vec<u8>>,
    files: Vec<ArtifactFileDigest>,
}

impl<'a> PackageScanner<'a> {
    fn new(
        limits: ArtifactStoreLimits,
        staging: &'a Path,
        cancellation: &'a dyn ImportCancellation,
    ) -> Self {
        Self {
            limits,
            staging,
            cancellation,
            file_count: 0,
            total_bytes: 0,
            collision_keys: HashSet::new(),
            manifest_bytes: None,
            files: Vec::new(),
        }
    }

    fn register_file(&mut self, normalized: &str) -> Result<(), PluginArtifactStoreError> {
        self.file_count += 1;
        if self.file_count > self.limits.max_file_count {
            return Err(PluginArtifactStoreError::TooManyFiles {
                observed: self.file_count,
                limit: self.limits.max_file_count,
            });
        }
        let key = windows_collision_key(normalized)?;
        if !self.collision_keys.insert(key) {
            return Err(PluginArtifactStoreError::DuplicateEntry {
                path: normalized.to_owned(),
            });
        }
        Ok(())
    }

    fn check_declared_size(
        &self,
        normalized: &str,
        size: u64,
    ) -> Result<(), PluginArtifactStoreError> {
        let limit = if normalized == MANIFEST_FILE {
            self.limits.max_manifest_bytes
        } else {
            self.limits.max_single_file_bytes
        };
        if size > limit {
            return Err(PluginArtifactStoreError::FileTooLarge {
                path: normalized.to_owned(),
                observed: size,
                limit,
            });
        }
        Ok(())
    }

    fn consume_file(
        &mut self,
        normalized: &str,
        mut reader: impl Read,
    ) -> Result<(), PluginArtifactStoreError> {
        if normalized == MANIFEST_FILE {
            let bytes = self.read_bounded(
                normalized,
                &mut reader,
                self.limits.max_manifest_bytes,
            )?;
            if self.manifest_bytes.replace(bytes).is_some() {
                return Err(PluginArtifactStoreError::DuplicateEntry {
                    path: normalized.to_owned(),
                });
            }
            return Ok(());
        }

        let destination = self.staging.join(PACKAGE_DIRECTORY).join(
            normalized
                .split('/')
                .fold(PathBuf::new(), |mut path, component| {
                    path.push(component);
                    path
                }),
        );
        let parent = destination.parent().ok_or_else(|| {
            PluginArtifactStoreError::UnsafePackagePath {
                path: normalized.to_owned(),
                reason: "file has no package parent".into(),
            }
        })?;
        fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&destination)
            .map_err(|error| io_error(&destination, error))?;
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        let result = (|| {
            let mut buffer = vec![0u8; COPY_BUFFER_BYTES];
            loop {
                check_canceled(self.cancellation)?;
                let read = reader
                    .read(&mut buffer)
                    .map_err(|error| io_error(Path::new(normalized), error))?;
                if read == 0 {
                    break;
                }
                size = checked_size(
                    normalized,
                    size,
                    read as u64,
                    self.limits.max_single_file_bytes,
                )?;
                self.total_bytes = checked_total_size(
                    self.total_bytes,
                    read as u64,
                    self.limits.max_total_bytes,
                )?;
                hasher.update(&buffer[..read]);
                output
                    .write_all(&buffer[..read])
                    .map_err(|error| io_error(&destination, error))?;
            }
            output
                .sync_all()
                .map_err(|error| io_error(&destination, error))
        })();
        if let Err(error) = result {
            drop(output);
            let _ = fs::remove_file(&destination);
            return Err(error);
        }
        self.files.push(ArtifactFileDigest {
            normalized_relative_path: normalized.to_owned(),
            digest: DigestHex::from(hex::encode(hasher.finalize())),
            size_bytes: size,
        });
        Ok(())
    }

    fn read_bounded(
        &mut self,
        normalized: &str,
        reader: &mut impl Read,
        limit: u64,
    ) -> Result<Vec<u8>, PluginArtifactStoreError> {
        let capacity = usize::try_from(limit.min(64 * 1024)).unwrap_or(64 * 1024);
        let mut bytes = Vec::with_capacity(capacity);
        let mut buffer = vec![0u8; COPY_BUFFER_BYTES];
        loop {
            check_canceled(self.cancellation)?;
            let read = reader
                .read(&mut buffer)
                .map_err(|error| io_error(Path::new(normalized), error))?;
            if read == 0 {
                break;
            }
            let observed = checked_size(normalized, bytes.len() as u64, read as u64, limit)?;
            self.total_bytes = checked_total_size(
                self.total_bytes,
                read as u64,
                self.limits.max_total_bytes,
            )?;
            bytes.extend_from_slice(&buffer[..read]);
            debug_assert_eq!(bytes.len() as u64, observed);
        }
        Ok(bytes)
    }

    fn finish(mut self) -> Result<ScannedPackage, PluginArtifactStoreError> {
        let manifest_bytes = self.manifest_bytes.take().ok_or_else(|| {
            PluginArtifactStoreError::InvalidManifest("manifest.json is required".into())
        })?;
        if !self
            .files
            .iter()
            .any(|file| file.normalized_relative_path == ENTRYPOINT_FILE)
        {
            return Err(PluginArtifactStoreError::Contract(
                "main.mjs is required".into(),
            ));
        }
        self.files.sort_by(|left, right| {
            left.normalized_relative_path
                .cmp(&right.normalized_relative_path)
        });
        Ok(ScannedPackage {
            manifest_bytes,
            files: self.files,
        })
    }
}

struct ScannedPackage {
    manifest_bytes: Vec<u8>,
    files: Vec<ArtifactFileDigest>,
}

struct StagingGuard {
    staging_parent: PathBuf,
    path: Option<PathBuf>,
}

impl StagingGuard {
    fn path(&self) -> &Path {
        self.path.as_deref().expect("active staging path")
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
        if path.parent() == Some(self.staging_parent.as_path())
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("import-"))
        {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn inventory_published_files(
    package_root: &Path,
    limits: ArtifactStoreLimits,
    cancellation: &dyn ImportCancellation,
) -> Result<Vec<ArtifactFileDigest>, PluginArtifactStoreError> {
    let mut observed = BTreeMap::new();
    let mut collisions = HashSet::new();
    let mut total = 0u64;
    let mut count = 0usize;
    for entry in WalkDir::new(package_root).follow_links(false).min_depth(1) {
        check_canceled(cancellation)?;
        let entry = entry.map_err(|error| PluginArtifactStoreError::Io {
            path: error
                .path()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| package_root.to_path_buf()),
            source: error
                .into_io_error()
                .unwrap_or_else(|| io::Error::other("cannot inventory artifact")),
        })?;
        let relative = entry.path().strip_prefix(package_root).map_err(|_| {
            PluginArtifactStoreError::UnsafePackagePath {
                path: entry.path().display().to_string(),
                reason: "published entry escapes package root".into(),
            }
        })?;
        let normalized = normalize_relative_path(relative)?;
        let key = windows_collision_key(&normalized)?;
        if !collisions.insert(key) {
            return Err(PluginArtifactStoreError::DuplicateEntry { path: normalized });
        }
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|error| io_error(entry.path(), error))?;
        if metadata.file_type().is_symlink() {
            return Err(PluginArtifactStoreError::UnsafePackagePath {
                path: normalized,
                reason: "published package contains a symbolic link".into(),
            });
        }
        if metadata.is_dir() {
            validate_allowed_directory(&normalized)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(PluginArtifactStoreError::UnsafePackagePath {
                path: normalized,
                reason: "published package contains a special file".into(),
            });
        }
        validate_allowed_file(&normalized)?;
        count += 1;
        if count > limits.max_file_count {
            return Err(PluginArtifactStoreError::TooManyFiles {
                observed: count,
                limit: limits.max_file_count,
            });
        }
        let limit = if normalized == MANIFEST_FILE {
            limits.max_manifest_bytes
        } else {
            limits.max_single_file_bytes
        };
        if metadata.len() > limit {
            return Err(PluginArtifactStoreError::FileTooLarge {
                path: normalized,
                observed: metadata.len(),
                limit,
            });
        }
        total = checked_total_size(total, metadata.len(), limits.max_total_bytes)?;
        if normalized == MANIFEST_FILE {
            continue;
        }
        let file = File::open(entry.path()).map_err(|error| io_error(entry.path(), error))?;
        let (digest, size) = hash_reader(file, &normalized, limit, cancellation)?;
        observed.insert(
            normalized.clone(),
            ArtifactFileDigest {
                normalized_relative_path: normalized,
                digest,
                size_bytes: size,
            },
        );
    }
    Ok(observed.into_values().collect())
}

fn hash_reader(
    mut reader: impl Read,
    normalized: &str,
    limit: u64,
    cancellation: &dyn ImportCancellation,
) -> Result<(DigestHex, u64), PluginArtifactStoreError> {
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buffer = vec![0u8; COPY_BUFFER_BYTES];
    loop {
        check_canceled(cancellation)?;
        let read = reader
            .read(&mut buffer)
            .map_err(|error| io_error(Path::new(normalized), error))?;
        if read == 0 {
            break;
        }
        size = checked_size(normalized, size, read as u64, limit)?;
        hasher.update(&buffer[..read]);
    }
    Ok((DigestHex::from(hex::encode(hasher.finalize())), size))
}

fn normalize_relative_path(path: &Path) -> Result<String, PluginArtifactStoreError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(PluginArtifactStoreError::UnsafePackagePath {
            path: path.display().to_string(),
            reason: "path must be non-empty and relative".into(),
        });
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| {
                    PluginArtifactStoreError::UnsafePackagePath {
                        path: path.display().to_string(),
                        reason: "path must be valid UTF-8".into(),
                    }
                })?;
                if value.is_empty()
                    || value.contains('\\')
                    || value.contains('/')
                    || value.contains(':')
                    || value.len() > MAX_PATH_COMPONENT_BYTES
                {
                    return Err(PluginArtifactStoreError::UnsafePackagePath {
                        path: path.display().to_string(),
                        reason: "path component contains a forbidden separator or drive marker"
                            .into(),
                    });
                }
                let nfc = value.nfc().collect::<String>();
                if nfc != value {
                    return Err(PluginArtifactStoreError::UnsafePackagePath {
                        path: path.display().to_string(),
                        reason: "path components must use Unicode NFC normalization".into(),
                    });
                }
                components.push(value);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(PluginArtifactStoreError::UnsafePackagePath {
                    path: path.display().to_string(),
                    reason: "absolute and traversal components are forbidden".into(),
                });
            }
        }
    }
    if components.is_empty() {
        return Err(PluginArtifactStoreError::UnsafePackagePath {
            path: path.display().to_string(),
            reason: "path has no regular components".into(),
        });
    }
    let normalized = components.join("/");
    if normalized.len() > MAX_NORMALIZED_PATH_BYTES {
        return Err(PluginArtifactStoreError::UnsafePackagePath {
            path: path.display().to_string(),
            reason: format!(
                "normalized path exceeds {MAX_NORMALIZED_PATH_BYTES} UTF-8 bytes"
            ),
        });
    }
    windows_collision_key(&normalized)?;
    Ok(normalized)
}

fn windows_collision_key(path: &str) -> Result<String, PluginArtifactStoreError> {
    let mut normalized = Vec::new();
    for component in path.split('/') {
        let trimmed = component.trim_end_matches([' ', '.']);
        if trimmed.is_empty() || trimmed != component || is_windows_reserved_name(trimmed) {
            return Err(PluginArtifactStoreError::UnsafePackagePath {
                path: path.to_owned(),
                reason: "path is not stable under Windows filename semantics".into(),
            });
        }
        normalized.push(trimmed.to_lowercase());
    }
    Ok(normalized.join("/"))
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

fn validate_allowed_file(normalized: &str) -> Result<(), PluginArtifactStoreError> {
    if normalized == MANIFEST_FILE
        || normalized == ENTRYPOINT_FILE
        || normalized == SOURCE_MAP_FILE
        || normalized
            .strip_prefix(&format!("{RESOURCES_DIRECTORY}/"))
            .is_some_and(|suffix| !suffix.is_empty())
    {
        Ok(())
    } else {
        Err(PluginArtifactStoreError::UnsupportedEntry {
            path: normalized.to_owned(),
        })
    }
}

fn validate_allowed_directory(normalized: &str) -> Result<(), PluginArtifactStoreError> {
    if normalized == RESOURCES_DIRECTORY
        || normalized.starts_with(&format!("{RESOURCES_DIRECTORY}/"))
    {
        Ok(())
    } else {
        Err(PluginArtifactStoreError::UnsupportedEntry {
            path: normalized.to_owned(),
        })
    }
}

fn zip_entry_is_special(mode: Option<u32>) -> bool {
    if zip_safe::zip_entry_is_symlink(mode) {
        return true;
    }
    let Some(mode) = mode else {
        return false;
    };
    let kind = mode & 0o170000;
    kind != 0 && kind != 0o100000 && kind != 0o040000
}

fn checked_size(
    path: &str,
    current: u64,
    additional: u64,
    limit: u64,
) -> Result<u64, PluginArtifactStoreError> {
    let observed = current.saturating_add(additional);
    if observed > limit {
        return Err(PluginArtifactStoreError::FileTooLarge {
            path: path.to_owned(),
            observed,
            limit,
        });
    }
    Ok(observed)
}

fn checked_total_size(
    current: u64,
    additional: u64,
    limit: u64,
) -> Result<u64, PluginArtifactStoreError> {
    let observed = current.saturating_add(additional);
    if observed > limit {
        return Err(PluginArtifactStoreError::TotalSizeExceeded { observed, limit });
    }
    Ok(observed)
}

fn check_canceled(
    cancellation: &dyn ImportCancellation,
) -> Result<(), PluginArtifactStoreError> {
    if cancellation.is_cancelled() {
        Err(PluginArtifactStoreError::Canceled)
    } else {
        Ok(())
    }
}

fn validate_digest_segment(value: &str) -> Result<(), PluginArtifactStoreError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(PluginArtifactStoreError::UnsafeManagedPath {
            path: PathBuf::from(value),
        })
    }
}

fn ensure_directory_without_symlink(path: &Path) -> Result<(), PluginArtifactStoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(PluginArtifactStoreError::UnsafeManagedPath {
                path: path.to_path_buf(),
            })
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|error| io_error(path, error))?;
            let metadata =
                fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(PluginArtifactStoreError::UnsafeManagedPath {
                    path: path.to_path_buf(),
                });
            }
            Ok(())
        }
        Err(error) => Err(io_error(path, error)),
    }
}

fn ensure_direct_child_directory(
    parent: &Path,
    child: &Path,
) -> Result<(), PluginArtifactStoreError> {
    if child.parent() != Some(parent) {
        return Err(PluginArtifactStoreError::UnsafeManagedPath {
            path: child.to_path_buf(),
        });
    }
    ensure_directory_without_symlink(child)?;
    let canonical = fs::canonicalize(child).map_err(|error| io_error(child, error))?;
    if canonical.parent() != Some(parent) {
        return Err(PluginArtifactStoreError::UnsafeManagedPath {
            path: child.to_path_buf(),
        });
    }
    Ok(())
}

fn read_regular_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, PluginArtifactStoreError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > limit {
        return Err(PluginArtifactStoreError::UnsafeManagedPath {
            path: path.to_path_buf(),
        });
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .map_err(|error| io_error(path, error))?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error(path, error))?;
    if bytes.len() as u64 > limit {
        return Err(PluginArtifactStoreError::FileTooLarge {
            path: path.display().to_string(),
            observed: bytes.len() as u64,
            limit,
        });
    }
    Ok(bytes)
}

fn verify_artifact_root_inventory(
    artifact_root: &Path,
) -> Result<(), PluginArtifactStoreError> {
    let mut entries = fs::read_dir(artifact_root)
        .map_err(|error| io_error(artifact_root, error))?
        .map(|entry| entry.map_err(|error| io_error(artifact_root, error)))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    if entries.len() != 2 {
        return Err(PluginArtifactStoreError::UnsafeManagedPath {
            path: artifact_root.to_path_buf(),
        });
    }
    for entry in entries {
        let name = entry.file_name();
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|error| io_error(&entry.path(), error))?;
        let valid = if name == ARTIFACT_RECORD_FILE {
            metadata.is_file() && !metadata.file_type().is_symlink()
        } else if name == PACKAGE_DIRECTORY {
            metadata.is_dir() && !metadata.file_type().is_symlink()
        } else {
            false
        };
        if !valid {
            return Err(PluginArtifactStoreError::UnsafeManagedPath {
                path: entry.path(),
            });
        }
    }
    Ok(())
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), PluginArtifactStoreError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| io_error(path, error))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(path, error))
}

fn remove_owned_published_directory(artifacts_root: &Path, target: &Path) {
    if target.parent() != Some(artifacts_root)
        || !target
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.len() == 64
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            })
    {
        return;
    }
    let _ = fs::remove_dir_all(target);
    let _ = sync_directory_if_supported(artifacts_root);
}

fn sync_tree_directories(root: &Path) -> Result<(), PluginArtifactStoreError> {
    let mut directories = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_dir())
        .map(|entry| entry.into_path())
        .collect::<Vec<_>>();
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        sync_directory_if_supported(&directory)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory_if_supported(path: &Path) -> Result<(), PluginArtifactStoreError> {
    match File::open(path).and_then(|file| file.sync_all()) {
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
fn sync_directory_if_supported(_path: &Path) -> Result<(), PluginArtifactStoreError> {
    Ok(())
}

fn io_error(path: &Path, source: io::Error) -> PluginArtifactStoreError {
    PluginArtifactStoreError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn strict_json_from_slice<T>(bytes: &[u8]) -> Result<T, PluginArtifactStoreError>
where
    T: for<'de> Deserialize<'de>,
{
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictJsonValueSeed
        .deserialize(&mut deserializer)
        .map_err(|error| PluginArtifactStoreError::InvalidManifest(error.to_string()))?;
    deserializer
        .end()
        .map_err(|error| PluginArtifactStoreError::InvalidManifest(error.to_string()))?;
    serde_json::from_value(value)
        .map_err(|error| PluginArtifactStoreError::InvalidManifest(error.to_string()))
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
        Ok(Value::String(value.to_owned()))
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
            let value = map.next_value_seed(StrictJsonValueSeed)?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}
