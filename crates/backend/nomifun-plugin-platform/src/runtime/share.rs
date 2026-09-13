use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use nomifun_agent_contracts::{
    canonical_json_bytes, digest_bytes, ArtifactId, DigestHex, MiniAppId,
    MiniAppImportedTestProvenance, MiniAppReleaseArtifactV1, MiniAppShareBundleId,
    MiniAppShareBundleV1, MiniAppSourceBundle, MINIAPP_RELEASE_PROFILE_VERSION,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::runtime::{
    PluginRuntimeReleaseFileBytes, PluginRuntimeSourceFile, PluginRuntimeSourceFileDigest,
    PluginRuntimeSourceSnapshot, PluginRuntimeStoredRelease, MINIAPP_SOURCE_STORE_FORMAT_VERSION,
};

const BUNDLE_FILE: &str = "bundle.json";
const ARTIFACT_FILE: &str = "artifact.json";
const MANIFEST_FILE: &str = "manifest.json";
const FILES_DIRECTORY: &str = "files";
const RELEASE_DIRECTORY: &str = "release";
const SOURCE_DIRECTORY: &str = "source";
const SOURCE_SNAPSHOT_FILE: &str = "snapshot.json";
const DEPENDENCY_LOCK_FILE: &str = "dependency-lock.json";
const STAGING_PREFIX: &str = ".nomifun-miniapp-share-staging-";
const MAX_PATH_BYTES: usize = 1_024;
const MAX_COMPONENT_BYTES: usize = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluginRuntimeShareBundleLimits {
    pub max_release_file_count: usize,
    pub max_source_file_count: usize,
    pub max_single_file_bytes: u64,
    pub max_release_bytes: u64,
    pub max_source_bytes: u64,
    pub max_dependency_lock_bytes: u64,
    pub max_metadata_bytes: u64,
    pub max_bundle_bytes: u64,
}

impl Default for PluginRuntimeShareBundleLimits {
    fn default() -> Self {
        Self {
            max_release_file_count: 4_096,
            max_source_file_count: 4_096,
            max_single_file_bytes: 64 * 1024 * 1024,
            max_release_bytes: 256 * 1024 * 1024,
            max_source_bytes: 256 * 1024 * 1024,
            max_dependency_lock_bytes: 8 * 1024 * 1024,
            max_metadata_bytes: 4 * 1024 * 1024,
            max_bundle_bytes: 528 * 1024 * 1024,
        }
    }
}

impl PluginRuntimeShareBundleLimits {
    fn validate(self) -> Result<Self, PluginRuntimeShareBundleError> {
        if self.max_release_file_count == 0
            || self.max_source_file_count == 0
            || self.max_single_file_bytes == 0
            || self.max_release_bytes < self.max_single_file_bytes
            || self.max_source_bytes < self.max_single_file_bytes
            || self.max_dependency_lock_bytes == 0
            || self.max_metadata_bytes == 0
            || self.max_bundle_bytes
                < self
                    .max_release_bytes
                    .saturating_add(self.max_source_bytes)
        {
            return Err(PluginRuntimeShareBundleError::InvalidLimits);
        }
        Ok(self)
    }
}

#[derive(Debug, Error)]
pub enum PluginRuntimeShareBundleError {
    #[error("Plugin Share Bundle limits are invalid")]
    InvalidLimits,
    #[error("Plugin Share Bundle input is invalid: {0}")]
    InvalidInput(String),
    #[error("Plugin Share Bundle path is invalid: {path} ({reason})")]
    InvalidPath { path: String, reason: String },
    #[error("Plugin Share Bundle destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("Plugin Share Bundle inventory is invalid: {0}")]
    InvalidInventory(String),
    #[error("Plugin Share Bundle digest or byte inventory was modified: {0}")]
    Tampered(String),
    #[error("Plugin Share Bundle exceeds a size limit: {0}")]
    Oversize(String),
    #[error("Plugin Share Bundle filesystem operation failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Plugin Share Bundle canonical serialization failed: {0}")]
    Canonical(String),
}

pub struct PluginRuntimeShareSourceExport<'a> {
    pub snapshot: &'a PluginRuntimeSourceSnapshot,
    pub source_archive_artifact_id: ArtifactId,
    pub dependency_lock_artifact_id: ArtifactId,
}

