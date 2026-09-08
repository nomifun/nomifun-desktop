use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use nomifun_agent_contracts::{
    canonical_json_bytes, digest_bytes, ArtifactEnvelope, ArtifactId, DigestHex,
    JavaScriptBuildProfile, MiniAppReleaseArtifactV1, MiniAppReleaseFile,
    MiniAppReleaseManifestArtifact, VersionString, MINIAPP_RELEASE_PROFILE_VERSION,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::MiniAppSourceScope;

pub const MINIAPP_RELEASE_STORE_FORMAT_VERSION: &str = "2.0.0";

const RELEASES_DIRECTORY: &str = "releases";
const MINIAPPS_DIRECTORY: &str = "miniapps";
const PROJECTS_DIRECTORY: &str = "projects";
const ARTIFACTS_DIRECTORY: &str = "artifacts";
const FILES_DIRECTORY: &str = "files";
const STAGING_DIRECTORY: &str = ".staging";
const ARTIFACT_RECORD_FILE: &str = "artifact.json";
const MANIFEST_FILE: &str = "manifest.json";
const STAGING_PREFIX: &str = "publish-";
const MAX_PATH_BYTES: usize = 1024;
const MAX_COMPONENT_BYTES: usize = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MiniAppReleaseContentKind {
    UiOnly,
    Service,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MiniAppReleaseStoreLimits {
    pub max_file_count: usize,
    pub max_single_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_metadata_bytes: u64,
}

impl Default for MiniAppReleaseStoreLimits {
    fn default() -> Self {
        Self {
            max_file_count: 4_096,
            max_single_file_bytes: 64 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
            max_metadata_bytes: 4 * 1024 * 1024,
        }
    }
}

impl MiniAppReleaseStoreLimits {
    fn validate(self) -> Result<Self, MiniAppReleaseStoreError> {
        if self.max_file_count == 0
            || self.max_single_file_bytes == 0
            || self.max_total_bytes < self.max_single_file_bytes
            || self.max_metadata_bytes == 0
        {
            return Err(MiniAppReleaseStoreError::InvalidLimits);
        }
        Ok(self)
    }
}

#[derive(Debug, Error)]
pub enum MiniAppReleaseStoreError {
    #[error("MiniApp Release Store limits are invalid")]
    InvalidLimits,
    #[error("MiniApp Release Store scope is invalid: {0}")]
    InvalidScope(String),
    #[error("MiniApp Release input is invalid: {0}")]
    InvalidInput(String),
    #[error("MiniApp Release path is invalid: {path} ({reason})")]
    InvalidPath { path: String, reason: String },
    #[error("UI-only MiniApp Release cannot contain Service file {path}")]
    ServiceFileForbidden { path: String },
    #[error("MiniApp Release file is empty: {path}")]
    EmptyFile { path: String },
    #[error("MiniApp Release file is too large: {path} ({observed} > {limit})")]
    FileTooLarge {
        path: String,
        observed: u64,
        limit: u64,
    },
    #[error("MiniApp Release contains too many files ({observed} > {limit})")]
    TooManyFiles { observed: usize, limit: usize },
    #[error("MiniApp Release exceeds the total byte limit ({observed} > {limit})")]
    TotalSizeExceeded { observed: u64, limit: u64 },
    #[error("MiniApp Release file does not match artifact metadata: {path}")]
    FileMismatch { path: String },
    #[error("MiniApp Release digest mismatch: expected {expected}, observed {observed}")]
    DigestMismatch { expected: String, observed: String },
    #[error("MiniApp Release {0} is not present")]
    NotFound(String),
    #[error("published MiniApp Release is missing or has been modified: {0}")]
    PublishedReleaseMismatch(String),
    #[error("MiniApp Release storage is corrupt: {0}")]
    Corrupt(String),
    #[error("MiniApp Release staging cleanup failed: {0}")]
    StagingCleanup(String),
    #[error("MiniApp Release filesystem operation failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("MiniApp Release canonical serialization failed: {0}")]
    Canonical(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppReleaseFileBytes {
    pub normalized_relative_path: String,
    pub bytes: Vec<u8>,
}

impl MiniAppReleaseFileBytes {
    pub fn new(path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            normalized_relative_path: path.into(),
            bytes: bytes.into(),
        }
    }
}

/// The immutable identity that a caller can use to verify that a content-
/// addressed Artifact is the exact Artifact it expected.
///
/// `artifact_digest` intentionally remains content-addressed (Manifest +
/// declared file digests), so two Build lineages may reuse the same bytes.
/// `artifact_id` is nevertheless part of the persisted identity and must not
/// be silently replaced when an Artifact record is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppReleaseArtifactIdentity {
    pub artifact_id: ArtifactId,
    pub artifact_digest: DigestHex,
    pub manifest_digest: DigestHex,
}

impl MiniAppReleaseArtifactIdentity {
    pub fn from_artifact(artifact: &MiniAppReleaseArtifactV1) -> Self {
        Self {
            artifact_id: artifact.artifact_id.clone(),
            artifact_digest: artifact.artifact_digest.clone(),
            manifest_digest: artifact.manifest.payload_digest.clone(),
        }
    }

    pub fn validate(&self) -> Result<(), MiniAppReleaseStoreError> {
        if self.artifact_id.as_ref().is_empty() {
            return Err(MiniAppReleaseStoreError::InvalidInput(
                "expected artifact identity requires a non-empty artifact_id".into(),
            ));
        }
        validate_digest(self.artifact_digest.as_ref())?;
        validate_digest(self.manifest_digest.as_ref())?;
        Ok(())
    }

    fn matches(&self, artifact: &MiniAppReleaseArtifactV1) -> bool {
        self.artifact_id == artifact.artifact_id
            && self.artifact_digest == artifact.artifact_digest
            && self.manifest_digest == artifact.manifest.payload_digest
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MiniAppReleasePublishRequest {
    pub scope: MiniAppSourceScope,
    pub source_snapshot_digest: DigestHex,
    pub dependency_lock_digest: DigestHex,
    pub build_profile: JavaScriptBuildProfile,
    pub build_profile_version: VersionString,
    pub build_generation: u64,
    pub artifact: MiniAppReleaseArtifactV1,
    pub files: Vec<MiniAppReleaseFileBytes>,
    pub content_kind: MiniAppReleaseContentKind,
}

impl MiniAppReleasePublishRequest {
    pub fn ui_only(
        scope: MiniAppSourceScope,
        source_snapshot_digest: DigestHex,
        dependency_lock_digest: DigestHex,
        build_generation: u64,
        artifact: MiniAppReleaseArtifactV1,
        files: Vec<MiniAppReleaseFileBytes>,
    ) -> Self {
        Self {
            scope,
            source_snapshot_digest,
            dependency_lock_digest,
            build_profile: JavaScriptBuildProfile::MiniAppReleaseV1,
            build_profile_version: MINIAPP_RELEASE_PROFILE_VERSION.into(),
            build_generation,
            artifact,
            files,
            content_kind: MiniAppReleaseContentKind::UiOnly,
        }
    }

    pub fn service(
        scope: MiniAppSourceScope,
        source_snapshot_digest: DigestHex,
        dependency_lock_digest: DigestHex,
        build_generation: u64,
        artifact: MiniAppReleaseArtifactV1,
        files: Vec<MiniAppReleaseFileBytes>,
    ) -> Self {
        Self {
            scope,
            source_snapshot_digest,
            dependency_lock_digest,
            build_profile: JavaScriptBuildProfile::MiniAppReleaseV1,
            build_profile_version: MINIAPP_RELEASE_PROFILE_VERSION.into(),
            build_generation,
            artifact,
            files,
            content_kind: MiniAppReleaseContentKind::Service,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MiniAppStoredRelease {
    pub scope: MiniAppSourceScope,
    pub artifact: MiniAppReleaseArtifactV1,
    pub manifest_bytes: Vec<u8>,
    pub files: Vec<MiniAppReleaseFileBytes>,
    pub managed_relative_path: String,
    pub artifact_root: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MiniAppReleasePublishResult {
    pub stored: MiniAppStoredRelease,
    pub already_present: bool,
}

#[derive(Clone, Debug)]
pub struct MiniAppReleaseStore {
    managed_root: PathBuf,
    releases_root: PathBuf,
    staging_root: PathBuf,
    limits: MiniAppReleaseStoreLimits,
    mutation_lock: Arc<Mutex<()>>,
}

impl MiniAppReleaseStore {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, MiniAppReleaseStoreError> {
        Self::new_with_limits(root, MiniAppReleaseStoreLimits::default())
    }

    pub fn new_with_limits(
        root: impl AsRef<Path>,
        limits: MiniAppReleaseStoreLimits,
    ) -> Result<Self, MiniAppReleaseStoreError> {
        let limits = limits.validate()?;
        let requested_root = root.as_ref();
        ensure_directory_without_symlink(requested_root)?;
        let managed_root =
            fs::canonicalize(requested_root).map_err(|error| io_error(requested_root, error))?;
        let releases_root = managed_root.join(RELEASES_DIRECTORY);
        let staging_root = managed_root.join(STAGING_DIRECTORY);
        ensure_direct_child_directory(&managed_root, &releases_root)?;
        ensure_direct_child_directory(&managed_root, &staging_root)?;
        Ok(Self {
            managed_root,
            releases_root,
            staging_root,
            limits,
            mutation_lock: Arc::new(Mutex::new(())),
        })
    }

    pub fn managed_root(&self) -> &Path {
        &self.managed_root
    }

    pub fn staging_root(&self) -> &Path {
        &self.staging_root
    }

    pub fn publish(
        &self,
        request: MiniAppReleasePublishRequest,
    ) -> Result<MiniAppReleasePublishResult, MiniAppReleaseStoreError> {
        let prepared = prepare_request(request, self.limits)?;
        let _guard = self.lock_mutation()?;
        let parent = self.ensure_artifact_parent(&prepared.scope)?;
        let final_root = parent.join(prepared.artifact.artifact_digest.as_ref());

        if fs::symlink_metadata(&final_root).is_ok() {
            let stored = self.verify_published(&prepared.scope, &final_root, None)?;
            compare_stored(&stored, &prepared)?;
            return Ok(MiniAppReleasePublishResult {
                stored,
                already_present: true,
            });
        }

        let mut staging = self.allocate_staging()?;
        let staged_root = staging.path().join("release");
        write_release_tree(&staged_root, &prepared, self.limits)?;
        sync_tree_directories(&staged_root)?;
        match fs::rename(&staged_root, &final_root) {
            Ok(()) => {}
            Err(error)
                if error.kind() == io::ErrorKind::AlreadyExists
                    || final_root.exists() =>
            {
                let stored = self.verify_published(&prepared.scope, &final_root, None)?;
                compare_stored(&stored, &prepared)?;
                return Ok(MiniAppReleasePublishResult {
                    stored,
                    already_present: true,
                });
            }
            Err(error) => return Err(io_error(&final_root, error)),
        }
        sync_directory_if_supported(&parent)?;

        let published = self.verify_published(&prepared.scope, &final_root, None);
        match published {
            Ok(stored) => {
                staging.commit()?;
                Ok(MiniAppReleasePublishResult {
                    stored,
                    already_present: false,
                })
            }
            Err(error) => {
                remove_owned_release_directory(&parent, &final_root);
                Err(error)
            }
        }
    }

    pub fn publish_for(
        &self,
        owner: impl AsRef<str>,
        miniapp_id: impl AsRef<str>,
        project_id: impl AsRef<str>,
        source_snapshot_digest: DigestHex,
        dependency_lock_digest: DigestHex,
        build_generation: u64,
        artifact: &MiniAppReleaseArtifactV1,
        files: &[MiniAppReleaseFileBytes],
    ) -> Result<MiniAppReleasePublishResult, MiniAppReleaseStoreError> {
        let scope = MiniAppSourceScope::new(owner, miniapp_id, project_id)
            .map_err(|error| MiniAppReleaseStoreError::InvalidScope(error.to_string()))?;
        self.publish(MiniAppReleasePublishRequest::ui_only(
            scope,
            source_snapshot_digest,
            dependency_lock_digest,
            build_generation,
            artifact.clone(),
            files.to_vec(),
        ))
    }

    pub fn publish_service_for(
        &self,
        owner: impl AsRef<str>,
        miniapp_id: impl AsRef<str>,
        project_id: impl AsRef<str>,
        source_snapshot_digest: DigestHex,
        dependency_lock_digest: DigestHex,
        build_generation: u64,
        artifact: &MiniAppReleaseArtifactV1,
        files: &[MiniAppReleaseFileBytes],
    ) -> Result<MiniAppReleasePublishResult, MiniAppReleaseStoreError> {
        let scope = MiniAppSourceScope::new(owner, miniapp_id, project_id)
            .map_err(|error| MiniAppReleaseStoreError::InvalidScope(error.to_string()))?;
        self.publish(MiniAppReleasePublishRequest::service(
            scope,
            source_snapshot_digest,
            dependency_lock_digest,
            build_generation,
            artifact.clone(),
            files.to_vec(),
        ))
    }

    pub fn load(
        &self,
        scope: MiniAppSourceScope,
        artifact_digest: impl AsRef<str>,
    ) -> Result<MiniAppStoredRelease, MiniAppReleaseStoreError> {
        let digest = validate_digest(artifact_digest.as_ref())?;
        let _guard = self.lock_mutation()?;
        let root = self
            .artifact_parent(&scope)?
            .join(digest.as_ref());
        if fs::symlink_metadata(&root).is_err() {
            return Err(MiniAppReleaseStoreError::NotFound(digest.0));
        }
        self.verify_published(&scope, &root, None)
    }

    /// Load an Artifact and require all three immutable identity fields to
    /// match the caller's expected typed identity.
    ///
    /// This is intentionally separate from `load`: content-addressed reuse
    /// can legitimately receive a new Build's proposed `artifact_id`, while a
    /// database Release/Artifact cross-check must be able to reject a
    /// tampered or wrong identity explicitly.
    pub fn load_exact(
        &self,
        scope: MiniAppSourceScope,
        expected: &MiniAppReleaseArtifactIdentity,
    ) -> Result<MiniAppStoredRelease, MiniAppReleaseStoreError> {
        validate_scope(&scope)?;
        expected.validate()?;
        let _guard = self.lock_mutation()?;
        let root = self
            .artifact_parent(&scope)?
            .join(expected.artifact_digest.as_ref());
        if fs::symlink_metadata(&root).is_err() {
            return Err(MiniAppReleaseStoreError::NotFound(
                expected.artifact_digest.0.clone(),
            ));
        }
        self.verify_published(&scope, &root, Some(expected))
    }

    pub fn verify(
        &self,
        scope: MiniAppSourceScope,
        artifact_digest: impl AsRef<str>,
    ) -> Result<(), MiniAppReleaseStoreError> {
        self.load(scope, artifact_digest).map(|_| ())
    }

    pub fn verify_exact(
        &self,
        scope: MiniAppSourceScope,
        expected: &MiniAppReleaseArtifactIdentity,
    ) -> Result<(), MiniAppReleaseStoreError> {
        self.load_exact(scope, expected).map(|_| ())
    }

    pub fn cleanup_staging(&self) -> Result<usize, MiniAppReleaseStoreError> {
        let _guard = self.lock_mutation()?;
        let mut removed = 0usize;
        for entry in fs::read_dir(&self.staging_root)
            .map_err(|error| io_error(&self.staging_root, error))?
        {
            let entry = entry.map_err(|error| io_error(&self.staging_root, error))?;
            let path = entry.path();
            if !is_owned_staging_directory(&self.staging_root, &path) {
                continue;
            }
            fs::remove_dir_all(&path)
                .map_err(|error| MiniAppReleaseStoreError::StagingCleanup(error.to_string()))?;
            removed += 1;
        }
        sync_directory_if_supported(&self.staging_root)?;
        Ok(removed)
    }

    pub fn cleanup_failed_staging(&self) -> Result<usize, MiniAppReleaseStoreError> {
        self.cleanup_staging()
    }

    fn lock_mutation(&self) -> Result<std::sync::MutexGuard<'_, ()>, MiniAppReleaseStoreError> {
        self.mutation_lock
            .lock()
            .map_err(|_| MiniAppReleaseStoreError::StagingCleanup("store mutex poisoned".into()))
    }

    fn allocate_staging(&self) -> Result<StagingGuard, MiniAppReleaseStoreError> {
        for _ in 0..8 {
            let path = self
                .staging_root
                .join(format!("{STAGING_PREFIX}{}", Uuid::now_v7()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(StagingGuard {
                        staging_parent: self.staging_root.clone(),
                        path: Some(path),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error(&path, error)),
            }
        }
        Err(MiniAppReleaseStoreError::Io {
            path: self.staging_root.clone(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "could not allocate a unique MiniApp Release staging directory",
            ),
        })
    }

    fn ensure_artifact_parent(
        &self,
        scope: &MiniAppSourceScope,
    ) -> Result<PathBuf, MiniAppReleaseStoreError> {
        let owner_root = self.releases_root.join(&scope.owner_id);
        ensure_direct_child_directory(&self.releases_root, &owner_root)?;
        let miniapps_root = owner_root.join(MINIAPPS_DIRECTORY);
        ensure_direct_child_directory(&owner_root, &miniapps_root)?;
        let miniapp_root = miniapps_root.join(scope.miniapp_id.as_ref());
        ensure_direct_child_directory(&miniapps_root, &miniapp_root)?;
        let projects_root = miniapp_root.join(PROJECTS_DIRECTORY);
        ensure_direct_child_directory(&miniapp_root, &projects_root)?;
        let project_root = projects_root.join(scope.project_id.as_ref());
        ensure_direct_child_directory(&projects_root, &project_root)?;
        let artifacts_root = project_root.join(ARTIFACTS_DIRECTORY);
        ensure_direct_child_directory(&project_root, &artifacts_root)?;
        Ok(artifacts_root)
    }

    fn artifact_parent(
        &self,
        scope: &MiniAppSourceScope,
    ) -> Result<PathBuf, MiniAppReleaseStoreError> {
        Ok(self
            .releases_root
            .join(&scope.owner_id)
            .join(MINIAPPS_DIRECTORY)
            .join(scope.miniapp_id.as_ref())
            .join(PROJECTS_DIRECTORY)
            .join(scope.project_id.as_ref())
            .join(ARTIFACTS_DIRECTORY))
    }

    fn verify_published(
        &self,
        expected_scope: &MiniAppSourceScope,
        artifact_root: &Path,
        expected_identity: Option<&MiniAppReleaseArtifactIdentity>,
    ) -> Result<MiniAppStoredRelease, MiniAppReleaseStoreError> {
        let expected_digest = artifact_root
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                MiniAppReleaseStoreError::PublishedReleaseMismatch(
                    artifact_root.display().to_string(),
                )
            })?
            .to_owned();
        let mismatch = || {
            MiniAppReleaseStoreError::PublishedReleaseMismatch(expected_digest.clone())
        };
        let metadata = fs::symlink_metadata(artifact_root).map_err(|_| mismatch())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(mismatch());
        }
        let expected_parent = self.artifact_parent(expected_scope)?;
        if artifact_root.parent() != Some(expected_parent.as_path()) {
            return Err(mismatch());
        }
        verify_release_inventory(artifact_root).map_err(|_| mismatch())?;

        let record: ArtifactRecord = parse_canonical(
            &read_regular_bounded(
                &artifact_root.join(ARTIFACT_RECORD_FILE),
                self.limits.max_metadata_bytes,
            )?,
            self.limits.max_metadata_bytes,
        )
        .map_err(|_| mismatch())?;
        if record.scope != *expected_scope
            || record.format_version != MINIAPP_RELEASE_STORE_FORMAT_VERSION
        {
            return Err(mismatch());
        }
        let artifact = &record.artifact.payload;
        let digest = validate_digest(artifact.artifact_digest.as_ref())?;
        if digest.as_ref() != expected_digest {
            return Err(mismatch());
        }
        if expected_identity.is_some_and(|expected| !expected.matches(artifact)) {
            return Err(mismatch());
        }
        validate_record(&record, self.limits)?;

        let manifest_bytes = read_regular_bounded(
            &artifact_root.join(MANIFEST_FILE),
            self.limits.max_metadata_bytes,
        )
        .map_err(|_| mismatch())?;
        let manifest: MiniAppReleaseManifestArtifact =
            parse_canonical(&manifest_bytes, self.limits.max_metadata_bytes)
                .map_err(|_| mismatch())?;
        if manifest != artifact.manifest {
            return Err(mismatch());
        }

        let files_root = artifact_root.join(FILES_DIRECTORY);
        let observed = scan_published_files(&files_root, self.limits).map_err(|_| mismatch())?;
        if observed != artifact.files {
            return Err(mismatch());
        }
        let files = read_published_file_bytes(&files_root, &artifact.files, self.limits)
            .map_err(|_| mismatch())?;
        Ok(MiniAppStoredRelease {
            scope: record.scope,
            artifact: artifact.clone(),
            manifest_bytes,
            files,
            managed_relative_path: format!(
                "{RELEASES_DIRECTORY}/{}/{MINIAPPS_DIRECTORY}/{}/{PROJECTS_DIRECTORY}/{}/{ARTIFACTS_DIRECTORY}/{}",
                expected_scope.owner_id,
                expected_scope.miniapp_id.as_ref(),
                expected_scope.project_id.as_ref(),
                expected_digest
            ),
            artifact_root: artifact_root.to_path_buf(),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct PreparedRelease {
    scope: MiniAppSourceScope,
    source_snapshot_digest: DigestHex,
    dependency_lock_digest: DigestHex,
    build_profile: JavaScriptBuildProfile,
    build_profile_version: VersionString,
    build_generation: u64,
    artifact: MiniAppReleaseArtifactV1,
    files: Vec<MiniAppReleaseFileBytes>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactRecord {
    format_version: String,
    scope: MiniAppSourceScope,
    artifact: ArtifactEnvelope<MiniAppReleaseArtifactV1>,
}

fn prepare_request(
    request: MiniAppReleasePublishRequest,
    limits: MiniAppReleaseStoreLimits,
) -> Result<PreparedRelease, MiniAppReleaseStoreError> {
    validate_scope(&request.scope)?;
    validate_digest(request.source_snapshot_digest.as_ref())?;
    validate_digest(request.dependency_lock_digest.as_ref())?;
    if request.build_generation == 0 {
        return Err(MiniAppReleaseStoreError::InvalidInput(
            "managed Release requires a positive build generation".into(),
        ));
    }
    if request.build_profile != JavaScriptBuildProfile::MiniAppReleaseV1
        || request.build_profile_version.as_ref() != MINIAPP_RELEASE_PROFILE_VERSION
    {
        return Err(MiniAppReleaseStoreError::InvalidInput(
            "MiniApp Release uses the fixed MiniAppReleaseV1 profile version".into(),
        ));
    }
    request
        .artifact
        .validate()
        .map_err(|error| MiniAppReleaseStoreError::InvalidInput(error.to_string()))?;
    match (
        request.content_kind,
        request.artifact.manifest.payload.service.is_some(),
    ) {
        (MiniAppReleaseContentKind::UiOnly, false)
        | (MiniAppReleaseContentKind::Service, true) => {}
        (MiniAppReleaseContentKind::UiOnly, true) => {
            return Err(MiniAppReleaseStoreError::InvalidInput(
                "UI-only Release publish cannot accept a Service artifact".into(),
            ));
        }
        (MiniAppReleaseContentKind::Service, false) => {
            return Err(MiniAppReleaseStoreError::InvalidInput(
                "Service Release publish requires a Service artifact".into(),
            ));
        }
    }
    if request.artifact.manifest.payload.build_profile_version
        != MINIAPP_RELEASE_PROFILE_VERSION.into()
    {
        return Err(MiniAppReleaseStoreError::InvalidInput(
            "artifact and publish request build profiles differ".into(),
        ));
    }
    if request.artifact.manifest.payload.dependency_lock_digest
        != request.dependency_lock_digest
    {
        return Err(MiniAppReleaseStoreError::InvalidInput(
            "artifact and publish request dependency locks differ".into(),
        ));
    }

    let mut files = request.files;
    files.sort_by(|left, right| {
        left.normalized_relative_path
            .cmp(&right.normalized_relative_path)
    });
    if files.len() > limits.max_file_count {
        return Err(MiniAppReleaseStoreError::TooManyFiles {
            observed: files.len(),
            limit: limits.max_file_count,
        });
    }
    let mut collision_keys = BTreeSet::new();
    let mut by_path = BTreeMap::new();
    let mut total_size = 0u64;
    for file in files {
        validate_release_path(&file.normalized_relative_path, request.content_kind)?;
        if !collision_keys.insert(windows_collision_key(&file.normalized_relative_path)?) {
            return Err(MiniAppReleaseStoreError::InvalidPath {
                path: file.normalized_relative_path,
                reason: "duplicate or Windows case-colliding path".into(),
            });
        }
        if file.bytes.is_empty() {
            return Err(MiniAppReleaseStoreError::EmptyFile {
                path: file.normalized_relative_path,
            });
        }
        let size = u64::try_from(file.bytes.len()).map_err(|_| {
            MiniAppReleaseStoreError::FileTooLarge {
                path: file.normalized_relative_path.clone(),
                observed: u64::MAX,
                limit: limits.max_single_file_bytes,
            }
        })?;
        if size > limits.max_single_file_bytes {
            return Err(MiniAppReleaseStoreError::FileTooLarge {
                path: file.normalized_relative_path,
                observed: size,
                limit: limits.max_single_file_bytes,
            });
        }
        total_size = total_size.saturating_add(size);
        if total_size > limits.max_total_bytes {
            return Err(MiniAppReleaseStoreError::TotalSizeExceeded {
                observed: total_size,
                limit: limits.max_total_bytes,
            });
        }
        let path = file.normalized_relative_path.clone();
        if by_path.insert(path.clone(), file).is_some() {
            return Err(MiniAppReleaseStoreError::FileMismatch { path });
        }
    }
    let files = by_path.into_values().collect::<Vec<_>>();
    if files.len() != request.artifact.files.len() {
        return Err(MiniAppReleaseStoreError::FileMismatch {
            path: "<inventory>".into(),
        });
    }
    for expected in &request.artifact.files {
        let Some(actual) = files
            .iter()
            .find(|file| file.normalized_relative_path == expected.normalized_relative_path)
        else {
            return Err(MiniAppReleaseStoreError::FileMismatch {
                path: expected.normalized_relative_path.clone(),
            });
        };
        if digest_bytes(&actual.bytes) != expected.digest
            || actual.bytes.len() as u64 != expected.size_bytes
        {
            return Err(MiniAppReleaseStoreError::FileMismatch {
                path: expected.normalized_relative_path.clone(),
            });
        }
    }
    Ok(PreparedRelease {
        scope: request.scope,
        source_snapshot_digest: request.source_snapshot_digest,
        dependency_lock_digest: request.dependency_lock_digest,
        build_profile: request.build_profile,
        build_profile_version: request.build_profile_version,
        build_generation: request.build_generation,
        artifact: request.artifact,
        files,
    })
}

fn validate_record(
    record: &ArtifactRecord,
    limits: MiniAppReleaseStoreLimits,
) -> Result<(), MiniAppReleaseStoreError> {
    validate_scope(&record.scope)?;
    if !record
        .artifact
        .verify()
        .map_err(|error| MiniAppReleaseStoreError::Corrupt(error.to_string()))?
    {
        return Err(MiniAppReleaseStoreError::Corrupt(
            "artifact identity envelope digest does not match its typed payload".into(),
        ));
    }
    let artifact = &record.artifact.payload;
    if artifact.manifest.payload.build_profile_version
        != MINIAPP_RELEASE_PROFILE_VERSION.into()
    {
        return Err(MiniAppReleaseStoreError::Corrupt(
            "artifact MiniApp Release profile is invalid".into(),
        ));
    }
    artifact.validate().map_err(|error| {
        MiniAppReleaseStoreError::Corrupt(format!("artifact contract: {error}"))
    })?;
    if artifact.files.len() > limits.max_file_count {
        return Err(MiniAppReleaseStoreError::TooManyFiles {
            observed: artifact.files.len(),
            limit: limits.max_file_count,
        });
    }
    Ok(())
}

fn write_release_tree(
    root: &Path,
    release: &PreparedRelease,
    _limits: MiniAppReleaseStoreLimits,
) -> Result<(), MiniAppReleaseStoreError> {
    fs::create_dir_all(root).map_err(|error| io_error(root, error))?;
    let files_root = root.join(FILES_DIRECTORY);
    fs::create_dir(&files_root).map_err(|error| io_error(&files_root, error))?;
    let record = ArtifactRecord {
        format_version: MINIAPP_RELEASE_STORE_FORMAT_VERSION.into(),
        scope: release.scope.clone(),
        artifact: ArtifactEnvelope::new(release.artifact.clone())
            .map_err(|error| MiniAppReleaseStoreError::Canonical(error.to_string()))?,
    };
    write_new_synced(
        &root.join(ARTIFACT_RECORD_FILE),
        &canonical_bytes(&record)?,
    )?;
    write_new_synced(
        &root.join(MANIFEST_FILE),
        &canonical_bytes(&release.artifact.manifest)?,
    )?;
    let mut created = BTreeSet::new();
    for file in &release.files {
        let target = join_relative(&files_root, &file.normalized_relative_path)?;
        let parent = target
            .parent()
            .ok_or_else(|| MiniAppReleaseStoreError::Corrupt("release file has no parent".into()))?;
        ensure_relative_parent_directories(&files_root, parent, &mut created)?;
        write_new_synced(&target, &file.bytes)?;
    }
    Ok(())
}

fn compare_stored(
    stored: &MiniAppStoredRelease,
    incoming: &PreparedRelease,
) -> Result<(), MiniAppReleaseStoreError> {
    if stored.scope != incoming.scope
        || stored.artifact.artifact_digest != incoming.artifact.artifact_digest
        || stored.artifact.manifest != incoming.artifact.manifest
        || stored.artifact.files != incoming.artifact.files
        || stored.files != incoming.files
    {
        return Err(MiniAppReleaseStoreError::PublishedReleaseMismatch(
            incoming.artifact.artifact_digest.0.clone(),
        ));
    }
    Ok(())
}

fn scan_published_files(
    files_root: &Path,
    limits: MiniAppReleaseStoreLimits,
) -> Result<Vec<MiniAppReleaseFile>, MiniAppReleaseStoreError> {
    let metadata = fs::symlink_metadata(files_root).map_err(|error| io_error(files_root, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(MiniAppReleaseStoreError::Corrupt(
            "release files root must be a regular directory".into(),
        ));
    }
    let mut observed = BTreeMap::new();
    let mut total = 0u64;
    collect_release_files(files_root, files_root, &mut observed, &mut total, limits)?;
    Ok(observed.into_values().collect())
}

fn collect_release_files(
    root: &Path,
    current: &Path,
    observed: &mut BTreeMap<String, MiniAppReleaseFile>,
    total: &mut u64,
    limits: MiniAppReleaseStoreLimits,
) -> Result<(), MiniAppReleaseStoreError> {
    for entry in fs::read_dir(current).map_err(|error| io_error(current, error))? {
        let entry = entry.map_err(|error| io_error(current, error))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(MiniAppReleaseStoreError::Corrupt(
                "release symlinks are forbidden".into(),
            ));
        }
        if metadata.is_dir() {
            collect_release_files(root, &path, observed, total, limits)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(MiniAppReleaseStoreError::Corrupt(
                "release inventory contains a special file".into(),
            ));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| MiniAppReleaseStoreError::Corrupt("release path escaped root".into()))?;
        let normalized = normalize_filesystem_path(relative)?;
        validate_stored_release_path(&normalized)?;
        let bytes = read_regular_bounded(&path, limits.max_single_file_bytes)?;
        if bytes.is_empty() {
            return Err(MiniAppReleaseStoreError::EmptyFile { path: normalized });
        }
        *total = total.saturating_add(bytes.len() as u64);
        if *total > limits.max_total_bytes {
            return Err(MiniAppReleaseStoreError::TotalSizeExceeded {
                observed: *total,
                limit: limits.max_total_bytes,
            });
        }
        if observed
            .insert(
                normalized.clone(),
                MiniAppReleaseFile {
                    normalized_relative_path: normalized.clone(),
                    digest: digest_bytes(&bytes),
                    size_bytes: bytes.len() as u64,
                },
            )
            .is_some()
        {
            return Err(MiniAppReleaseStoreError::InvalidPath {
                path: normalized,
                reason: "duplicate release path".into(),
            });
        }
        if observed.len() > limits.max_file_count {
            return Err(MiniAppReleaseStoreError::TooManyFiles {
                observed: observed.len(),
                limit: limits.max_file_count,
            });
        }
    }
    Ok(())
}

fn read_published_file_bytes(
    files_root: &Path,
    expected: &[MiniAppReleaseFile],
    limits: MiniAppReleaseStoreLimits,
) -> Result<Vec<MiniAppReleaseFileBytes>, MiniAppReleaseStoreError> {
    let mut files = Vec::with_capacity(expected.len());
    for file in expected {
        let path = join_relative(files_root, &file.normalized_relative_path)?;
        let bytes = read_regular_bounded(&path, limits.max_single_file_bytes)?;
        if digest_bytes(&bytes) != file.digest || bytes.len() as u64 != file.size_bytes {
            return Err(MiniAppReleaseStoreError::FileMismatch {
                path: file.normalized_relative_path.clone(),
            });
        }
        files.push(MiniAppReleaseFileBytes {
            normalized_relative_path: file.normalized_relative_path.clone(),
            bytes,
        });
    }
    Ok(files)
}

fn validate_scope(scope: &MiniAppSourceScope) -> Result<(), MiniAppReleaseStoreError> {
    MiniAppSourceScope::new(
        &scope.owner_id,
        scope.miniapp_id.as_ref(),
        scope.project_id.as_ref(),
    )
    .map(|_| ())
    .map_err(|error| MiniAppReleaseStoreError::InvalidScope(error.to_string()))
}

fn validate_release_path(
    path: &str,
    content_kind: MiniAppReleaseContentKind,
) -> Result<(), MiniAppReleaseStoreError> {
    validate_relative_path(path)?;
    if content_kind == MiniAppReleaseContentKind::UiOnly
        && (path == "service/main.mjs" || path.starts_with("service/"))
    {
        return Err(MiniAppReleaseStoreError::ServiceFileForbidden {
            path: path.to_owned(),
        });
    }
    if path == "ui/index.html"
        || path.starts_with("ui/")
        || (content_kind == MiniAppReleaseContentKind::Service
            && path == "service/main.mjs")
    {
        Ok(())
    } else {
        Err(MiniAppReleaseStoreError::InvalidPath {
            path: path.to_owned(),
            reason: match content_kind {
                MiniAppReleaseContentKind::UiOnly => {
                    "UI-only Release permits ui/index.html and ui/** only"
                }
                MiniAppReleaseContentKind::Service => {
                    "Service Release permits ui/index.html, ui/**, and service/main.mjs only"
                }
            }
            .into(),
        })
    }
}

fn validate_stored_release_path(path: &str) -> Result<(), MiniAppReleaseStoreError> {
    validate_relative_path(path)?;
    if path == "ui/index.html" || path.starts_with("ui/") || path == "service/main.mjs" {
        Ok(())
    } else {
        Err(MiniAppReleaseStoreError::InvalidPath {
            path: path.to_owned(),
            reason: "MiniApp Release permits ui/index.html, ui/**, and optional service/main.mjs only"
                .into(),
        })
    }
}

fn validate_relative_path(path: &str) -> Result<(), MiniAppReleaseStoreError> {
    if path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || path.trim() != path
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path.contains(':')
    {
        return Err(MiniAppReleaseStoreError::InvalidPath {
            path: path.to_owned(),
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
            return Err(MiniAppReleaseStoreError::InvalidPath {
                path: path.to_owned(),
                reason: "path is not stable under Windows filename/NFC semantics".into(),
            });
        }
    }
    Ok(())
}

fn normalize_filesystem_path(path: &Path) -> Result<String, MiniAppReleaseStoreError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(MiniAppReleaseStoreError::InvalidPath {
            path: path.display().to_string(),
            reason: "path must be relative".into(),
        });
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_str().ok_or_else(|| {
                MiniAppReleaseStoreError::InvalidPath {
                    path: path.display().to_string(),
                    reason: "path must be valid UTF-8".into(),
                }
            })?),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(MiniAppReleaseStoreError::InvalidPath {
                    path: path.display().to_string(),
                    reason: "path contains traversal or root components".into(),
                });
            }
        }
    }
    let normalized = parts.join("/");
    validate_relative_path(&normalized)?;
    Ok(normalized)
}

fn windows_collision_key(path: &str) -> Result<String, MiniAppReleaseStoreError> {
    validate_relative_path(path)?;
    Ok(path
        .split('/')
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("/"))
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

fn validate_digest(value: &str) -> Result<DigestHex, MiniAppReleaseStoreError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(DigestHex::from(value.to_owned()))
    } else {
        Err(MiniAppReleaseStoreError::InvalidInput(
            "digest must be a 64-character lowercase hexadecimal value".into(),
        ))
    }
}

fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, MiniAppReleaseStoreError> {
    canonical_json_bytes(value)
        .map_err(|error| MiniAppReleaseStoreError::Canonical(error.to_string()))
}

fn parse_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
    limit: u64,
) -> Result<T, MiniAppReleaseStoreError> {
    if bytes.len() as u64 > limit {
        return Err(MiniAppReleaseStoreError::InvalidInput(
            "canonical record exceeds its size limit".into(),
        ));
    }
    let value: T = serde_json::from_slice(bytes)
        .map_err(|error| MiniAppReleaseStoreError::InvalidInput(error.to_string()))?;
    if canonical_bytes(&value)? != bytes {
        return Err(MiniAppReleaseStoreError::InvalidInput(
            "record must use canonical JSON".into(),
        ));
    }
    Ok(value)
}

fn join_relative(root: &Path, path: &str) -> Result<PathBuf, MiniAppReleaseStoreError> {
    validate_relative_path(path)?;
    let target = path
        .split('/')
        .fold(root.to_path_buf(), |current, component| current.join(component));
    if !target.starts_with(root) {
        return Err(MiniAppReleaseStoreError::Corrupt(
            "relative path escaped its root".into(),
        ));
    }
    Ok(target)
}

fn ensure_directory_without_symlink(path: &Path) -> Result<(), MiniAppReleaseStoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(MiniAppReleaseStoreError::Corrupt(format!(
                "managed path is not a regular directory: {}",
                path.display()
            )))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|error| io_error(path, error))?;
            Ok(())
        }
        Err(error) => Err(io_error(path, error)),
    }
}

fn ensure_direct_child_directory(
    parent: &Path,
    child: &Path,
) -> Result<(), MiniAppReleaseStoreError> {
    if child.parent() != Some(parent) {
        return Err(MiniAppReleaseStoreError::Corrupt(format!(
            "managed path is not a direct child: {}",
            child.display()
        )));
    }
    ensure_directory_without_symlink(child)?;
    let canonical_parent = fs::canonicalize(parent).map_err(|error| io_error(parent, error))?;
    let canonical_child = fs::canonicalize(child).map_err(|error| io_error(child, error))?;
    if canonical_child.parent() != Some(canonical_parent.as_path()) {
        return Err(MiniAppReleaseStoreError::Corrupt(format!(
            "managed path escaped its parent: {}",
            child.display()
        )));
    }
    Ok(())
}

fn ensure_relative_parent_directories(
    root: &Path,
    parent: &Path,
    created: &mut BTreeSet<String>,
) -> Result<(), MiniAppReleaseStoreError> {
    if !parent.starts_with(root) {
        return Err(MiniAppReleaseStoreError::Corrupt(
            "file parent escaped its root".into(),
        ));
    }
    let relative = parent
        .strip_prefix(root)
        .map_err(|_| MiniAppReleaseStoreError::Corrupt("file parent escaped root".into()))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(value) = component else {
            return Err(MiniAppReleaseStoreError::Corrupt(
                "file parent contains a non-normal component".into(),
            ));
        };
        let value = value.to_str().ok_or_else(|| {
            MiniAppReleaseStoreError::Corrupt("file parent is not UTF-8".into())
        })?;
        current.push(value);
        if created.insert(current.display().to_string()) {
            ensure_directory_without_symlink(&current)?;
        }
    }
    Ok(())
}

