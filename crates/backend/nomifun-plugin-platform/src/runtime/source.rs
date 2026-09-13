use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use nomifun_agent_contracts::{
    canonical_json_bytes, digest_bytes, DigestHex, JavaScriptBuildProfile, MiniAppId,
    MiniAppProjectId, VersionString, MINIAPP_RELEASE_PROFILE_VERSION,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const MINIAPP_SOURCE_STORE_FORMAT_VERSION: &str = "1.0.0";
pub const MINIAPP_SOURCE_BUILD_PROFILE_VERSION: &str = MINIAPP_RELEASE_PROFILE_VERSION;

const SOURCES_DIRECTORY: &str = "sources";
const MINIAPPS_DIRECTORY: &str = "miniapps";
const PROJECTS_DIRECTORY: &str = "projects";
const SOURCE_DIRECTORY: &str = "source";
const REVISIONS_DIRECTORY: &str = "revisions";
const STAGING_DIRECTORY: &str = ".staging";
const PROJECT_RECORD_FILE: &str = "project.json";
const DEPENDENCY_LOCK_FILE: &str = "dependency-lock.json";
const HEAD_FILE: &str = "head.json";
const SNAPSHOT_FILE: &str = "snapshot.json";
const STAGING_PREFIX_CREATE: &str = "create-";
const STAGING_PREFIX_REPLACE: &str = "replace-";
const MAX_NORMALIZED_PATH_BYTES: usize = 1024;
const MAX_PATH_COMPONENT_BYTES: usize = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginRuntimeSourceContentKind {
    UiOnly,
    Service,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluginRuntimeSourceStoreLimits {
    pub max_file_count: usize,
    pub max_single_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_metadata_bytes: u64,
    pub max_dependency_lock_bytes: u64,
}

impl Default for PluginRuntimeSourceStoreLimits {
    fn default() -> Self {
        Self {
            max_file_count: 4_096,
            max_single_file_bytes: 64 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
            max_metadata_bytes: 1024 * 1024,
            max_dependency_lock_bytes: 8 * 1024 * 1024,
        }
    }
}

impl PluginRuntimeSourceStoreLimits {
    fn validate(self) -> Result<Self, PluginRuntimeSourceStoreError> {
        if self.max_file_count == 0
            || self.max_single_file_bytes == 0
            || self.max_total_bytes < self.max_single_file_bytes
            || self.max_metadata_bytes == 0
            || self.max_dependency_lock_bytes == 0
        {
            return Err(PluginRuntimeSourceStoreError::InvalidLimits);
        }
        Ok(self)
    }
}

#[derive(Debug, Error)]
pub enum PluginRuntimeSourceStoreError {
    #[error("Plugin Source Store limits are invalid")]
    InvalidLimits,
    #[error("Plugin Source Store scope field {field} is invalid: {reason}")]
    InvalidScope {
        field: &'static str,
        reason: String,
    },
    #[error("Plugin Source Store display name is invalid: {0}")]
    InvalidDisplayName(String),
    #[error("Plugin Source path is invalid: {path} ({reason})")]
    InvalidPath { path: String, reason: String },
    #[error("Plugin Source path collides under Windows semantics: {path}")]
    PathCollision { path: String },
    #[error("Plugin Source file is empty: {path}")]
    EmptyFile { path: String },
    #[error("Plugin Source file is too large: {path} ({observed} > {limit})")]
    FileTooLarge {
        path: String,
        observed: u64,
        limit: u64,
    },
    #[error("Plugin Source contains too many files ({observed} > {limit})")]
    TooManyFiles { observed: usize, limit: usize },
    #[error("Plugin Source exceeds the total byte limit ({observed} > {limit})")]
    TotalSizeExceeded { observed: u64, limit: u64 },
    #[error("Plugin Source cannot contain Service files: {path}")]
    ServiceSourceForbidden { path: String },
    #[error("Plugin Source project already exists")]
    ProjectAlreadyExists,
    #[error("Plugin Source project was not found")]
    ProjectNotFound,
    #[error("Plugin Source project ownership or identity does not match")]
    ScopeMismatch,
    #[error("Plugin Source compare-and-swap conflict: expected {expected}, observed {observed}")]
    CompareAndSwapConflict { expected: String, observed: String },
    #[error("Plugin Source snapshot digest mismatch: expected {expected}, observed {observed}")]
    DigestMismatch { expected: String, observed: String },
    #[error("Plugin Source record is invalid: {0}")]
    InvalidRecord(String),
    #[error("Plugin Source tree is corrupt: {0}")]
    CorruptSource(String),
    #[error("Plugin Source staging cleanup failed: {0}")]
    StagingCleanup(String),
    #[error("Plugin Source filesystem operation failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Plugin Source canonical serialization failed: {0}")]
    Canonical(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeSourceScope {
    pub owner_id: String,
    pub miniapp_id: MiniAppId,
    pub project_id: MiniAppProjectId,
}

impl PluginRuntimeSourceScope {
    pub fn new(
        owner_id: impl AsRef<str>,
        miniapp_id: impl AsRef<str>,
        project_id: impl AsRef<str>,
    ) -> Result<Self, PluginRuntimeSourceStoreError> {
        let owner_id = validate_scope_segment("owner_id", owner_id.as_ref())?;
        let miniapp_id = validate_scope_segment("miniapp_id", miniapp_id.as_ref())?;
        let project_id = validate_scope_segment("project_id", project_id.as_ref())?;
        Ok(Self {
            owner_id,
            miniapp_id: MiniAppId::from(miniapp_id),
            project_id: MiniAppProjectId::from(project_id),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeDependencyLockV1 {
    pub format_version: VersionString,
    pub dependencies: BTreeMap<String, String>,
}

impl PluginRuntimeDependencyLockV1 {
    pub fn empty() -> Self {
        Self {
            format_version: VersionString::from(MINIAPP_SOURCE_STORE_FORMAT_VERSION),
            dependencies: BTreeMap::new(),
        }
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PluginRuntimeSourceStoreError> {
        self.validate()?;
        canonical_json_bytes(self).map_err(|error| PluginRuntimeSourceStoreError::Canonical(error.to_string()))
    }

    pub fn digest(&self) -> Result<DigestHex, PluginRuntimeSourceStoreError> {
        Ok(digest_bytes(&self.canonical_bytes()?))
    }

    fn validate(&self) -> Result<(), PluginRuntimeSourceStoreError> {
        if self.format_version.as_ref() != MINIAPP_SOURCE_STORE_FORMAT_VERSION {
            return Err(PluginRuntimeSourceStoreError::InvalidRecord(
                "dependency lock format version is unsupported".into(),
            ));
        }
        for (name, version) in &self.dependencies {
            if !is_safe_machine_key(name) || !is_safe_machine_key(version) {
                return Err(PluginRuntimeSourceStoreError::InvalidRecord(
                    "dependency lock keys and versions must be stable machine keys".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeSourceFileDigest {
    pub normalized_relative_path: String,
    pub digest: DigestHex,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeSourceFile {
    pub normalized_relative_path: String,
    pub digest: DigestHex,
    pub size_bytes: u64,
    pub bytes: Vec<u8>,
}

impl PluginRuntimeSourceFile {
    pub fn new(path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        let bytes = bytes.into();
        let size_bytes = bytes.len() as u64;
        Self {
            normalized_relative_path: path.into(),
            digest: digest_bytes(&bytes),
            size_bytes,
            bytes,
        }
    }

    fn digest_record(&self) -> PluginRuntimeSourceFileDigest {
        PluginRuntimeSourceFileDigest {
            normalized_relative_path: self.normalized_relative_path.clone(),
            digest: self.digest.clone(),
            size_bytes: self.size_bytes,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeSourceFileInput {
    pub normalized_relative_path: String,
    pub bytes: Vec<u8>,
}

impl PluginRuntimeSourceFileInput {
    pub fn new(path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            normalized_relative_path: path.into(),
            bytes: bytes.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeSourceExactImportRequest {
    pub scope: PluginRuntimeSourceScope,
    pub display_name: String,
    pub content_kind: PluginRuntimeSourceContentKind,
    pub dependency_lock: Vec<u8>,
    pub files: Vec<PluginRuntimeSourceFileInput>,
    pub expected_source_snapshot_digest: DigestHex,
    pub expected_dependency_lock_digest: DigestHex,
    pub build_generation: u64,
    pub build_profile_version: VersionString,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeSourceProject {
    pub owner_id: String,
    pub miniapp_id: MiniAppId,
    pub project_id: MiniAppProjectId,
    pub display_name: String,
    pub managed_relative_path: String,
    pub source_snapshot_digest: DigestHex,
    pub dependency_lock_digest: DigestHex,
    pub build_profile: JavaScriptBuildProfile,
    pub build_profile_version: VersionString,
    pub source_revision: u64,
    pub build_generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeSourceSnapshot {
    pub project: PluginRuntimeSourceProject,
    pub source_snapshot_digest: DigestHex,
    pub dependency_lock_digest: DigestHex,
    pub dependency_lock: Vec<u8>,
    pub files: Vec<PluginRuntimeSourceFile>,
}

#[derive(Clone, Debug)]
pub struct PluginRuntimePreparedSourceMutation {
    scope: PluginRuntimeSourceScope,
    content_kind: PluginRuntimeSourceContentKind,
    files: Vec<PluginRuntimeSourceFileInput>,
    pub expected_source_snapshot_digest: DigestHex,
    pub next_source_snapshot_digest: DigestHex,
    pub expected_build_generation: u64,
    pub next_build_generation: u64,
}

impl PluginRuntimePreparedSourceMutation {
    pub fn is_noop(&self) -> bool {
        self.expected_source_snapshot_digest == self.next_source_snapshot_digest
    }
}

impl PluginRuntimeSourceSnapshot {
    pub fn digest(&self) -> &DigestHex {
        &self.source_snapshot_digest
    }

    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files
            .iter()
            .find(|file| file.normalized_relative_path == path)
            .map(|file| file.bytes.as_slice())
    }

    pub fn content_kind(&self) -> PluginRuntimeSourceContentKind {
        source_content_kind(self.files.iter().map(|file| {
            file.normalized_relative_path.as_str()
        }))
    }

    pub fn service_main_mjs(&self) -> Option<&[u8]> {
        self.file("service/main.mjs")
    }
}

#[derive(Clone, Debug)]
pub struct PluginRuntimeSourceStore {
    managed_root: PathBuf,
    sources_root: PathBuf,
    staging_root: PathBuf,
    limits: PluginRuntimeSourceStoreLimits,
    mutation_lock: Arc<Mutex<()>>,
}

impl PluginRuntimeSourceStore {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, PluginRuntimeSourceStoreError> {
        Self::new_with_limits(root, PluginRuntimeSourceStoreLimits::default())
    }

    pub fn new_with_limits(
        root: impl AsRef<Path>,
        limits: PluginRuntimeSourceStoreLimits,
    ) -> Result<Self, PluginRuntimeSourceStoreError> {
        let limits = limits.validate()?;
        let requested_root = root.as_ref();
        ensure_directory_without_symlink(requested_root)?;
        let managed_root = fs::canonicalize(requested_root)
            .map_err(|error| io_error(requested_root, error))?;
        let sources_root = managed_root.join(SOURCES_DIRECTORY);
        let staging_root = managed_root.join(STAGING_DIRECTORY);
        ensure_direct_child_directory(&managed_root, &sources_root)?;
        ensure_direct_child_directory(&managed_root, &staging_root)?;
        Ok(Self {
            managed_root,
            sources_root,
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

    pub fn create_project(
        &self,
        owner: impl AsRef<str>,
        miniapp_id: impl AsRef<str>,
        project_id: impl AsRef<str>,
        display_name: impl Into<String>,
    ) -> Result<PluginRuntimeSourceProject, PluginRuntimeSourceStoreError> {
        self.create_project_with_service(
            owner,
            miniapp_id,
            project_id,
            display_name,
            None,
        )
    }

    pub fn create_service_project(
        &self,
        owner: impl AsRef<str>,
        miniapp_id: impl AsRef<str>,
        project_id: impl AsRef<str>,
        display_name: impl Into<String>,
        service_main_mjs: impl Into<Vec<u8>>,
    ) -> Result<PluginRuntimeSourceProject, PluginRuntimeSourceStoreError> {
        self.create_project_with_service(
            owner,
            miniapp_id,
            project_id,
            display_name,
            Some(service_main_mjs.into()),
        )
    }

    pub fn import_project_exact(
        &self,
        request: PluginRuntimeSourceExactImportRequest,
    ) -> Result<PluginRuntimeSourceProject, PluginRuntimeSourceStoreError> {
        let scope = PluginRuntimeSourceScope::new(
            &request.scope.owner_id,
            request.scope.miniapp_id.as_ref(),
            request.scope.project_id.as_ref(),
        )?;
        let display_name = validate_display_name(request.display_name)?;
        let expected_source_snapshot_digest =
            validate_digest_value(request.expected_source_snapshot_digest.as_ref())?;
        let expected_dependency_lock_digest =
            validate_digest_value(request.expected_dependency_lock_digest.as_ref())?;
        if request.build_generation == 0
            || request.build_profile_version.as_ref()
                != MINIAPP_SOURCE_BUILD_PROFILE_VERSION
        {
            return Err(PluginRuntimeSourceStoreError::InvalidRecord(
                "exact Source import requires a positive build generation and the current build profile version"
                    .into(),
            ));
        }

        let dependency_lock = parse_canonical::<PluginRuntimeDependencyLockV1>(
            &request.dependency_lock,
            self.limits.max_dependency_lock_bytes,
        )?;
        let observed_dependency_lock_digest = dependency_lock.digest()?;
        if observed_dependency_lock_digest != expected_dependency_lock_digest {
            return Err(PluginRuntimeSourceStoreError::DigestMismatch {
                expected: expected_dependency_lock_digest.0,
                observed: observed_dependency_lock_digest.0,
            });
        }

        let prepared =
            prepare_source_files(request.files, request.content_kind, self.limits)?;
        let observed_content_kind = source_content_kind(
            prepared
                .iter()
                .map(|file| file.normalized_relative_path.as_str()),
        );
        if observed_content_kind != request.content_kind {
            return Err(PluginRuntimeSourceStoreError::InvalidRecord(
                "exact Source import content kind does not match its file inventory".into(),
            ));
        }
        let snapshot = snapshot_record(&prepared)?;
        if snapshot.snapshot_digest != expected_source_snapshot_digest {
            return Err(PluginRuntimeSourceStoreError::DigestMismatch {
                expected: expected_source_snapshot_digest.0,
                observed: snapshot.snapshot_digest.0,
            });
        }

        let project_record = ProjectRecord {
            format_version: MINIAPP_SOURCE_STORE_FORMAT_VERSION.into(),
            owner_id: scope.owner_id.clone(),
            miniapp_id: scope.miniapp_id.as_ref().into(),
            project_id: scope.project_id.as_ref().into(),
            display_name,
        };
        let head = HeadRecord {
            format_version: MINIAPP_SOURCE_STORE_FORMAT_VERSION.into(),
            snapshot_digest: snapshot.snapshot_digest.clone(),
            dependency_lock_digest: observed_dependency_lock_digest,
            build_profile: JavaScriptBuildProfile::MiniAppReleaseV1,
            build_profile_version: request.build_profile_version.as_ref().into(),
            source_revision: 1,
            build_generation: request.build_generation,
        };

        let _guard = self.lock_mutation()?;
        self.install_new_project_unlocked(
            &scope,
            project_record,
            &request.dependency_lock,
            &prepared,
            &snapshot,
            &head,
        )
    }

    fn create_project_with_service(
        &self,
        owner: impl AsRef<str>,
        miniapp_id: impl AsRef<str>,
        project_id: impl AsRef<str>,
        display_name: impl Into<String>,
        service_main_mjs: Option<Vec<u8>>,
    ) -> Result<PluginRuntimeSourceProject, PluginRuntimeSourceStoreError> {
        let scope = PluginRuntimeSourceScope::new(owner, miniapp_id, project_id)?;
        let display_name = validate_display_name(display_name.into())?;

        let project_record = ProjectRecord {
            format_version: MINIAPP_SOURCE_STORE_FORMAT_VERSION.into(),
            owner_id: scope.owner_id.clone(),
            miniapp_id: scope.miniapp_id.as_ref().into(),
            project_id: scope.project_id.as_ref().into(),
            display_name,
        };

        let lock = PluginRuntimeDependencyLockV1::empty();
        let lock_bytes = lock.canonical_bytes()?;

        let mut files = vec![PluginRuntimeSourceFileInput::new(
            "ui/index.html",
            default_index_html(&project_record.display_name),
        )];
        let content_kind = if let Some(service_main_mjs) = service_main_mjs {
            files.push(PluginRuntimeSourceFileInput::new(
                "service/main.mjs",
                service_main_mjs,
            ));
            PluginRuntimeSourceContentKind::Service
        } else {
            PluginRuntimeSourceContentKind::UiOnly
        };
        let prepared = prepare_source_files(files, content_kind, self.limits)?;
        let snapshot = snapshot_record(&prepared)?;
        let head = HeadRecord {
            format_version: MINIAPP_SOURCE_STORE_FORMAT_VERSION.into(),
            snapshot_digest: snapshot.snapshot_digest.clone(),
            dependency_lock_digest: digest_bytes(&lock_bytes),
            build_profile: JavaScriptBuildProfile::MiniAppReleaseV1,
            build_profile_version: MINIAPP_SOURCE_BUILD_PROFILE_VERSION.into(),
            source_revision: 1,
            build_generation: 1,
        };

        let _guard = self.lock_mutation()?;
        self.install_new_project_unlocked(
            &scope,
            project_record,
            &lock_bytes,
            &prepared,
            &snapshot,
            &head,
        )
    }

    pub fn read_snapshot(
        &self,
        owner: impl AsRef<str>,
        miniapp_id: impl AsRef<str>,
        project_id: impl AsRef<str>,
        expected_digest: impl AsRef<str>,
    ) -> Result<PluginRuntimeSourceSnapshot, PluginRuntimeSourceStoreError> {
        let scope = PluginRuntimeSourceScope::new(owner, miniapp_id, project_id)?;
        let expected_digest = validate_digest_value(expected_digest.as_ref())?;
        let _guard = self.lock_mutation()?;
        let snapshot = self.read_snapshot_unlocked(&scope)?;
        if snapshot.source_snapshot_digest != expected_digest {
            return Err(PluginRuntimeSourceStoreError::CompareAndSwapConflict {
                expected: expected_digest.0,
                observed: snapshot.source_snapshot_digest.0,
            });
        }
        Ok(snapshot)
    }

    pub fn current_snapshot(
        &self,
        owner: impl AsRef<str>,
        miniapp_id: impl AsRef<str>,
        project_id: impl AsRef<str>,
    ) -> Result<PluginRuntimeSourceSnapshot, PluginRuntimeSourceStoreError> {
        let scope = PluginRuntimeSourceScope::new(owner, miniapp_id, project_id)?;
        let _guard = self.lock_mutation()?;
        self.read_snapshot_unlocked(&scope)
    }

    pub fn prepare_file_replace(
        &self,
        owner: impl AsRef<str>,
        miniapp_id: impl AsRef<str>,
        project_id: impl AsRef<str>,
        expected_digest: impl AsRef<str>,
        path: impl AsRef<str>,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<PluginRuntimePreparedSourceMutation, PluginRuntimeSourceStoreError> {
        let scope = PluginRuntimeSourceScope::new(owner, miniapp_id, project_id)?;
        let expected_digest = validate_digest_value(expected_digest.as_ref())?;
        let path = path.as_ref();
        let replacement = bytes.into();
        let _guard = self.lock_mutation()?;
        let current = self.read_snapshot_unlocked(&scope)?;
        if current.source_snapshot_digest != expected_digest {
            return Err(PluginRuntimeSourceStoreError::CompareAndSwapConflict {
                expected: expected_digest.0,
                observed: current.source_snapshot_digest.0,
            });
        }
        if current
            .files
            .iter()
            .all(|file| file.normalized_relative_path != path)
            && path != crate::runtime::PLUGIN_RUNTIME_MANIFEST_PATH
        {
            return Err(PluginRuntimeSourceStoreError::InvalidPath {
                path: path.to_owned(),
                reason: "Source edit may replace only an existing managed file".into(),
            });
        }
        let remove_surface = path == "ui/index.html" && replacement.is_empty()
            && current.content_kind() == PluginRuntimeSourceContentKind::Service;
        let mut inputs = current
            .files
            .iter()
            .filter(|file| !(remove_surface && file.normalized_relative_path == path))
            .map(|file| {
                PluginRuntimeSourceFileInput::new(
                    file.normalized_relative_path.clone(),
                    if file.normalized_relative_path == path {
                        replacement.clone()
                    } else {
                        file.bytes.clone()
                    },
                )
            })
            .collect::<Vec<_>>();
        if path == crate::runtime::PLUGIN_RUNTIME_MANIFEST_PATH && current.file(path).is_none() {
            inputs.push(PluginRuntimeSourceFileInput::new(path, replacement));
        }
        let content_kind = current.content_kind();
        let prepared = prepare_source_files(inputs.clone(), content_kind, self.limits)?;
        let next_snapshot = snapshot_record(&prepared)?;
        let next_build_generation = if next_snapshot.snapshot_digest
            == current.source_snapshot_digest
        {
            current.project.build_generation
        } else {
            checked_increment(current.project.build_generation, "build generation")?
        };
        Ok(PluginRuntimePreparedSourceMutation {
            scope,
            content_kind,
            files: inputs,
            expected_source_snapshot_digest: current.source_snapshot_digest,
            next_source_snapshot_digest: next_snapshot.snapshot_digest,
            expected_build_generation: current.project.build_generation,
            next_build_generation,
        })
    }

    pub fn commit_prepared_source(
        &self,
        prepared: &PluginRuntimePreparedSourceMutation,
    ) -> Result<PluginRuntimeSourceProject, PluginRuntimeSourceStoreError> {
        if prepared.is_noop() {
            return Ok(self
                .current_snapshot(
                    &prepared.scope.owner_id,
                    prepared.scope.miniapp_id.as_ref(),
                    prepared.scope.project_id.as_ref(),
                )?
                .project);
        }
        self.replace_source_with_kind(
            &prepared.scope.owner_id,
            prepared.scope.miniapp_id.as_ref(),
            prepared.scope.project_id.as_ref(),
            prepared.expected_source_snapshot_digest.as_ref(),
            prepared.files.clone(),
            prepared.content_kind,
        )
    }

    pub fn read_revision_files(
        &self,
        owner: impl AsRef<str>,
        miniapp_id: impl AsRef<str>,
        project_id: impl AsRef<str>,
        source_snapshot_digest: impl AsRef<str>,
    ) -> Result<Vec<PluginRuntimeSourceFile>, PluginRuntimeSourceStoreError> {
        let scope = PluginRuntimeSourceScope::new(owner, miniapp_id, project_id)?;
        let source_snapshot_digest =
            validate_digest_value(source_snapshot_digest.as_ref())?;
        let _guard = self.lock_mutation()?;
        let (_, _, _, _, source_root) = self.load_project_unlocked(&scope)?;
        let revision_root = source_root
            .join(REVISIONS_DIRECTORY)
            .join(source_snapshot_digest.as_ref());
        let snapshot = self.read_revision_unlocked(&revision_root)?;
        if snapshot.snapshot_digest != source_snapshot_digest {
            return Err(PluginRuntimeSourceStoreError::DigestMismatch {
                expected: source_snapshot_digest.0,
                observed: snapshot.snapshot_digest.0,
            });
        }
        read_revision_files(&revision_root, &snapshot, self.limits)
    }

    pub fn replace_source(
        &self,
        owner: impl AsRef<str>,
        miniapp_id: impl AsRef<str>,
        project_id: impl AsRef<str>,
        expected_digest: impl AsRef<str>,
        files: Vec<PluginRuntimeSourceFileInput>,
    ) -> Result<PluginRuntimeSourceProject, PluginRuntimeSourceStoreError> {
        self.replace_source_with_kind(
            owner,
            miniapp_id,
            project_id,
            expected_digest,
            files,
            PluginRuntimeSourceContentKind::UiOnly,
        )
    }

    pub fn replace_service_source(
        &self,
        owner: impl AsRef<str>,
        miniapp_id: impl AsRef<str>,
        project_id: impl AsRef<str>,
        expected_digest: impl AsRef<str>,
        files: Vec<PluginRuntimeSourceFileInput>,
    ) -> Result<PluginRuntimeSourceProject, PluginRuntimeSourceStoreError> {
        self.replace_source_with_kind(
            owner,
            miniapp_id,
            project_id,
            expected_digest,
            files,
            PluginRuntimeSourceContentKind::Service,
        )
    }

    fn replace_source_with_kind(
        &self,
        owner: impl AsRef<str>,
        miniapp_id: impl AsRef<str>,
        project_id: impl AsRef<str>,
        expected_digest: impl AsRef<str>,
        files: Vec<PluginRuntimeSourceFileInput>,
        content_kind: PluginRuntimeSourceContentKind,
    ) -> Result<PluginRuntimeSourceProject, PluginRuntimeSourceStoreError> {
        let scope = PluginRuntimeSourceScope::new(owner, miniapp_id, project_id)?;
        let expected_digest = validate_digest_value(expected_digest.as_ref())?;
        let prepared = prepare_source_files(files, content_kind, self.limits)?;
        let next_snapshot = snapshot_record(&prepared)?;
        let _guard = self.lock_mutation()?;
        let current = self.read_snapshot_unlocked(&scope)?;
        if current.source_snapshot_digest != expected_digest {
            return Err(PluginRuntimeSourceStoreError::CompareAndSwapConflict {
                expected: expected_digest.0,
                observed: current.source_snapshot_digest.0,
            });
        }
        if next_snapshot.snapshot_digest == current.source_snapshot_digest {
            return Ok(current.project);
        }

        let source_root = self.source_root(&scope);
        let revisions_root = source_root.join(REVISIONS_DIRECTORY);
        let next_source_revision = checked_increment(current.project.source_revision, "source revision")?;
        let next_build_generation =
            checked_increment(current.project.build_generation, "build generation")?;

        let mut staging = self.allocate_staging(STAGING_PREFIX_REPLACE)?;
        let staged_revision = staging.path().join("revision");
        write_revision(&staged_revision, &prepared, &next_snapshot)?;
        let final_revision = revisions_root.join(next_snapshot.snapshot_digest.as_ref());
        if fs::symlink_metadata(&final_revision).is_ok() {
            let existing = self.read_revision_unlocked(&final_revision)?;
            if existing != next_snapshot {
                return Err(PluginRuntimeSourceStoreError::CorruptSource(
                    "an existing source revision has the same digest but different content".into(),
                ));
            }
        } else {
            fs::rename(&staged_revision, &final_revision)
                .map_err(|error| io_error(&final_revision, error))?;
            sync_directory_if_supported(&revisions_root)?;
        }

        let next_head = HeadRecord {
            format_version: MINIAPP_SOURCE_STORE_FORMAT_VERSION.into(),
            snapshot_digest: next_snapshot.snapshot_digest.clone(),
            dependency_lock_digest: current.dependency_lock_digest.clone(),
            build_profile: current.project.build_profile,
            build_profile_version: current.project.build_profile_version.as_ref().into(),
            source_revision: next_source_revision,
            build_generation: next_build_generation,
        };
        let staged_head = staging.path().join("head.json");
        write_new_synced(&staged_head, &canonical_bytes(&next_head)?)?;
        atomic_replace_file(&staged_head, &source_root.join(HEAD_FILE))?;
        staging.commit()?;
        self.read_project_summary_unlocked(&scope)
    }

    pub fn delete_project(
        &self,
        owner: impl AsRef<str>,
        miniapp_id: impl AsRef<str>,
        project_id: impl AsRef<str>,
    ) -> Result<(), PluginRuntimeSourceStoreError> {
        let scope = PluginRuntimeSourceScope::new(owner, miniapp_id, project_id)?;
        let _guard = self.lock_mutation()?;
        let project_root = self.project_root(&scope);
        let project = self.load_project_root_unlocked(&scope)?;
        fs::remove_dir_all(&project_root).map_err(|error| io_error(&project_root, error))?;
        let parent = project
            .parent()
            .ok_or_else(|| PluginRuntimeSourceStoreError::CorruptSource("project has no parent".into()))?;
        sync_directory_if_supported(parent)
    }

    /// Idempotently remove one owner-scoped Project source tree.
    ///
    /// Permanent Delete intentionally starts from the same root on every retry:
    /// an already-absent source tree is success, while a symlink/corrupt parent
    /// remains a hard failure.
    pub fn purge_project(
        &self,
        owner: impl AsRef<str>,
        miniapp_id: impl AsRef<str>,
        project_id: impl AsRef<str>,
    ) -> Result<(), PluginRuntimeSourceStoreError> {
        let scope = PluginRuntimeSourceScope::new(owner, miniapp_id, project_id)?;
        let _guard = self.lock_mutation()?;
        let parent = self.project_parent(&scope);
        let Some(canonical_parent) =
            canonical_existing_directory_chain(&self.sources_root, &parent)?
        else {
            return Ok(());
        };
        let project_root = self.project_root(&scope);
        let metadata = match fs::symlink_metadata(&project_root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(io_error(&project_root, error)),
        };
        if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(
                "project purge target must be a regular directory".into(),
            ));
        }
        let canonical_project =
            fs::canonicalize(&project_root).map_err(|error| io_error(&project_root, error))?;
        if canonical_project.parent() != Some(canonical_parent.as_path()) {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(
                "project purge target escaped its owner boundary".into(),
            ));
        }
        validate_removal_tree(&canonical_project)?;
        fs::remove_dir_all(&canonical_project)
            .map_err(|error| io_error(&canonical_project, error))?;
        sync_directory_if_supported(&canonical_parent)
    }

    pub fn cleanup_staging(&self) -> Result<usize, PluginRuntimeSourceStoreError> {
        let _guard = self.lock_mutation()?;
        let mut removed = 0usize;
        let entries = fs::read_dir(&self.staging_root)
            .map_err(|error| io_error(&self.staging_root, error))?;
        for entry in entries {
            let entry = entry.map_err(|error| io_error(&self.staging_root, error))?;
            let path = entry.path();
            if !is_owned_staging_directory(&self.staging_root, &path) {
                continue;
            }
            fs::remove_dir_all(&path)
                .map_err(|error| PluginRuntimeSourceStoreError::StagingCleanup(error.to_string()))?;
            removed += 1;
        }
        sync_directory_if_supported(&self.staging_root)?;
        Ok(removed)
    }

    pub fn cleanup_failed_staging(&self) -> Result<usize, PluginRuntimeSourceStoreError> {
        self.cleanup_staging()
    }

    fn lock_mutation(&self) -> Result<std::sync::MutexGuard<'_, ()>, PluginRuntimeSourceStoreError> {
        self.mutation_lock
            .lock()
            .map_err(|_| PluginRuntimeSourceStoreError::StagingCleanup("store mutex poisoned".into()))
    }

    fn allocate_staging(&self, prefix: &str) -> Result<StagingGuard, PluginRuntimeSourceStoreError> {
        for _ in 0..8 {
            let path = self
                .staging_root
                .join(format!("{prefix}{}", Uuid::now_v7()));
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
        Err(PluginRuntimeSourceStoreError::Io {
            path: self.staging_root.clone(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "could not allocate a unique Plugin Source staging directory",
            ),
        })
    }

    fn install_new_project_unlocked(
        &self,
        scope: &PluginRuntimeSourceScope,
        project_record: ProjectRecord,
        dependency_lock: &[u8],
        files: &[PluginRuntimeSourceFile],
        snapshot: &SnapshotRecord,
        head: &HeadRecord,
    ) -> Result<PluginRuntimeSourceProject, PluginRuntimeSourceStoreError> {
        let parent = self.ensure_project_parent(scope)?;
        let final_project_root = parent.join(scope.project_id.as_ref());
        match fs::symlink_metadata(&final_project_root) {
            Ok(_) => return Err(PluginRuntimeSourceStoreError::ProjectAlreadyExists),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(&final_project_root, error)),
        }

        let mut staging = self.allocate_staging(STAGING_PREFIX_CREATE)?;
        let staged_project_root = staging.path().join("project");
        let staged_source_root = staged_project_root.join(SOURCE_DIRECTORY);
        fs::create_dir(&staged_project_root)
            .map_err(|error| io_error(&staged_project_root, error))?;
        fs::create_dir(&staged_source_root)
            .map_err(|error| io_error(&staged_source_root, error))?;
        fs::create_dir(staged_source_root.join(REVISIONS_DIRECTORY))
            .map_err(|error| io_error(staged_source_root.join(REVISIONS_DIRECTORY), error))?;

        write_new_synced(
            &staged_project_root.join(PROJECT_RECORD_FILE),
            &canonical_bytes(&project_record)?,
        )?;
        write_new_synced(
            &staged_project_root.join(DEPENDENCY_LOCK_FILE),
            dependency_lock,
        )?;
        let revision_root = staged_source_root
            .join(REVISIONS_DIRECTORY)
            .join(snapshot.snapshot_digest.as_ref());
        write_revision(&revision_root, files, snapshot)?;
        write_new_synced(
            &staged_source_root.join(HEAD_FILE),
            &canonical_bytes(head)?,
        )?;
        sync_tree_directories(&staged_project_root)?;

        match fs::rename(&staged_project_root, &final_project_root) {
            Ok(()) => {}
            Err(error)
                if error.kind() == io::ErrorKind::AlreadyExists
                    || final_project_root.exists() =>
            {
                return Err(PluginRuntimeSourceStoreError::ProjectAlreadyExists);
            }
            Err(error) => return Err(io_error(&final_project_root, error)),
        }
        sync_directory_if_supported(&parent)?;
        staging.commit()?;
        self.read_project_summary_unlocked(scope)
    }

    fn ensure_project_parent(
        &self,
        scope: &PluginRuntimeSourceScope,
    ) -> Result<PathBuf, PluginRuntimeSourceStoreError> {
        let owner_root = self.sources_root.join(&scope.owner_id);
        ensure_direct_child_directory(&self.sources_root, &owner_root)?;
        let miniapps_root = owner_root.join(MINIAPPS_DIRECTORY);
        ensure_direct_child_directory(&owner_root, &miniapps_root)?;
        let miniapp_root = miniapps_root.join(scope.miniapp_id.as_ref());
        ensure_direct_child_directory(&miniapps_root, &miniapp_root)?;
        let projects_root = miniapp_root.join(PROJECTS_DIRECTORY);
        ensure_direct_child_directory(&miniapp_root, &projects_root)?;
        Ok(projects_root)
    }

    fn project_parent(&self, scope: &PluginRuntimeSourceScope) -> PathBuf {
        self.sources_root
            .join(&scope.owner_id)
            .join(MINIAPPS_DIRECTORY)
            .join(scope.miniapp_id.as_ref())
            .join(PROJECTS_DIRECTORY)
    }

    fn project_root(&self, scope: &PluginRuntimeSourceScope) -> PathBuf {
        self.project_parent(scope).join(scope.project_id.as_ref())
    }

    fn source_root(&self, scope: &PluginRuntimeSourceScope) -> PathBuf {
        self.project_root(scope).join(SOURCE_DIRECTORY)
    }

    fn read_project_summary_unlocked(
        &self,
        scope: &PluginRuntimeSourceScope,
    ) -> Result<PluginRuntimeSourceProject, PluginRuntimeSourceStoreError> {
        let (_project_root, record, head, lock_bytes, _) = self.load_project_unlocked(scope)?;
        let lock = parse_canonical::<PluginRuntimeDependencyLockV1>(
            &lock_bytes,
            self.limits.max_dependency_lock_bytes,
        )?;
        let lock_digest = digest_bytes(&lock_bytes);
        if lock_digest != head.dependency_lock_digest {
            return Err(PluginRuntimeSourceStoreError::DigestMismatch {
                expected: head.dependency_lock_digest.0,
                observed: lock_digest.0,
            });
        }
        Ok(PluginRuntimeSourceProject {
            owner_id: record.owner_id,
            miniapp_id: MiniAppId::from(record.miniapp_id),
            project_id: MiniAppProjectId::from(record.project_id),
            display_name: record.display_name,
            managed_relative_path: self
                .managed_relative_source_path(scope),
            source_snapshot_digest: head.snapshot_digest,
            dependency_lock_digest: lock.digest()?,
            build_profile: head.build_profile,
            build_profile_version: VersionString::from(head.build_profile_version),
            source_revision: head.source_revision,
            build_generation: head.build_generation,
        })
    }

    fn read_snapshot_unlocked(
        &self,
        scope: &PluginRuntimeSourceScope,
    ) -> Result<PluginRuntimeSourceSnapshot, PluginRuntimeSourceStoreError> {
        let (_, record, head, lock_bytes, source_root) = self.load_project_unlocked(scope)?;
        let lock = parse_canonical::<PluginRuntimeDependencyLockV1>(
            &lock_bytes,
            self.limits.max_dependency_lock_bytes,
        )?;
        let lock_digest = digest_bytes(&lock_bytes);
        if lock_digest != head.dependency_lock_digest || lock.digest()? != head.dependency_lock_digest {
            return Err(PluginRuntimeSourceStoreError::DigestMismatch {
                expected: head.dependency_lock_digest.0,
                observed: lock_digest.0,
            });
        }

        validate_digest_value(head.snapshot_digest.as_ref())?;
        let revision_root = source_root
            .join(REVISIONS_DIRECTORY)
            .join(head.snapshot_digest.as_ref());
        let snapshot = self.read_revision_unlocked(&revision_root)?;
        if snapshot.snapshot_digest != head.snapshot_digest {
            return Err(PluginRuntimeSourceStoreError::DigestMismatch {
                expected: head.snapshot_digest.0,
                observed: snapshot.snapshot_digest.0,
            });
        }
        let files = read_revision_files(&revision_root, &snapshot, self.limits)?;
        let project = PluginRuntimeSourceProject {
            owner_id: record.owner_id,
            miniapp_id: MiniAppId::from(record.miniapp_id),
            project_id: MiniAppProjectId::from(record.project_id),
            display_name: record.display_name,
            managed_relative_path: self.managed_relative_source_path(scope),
            source_snapshot_digest: head.snapshot_digest.clone(),
            dependency_lock_digest: head.dependency_lock_digest.clone(),
            build_profile: head.build_profile,
            build_profile_version: VersionString::from(head.build_profile_version),
            source_revision: head.source_revision,
            build_generation: head.build_generation,
        };
        Ok(PluginRuntimeSourceSnapshot {
            project,
            source_snapshot_digest: snapshot.snapshot_digest,
            dependency_lock_digest: lock_digest,
            dependency_lock: lock_bytes,
            files,
        })
    }

    fn load_project_unlocked(
        &self,
        scope: &PluginRuntimeSourceScope,
    ) -> Result<(PathBuf, ProjectRecord, HeadRecord, Vec<u8>, PathBuf), PluginRuntimeSourceStoreError>
    {
        let project_root = self.load_project_root_unlocked(scope)?;
        verify_project_inventory(&project_root)?;
        let project_record: ProjectRecord = parse_canonical(
            &read_regular_bounded(
                &project_root.join(PROJECT_RECORD_FILE),
                self.limits.max_metadata_bytes,
            )?,
            self.limits.max_metadata_bytes,
        )?;
        if project_record.format_version != MINIAPP_SOURCE_STORE_FORMAT_VERSION
            || project_record.owner_id != scope.owner_id
            || project_record.miniapp_id != scope.miniapp_id.as_ref()
            || project_record.project_id != scope.project_id.as_ref()
        {
            return Err(PluginRuntimeSourceStoreError::ScopeMismatch);
        }
        let lock_path = project_root.join(DEPENDENCY_LOCK_FILE);
        let lock_bytes = read_regular_bounded(
            &lock_path,
            self.limits.max_dependency_lock_bytes,
        )?;
        let source_root = project_root.join(SOURCE_DIRECTORY);
        verify_source_root_inventory(&source_root)?;
        let head: HeadRecord = parse_canonical(
            &read_regular_bounded(&source_root.join(HEAD_FILE), self.limits.max_metadata_bytes)?,
            self.limits.max_metadata_bytes,
        )?;
        head.validate()?;
        Ok((project_root, project_record, head, lock_bytes, source_root))
    }

    fn load_project_root_unlocked(
        &self,
        scope: &PluginRuntimeSourceScope,
    ) -> Result<PathBuf, PluginRuntimeSourceStoreError> {
        let project_root = self.project_root(scope);
        let metadata = match fs::symlink_metadata(&project_root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(PluginRuntimeSourceStoreError::ProjectNotFound);
            }
            Err(error) => return Err(io_error(&project_root, error)),
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(
                "project root must be a regular directory".into(),
            ));
        }
        let expected_parent = self.project_parent(scope);
        let canonical_parent =
            fs::canonicalize(&expected_parent).map_err(|error| io_error(&expected_parent, error))?;
        let canonical_project =
            fs::canonicalize(&project_root).map_err(|error| io_error(&project_root, error))?;
        if canonical_project.parent() != Some(canonical_parent.as_path()) {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(
                "project root escaped its owner/miniapp/project parent".into(),
            ));
        }
        Ok(canonical_project)
    }

    fn read_revision_unlocked(
        &self,
        revision_root: &Path,
    ) -> Result<SnapshotRecord, PluginRuntimeSourceStoreError> {
        let metadata = fs::symlink_metadata(revision_root)
            .map_err(|error| io_error(revision_root, error))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(
                "source revision must be a regular directory".into(),
            ));
        }
        let record_path = revision_root.join(SNAPSHOT_FILE);
        let record: SnapshotRecord = parse_canonical(
            &read_regular_bounded(&record_path, self.limits.max_metadata_bytes)?,
            self.limits.max_metadata_bytes,
        )?;
        record.validate()?;
        let directory_name = revision_root
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| PluginRuntimeSourceStoreError::CorruptSource("invalid revision name".into()))?;
        if directory_name != record.snapshot_digest.as_ref() {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(
                "revision directory does not match snapshot digest".into(),
            ));
        }
        Ok(record)
    }

    fn managed_relative_source_path(&self, scope: &PluginRuntimeSourceScope) -> String {
        format!(
            "{SOURCES_DIRECTORY}/{}/{MINIAPPS_DIRECTORY}/{}/{PROJECTS_DIRECTORY}/{}/{SOURCE_DIRECTORY}",
            scope.owner_id,
            scope.miniapp_id.as_ref(),
            scope.project_id.as_ref()
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectRecord {
    format_version: String,
    owner_id: String,
    miniapp_id: String,
    project_id: String,
    display_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadRecord {
    format_version: String,
    snapshot_digest: DigestHex,
    dependency_lock_digest: DigestHex,
    build_profile: JavaScriptBuildProfile,
    build_profile_version: String,
    source_revision: u64,
    build_generation: u64,
}

impl HeadRecord {
    fn validate(&self) -> Result<(), PluginRuntimeSourceStoreError> {
        if self.format_version != MINIAPP_SOURCE_STORE_FORMAT_VERSION
            || self.build_profile != JavaScriptBuildProfile::MiniAppReleaseV1
            || self.build_profile_version != MINIAPP_SOURCE_BUILD_PROFILE_VERSION
            || self.source_revision == 0
            || self.build_generation == 0
        {
            return Err(PluginRuntimeSourceStoreError::InvalidRecord(
                "source head has an unsupported format, profile, or revision".into(),
            ));
        }
        validate_digest_value(self.snapshot_digest.as_ref())?;
        validate_digest_value(self.dependency_lock_digest.as_ref())?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotRecord {
    format_version: String,
    files: Vec<PluginRuntimeSourceFileDigest>,
    snapshot_digest: DigestHex,
}

impl SnapshotRecord {
    fn validate(&self) -> Result<(), PluginRuntimeSourceStoreError> {
        if self.format_version != MINIAPP_SOURCE_STORE_FORMAT_VERSION {
            return Err(PluginRuntimeSourceStoreError::InvalidRecord(
                "source snapshot format version is unsupported".into(),
            ));
        }
        let rebuilt = build_snapshot_record(self.files.clone())?;
        if rebuilt.snapshot_digest != self.snapshot_digest {
            return Err(PluginRuntimeSourceStoreError::DigestMismatch {
                expected: self.snapshot_digest.0.clone(),
                observed: rebuilt.snapshot_digest.0,
            });
        }
        Ok(())
    }
}

fn prepare_source_files(
    files: Vec<PluginRuntimeSourceFileInput>,
    content_kind: PluginRuntimeSourceContentKind,
    limits: PluginRuntimeSourceStoreLimits,
) -> Result<Vec<PluginRuntimeSourceFile>, PluginRuntimeSourceStoreError> {
    if files.len() > limits.max_file_count {
        return Err(PluginRuntimeSourceStoreError::TooManyFiles {
            observed: files.len(),
            limit: limits.max_file_count,
        });
    }
    let mut collision_keys = BTreeSet::new();
    let mut by_path = BTreeMap::new();
    let mut total_size = 0u64;
    for input in files {
        validate_source_path(&input.normalized_relative_path, content_kind)?;
        let collision_key = windows_collision_key(&input.normalized_relative_path)?;
        if !collision_keys.insert(collision_key) {
            return Err(PluginRuntimeSourceStoreError::PathCollision {
                path: input.normalized_relative_path,
            });
        }
        if input.bytes.is_empty() {
            return Err(PluginRuntimeSourceStoreError::EmptyFile {
                path: input.normalized_relative_path,
            });
        }
        let size_bytes = u64::try_from(input.bytes.len()).map_err(|_| {
            PluginRuntimeSourceStoreError::FileTooLarge {
                path: input.normalized_relative_path.clone(),
                observed: u64::MAX,
                limit: limits.max_single_file_bytes,
            }
        })?;
        if size_bytes > limits.max_single_file_bytes {
            return Err(PluginRuntimeSourceStoreError::FileTooLarge {
                path: input.normalized_relative_path,
                observed: size_bytes,
                limit: limits.max_single_file_bytes,
            });
        }
        total_size = total_size.saturating_add(size_bytes);
        if total_size > limits.max_total_bytes {
            return Err(PluginRuntimeSourceStoreError::TotalSizeExceeded {
                observed: total_size,
                limit: limits.max_total_bytes,
            });
        }
        let file = PluginRuntimeSourceFile {
            normalized_relative_path: input.normalized_relative_path.clone(),
            digest: digest_bytes(&input.bytes),
            size_bytes,
            bytes: input.bytes,
        };
        if by_path.insert(file.normalized_relative_path.clone(), file).is_some() {
            return Err(PluginRuntimeSourceStoreError::PathCollision {
                path: input.normalized_relative_path,
            });
        }
    }
    validate_required_source_entrypoints(by_path.keys().map(String::as_str), content_kind)?;
    Ok(by_path.into_values().collect())
}

fn snapshot_record(
    files: &[PluginRuntimeSourceFile],
) -> Result<SnapshotRecord, PluginRuntimeSourceStoreError> {
    build_snapshot_record(files.iter().map(PluginRuntimeSourceFile::digest_record).collect())
}

fn build_snapshot_record(
    mut files: Vec<PluginRuntimeSourceFileDigest>,
) -> Result<SnapshotRecord, PluginRuntimeSourceStoreError> {
    files.sort_by(|left, right| {
        left.normalized_relative_path
            .cmp(&right.normalized_relative_path)
    });
    let mut collisions = BTreeSet::new();
    for file in &files {
        validate_stored_source_path(&file.normalized_relative_path)?;
        validate_digest_value(file.digest.as_ref())?;
        if file.size_bytes == 0 {
            return Err(PluginRuntimeSourceStoreError::EmptyFile {
                path: file.normalized_relative_path.clone(),
            });
        }
        if !collisions.insert(windows_collision_key(&file.normalized_relative_path)?) {
            return Err(PluginRuntimeSourceStoreError::PathCollision {
                path: file.normalized_relative_path.clone(),
            });
        }
    }
    let content_kind = source_content_kind(
        files
            .iter()
            .map(|file| file.normalized_relative_path.as_str()),
    );
    validate_required_source_entrypoints(
        files
            .iter()
            .map(|file| file.normalized_relative_path.as_str()),
        content_kind,
    )?;
    let payload = SnapshotPayload {
        format_version: MINIAPP_SOURCE_STORE_FORMAT_VERSION,
        files: &files,
    };
    let snapshot_digest = canonical_digest(&payload)?;
    Ok(SnapshotRecord {
        format_version: MINIAPP_SOURCE_STORE_FORMAT_VERSION.into(),
        files,
        snapshot_digest,
    })
}

#[derive(Serialize)]
struct SnapshotPayload<'a> {
    format_version: &'static str,
    files: &'a [PluginRuntimeSourceFileDigest],
}

fn write_revision(
    revision_root: &Path,
    files: &[PluginRuntimeSourceFile],
    snapshot: &SnapshotRecord,
) -> Result<(), PluginRuntimeSourceStoreError> {
    fs::create_dir(revision_root).map_err(|error| io_error(revision_root, error))?;
    let mut created_parents = BTreeSet::new();
    for file in files {
        let target = join_relative(revision_root, &file.normalized_relative_path)?;
        let parent = target
            .parent()
            .ok_or_else(|| PluginRuntimeSourceStoreError::CorruptSource("source file has no parent".into()))?;
        ensure_relative_parent_directories(revision_root, parent, &mut created_parents)?;
        write_new_synced(&target, &file.bytes)?;
    }
    write_new_synced(
        &revision_root.join(SNAPSHOT_FILE),
        &canonical_bytes(snapshot)?,
    )?;
    sync_tree_directories(revision_root)
}

fn read_revision_files(
    revision_root: &Path,
    snapshot: &SnapshotRecord,
    limits: PluginRuntimeSourceStoreLimits,
) -> Result<Vec<PluginRuntimeSourceFile>, PluginRuntimeSourceStoreError> {
    let mut observed = BTreeMap::new();
    let mut total_size = 0u64;
    collect_source_files(revision_root, revision_root, &mut observed, &mut total_size, limits)?;
    let expected_paths = snapshot
        .files
        .iter()
        .map(|file| file.normalized_relative_path.clone())
        .collect::<BTreeSet<_>>();
    let observed_paths = observed.keys().cloned().collect::<BTreeSet<_>>();
    if expected_paths != observed_paths {
        return Err(PluginRuntimeSourceStoreError::CorruptSource(
            "source revision inventory differs from its canonical snapshot".into(),
        ));
    }
    let mut result = Vec::with_capacity(snapshot.files.len());
    for expected in &snapshot.files {
        let actual = observed.remove(&expected.normalized_relative_path).ok_or_else(|| {
            PluginRuntimeSourceStoreError::CorruptSource("source file disappeared during read".into())
        })?;
        if actual.digest != expected.digest || actual.size_bytes != expected.size_bytes {
            return Err(PluginRuntimeSourceStoreError::DigestMismatch {
                expected: expected.digest.0.clone(),
                observed: actual.digest.0,
            });
        }
        result.push(actual);
    }
    Ok(result)
}

fn collect_source_files(
    root: &Path,
    current: &Path,
    observed: &mut BTreeMap<String, PluginRuntimeSourceFile>,
    total_size: &mut u64,
    limits: PluginRuntimeSourceStoreLimits,
) -> Result<(), PluginRuntimeSourceStoreError> {
    let mut entries = fs::read_dir(current)
        .map_err(|error| io_error(current, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| io_error(current, error))?;
    entries.sort_by_key(|entry| entry.file_name());
    if entries.is_empty() {
        return Err(PluginRuntimeSourceStoreError::CorruptSource(
            "source revision contains an empty directory".into(),
        ));
    }
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(
                "symbolic links are forbidden in Plugin Source".into(),
            ));
        }
        if metadata.is_dir() {
            let canonical =
                fs::canonicalize(&path).map_err(|error| io_error(&path, error))?;
            let canonical_root =
                fs::canonicalize(root).map_err(|error| io_error(root, error))?;
            if !canonical.starts_with(&canonical_root) {
                return Err(PluginRuntimeSourceStoreError::CorruptSource(
                    "source directory escaped its revision root".into(),
                ));
            }
            collect_source_files(root, &path, observed, total_size, limits)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(
                "only regular files and directories are allowed in Source".into(),
            ));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| PluginRuntimeSourceStoreError::CorruptSource("source path escaped root".into()))?;
        let normalized = normalize_filesystem_relative_path(relative)?;
        if normalized == SNAPSHOT_FILE {
            continue;
        }
        validate_stored_source_path(&normalized)?;
        if metadata.len() == 0 {
            return Err(PluginRuntimeSourceStoreError::EmptyFile { path: normalized });
        }
        if metadata.len() > limits.max_single_file_bytes {
            return Err(PluginRuntimeSourceStoreError::FileTooLarge {
                path: normalized,
                observed: metadata.len(),
                limit: limits.max_single_file_bytes,
            });
        }
        let bytes = read_regular_bounded(&path, limits.max_single_file_bytes)?;
        let size_bytes = u64::try_from(bytes.len()).map_err(|_| {
            PluginRuntimeSourceStoreError::FileTooLarge {
                path: normalized.clone(),
                observed: u64::MAX,
                limit: limits.max_single_file_bytes,
            }
        })?;
        if size_bytes != metadata.len() {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(
                "source file changed while it was being read".into(),
            ));
        }
        *total_size = total_size.saturating_add(size_bytes);
        if *total_size > limits.max_total_bytes {
            return Err(PluginRuntimeSourceStoreError::TotalSizeExceeded {
                observed: *total_size,
                limit: limits.max_total_bytes,
            });
        }
        let file = PluginRuntimeSourceFile {
            normalized_relative_path: normalized.clone(),
            digest: digest_bytes(&bytes),
            size_bytes,
            bytes,
        };
        if observed.insert(normalized.clone(), file).is_some() {
            return Err(PluginRuntimeSourceStoreError::PathCollision { path: normalized });
        }
        if observed.len() > limits.max_file_count {
            return Err(PluginRuntimeSourceStoreError::TooManyFiles {
                observed: observed.len(),
                limit: limits.max_file_count,
            });
        }
    }
    Ok(())
}

fn validate_source_path(
    path: &str,
    content_kind: PluginRuntimeSourceContentKind,
) -> Result<(), PluginRuntimeSourceStoreError> {
    validate_relative_path(path)?;
    if content_kind == PluginRuntimeSourceContentKind::UiOnly
        && (path == "service/main.mjs" || path.starts_with("service/"))
    {
        return Err(PluginRuntimeSourceStoreError::ServiceSourceForbidden {
            path: path.to_owned(),
        });
    }
    if path == crate::runtime::PLUGIN_RUNTIME_MANIFEST_PATH
        || path == "ui/index.html"
        || path.starts_with("ui/")
        || (content_kind == PluginRuntimeSourceContentKind::Service
            && path == "service/main.mjs")
    {
        Ok(())
    } else {
        Err(PluginRuntimeSourceStoreError::InvalidPath {
            path: path.to_owned(),
            reason: match content_kind {
                PluginRuntimeSourceContentKind::UiOnly => {
                    "UI-only Plugin Source permits ui/index.html and ui/** only"
                }
                PluginRuntimeSourceContentKind::Service => {
                    "Service Plugin Source permits ui/index.html, ui/**, and service/main.mjs only"
                }
            }
            .into(),
        })
    }
}

fn validate_stored_source_path(path: &str) -> Result<(), PluginRuntimeSourceStoreError> {
    validate_relative_path(path)?;
    if path == crate::runtime::PLUGIN_RUNTIME_MANIFEST_PATH || path == "ui/index.html" || path.starts_with("ui/") || path == "service/main.mjs" {
        Ok(())
    } else {
        Err(PluginRuntimeSourceStoreError::InvalidPath {
            path: path.to_owned(),
            reason: "Plugin Source permits ui/index.html, ui/**, and optional service/main.mjs only"
                .into(),
        })
    }
}

fn validate_required_source_entrypoints<'a>(
    paths: impl IntoIterator<Item = &'a str>,
    content_kind: PluginRuntimeSourceContentKind,
) -> Result<(), PluginRuntimeSourceStoreError> {
    let paths = paths.into_iter().collect::<BTreeSet<_>>();
    if content_kind == PluginRuntimeSourceContentKind::UiOnly && !paths.contains("ui/index.html") {
        return Err(PluginRuntimeSourceStoreError::InvalidRecord(
            "Plugin Source requires ui/index.html".into(),
        ));
    }
    if content_kind == PluginRuntimeSourceContentKind::Service
        && !paths.contains("service/main.mjs")
    {
        return Err(PluginRuntimeSourceStoreError::InvalidRecord(
            "Service Plugin Source requires service/main.mjs".into(),
        ));
    }
    Ok(())
}

fn source_content_kind<'a>(
    paths: impl IntoIterator<Item = &'a str>,
) -> PluginRuntimeSourceContentKind {
    if paths
        .into_iter()
        .any(|path| path == "service/main.mjs")
    {
        PluginRuntimeSourceContentKind::Service
    } else {
        PluginRuntimeSourceContentKind::UiOnly
    }
}

fn validate_relative_path(path: &str) -> Result<(), PluginRuntimeSourceStoreError> {
    if path.is_empty()
        || path.len() > MAX_NORMALIZED_PATH_BYTES
        || path.trim() != path
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path.contains(':')
    {
        return Err(PluginRuntimeSourceStoreError::InvalidPath {
            path: path.to_owned(),
            reason: "path must be normalized, relative, slash-separated, and traversal-free".into(),
        });
    }
    for component in path.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.len() > MAX_PATH_COMPONENT_BYTES
            || component.ends_with(['.', ' '])
            || is_windows_reserved_name(component)
            || component.chars().any(is_combining_mark)
        {
            return Err(PluginRuntimeSourceStoreError::InvalidPath {
                path: path.to_owned(),
                reason: "path is not stable under Windows filename and NFC semantics".into(),
            });
        }
    }
    Ok(())
}

fn normalize_filesystem_relative_path(path: &Path) -> Result<String, PluginRuntimeSourceStoreError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(PluginRuntimeSourceStoreError::InvalidPath {
            path: path.display().to_string(),
            reason: "path must be relative".into(),
        });
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| {
                    PluginRuntimeSourceStoreError::InvalidPath {
                        path: path.display().to_string(),
                        reason: "path must be valid UTF-8".into(),
                    }
                })?;
                parts.push(value);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(PluginRuntimeSourceStoreError::InvalidPath {
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

fn windows_collision_key(path: &str) -> Result<String, PluginRuntimeSourceStoreError> {
    validate_relative_path(path)?;
    Ok(path
        .split('/')
        .map(|component| component.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("/"))
}

fn validate_scope_segment(
    field: &'static str,
    value: &str,
) -> Result<String, PluginRuntimeSourceStoreError> {
    if value.is_empty()
        || value.len() > MAX_PATH_COMPONENT_BYTES
        || value.trim() != value
        || value != value.to_ascii_lowercase()
        || !is_safe_machine_key(value)
        || value == "."
        || value == ".."
        || value.ends_with(['.', ' '])
        || is_windows_reserved_name(value)
    {
        return Err(PluginRuntimeSourceStoreError::InvalidScope {
            field,
            reason: "scope identifiers must be lowercase stable machine-key path segments".into(),
        });
    }
    Ok(value.to_owned())
}

fn is_safe_machine_key(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_display_name(value: String) -> Result<String, PluginRuntimeSourceStoreError> {
    if value.trim().is_empty()
        || value.contains('\0')
        || value.chars().count() > 255
    {
        return Err(PluginRuntimeSourceStoreError::InvalidDisplayName(
            "display name must be 1..255 characters and contain no NUL".into(),
        ));
    }
    Ok(value)
}

fn default_index_html(display_name: &str) -> Vec<u8> {
    let escaped = escape_html(display_name);
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{escaped}</title>\n</head>\n<body>\n<main><h1>{escaped}</h1></main>\n</body>\n</html>\n"
    )
    .into_bytes()
}

fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
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

fn validate_digest_value(value: &str) -> Result<DigestHex, PluginRuntimeSourceStoreError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(DigestHex::from(value.to_owned()))
    } else {
        Err(PluginRuntimeSourceStoreError::InvalidRecord(
            "digest must be a 64-character lowercase hexadecimal value".into(),
        ))
    }
}

fn checked_increment(value: u64, field: &str) -> Result<u64, PluginRuntimeSourceStoreError> {
    value
        .checked_add(1)
        .ok_or_else(|| PluginRuntimeSourceStoreError::InvalidRecord(format!("{field} overflow")))
}

fn canonical_digest<T: Serialize>(value: &T) -> Result<DigestHex, PluginRuntimeSourceStoreError> {
    Ok(digest_bytes(&canonical_bytes(value)?))
}

fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, PluginRuntimeSourceStoreError> {
    canonical_json_bytes(value)
        .map_err(|error| PluginRuntimeSourceStoreError::Canonical(error.to_string()))
}

fn parse_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
    limit: u64,
) -> Result<T, PluginRuntimeSourceStoreError> {
    if bytes.len() as u64 > limit {
        return Err(PluginRuntimeSourceStoreError::InvalidRecord(
            "canonical record exceeds its size limit".into(),
        ));
    }
    let value = serde_json::from_slice(bytes)
        .map_err(|error| PluginRuntimeSourceStoreError::InvalidRecord(error.to_string()))?;
    if canonical_bytes(&value)? != bytes {
        return Err(PluginRuntimeSourceStoreError::InvalidRecord(
            "record must use canonical JSON without duplicate or reordered fields".into(),
        ));
    }
    Ok(value)
}

fn join_relative(root: &Path, path: &str) -> Result<PathBuf, PluginRuntimeSourceStoreError> {
    validate_relative_path(path)?;
    let target = path
        .split('/')
        .fold(root.to_path_buf(), |current, component| current.join(component));
    if !target.starts_with(root) {
        return Err(PluginRuntimeSourceStoreError::CorruptSource(
            "relative path escaped its root".into(),
        ));
    }
    Ok(target)
}

fn ensure_directory_without_symlink(path: &Path) -> Result<(), PluginRuntimeSourceStoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_reparse_or_symlink(&metadata) || !metadata.is_dir() => {
            Err(PluginRuntimeSourceStoreError::CorruptSource(format!(
                "managed path is not a regular directory: {}",
                path.display()
            )))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|error| io_error(path, error))?;
            let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
            if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
                return Err(PluginRuntimeSourceStoreError::CorruptSource(format!(
                    "managed path became unsafe: {}",
                    path.display()
                )));
            }
            Ok(())
        }
        Err(error) => Err(io_error(path, error)),
    }
}

fn ensure_direct_child_directory(
    parent: &Path,
    child: &Path,
) -> Result<(), PluginRuntimeSourceStoreError> {
    if child.parent() != Some(parent) {
        return Err(PluginRuntimeSourceStoreError::CorruptSource(format!(
            "managed directory is not a direct child: {}",
            child.display()
        )));
    }
    ensure_directory_without_symlink(child)?;
    let canonical_parent = fs::canonicalize(parent).map_err(|error| io_error(parent, error))?;
    let canonical_child = fs::canonicalize(child).map_err(|error| io_error(child, error))?;
    if canonical_child.parent() != Some(canonical_parent.as_path()) {
        return Err(PluginRuntimeSourceStoreError::CorruptSource(format!(
            "managed directory escaped its parent: {}",
            child.display()
        )));
    }
    Ok(())
}

fn canonical_existing_directory_chain(
    root: &Path,
    target: &Path,
) -> Result<Option<PathBuf>, PluginRuntimeSourceStoreError> {
    let relative = target.strip_prefix(root).map_err(|_| {
        PluginRuntimeSourceStoreError::CorruptSource(
            "managed directory escaped its store root".into(),
        )
    })?;
    let root_metadata = fs::symlink_metadata(root).map_err(|error| io_error(root, error))?;
    if is_reparse_or_symlink(&root_metadata) || !root_metadata.is_dir() {
        return Err(PluginRuntimeSourceStoreError::CorruptSource(
            "managed store root is not a regular directory".into(),
        ));
    }
    let canonical_root = fs::canonicalize(root).map_err(|error| io_error(root, error))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(value) = component else {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(
                "managed directory contains a non-normal component".into(),
            ));
        };
        current.push(value);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(&current, error)),
        };
        if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(format!(
                "managed directory is not a regular directory: {}",
                current.display()
            )));
        }
    }
    let canonical_target =
        fs::canonicalize(target).map_err(|error| io_error(target, error))?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err(PluginRuntimeSourceStoreError::CorruptSource(
            "managed directory escaped its canonical store root".into(),
        ));
    }
    Ok(Some(canonical_target))
}

fn validate_removal_tree(root: &Path) -> Result<(), PluginRuntimeSourceStoreError> {
    let metadata = fs::symlink_metadata(root).map_err(|error| io_error(root, error))?;
    if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
        return Err(PluginRuntimeSourceStoreError::CorruptSource(format!(
            "removal target is not a regular directory: {}",
            root.display()
        )));
    }
    for entry in fs::read_dir(root).map_err(|error| io_error(root, error))? {
        let entry = entry.map_err(|error| io_error(root, error))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if is_reparse_or_symlink(&metadata) {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(format!(
                "removal target contains a symlink or reparse point: {}",
                path.display()
            )));
        }
        if metadata.is_dir() {
            validate_removal_tree(&path)?;
        } else if !metadata.is_file() {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(format!(
                "removal target contains a special file: {}",
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

fn ensure_relative_parent_directories(
    root: &Path,
    parent: &Path,
    created: &mut BTreeSet<String>,
) -> Result<(), PluginRuntimeSourceStoreError> {
    if !parent.starts_with(root) {
        return Err(PluginRuntimeSourceStoreError::CorruptSource(
            "source parent escaped its revision root".into(),
        ));
    }
    let relative = parent
        .strip_prefix(root)
        .map_err(|_| PluginRuntimeSourceStoreError::CorruptSource("source parent escaped root".into()))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(value) = component else {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(
                "source parent contains a non-normal component".into(),
            ));
        };
        let value = value.to_str().ok_or_else(|| {
            PluginRuntimeSourceStoreError::CorruptSource("source parent is not UTF-8".into())
        })?;
        current.push(value);
        let key = current.display().to_string();
        if created.insert(key) {
            ensure_directory_without_symlink(&current)?;
            let canonical_root =
                fs::canonicalize(root).map_err(|error| io_error(root, error))?;
            let canonical_current =
                fs::canonicalize(&current).map_err(|error| io_error(&current, error))?;
            if !canonical_current.starts_with(&canonical_root) {
                return Err(PluginRuntimeSourceStoreError::CorruptSource(
                    "source parent escaped its revision root".into(),
                ));
            }
        }
    }
    Ok(())
}

fn verify_project_inventory(project_root: &Path) -> Result<(), PluginRuntimeSourceStoreError> {
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(project_root).map_err(|error| io_error(project_root, error))? {
        let entry = entry.map_err(|error| io_error(project_root, error))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| io_error(entry.path(), error))?;
        let expected = matches!(
            name.as_str(),
            PROJECT_RECORD_FILE | DEPENDENCY_LOCK_FILE | SOURCE_DIRECTORY
        );
        if !expected
            || metadata.file_type().is_symlink()
            || (name == SOURCE_DIRECTORY && !metadata.is_dir())
            || (name != SOURCE_DIRECTORY && !metadata.is_file())
        {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(format!(
                "unexpected project inventory entry: {}",
                entry.path().display()
            )));
        }
        if !names.insert(name) {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(
                "project inventory contains duplicate names".into(),
            ));
        }
    }
    if names != BTreeSet::from([
        PROJECT_RECORD_FILE.to_owned(),
        DEPENDENCY_LOCK_FILE.to_owned(),
        SOURCE_DIRECTORY.to_owned(),
    ]) {
        return Err(PluginRuntimeSourceStoreError::CorruptSource(
            "project inventory is incomplete".into(),
        ));
    }
    Ok(())
}