pub struct PluginRuntimeShareBundleExport<'a> {
    pub bundle_id: MiniAppShareBundleId,
    pub source_miniapp_id: Option<MiniAppId>,
    pub release: &'a PluginRuntimeStoredRelease,
    pub source: Option<PluginRuntimeShareSourceExport<'a>>,
    pub test_provenance: Option<MiniAppImportedTestProvenance>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PluginRuntimeImportedRelease {
    pub artifact: MiniAppReleaseArtifactV1,
    pub manifest_bytes: Vec<u8>,
    pub files: Vec<PluginRuntimeReleaseFileBytes>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeImportedSource {
    pub source: MiniAppSourceBundle,
    pub dependency_lock: Vec<u8>,
    pub files: Vec<PluginRuntimeSourceFile>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PluginRuntimeImportedShareBundle {
    pub bundle: MiniAppShareBundleV1,
    pub release: PluginRuntimeImportedRelease,
    pub source: Option<PluginRuntimeImportedSource>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PluginRuntimeShareImportEntry {
    ShareBundle(PluginRuntimeImportedShareBundle),
    PrebuiltRelease(PluginRuntimeImportedRelease),
}

#[derive(Clone, Debug)]
pub struct PluginRuntimeShareBundleFilesystem {
    limits: PluginRuntimeShareBundleLimits,
}

impl Default for PluginRuntimeShareBundleFilesystem {
    fn default() -> Self {
        Self {
            limits: PluginRuntimeShareBundleLimits::default(),
        }
    }
}

impl PluginRuntimeShareBundleFilesystem {
    pub fn new(limits: PluginRuntimeShareBundleLimits) -> Result<Self, PluginRuntimeShareBundleError> {
        Ok(Self {
            limits: limits.validate()?,
        })
    }

    pub fn export(
        &self,
        request: PluginRuntimeShareBundleExport<'_>,
        destination: impl AsRef<Path>,
    ) -> Result<MiniAppShareBundleV1, PluginRuntimeShareBundleError> {
        let prepared = prepare_export(request, self.limits)?;
        let destination = destination.as_ref();
        let parent = destination.parent().ok_or_else(|| {
            PluginRuntimeShareBundleError::InvalidInput(
                "Share Bundle destination must have an existing parent directory".into(),
            )
        })?;
        ensure_regular_directory(parent)?;
        ensure_normal_destination(destination)?;
        match fs::symlink_metadata(destination) {
            Ok(_) => {
                return Err(PluginRuntimeShareBundleError::DestinationExists(
                    destination.to_path_buf(),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(destination, error)),
        }

        let mut staging = StagingGuard::allocate(parent)?;
        write_bundle_tree(staging.path(), &prepared)?;
        sync_tree_directories(staging.path())?;
        match fs::rename(staging.path(), destination) {
            Ok(()) => {}
            Err(error)
                if error.kind() == io::ErrorKind::AlreadyExists
                    || fs::symlink_metadata(destination).is_ok() =>
            {
                return Err(PluginRuntimeShareBundleError::DestinationExists(
                    destination.to_path_buf(),
                ));
            }
            Err(error) => return Err(io_error(destination, error)),
        }
        sync_directory_if_supported(parent)?;
        staging.disarm();
        Ok(prepared.bundle)
    }

    pub fn import_share_bundle(
        &self,
        root: impl AsRef<Path>,
    ) -> Result<PluginRuntimeImportedShareBundle, PluginRuntimeShareBundleError> {
        import_share_bundle(root.as_ref(), self.limits)
    }

    pub fn import_prebuilt_release(
        &self,
        root: impl AsRef<Path>,
    ) -> Result<PluginRuntimeImportedRelease, PluginRuntimeShareBundleError> {
        let root = root.as_ref();
        let observed = scan_tree(root, self.limits)?;
        if observed.files.contains(BUNDLE_FILE) {
            return Err(PluginRuntimeShareBundleError::InvalidInventory(
                "a Share Bundle must be imported through import_share_bundle".into(),
            ));
        }
        import_release(root, "", &observed, self.limits)
    }

    pub fn import(
        &self,
        root: impl AsRef<Path>,
    ) -> Result<PluginRuntimeShareImportEntry, PluginRuntimeShareBundleError> {
        let root = root.as_ref();
        let observed = scan_tree(root, self.limits)?;
        let has_bundle = observed.files.contains(BUNDLE_FILE);
        let has_artifact = observed.files.contains(ARTIFACT_FILE);
        match (has_bundle, has_artifact) {
            (true, false) => {
                import_share_bundle_with_inventory(root, observed, self.limits)
                    .map(PluginRuntimeShareImportEntry::ShareBundle)
            }
            (false, true) => import_release(root, "", &observed, self.limits)
                .map(PluginRuntimeShareImportEntry::PrebuiltRelease),
            _ => Err(PluginRuntimeShareBundleError::InvalidInventory(
                "directory must contain exactly one supported Share Bundle or prebuilt Release layout"
                    .into(),
            )),
        }
    }
}

pub fn cleanup_miniapp_share_staging(
    parent: impl AsRef<Path>,
) -> Result<usize, PluginRuntimeShareBundleError> {
    let parent = parent.as_ref();
    ensure_regular_directory(parent)?;
    let mut removed = 0;
    for entry in fs::read_dir(parent).map_err(|error| io_error(parent, error))? {
        let entry = entry.map_err(|error| io_error(parent, error))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(id) = name.strip_prefix(STAGING_PREFIX) else {
            continue;
        };
        if Uuid::parse_str(id).is_err() {
            continue;
        }
        let path = entry.path();
        validate_removal_tree(&path)?;
        fs::remove_dir_all(&path).map_err(|error| io_error(&path, error))?;
        removed += 1;
    }
    sync_directory_if_supported(parent)?;
    Ok(removed)
}

struct PreparedExport {
    bundle: MiniAppShareBundleV1,
    release: PluginRuntimeImportedRelease,
    source: Option<PreparedSource>,
}

struct PreparedSource {
    snapshot: SourceSnapshotRecord,
    dependency_lock: Vec<u8>,
    files: Vec<PluginRuntimeSourceFile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceSnapshotRecord {
    format_version: String,
    files: Vec<PluginRuntimeSourceFileDigest>,
    snapshot_digest: DigestHex,
}

#[derive(Serialize)]
struct SourceSnapshotDigestInput<'a> {
    format_version: &'a str,
    files: &'a [PluginRuntimeSourceFileDigest],
}

struct ObservedTree {
    files: BTreeSet<String>,
    directories: BTreeSet<String>,
}

fn prepare_export(
    request: PluginRuntimeShareBundleExport<'_>,
    limits: PluginRuntimeShareBundleLimits,
) -> Result<PreparedExport, PluginRuntimeShareBundleError> {
    let release = validate_release_payload(
        request.release.artifact.clone(),
        request.release.manifest_bytes.clone(),
        request.release.files.clone(),
        limits,
    )?;

    let (source_contract, source) = match request.source {
        Some(source) => {
            let source_miniapp_id = request.source_miniapp_id.as_ref().ok_or_else(|| {
                PluginRuntimeShareBundleError::InvalidInput(
                    "a Share Bundle carrying Source must identify its source Plugin".into(),
                )
            })?;
            if &source.snapshot.project.miniapp_id != source_miniapp_id {
                return Err(PluginRuntimeShareBundleError::InvalidInput(
                    "source Plugin identity does not match the Source snapshot".into(),
                ));
            }
            if source.snapshot.project.project_id.as_ref().is_empty()
                || source.source_archive_artifact_id.as_ref().is_empty()
                || source.dependency_lock_artifact_id.as_ref().is_empty()
            {
                return Err(PluginRuntimeShareBundleError::InvalidInput(
                    "Source artifact and Project identities must be non-empty".into(),
                ));
            }
            if source.snapshot.project.build_profile_version.as_ref()
                != MINIAPP_RELEASE_PROFILE_VERSION
                || source.snapshot.dependency_lock_digest
                    != release.artifact.manifest.payload.dependency_lock_digest
            {
                return Err(PluginRuntimeShareBundleError::InvalidInput(
                    "Source, dependency lock, Build Profile, and Release do not form one lineage"
                        .into(),
                ));
            }
            let prepared = validate_source_payload(
                source.snapshot.files.clone(),
                source.snapshot.dependency_lock.clone(),
                source.snapshot.source_snapshot_digest.clone(),
                source.snapshot.dependency_lock_digest.clone(),
                limits,
            )?;
            let contract = MiniAppSourceBundle {
                project_id: source.snapshot.project.project_id.clone(),
                source_archive_artifact_id: source.source_archive_artifact_id,
                source_snapshot_digest: prepared.snapshot.snapshot_digest.clone(),
                dependency_lock_artifact_id: source.dependency_lock_artifact_id,
                dependency_lock_digest: digest_bytes(&prepared.dependency_lock),
                build_profile_version: source.snapshot.project.build_profile_version.clone(),
            };
            (Some(contract), Some(prepared))
        }
        None => (None, None),
    };

    let bundle = MiniAppShareBundleV1::new(
        request.bundle_id,
        request.source_miniapp_id,
        release.artifact.clone(),
        source_contract,
        request.test_provenance,
    )
    .map_err(|error| PluginRuntimeShareBundleError::InvalidInput(error.to_string()))?;
    Ok(PreparedExport {
        bundle,
        release,
        source,
    })
}

fn validate_release_payload(
    artifact: MiniAppReleaseArtifactV1,
    manifest_bytes: Vec<u8>,
    mut files: Vec<PluginRuntimeReleaseFileBytes>,
    limits: PluginRuntimeShareBundleLimits,
) -> Result<PluginRuntimeImportedRelease, PluginRuntimeShareBundleError> {
    artifact
        .validate()
        .map_err(|error| PluginRuntimeShareBundleError::InvalidInput(error.to_string()))?;
    if artifact.files.len() > limits.max_release_file_count {
        return Err(PluginRuntimeShareBundleError::Oversize(
            "Release contains too many files".into(),
        ));
    }
    let expected_manifest = canonical_bytes(&artifact.manifest)?;
    if manifest_bytes != expected_manifest {
        return Err(PluginRuntimeShareBundleError::Tampered(
            "Release manifest bytes are not canonical or do not match the Artifact".into(),
        ));
    }
    files.sort_by(|left, right| {
        left.normalized_relative_path
            .cmp(&right.normalized_relative_path)
    });
    let mut collisions = BTreeSet::new();
    let mut total = 0u64;
    if files.len() != artifact.files.len() {
        return Err(PluginRuntimeShareBundleError::Tampered(
            "Release file inventory is incomplete".into(),
        ));
    }
    for (actual, expected) in files.iter().zip(&artifact.files) {
        validate_portable_relative_path(&actual.normalized_relative_path)?;
        if !collisions.insert(portable_collision_key(&actual.normalized_relative_path)?) {
            return Err(PluginRuntimeShareBundleError::InvalidPath {
                path: actual.normalized_relative_path.clone(),
                reason: "duplicate or case/NFC-colliding Release path".into(),
            });
        }
        if actual.normalized_relative_path != expected.normalized_relative_path
            || actual.bytes.len() as u64 != expected.size_bytes
            || digest_bytes(&actual.bytes) != expected.digest
        {
            return Err(PluginRuntimeShareBundleError::Tampered(format!(
                "Release file {} does not match its Artifact inventory",
                expected.normalized_relative_path
            )));
        }
        enforce_file_size(
            &actual.normalized_relative_path,
            actual.bytes.len() as u64,
            limits.max_single_file_bytes,
        )?;
        total = total.saturating_add(actual.bytes.len() as u64);
        if total > limits.max_release_bytes {
            return Err(PluginRuntimeShareBundleError::Oversize(
                "Release bytes exceed the configured total limit".into(),
            ));
        }
    }
    Ok(PluginRuntimeImportedRelease {
        artifact,
        manifest_bytes,
        files,
    })
}

fn validate_source_payload(
    mut files: Vec<PluginRuntimeSourceFile>,
    dependency_lock: Vec<u8>,
    expected_snapshot_digest: DigestHex,
    expected_lock_digest: DigestHex,
    limits: PluginRuntimeShareBundleLimits,
) -> Result<PreparedSource, PluginRuntimeShareBundleError> {
    if files.len() > limits.max_source_file_count {
        return Err(PluginRuntimeShareBundleError::Oversize(
            "Source contains too many files".into(),
        ));
    }
    enforce_file_size(
        DEPENDENCY_LOCK_FILE,
        dependency_lock.len() as u64,
        limits.max_dependency_lock_bytes,
    )?;
    if digest_bytes(&dependency_lock) != expected_lock_digest {
        return Err(PluginRuntimeShareBundleError::Tampered(
            "dependency lock digest does not match the Source snapshot".into(),
        ));
    }
    files.sort_by(|left, right| {
        left.normalized_relative_path
            .cmp(&right.normalized_relative_path)
    });
    let mut digest_files = Vec::with_capacity(files.len());
    let mut collisions = BTreeSet::new();
    let mut total = 0u64;
    for file in &files {
        validate_portable_relative_path(&file.normalized_relative_path)?;
        if !collisions.insert(portable_collision_key(&file.normalized_relative_path)?) {
            return Err(PluginRuntimeShareBundleError::InvalidPath {
                path: file.normalized_relative_path.clone(),
                reason: "duplicate or case/NFC-colliding Source path".into(),
            });
        }
        if file.bytes.is_empty()
            || file.bytes.len() as u64 != file.size_bytes
            || digest_bytes(&file.bytes) != file.digest
        {
            return Err(PluginRuntimeShareBundleError::Tampered(format!(
                "Source file {} does not match its snapshot inventory",
                file.normalized_relative_path
            )));
        }
        enforce_file_size(
            &file.normalized_relative_path,
            file.size_bytes,
            limits.max_single_file_bytes,
        )?;
        total = total.saturating_add(file.size_bytes);
        if total > limits.max_source_bytes {
            return Err(PluginRuntimeShareBundleError::Oversize(
                "Source bytes exceed the configured total limit".into(),
            ));
        }
        digest_files.push(PluginRuntimeSourceFileDigest {
            normalized_relative_path: file.normalized_relative_path.clone(),
            digest: file.digest.clone(),
            size_bytes: file.size_bytes,
        });
    }
    let observed_snapshot_digest = source_snapshot_digest(&digest_files)?;
    if observed_snapshot_digest != expected_snapshot_digest {
        return Err(PluginRuntimeShareBundleError::Tampered(
            "Source snapshot digest does not match its file inventory".into(),
        ));
    }
    Ok(PreparedSource {
        snapshot: SourceSnapshotRecord {
            format_version: MINIAPP_SOURCE_STORE_FORMAT_VERSION.into(),
            files: digest_files,
            snapshot_digest: observed_snapshot_digest,
        },
        dependency_lock,
        files,
    })
}

fn write_bundle_tree(
    root: &Path,
    prepared: &PreparedExport,
) -> Result<(), PluginRuntimeShareBundleError> {
    write_new_synced(&root.join(BUNDLE_FILE), &canonical_bytes(&prepared.bundle)?)?;
    write_release_tree(&root.join(RELEASE_DIRECTORY), &prepared.release)?;
    if let Some(source) = &prepared.source {
        let source_root = root.join(SOURCE_DIRECTORY);
        fs::create_dir(&source_root).map_err(|error| io_error(&source_root, error))?;
        write_new_synced(
            &source_root.join(SOURCE_SNAPSHOT_FILE),
            &canonical_bytes(&source.snapshot)?,
        )?;
        write_new_synced(
            &source_root.join(DEPENDENCY_LOCK_FILE),
            &source.dependency_lock,
        )?;
        write_files(
            &source_root.join(FILES_DIRECTORY),
            source
                .files
                .iter()
                .map(|file| {
                    (
                        file.normalized_relative_path.as_str(),
                        file.bytes.as_slice(),
                    )
                }),
        )?;
    }
    Ok(())
}

fn write_release_tree(
    root: &Path,
    release: &PluginRuntimeImportedRelease,
) -> Result<(), PluginRuntimeShareBundleError> {
    fs::create_dir(root).map_err(|error| io_error(root, error))?;
    write_new_synced(
        &root.join(ARTIFACT_FILE),
        &canonical_bytes(&release.artifact)?,
    )?;
    write_new_synced(&root.join(MANIFEST_FILE), &release.manifest_bytes)?;
    write_files(
        &root.join(FILES_DIRECTORY),
        release.files.iter().map(|file| {
            (
                file.normalized_relative_path.as_str(),
                file.bytes.as_slice(),
            )
        }),
    )
}

fn write_files<'a>(
    root: &Path,
    files: impl Iterator<Item = (&'a str, &'a [u8])>,
) -> Result<(), PluginRuntimeShareBundleError> {
    fs::create_dir(root).map_err(|error| io_error(root, error))?;
    let mut created = BTreeSet::new();
    for (relative, bytes) in files {
        let target = join_relative(root, relative)?;
        let parent = target.parent().ok_or_else(|| {
            PluginRuntimeShareBundleError::InvalidInventory("file has no parent directory".into())
        })?;
        create_relative_directories(root, parent, &mut created)?;
        write_new_synced(&target, bytes)?;
    }
    Ok(())
}

fn import_share_bundle(
    root: &Path,
    limits: PluginRuntimeShareBundleLimits,
) -> Result<PluginRuntimeImportedShareBundle, PluginRuntimeShareBundleError> {
    let observed = scan_tree(root, limits)?;
    import_share_bundle_with_inventory(root, observed, limits)
}

fn import_share_bundle_with_inventory(
    root: &Path,
    observed: ObservedTree,
    limits: PluginRuntimeShareBundleLimits,
) -> Result<PluginRuntimeImportedShareBundle, PluginRuntimeShareBundleError> {
    let bundle: MiniAppShareBundleV1 = read_canonical(
        &root.join(BUNDLE_FILE),
        limits.max_metadata_bytes,
    )?;
    bundle
        .validate()
        .map_err(|error| PluginRuntimeShareBundleError::Tampered(error.to_string()))?;
    let release = import_release(root, RELEASE_DIRECTORY, &observed, limits)?;
    if release.artifact != bundle.release {
        return Err(PluginRuntimeShareBundleError::Tampered(
            "bundle.json and release/artifact.json identify different Releases".into(),
        ));
    }

    let source = match &bundle.source {
        Some(source_contract) => {
            let source = import_source(root, source_contract, &observed, limits)?;
            Some(source)
        }
        None => None,
    };
    let mut expected_files = BTreeSet::from([BUNDLE_FILE.to_owned()]);
    expected_files.extend(release_file_inventory(RELEASE_DIRECTORY, &release.artifact));
    if let Some(source) = &source {
        expected_files.extend(source_file_inventory(&source.files));
    }
    require_exact_inventory(&observed, &expected_files)?;
    Ok(PluginRuntimeImportedShareBundle {
        bundle,
        release,
        source,
    })
}

fn import_release(
    root: &Path,
    prefix: &str,
    observed: &ObservedTree,
    limits: PluginRuntimeShareBundleLimits,
) -> Result<PluginRuntimeImportedRelease, PluginRuntimeShareBundleError> {
    let artifact_path = rooted(root, prefix, ARTIFACT_FILE);
    let artifact: MiniAppReleaseArtifactV1 =
        read_canonical(&artifact_path, limits.max_metadata_bytes)?;
    artifact
        .validate()
        .map_err(|error| PluginRuntimeShareBundleError::Tampered(error.to_string()))?;
    let manifest_path = rooted(root, prefix, MANIFEST_FILE);
    let manifest_bytes = read_regular_bounded(&manifest_path, limits.max_metadata_bytes)?;
    let mut files = Vec::with_capacity(artifact.files.len());
    for file in &artifact.files {
        let path = rooted(
            root,
            prefix,
            &format!("{FILES_DIRECTORY}/{}", file.normalized_relative_path),
        );
        let bytes = read_regular_bounded(&path, limits.max_single_file_bytes)?;
        files.push(PluginRuntimeReleaseFileBytes::new(
            file.normalized_relative_path.clone(),
            bytes,
        ));
    }
    let release = validate_release_payload(artifact, manifest_bytes, files, limits)?;
    let expected = release_file_inventory(prefix, &release.artifact);
    if prefix.is_empty() {
        require_exact_inventory(observed, &expected)?;
    } else if !expected.is_subset(&observed.files) {
        return Err(PluginRuntimeShareBundleError::InvalidInventory(
            "Share Bundle Release inventory is incomplete".into(),
        ));
    }
    Ok(release)
}

fn import_source(
    root: &Path,
    source_contract: &MiniAppSourceBundle,
    observed: &ObservedTree,
    limits: PluginRuntimeShareBundleLimits,
) -> Result<PluginRuntimeImportedSource, PluginRuntimeShareBundleError> {
    let source_root = root.join(SOURCE_DIRECTORY);
    let record: SourceSnapshotRecord = read_canonical(
        &source_root.join(SOURCE_SNAPSHOT_FILE),
        limits.max_metadata_bytes,
    )?;
    if record.format_version != MINIAPP_SOURCE_STORE_FORMAT_VERSION {
        return Err(PluginRuntimeShareBundleError::InvalidInput(
            "Source snapshot format version is unsupported".into(),
        ));
    }
    if record.files.len() > limits.max_source_file_count {
        return Err(PluginRuntimeShareBundleError::Oversize(
            "Source contains too many files".into(),
        ));
    }
    let dependency_lock = read_regular_bounded(
        &source_root.join(DEPENDENCY_LOCK_FILE),
        limits.max_dependency_lock_bytes,
    )?;
    let mut files = Vec::with_capacity(record.files.len());
    for file in &record.files {
        let path = join_relative(&source_root.join(FILES_DIRECTORY), &file.normalized_relative_path)?;
        let bytes = read_regular_bounded(&path, limits.max_single_file_bytes)?;
        files.push(PluginRuntimeSourceFile {
            normalized_relative_path: file.normalized_relative_path.clone(),
            digest: file.digest.clone(),
            size_bytes: file.size_bytes,
            bytes,
        });
    }
    let prepared = validate_source_payload(
        files,
        dependency_lock,
        record.snapshot_digest.clone(),
        source_contract.dependency_lock_digest.clone(),
        limits,
    )?;
    if prepared.snapshot != record
        || prepared.snapshot.snapshot_digest != source_contract.source_snapshot_digest
    {
        return Err(PluginRuntimeShareBundleError::Tampered(
            "Source snapshot record does not match bundle.json".into(),
        ));
    }
    let imported = PluginRuntimeImportedSource {
        source: source_contract.clone(),
        dependency_lock: prepared.dependency_lock,
        files: prepared.files,
    };
    let expected = source_file_inventory(&imported.files);
    if !expected.is_subset(&observed.files) {
        return Err(PluginRuntimeShareBundleError::InvalidInventory(
            "Share Bundle Source inventory is incomplete".into(),
        ));
    }
    Ok(imported)
}

fn release_file_inventory(prefix: &str, artifact: &MiniAppReleaseArtifactV1) -> BTreeSet<String> {
    let base = if prefix.is_empty() {
        String::new()
    } else {
        format!("{prefix}/")
    };
    let mut files = BTreeSet::from([
        format!("{base}{ARTIFACT_FILE}"),
        format!("{base}{MANIFEST_FILE}"),
    ]);
    files.extend(
        artifact
            .files
            .iter()
            .map(|file| format!("{base}{FILES_DIRECTORY}/{}", file.normalized_relative_path)),
    );
    files
}

fn source_file_inventory(files: &[PluginRuntimeSourceFile]) -> BTreeSet<String> {
    let mut inventory = BTreeSet::from([
        format!("{SOURCE_DIRECTORY}/{SOURCE_SNAPSHOT_FILE}"),
        format!("{SOURCE_DIRECTORY}/{DEPENDENCY_LOCK_FILE}"),
    ]);
    inventory.extend(files.iter().map(|file| {
        format!(
            "{SOURCE_DIRECTORY}/{FILES_DIRECTORY}/{}",
            file.normalized_relative_path
        )
    }));
    inventory
}

fn require_exact_inventory(
    observed: &ObservedTree,
    expected_files: &BTreeSet<String>,
) -> Result<(), PluginRuntimeShareBundleError> {
    if &observed.files != expected_files {
        let extra = observed
            .files
            .difference(expected_files)
            .cloned()
            .collect::<Vec<_>>();
        let missing = expected_files
            .difference(&observed.files)
            .cloned()
            .collect::<Vec<_>>();
        return Err(PluginRuntimeShareBundleError::InvalidInventory(format!(
            "file inventory differs; extra={extra:?}, missing={missing:?}"
        )));
    }
    let expected_directories = expected_directory_inventory(expected_files);
    if observed.directories != expected_directories {
        return Err(PluginRuntimeShareBundleError::InvalidInventory(
            "directory inventory contains an extra or missing directory".into(),
        ));
    }
    Ok(())
}

fn expected_directory_inventory(files: &BTreeSet<String>) -> BTreeSet<String> {
    let mut directories = BTreeSet::new();
    for file in files {
        let mut current = Path::new(file).parent();
        while let Some(parent) = current {
            if parent.as_os_str().is_empty() {
                break;
            }
            directories.insert(path_to_slashes(parent));
            current = parent.parent();
        }
    }
    directories
}

fn scan_tree(
    root: &Path,
    limits: PluginRuntimeShareBundleLimits,
) -> Result<ObservedTree, PluginRuntimeShareBundleError> {
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
        limits,
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
    limits: PluginRuntimeShareBundleLimits,
) -> Result<(), PluginRuntimeShareBundleError> {
    for entry in fs::read_dir(current).map_err(|error| io_error(current, error))? {
        let entry = entry.map_err(|error| io_error(current, error))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if is_reparse_or_symlink(&metadata) {
            return Err(PluginRuntimeShareBundleError::InvalidInventory(format!(
                "symlinks, junctions, and reparse points are forbidden: {}",
                path.display()
            )));
        }
        let relative = path.strip_prefix(root).map_err(|_| {
            PluginRuntimeShareBundleError::InvalidInventory("bundle entry escaped its root".into())
        })?;
        let normalized = normalize_filesystem_path(relative)?;
        let collision_key = portable_collision_key(&normalized)?;
        if let Some(previous) = collision_keys.insert(collision_key, normalized.clone()) {
            return Err(PluginRuntimeShareBundleError::InvalidPath {
                path: normalized,
                reason: format!("case/NFC collision with {previous}"),
            });
        }
        if metadata.is_dir() {
            directories.insert(normalized);
            scan_directory(
                root,
                &path,
                files,
                directories,
                collision_keys,
                total_bytes,
                limits,
            )?;
        } else if metadata.is_file() {
            *total_bytes = total_bytes.saturating_add(metadata.len());
            if *total_bytes > limits.max_bundle_bytes {
                return Err(PluginRuntimeShareBundleError::Oversize(
                    "directory exceeds the configured bundle byte limit".into(),
                ));
            }
            files.insert(normalized);
        } else {
            return Err(PluginRuntimeShareBundleError::InvalidInventory(format!(
                "special filesystem entries are forbidden: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn source_snapshot_digest(
    files: &[PluginRuntimeSourceFileDigest],
) -> Result<DigestHex, PluginRuntimeShareBundleError> {
    Ok(digest_bytes(&canonical_bytes(&SourceSnapshotDigestInput {
        format_version: MINIAPP_SOURCE_STORE_FORMAT_VERSION,
        files,
    })?))
}

fn read_canonical<T: DeserializeOwned + Serialize>(
    path: &Path,
    limit: u64,
) -> Result<T, PluginRuntimeShareBundleError> {
    let bytes = read_regular_bounded(path, limit)?;
    let value: T = serde_json::from_slice(&bytes)
        .map_err(|error| PluginRuntimeShareBundleError::InvalidInput(error.to_string()))?;
    if canonical_bytes(&value)? != bytes {
        return Err(PluginRuntimeShareBundleError::Tampered(format!(
            "{} must contain byte-for-byte canonical JSON",
            path.display()
        )));
    }
    Ok(value)
}

fn read_regular_bounded(
    path: &Path,
    limit: u64,
) -> Result<Vec<u8>, PluginRuntimeShareBundleError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if is_reparse_or_symlink(&metadata) || !metadata.is_file() {
        return Err(PluginRuntimeShareBundleError::InvalidInventory(format!(
            "expected a regular file: {}",
            path.display()
        )));
    }
    enforce_file_size(&path.display().to_string(), metadata.len(), limit)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .map_err(|error| io_error(path, error))?
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| io_error(path, error))?;
    enforce_file_size(&path.display().to_string(), bytes.len() as u64, limit)?;
    Ok(bytes)
}

fn enforce_file_size(
    path: &str,
    observed: u64,
    limit: u64,
) -> Result<(), PluginRuntimeShareBundleError> {
    if observed > limit {
        Err(PluginRuntimeShareBundleError::Oversize(format!(
            "{path} is {observed} bytes; limit is {limit}"
        )))
    } else {
        Ok(())
    }
}

fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, PluginRuntimeShareBundleError> {
    canonical_json_bytes(value)
        .map_err(|error| PluginRuntimeShareBundleError::Canonical(error.to_string()))
}

fn rooted(root: &Path, prefix: &str, relative: &str) -> PathBuf {
    if prefix.is_empty() {
        root.join(relative)
    } else {
        root.join(prefix).join(relative)
    }
}

fn join_relative(root: &Path, relative: &str) -> Result<PathBuf, PluginRuntimeShareBundleError> {
    validate_portable_relative_path(relative)?;
    let target = relative
        .split('/')
        .fold(root.to_path_buf(), |path, component| path.join(component));
    if !target.starts_with(root) {
        return Err(PluginRuntimeShareBundleError::InvalidInventory(
            "relative path escaped its root".into(),
        ));
    }
    Ok(target)
}

fn normalize_filesystem_path(path: &Path) -> Result<String, PluginRuntimeShareBundleError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(PluginRuntimeShareBundleError::InvalidPath {
            path: path.display().to_string(),
            reason: "path must be a non-empty relative path".into(),
        });
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| PluginRuntimeShareBundleError::InvalidPath {
                    path: path.display().to_string(),
                    reason: "path must be valid UTF-8".into(),
                })?;
                components.push(value);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(PluginRuntimeShareBundleError::InvalidPath {
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

fn validate_portable_relative_path(path: &str) -> Result<(), PluginRuntimeShareBundleError> {
    if path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || path.trim() != path
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path.contains(':')
    {
        return Err(PluginRuntimeShareBundleError::InvalidPath {
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
            return Err(PluginRuntimeShareBundleError::InvalidPath {
                path: path.into(),
                reason: "path is unstable under Windows case or Unicode NFC semantics".into(),
            });
        }
    }
    Ok(())
}

fn portable_collision_key(path: &str) -> Result<String, PluginRuntimeShareBundleError> {
    validate_portable_relative_path(path)?;
    Ok(path
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<String>())
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

fn ensure_normal_destination(path: &Path) -> Result<(), PluginRuntimeShareBundleError> {
    let name = path.file_name().and_then(|value| value.to_str()).ok_or_else(|| {
        PluginRuntimeShareBundleError::InvalidInput(
            "Share Bundle destination must have a UTF-8 final component".into(),
        )
    })?;
    validate_portable_relative_path(name)?;
    Ok(())
}

fn ensure_regular_directory(path: &Path) -> Result<(), PluginRuntimeShareBundleError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
        return Err(PluginRuntimeShareBundleError::InvalidInventory(format!(
            "directory is a symlink, junction, reparse point, or non-directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn create_relative_directories(
    root: &Path,
    parent: &Path,
    created: &mut BTreeSet<PathBuf>,
) -> Result<(), PluginRuntimeShareBundleError> {
    let relative = parent.strip_prefix(root).map_err(|_| {
        PluginRuntimeShareBundleError::InvalidInventory("file parent escaped its root".into())
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(PluginRuntimeShareBundleError::InvalidInventory(
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

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), PluginRuntimeShareBundleError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| io_error(path, error))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(path, error))
}

fn sync_tree_directories(root: &Path) -> Result<(), PluginRuntimeShareBundleError> {
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
) -> Result<(), PluginRuntimeShareBundleError> {
    ensure_regular_directory(root)?;
    directories.push(root.to_path_buf());
    for entry in fs::read_dir(root).map_err(|error| io_error(root, error))? {
        let entry = entry.map_err(|error| io_error(root, error))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if is_reparse_or_symlink(&metadata) {
            return Err(PluginRuntimeShareBundleError::InvalidInventory(
                "staged bundle contains a symlink or reparse point".into(),
            ));
        }
        if metadata.is_dir() {
            collect_directories(&path, directories)?;
        } else if !metadata.is_file() {
            return Err(PluginRuntimeShareBundleError::InvalidInventory(
                "staged bundle contains a special file".into(),
            ));
        }
    }
    Ok(())
}

fn validate_removal_tree(root: &Path) -> Result<(), PluginRuntimeShareBundleError> {
    ensure_regular_directory(root)?;
    for entry in fs::read_dir(root).map_err(|error| io_error(root, error))? {
        let entry = entry.map_err(|error| io_error(root, error))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if is_reparse_or_symlink(&metadata) {
            return Err(PluginRuntimeShareBundleError::InvalidInventory(format!(
                "cleanup target contains a symlink or reparse point: {}",
                path.display()
            )));
        }
        if metadata.is_dir() {
            validate_removal_tree(&path)?;
        } else if !metadata.is_file() {
            return Err(PluginRuntimeShareBundleError::InvalidInventory(format!(
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

fn sync_directory_if_supported(path: &Path) -> Result<(), PluginRuntimeShareBundleError> {
    #[cfg(unix)]
    {
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
    {
        let _ = path;
        Ok(())
    }
}

fn io_error(path: impl Into<PathBuf>, source: io::Error) -> PluginRuntimeShareBundleError {
    PluginRuntimeShareBundleError::Io {
        path: path.into(),
        source,
    }
}

struct StagingGuard {
    parent: PathBuf,
    path: Option<PathBuf>,
}

impl StagingGuard {
    fn allocate(parent: &Path) -> Result<Self, PluginRuntimeShareBundleError> {
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
        if Uuid::parse_str(id).is_err() {
            return;
        }
        if validate_removal_tree(&path).is_ok() {
            let _ = fs::remove_dir_all(&path);
            let _ = sync_directory_if_supported(&self.parent);
        }
    }
}