fn verify_release_inventory(root: &Path) -> Result<(), MiniAppReleaseStoreError> {
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(root).map_err(|error| io_error(root, error))? {
        let entry = entry.map_err(|error| io_error(root, error))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|error| io_error(entry.path(), error))?;
        let valid = matches!(
            name.as_str(),
            ARTIFACT_RECORD_FILE | MANIFEST_FILE | FILES_DIRECTORY
        ) && !metadata.file_type().is_symlink()
            && ((name == FILES_DIRECTORY && metadata.is_dir())
                || (name != FILES_DIRECTORY && metadata.is_file()));
        if !valid {
            return Err(MiniAppReleaseStoreError::Corrupt(format!(
                "unexpected release inventory entry: {}",
                entry.path().display()
            )));
        }
        names.insert(name);
    }
    if names != BTreeSet::from([
        ARTIFACT_RECORD_FILE.to_owned(),
        MANIFEST_FILE.to_owned(),
        FILES_DIRECTORY.to_owned(),
    ]) {
        return Err(MiniAppReleaseStoreError::Corrupt(
            "release inventory is incomplete".into(),
        ));
    }
    Ok(())
}

fn read_regular_bounded(
    path: &Path,
    limit: u64,
) -> Result<Vec<u8>, MiniAppReleaseStoreError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(MiniAppReleaseStoreError::Corrupt(format!(
            "expected regular file: {}",
            path.display()
        )));
    }
    if metadata.len() > limit {
        return Err(MiniAppReleaseStoreError::FileTooLarge {
            path: path.display().to_string(),
            observed: metadata.len(),
            limit,
        });
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .map_err(|error| io_error(path, error))?
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| io_error(path, error))?;
    Ok(bytes)
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), MiniAppReleaseStoreError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| io_error(path, error))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(path, error))
}