fn verify_source_root_inventory(source_root: &Path) -> Result<(), PluginRuntimeSourceStoreError> {
    let metadata =
        fs::symlink_metadata(source_root).map_err(|error| io_error(source_root, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PluginRuntimeSourceStoreError::CorruptSource(
            "source root must be a regular directory".into(),
        ));
    }
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(source_root).map_err(|error| io_error(source_root, error))? {
        let entry = entry.map_err(|error| io_error(source_root, error))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|error| io_error(entry.path(), error))?;
        let expected = matches!(name.as_str(), HEAD_FILE | REVISIONS_DIRECTORY);
        if !expected
            || metadata.file_type().is_symlink()
            || (name == REVISIONS_DIRECTORY && !metadata.is_dir())
            || (name == HEAD_FILE && !metadata.is_file())
        {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(format!(
                "unexpected source root inventory entry: {}",
                entry.path().display()
            )));
        }
        names.insert(name);
    }
    if names != BTreeSet::from([HEAD_FILE.to_owned(), REVISIONS_DIRECTORY.to_owned()]) {
        return Err(PluginRuntimeSourceStoreError::CorruptSource(
            "source root inventory is incomplete".into(),
        ));
    }
    Ok(())
}

fn read_regular_bounded(
    path: &Path,
    limit: u64,
) -> Result<Vec<u8>, PluginRuntimeSourceStoreError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PluginRuntimeSourceStoreError::CorruptSource(format!(
            "expected a regular file: {}",
            path.display()
        )));
    }
    if metadata.len() > limit {
        return Err(PluginRuntimeSourceStoreError::FileTooLarge {
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
    if bytes.len() as u64 > limit {
        return Err(PluginRuntimeSourceStoreError::FileTooLarge {
            path: path.display().to_string(),
            observed: bytes.len() as u64,
            limit,
        });
    }
    Ok(bytes)
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), PluginRuntimeSourceStoreError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| io_error(path, error))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(path, error))
}