fn sync_tree_directories(root: &Path) -> Result<(), MiniAppReleaseStoreError> {
    let mut directories = Vec::new();
    collect_directories(root, &mut directories)?;
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for path in directories {
        sync_directory_if_supported(&path)?;
    }
    Ok(())
}

fn collect_directories(
    root: &Path,
    directories: &mut Vec<PathBuf>,
) -> Result<(), MiniAppReleaseStoreError> {
    let metadata = fs::symlink_metadata(root).map_err(|error| io_error(root, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(MiniAppReleaseStoreError::Corrupt(format!(
            "expected regular directory: {}",
            root.display()
        )));
    }
    directories.push(root.to_path_buf());
    for entry in fs::read_dir(root).map_err(|error| io_error(root, error))? {
        let entry = entry.map_err(|error| io_error(root, error))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(MiniAppReleaseStoreError::Corrupt(
                "release symlinks are forbidden".into(),
            ));
        }
        if metadata.is_dir() {
            collect_directories(&path, directories)?;
        }
    }
    Ok(())
}

fn remove_owned_release_directory(parent: &Path, target: &Path) {
    if target.parent() != Some(parent)
        || target
            .file_name()
            .and_then(|value| value.to_str())
            .is_none_or(|value| validate_digest(value).is_err())
    {
        return;
    }
    let _ = fs::remove_dir_all(target);
    let _ = sync_directory_if_supported(parent);
}