fn atomic_replace_file(
    staged: &Path,
    target: &Path,
) -> Result<(), PluginRuntimeSourceStoreError> {
    let parent = target
        .parent()
        .ok_or_else(|| PluginRuntimeSourceStoreError::CorruptSource("atomic target has no parent".into()))?;
    if target.parent() != Some(parent) {
        return Err(PluginRuntimeSourceStoreError::CorruptSource(
            "atomic target parent is invalid".into(),
        ));
    }
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(
                "atomic target must be a regular file".into(),
            ));
        }
        Ok(_) => {
            let backup = parent.join(format!(".head.previous-{}", Uuid::now_v7()));
            fs::rename(target, &backup).map_err(|error| io_error(target, error))?;
            if let Err(error) = fs::rename(staged, target) {
                let _ = fs::rename(&backup, target);
                return Err(io_error(target, error));
            }
            fs::remove_file(&backup).map_err(|error| io_error(&backup, error))?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::rename(staged, target).map_err(|error| io_error(target, error))?;
        }
        Err(error) => return Err(io_error(target, error)),
    }
    sync_directory_if_supported(parent)
}

fn sync_tree_directories(root: &Path) -> Result<(), PluginRuntimeSourceStoreError> {
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
) -> Result<(), PluginRuntimeSourceStoreError> {
    let metadata = fs::symlink_metadata(root).map_err(|error| io_error(root, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PluginRuntimeSourceStoreError::CorruptSource(format!(
            "expected a regular directory: {}",
            root.display()
        )));
    }
    directories.push(root.to_path_buf());
    for entry in fs::read_dir(root).map_err(|error| io_error(root, error))? {
        let entry = entry.map_err(|error| io_error(root, error))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(PluginRuntimeSourceStoreError::CorruptSource(
                "symbolic links are forbidden in managed storage".into(),
            ));
        }
        if metadata.is_dir() {
            collect_directories(&path, directories)?;
        }
    }
    Ok(())
}

fn sync_directory_if_supported(path: &Path) -> Result<(), PluginRuntimeSourceStoreError> {
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

fn io_error(path: impl Into<PathBuf>, source: io::Error) -> PluginRuntimeSourceStoreError {
    PluginRuntimeSourceStoreError::Io {
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

    fn commit(&mut self) -> Result<(), PluginRuntimeSourceStoreError> {
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
    let Some(uuid) = name
        .strip_prefix(STAGING_PREFIX_CREATE)
        .or_else(|| name.strip_prefix(STAGING_PREFIX_REPLACE))
    else {
        return false;
    };
    Uuid::parse_str(uuid).is_ok()
        && fs::symlink_metadata(path)
            .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
            .unwrap_or(false)
}