fn sync_directory_if_supported(path: &Path) -> Result<(), MiniAppReleaseStoreError> {
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

fn io_error(path: impl Into<PathBuf>, source: io::Error) -> MiniAppReleaseStoreError {
    MiniAppReleaseStoreError::Io {
        path: path.into(),
        source,
    }
}

struct StagingGuard {
    staging_parent: PathBuf,
    path: Option<PathBuf>,
}

impl StagingGuard {
    fn path(&self) -> &Path {
        self.path.as_deref().expect("staging guard must be armed")
    }

    fn commit(&mut self) -> Result<(), MiniAppReleaseStoreError> {
        let Some(path) = self.path.take() else {
            return Ok(());
        };
        if fs::symlink_metadata(&path).is_ok() {
            fs::remove_dir_all(&path).map_err(|error| io_error(&path, error))?;
        }
        sync_directory_if_supported(&self.staging_parent)
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        if is_owned_staging_directory(&self.staging_parent, &path) {
            let _ = fs::remove_dir_all(&path);
            let _ = sync_directory_if_supported(&self.staging_parent);
        }
    }
}

fn is_owned_staging_directory(parent: &Path, path: &Path) -> bool {
    if path.parent() != Some(parent) {
        return false;
    }
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let Some(uuid) = name.strip_prefix(STAGING_PREFIX) else {
        return false;
    };
    Uuid::parse_str(uuid).is_ok()
        && fs::symlink_metadata(path)
            .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
            .unwrap_or(false)
}
