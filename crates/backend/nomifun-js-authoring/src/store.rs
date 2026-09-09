use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::canonical::{canonical_json_bytes, strict_json_from_slice};
use crate::dependency::ExactDependencyLock;
use crate::error::{AuthoringError, io_error};
use crate::model::{
    NeverCancel, OperationCancellation, SourceScope, SourceStoreLimits, check_canceled,
};
use crate::scaffold::{PluginScaffoldRequest, render_plugin_scaffold};
use crate::snapshot::{CapturedSource, SourceSnapshot, capture_source_tree, verify_source_file};

const SOURCES_DIRECTORY: &str = "sources";
const PROJECTS_DIRECTORY: &str = "projects";
const SOURCE_DIRECTORY: &str = "source";
const STAGING_DIRECTORY: &str = ".staging";
const SCOPE_RECORD_FILE: &str = "scope.json";
const DEPENDENCY_LOCK_FILE: &str = "dependency-lock.json";
const SCOPE_RECORD_VERSION: &str = "1.0.0";
const MAX_DEPENDENCY_LOCK_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct SourceStore {
    managed_root: PathBuf,
    sources_root: PathBuf,
    staging_root: PathBuf,
    limits: SourceStoreLimits,
    mutation_lock: Arc<Mutex<()>>,
}

impl SourceStore {
    pub fn new(
        managed_root: impl AsRef<Path>,
        limits: SourceStoreLimits,
    ) -> Result<Self, AuthoringError> {
        let limits = limits.validate()?;
        let requested_root = managed_root.as_ref();
        ensure_directory_without_symlink(requested_root)?;
        let managed_root =
            fs::canonicalize(requested_root).map_err(|error| io_error(requested_root, error))?;
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

    pub fn create_plugin_project(
        &self,
        scope: SourceScope,
        request: &PluginScaffoldRequest,
        cancellation: &dyn OperationCancellation,
    ) -> Result<ScaffoldedPluginProject, AuthoringError> {
        let _mutation = self.lock_mutation()?;
        check_canceled(cancellation)?;
        let files = render_plugin_scaffold(request)?;
        let final_parent = self.ensure_project_parent(&scope)?;
        let final_project_root = final_parent.join(scope.project_id().as_ref());
        if fs::symlink_metadata(&final_project_root).is_ok() {
            return Err(AuthoringError::ProjectAlreadyExists);
        }

        let staging = self.allocate_staging("create")?;
        let staged_project_root = staging.path().join("project");
        let staged_source_root = staged_project_root.join(SOURCE_DIRECTORY);
        fs::create_dir(&staged_project_root)
            .map_err(|error| io_error(&staged_project_root, error))?;
        fs::create_dir(&staged_source_root)
            .map_err(|error| io_error(&staged_source_root, error))?;

        check_canceled(cancellation)?;
        let scope_record = ScopeRecord {
            format_version: SCOPE_RECORD_VERSION.into(),
            scope: scope.clone(),
        };
        write_new_synced(
            &staged_project_root.join(SCOPE_RECORD_FILE),
            &canonical_json_bytes(&scope_record)?,
        )?;
        for (relative, bytes) in files {
            check_canceled(cancellation)?;
            let target = relative.join(&staged_source_root);
            ensure_relative_parent_directories(&staged_source_root, &target)?;
            write_new_synced(&target, &bytes)?;
        }

        let capture = capture_source_tree(&staged_source_root, self.limits, cancellation)?;
        check_canceled(cancellation)?;
        match fs::rename(&staged_project_root, &final_project_root) {
            Ok(()) => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::AlreadyExists
                    || final_project_root.exists() =>
            {
                return Err(AuthoringError::ProjectAlreadyExists);
            }
            Err(error) => return Err(io_error(&final_project_root, error)),
        }
        sync_directory_if_supported(&final_parent)?;

        Ok(ScaffoldedPluginProject {
            project: self.load_project(&scope)?,
            capture,
        })
    }

    pub fn load_project(&self, scope: &SourceScope) -> Result<StoredSourceProject, AuthoringError> {
        let project_root = self.project_root(scope);
        let project_metadata = match fs::symlink_metadata(&project_root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(AuthoringError::ProjectNotFound);
            }
            Err(error) => return Err(io_error(&project_root, error)),
        };
        if project_metadata.file_type().is_symlink() || !project_metadata.is_dir() {
            return Err(AuthoringError::UnsafeManagedPath { path: project_root });
        }
        let canonical_project_root =
            fs::canonicalize(&project_root).map_err(|error| io_error(&project_root, error))?;
        let expected_parent = self.project_parent(scope);
        let canonical_parent = fs::canonicalize(&expected_parent)
            .map_err(|error| io_error(&expected_parent, error))?;
        if canonical_project_root.parent() != Some(canonical_parent.as_path()) {
            return Err(AuthoringError::UnsafeManagedPath {
                path: canonical_project_root,
            });
        }
        verify_project_inventory(&canonical_project_root)?;

        let record_path = canonical_project_root.join(SCOPE_RECORD_FILE);
        let record_bytes = read_regular_bounded(&record_path, 64 * 1024)?;
        let record: ScopeRecord = strict_json_from_slice(&record_bytes)
            .map_err(|error| AuthoringError::InvalidScopeRecord(error.to_string()))?;
        if canonical_json_bytes(&record)? != record_bytes {
            return Err(AuthoringError::InvalidScopeRecord(
                "scope record must use canonical JSON".into(),
            ));
        }
        if record.format_version != SCOPE_RECORD_VERSION || record.scope != *scope {
            return Err(AuthoringError::ScopeMismatch);
        }

        let source_root = canonical_project_root.join(SOURCE_DIRECTORY);
        let source_metadata =
            fs::symlink_metadata(&source_root).map_err(|error| io_error(&source_root, error))?;
        if source_metadata.file_type().is_symlink() || !source_metadata.is_dir() {
            return Err(AuthoringError::UnsafeManagedPath { path: source_root });
        }
        let source_root =
            fs::canonicalize(&source_root).map_err(|error| io_error(&source_root, error))?;
        if source_root.parent() != Some(canonical_project_root.as_path()) {
            return Err(AuthoringError::UnsafeManagedPath { path: source_root });
        }

        Ok(StoredSourceProject {
            scope: scope.clone(),
            managed_relative_path: managed_relative_source_path(scope),
            project_root: canonical_project_root,
            source_root,
        })
    }

    pub fn snapshot(
        &self,
        scope: &SourceScope,
        cancellation: &dyn OperationCancellation,
    ) -> Result<CapturedSource, AuthoringError> {
        let project = self.load_project(scope)?;
        capture_source_tree(&project.source_root, self.limits, cancellation)
    }

    pub fn write_initial_dependency_lock(
        &self,
        scope: &SourceScope,
        expected_source: &SourceSnapshot,
        lock: &ExactDependencyLock,
        cancellation: &dyn OperationCancellation,
    ) -> Result<crate::DigestHex, AuthoringError> {
        let _mutation = self.lock_mutation()?;
        check_canceled(cancellation)?;
        let project = self.load_project(scope)?;
        let captured =
            capture_source_tree(&project.source_root, self.limits, cancellation)?;
        if captured.snapshot() != expected_source {
            return Err(AuthoringError::SourceChanged {
                expected: expected_source.digest().as_ref().to_owned(),
                observed: captured.snapshot().digest().as_ref().to_owned(),
            });
        }
        lock.validate_against(captured.dependency_requests())?;
        let bytes = canonical_json_bytes(lock)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX)
            > MAX_DEPENDENCY_LOCK_BYTES
        {
            return Err(AuthoringError::FileTooLarge {
                path: DEPENDENCY_LOCK_FILE.to_owned(),
                observed: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                limit: MAX_DEPENDENCY_LOCK_BYTES,
            });
        }
        write_new_synced(
            &project.project_root.join(DEPENDENCY_LOCK_FILE),
            &bytes,
        )?;
        sync_directory_if_supported(&project.project_root)?;
        lock.digest()
    }

    pub fn load_dependency_lock(
        &self,
        scope: &SourceScope,
        cancellation: &dyn OperationCancellation,
    ) -> Result<ExactDependencyLock, AuthoringError> {
        let _mutation = self.lock_mutation()?;
        check_canceled(cancellation)?;
        let project = self.load_project(scope)?;
        let captured =
            capture_source_tree(&project.source_root, self.limits, cancellation)?;
        self.read_dependency_lock_unlocked(
            &project,
            captured.dependency_requests(),
        )
    }

    /// Apply one project-scoped Chat Dev source edit using an exact source CAS.
    ///
    /// The edit is fully validated in private staging before the live source
    /// entry is replaced. A dependency-changing edit is rejected until the
    /// application service has produced a matching exact lock through its
    /// dedicated dependency workflow.
    pub fn apply_source_edit(
        &self,
        scope: &SourceScope,
        expected_source: &SourceSnapshot,
        edit: SourceFileEdit,
        cancellation: &dyn OperationCancellation,
    ) -> Result<SourceEditOutcome, AuthoringError> {
        check_canceled(cancellation)?;
        let project = self.load_project(scope)?;
        let current = capture_source_tree(&project.source_root, self.limits, cancellation)?;
        ensure_expected_snapshot(expected_source, &current)?;
        let current_lock = self.read_dependency_lock_unlocked(
            &project,
            current.dependency_requests(),
        )?;

        let staging = self.allocate_staging("edit")?;
        let staged_source_root = staging.path().join(SOURCE_DIRECTORY);
        fs::create_dir(&staged_source_root)
            .map_err(|error| io_error(&staged_source_root, error))?;
        copy_snapshot_to_staging(
            &project.source_root,
            &staged_source_root,
            current.snapshot(),
            cancellation,
        )?;
        apply_staged_source_edit(&staged_source_root, &edit)?;

        let next = capture_source_tree(&staged_source_root, self.limits, cancellation)?;
        let lock_request = current_lock.request_digest().as_ref().to_owned();
        let next_request = next.dependency_requests().digest()?.as_ref().to_owned();
        if lock_request != next_request {
            return Err(AuthoringError::DependencyLockOutOfDate {
                lock_request,
                next_request,
            });
        }
        current_lock.validate_against(next.dependency_requests())?;

        check_canceled(cancellation)?;
        // Staging is intentionally outside the mutation lock so independent
        // projects and readers do not wait for a large source copy. Recheck
        // the exact Project head while holding the short commit fence.
        let _mutation = self.lock_mutation()?;
        let observed = capture_source_tree(&project.source_root, self.limits, cancellation)?;
        ensure_expected_snapshot(expected_source, &observed)?;
        let live_lock = self.read_dependency_lock_unlocked(
            &project,
            observed.dependency_requests(),
        )?;
        if live_lock.request_digest().as_ref() != next_request {
            return Err(AuthoringError::DependencyLockOutOfDate {
                lock_request: live_lock.request_digest().as_ref().to_owned(),
                next_request,
            });
        }
        live_lock.validate_against(next.dependency_requests())?;

        if next.snapshot() == observed.snapshot() {
            return Ok(SourceEditOutcome {
                capture: observed,
                changed: false,
            });
        }

        let staged_target = edit.path().join(&staged_source_root);
        let live_target = edit.path().join(&project.source_root);
        let backup_target = staging.path().join("previous-source-entry");
        match &edit {
            SourceFileEdit::Replace { .. } => {
                ensure_relative_parent_directories(&project.source_root, &live_target)?;
                atomic_replace_source_file(
                    &staged_target,
                    &live_target,
                    &backup_target,
                )?;
            }
            SourceFileEdit::Delete { .. } => {
                atomic_delete_source_file(&live_target, &backup_target)?;
            }
        }

        // The file swap is the commit boundary. Cancellation cannot be
        // reported after it, because the caller cannot roll the edit back.
        let final_capture =
            capture_source_tree(&project.source_root, self.limits, &NeverCancel)?;
        if final_capture.snapshot() != next.snapshot() {
            return Err(AuthoringError::SourceChanged {
                expected: next.snapshot().digest().as_ref().to_owned(),
                observed: final_capture.snapshot().digest().as_ref().to_owned(),
            });
        }
        Ok(SourceEditOutcome {
            capture: final_capture,
            changed: true,
        })
    }

    pub fn delete_project(
        &self,
        scope: &SourceScope,
    ) -> Result<(), AuthoringError> {
        let _mutation = self.lock_mutation()?;
        let project = match self.load_project(scope) {
            Ok(project) => project,
            Err(AuthoringError::ProjectNotFound) => return Ok(()),
            Err(error) => return Err(error),
        };
        let parent = project
            .project_root
            .parent()
            .ok_or_else(|| AuthoringError::UnsafeManagedPath {
                path: project.project_root.clone(),
            })?
            .to_path_buf();
        fs::remove_dir_all(&project.project_root)
            .map_err(|error| io_error(&project.project_root, error))?;
        sync_directory_if_supported(&parent)
    }

    pub fn stage_snapshot(
        &self,
        scope: &SourceScope,
        expected: &SourceSnapshot,
        cancellation: &dyn OperationCancellation,
    ) -> Result<StagedSource, AuthoringError> {
        check_canceled(cancellation)?;
        let project = self.load_project(scope)?;
        let current = capture_source_tree(&project.source_root, self.limits, cancellation)?;
        if current.snapshot() != expected {
            return Err(AuthoringError::SourceChanged {
                expected: expected.digest().as_ref().to_owned(),
                observed: current.snapshot().digest().as_ref().to_owned(),
            });
        }

        let mut staging = self.allocate_staging("build")?;
        let source_root = staging.path().join(SOURCE_DIRECTORY);
        let output_root = staging.path().join("output");
        fs::create_dir(&source_root).map_err(|error| io_error(&source_root, error))?;
        fs::create_dir(&output_root).map_err(|error| io_error(&output_root, error))?;

        for file in expected.files() {
            check_canceled(cancellation)?;
            let source = file.normalized_relative_path().join(&project.source_root);
            let target = file.normalized_relative_path().join(&source_root);
            ensure_relative_parent_directories(&source_root, &target)?;
            verify_source_file(&source, &target, file, cancellation)?;
        }
        check_canceled(cancellation)?;
        let capture = capture_source_tree(&source_root, self.limits, cancellation)?;
        if capture.snapshot() != expected {
            return Err(AuthoringError::SourceChanged {
                expected: expected.digest().as_ref().to_owned(),
                observed: capture.snapshot().digest().as_ref().to_owned(),
            });
        }

        let operation_root = staging.take_path();
        Ok(StagedSource {
            staging_parent: self.staging_root.clone(),
            source_root,
            output_root,
            operation_root,
            capture,
        })
    }

    fn lock_mutation(&self) -> Result<MutexGuard<'_, ()>, AuthoringError> {
        self.mutation_lock
            .lock()
            .map_err(|_| AuthoringError::MutationLockPoisoned)
    }

    fn read_dependency_lock_unlocked(
        &self,
        project: &StoredSourceProject,
        requests: &crate::DependencyRequestSet,
    ) -> Result<ExactDependencyLock, AuthoringError> {
        let path = project.project_root.join(DEPENDENCY_LOCK_FILE);
        let bytes = read_regular_bounded(&path, MAX_DEPENDENCY_LOCK_BYTES)?;
        let lock: ExactDependencyLock = strict_json_from_slice(&bytes)?;
        if canonical_json_bytes(&lock)? != bytes {
            return Err(AuthoringError::InvalidDependencyLock(
                "dependency lock must use canonical JSON".into(),
            ));
        }
        lock.validate_against(requests)?;
        Ok(lock)
    }

    fn allocate_staging(&self, kind: &str) -> Result<StagingGuard, AuthoringError> {
        for _ in 0..8 {
            let path = self.staging_root.join(format!("{kind}-{}", Uuid::now_v7()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(StagingGuard {
                        staging_parent: self.staging_root.clone(),
                        path: Some(path),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    continue;
                }
                Err(error) => return Err(io_error(&path, error)),
            }
        }
        Err(AuthoringError::Io {
            path: self.staging_root.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "could not allocate a unique source staging directory",
            ),
        })
    }

    fn ensure_project_parent(&self, scope: &SourceScope) -> Result<PathBuf, AuthoringError> {
        let owner_root = self.sources_root.join(scope.owner_id().as_ref());
        ensure_direct_child_directory(&self.sources_root, &owner_root)?;
        let projects_root = owner_root.join(PROJECTS_DIRECTORY);
        ensure_direct_child_directory(&owner_root, &projects_root)?;
        Ok(projects_root)
    }

    fn project_parent(&self, scope: &SourceScope) -> PathBuf {
        self.sources_root
            .join(scope.owner_id().as_ref())
            .join(PROJECTS_DIRECTORY)
    }

    fn project_root(&self, scope: &SourceScope) -> PathBuf {
        self.project_parent(scope).join(scope.project_id().as_ref())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSourceProject {
    scope: SourceScope,
    managed_relative_path: String,
    project_root: PathBuf,
    source_root: PathBuf,
}

impl StoredSourceProject {
    pub fn scope(&self) -> &SourceScope {
        &self.scope
    }

    pub fn managed_relative_path(&self) -> &str {
        &self.managed_relative_path
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn source_root(&self) -> &Path {
        &self.source_root
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScaffoldedPluginProject {
    project: StoredSourceProject,
    capture: CapturedSource,
}

impl ScaffoldedPluginProject {
    pub fn project(&self) -> &StoredSourceProject {
        &self.project
    }

    pub fn capture(&self) -> &CapturedSource {
        &self.capture
    }
}

#[derive(Debug)]
pub struct StagedSource {
    staging_parent: PathBuf,
    operation_root: PathBuf,
    source_root: PathBuf,
    output_root: PathBuf,
    capture: CapturedSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceFileEdit {
    Replace {
        path: crate::NormalizedSourcePath,
        bytes: Vec<u8>,
    },
    Delete {
        path: crate::NormalizedSourcePath,
    },
}

impl SourceFileEdit {
    pub fn replace(
        path: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<Self, AuthoringError> {
        let path = validate_edit_path(crate::NormalizedSourcePath::parse(path)?)?;
        Ok(Self::Replace {
            path,
            bytes: bytes.into(),
        })
    }

    pub fn delete(path: impl Into<String>) -> Result<Self, AuthoringError> {
        let path = validate_edit_path(crate::NormalizedSourcePath::parse(path)?)?;
        Ok(Self::Delete { path })
    }

    pub fn path(&self) -> &crate::NormalizedSourcePath {
        match self {
            Self::Replace { path, .. } | Self::Delete { path } => path,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceEditOutcome {
    capture: CapturedSource,
    changed: bool,
}

impl SourceEditOutcome {
    pub fn capture(&self) -> &CapturedSource {
        &self.capture
    }

    pub fn changed(&self) -> bool {
        self.changed
    }
}

impl StagedSource {
    pub fn operation_root(&self) -> &Path {
        &self.operation_root
    }

    pub fn source_root(&self) -> &Path {
        &self.source_root
    }

    pub fn output_root(&self) -> &Path {
        &self.output_root
    }

    pub fn capture(&self) -> &CapturedSource {
        &self.capture
    }
}

impl Drop for StagedSource {
    fn drop(&mut self) {
        cleanup_owned_staging(&self.staging_parent, &self.operation_root);
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeRecord {
    format_version: String,
    scope: SourceScope,
}

struct StagingGuard {
    staging_parent: PathBuf,
    path: Option<PathBuf>,
}

impl StagingGuard {
    fn path(&self) -> &Path {
        self.path.as_deref().expect("staging path is present")
    }

    fn take_path(&mut self) -> PathBuf {
        self.path.take().expect("staging path is present")
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            cleanup_owned_staging(&self.staging_parent, &path);
        }
    }
}

fn managed_relative_source_path(scope: &SourceScope) -> String {
    format!(
        "{SOURCES_DIRECTORY}/{}/{PROJECTS_DIRECTORY}/{}/{SOURCE_DIRECTORY}",
        scope.owner_id().as_ref(),
        scope.project_id().as_ref()
    )
}

fn ensure_expected_snapshot(
    expected: &SourceSnapshot,
    observed: &CapturedSource,
) -> Result<(), AuthoringError> {
    if observed.snapshot() != expected {
        return Err(AuthoringError::SourceChanged {
            expected: expected.digest().as_ref().to_owned(),
            observed: observed.snapshot().digest().as_ref().to_owned(),
        });
    }
    Ok(())
}

fn validate_edit_path(
    path: crate::NormalizedSourcePath,
) -> Result<crate::NormalizedSourcePath, AuthoringError> {
    if let Some(reason) = path.fixed_profile_rejection() {
        return Err(AuthoringError::ForbiddenSourceEntry {
            path: path.to_string(),
            reason: reason.into(),
        });
    }
    Ok(path)
}

fn copy_snapshot_to_staging(
    source_root: &Path,
    staged_source_root: &Path,
    snapshot: &SourceSnapshot,
    cancellation: &dyn OperationCancellation,
) -> Result<(), AuthoringError> {
    for file in snapshot.files() {
        check_canceled(cancellation)?;
        let source = file.normalized_relative_path().join(source_root);
        let target = file.normalized_relative_path().join(staged_source_root);
        ensure_relative_parent_directories(staged_source_root, &target)?;
        verify_source_file(&source, &target, file, cancellation)?;
    }
    Ok(())
}

fn apply_staged_source_edit(
    staged_source_root: &Path,
    edit: &SourceFileEdit,
) -> Result<(), AuthoringError> {
    let target = edit.path().join(staged_source_root);
    match edit {
        SourceFileEdit::Replace { bytes, .. } => {
            if let Ok(metadata) = fs::symlink_metadata(&target) {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(AuthoringError::UnsafeSourcePath {
                        path: edit.path().to_string(),
                        reason: "source edit target is not a regular file".into(),
                    });
                }
                fs::remove_file(&target).map_err(|error| io_error(&target, error))?;
            }
            ensure_relative_parent_directories(staged_source_root, &target)?;
            write_new_synced(&target, bytes)
        }
        SourceFileEdit::Delete { .. } => match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                Err(AuthoringError::UnsafeSourcePath {
                    path: edit.path().to_string(),
                    reason: "source edit target is not a regular file".into(),
                })
            }
            Ok(_) => fs::remove_file(&target).map_err(|error| io_error(&target, error)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error(&target, error)),
        },
    }
}

fn atomic_replace_source_file(
    staged: &Path,
    target: &Path,
    backup: &Path,
) -> Result<(), AuthoringError> {
    ensure_regular_or_missing_source_file(target)?;
    match fs::symlink_metadata(target) {
        Ok(_) => {
            fs::rename(target, backup).map_err(|error| io_error(target, error))?;
            if let Err(error) = fs::rename(staged, target) {
                let _ = fs::rename(backup, target);
                return Err(io_error(target, error));
            }
            if let Err(error) = fs::remove_file(backup) {
                let _ = fs::remove_file(target);
                let _ = fs::rename(backup, target);
                return Err(io_error(backup, error));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::rename(staged, target).map_err(|error| io_error(target, error))?;
        }
        Err(error) => return Err(io_error(target, error)),
    }
    sync_directory_if_supported(
        target
            .parent()
            .ok_or_else(|| AuthoringError::UnsafeManagedPath {
                path: target.to_path_buf(),
            })?,
    )
}

fn atomic_delete_source_file(
    target: &Path,
    backup: &Path,
) -> Result<(), AuthoringError> {
    ensure_regular_or_missing_source_file(target)?;
    match fs::symlink_metadata(target) {
        Ok(_) => {
            fs::rename(target, backup).map_err(|error| io_error(target, error))?;
            if let Err(error) = fs::remove_file(backup) {
                let _ = fs::rename(backup, target);
                return Err(io_error(backup, error));
            }
            sync_directory_if_supported(
                target
                    .parent()
                    .ok_or_else(|| AuthoringError::UnsafeManagedPath {
                        path: target.to_path_buf(),
                    })?,
            )
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(target, error)),
    }
}

fn ensure_regular_or_missing_source_file(path: &Path) -> Result<(), AuthoringError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(AuthoringError::UnsafeSourcePath {
                path: path.display().to_string(),
                reason: "source edit target is not a regular file".into(),
            })
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(path, error)),
    }
}

fn ensure_directory_without_symlink(path: &Path) -> Result<(), AuthoringError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(AuthoringError::UnsafeManagedPath {
                path: path.to_path_buf(),
            })
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|error| io_error(path, error))?;
            let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(AuthoringError::UnsafeManagedPath {
                    path: path.to_path_buf(),
                });
            }
            Ok(())
        }
        Err(error) => Err(io_error(path, error)),
    }
}

fn ensure_direct_child_directory(parent: &Path, child: &Path) -> Result<(), AuthoringError> {
    if child.parent() != Some(parent) {
        return Err(AuthoringError::UnsafeManagedPath {
            path: child.to_path_buf(),
        });
    }
    ensure_directory_without_symlink(child)?;
    let canonical_parent = fs::canonicalize(parent).map_err(|error| io_error(parent, error))?;
    let canonical_child = fs::canonicalize(child).map_err(|error| io_error(child, error))?;
    if canonical_child.parent() != Some(canonical_parent.as_path()) {
        return Err(AuthoringError::UnsafeManagedPath {
            path: child.to_path_buf(),
        });
    }
    Ok(())
}

fn ensure_relative_parent_directories(root: &Path, target: &Path) -> Result<(), AuthoringError> {
    let parent = target
        .parent()
        .ok_or_else(|| AuthoringError::UnsafeManagedPath {
            path: target.to_path_buf(),
        })?;
    if !parent.starts_with(root) {
        return Err(AuthoringError::UnsafeManagedPath {
            path: target.to_path_buf(),
        });
    }
    let relative = parent
        .strip_prefix(root)
        .map_err(|_| AuthoringError::UnsafeManagedPath {
            path: target.to_path_buf(),
        })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        ensure_directory_without_symlink(&current)?;
        let canonical = fs::canonicalize(&current).map_err(|error| io_error(&current, error))?;
        if !canonical.starts_with(root) {
            return Err(AuthoringError::UnsafeManagedPath { path: current });
        }
    }
    Ok(())
}

fn verify_project_inventory(project_root: &Path) -> Result<(), AuthoringError> {
    let mut entries = fs::read_dir(project_root)
        .map_err(|error| io_error(project_root, error))?
        .map(|entry| entry.map_err(|error| io_error(project_root, error)))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    if !(entries.len() == 2 || entries.len() == 3) {
        return Err(AuthoringError::UnsafeManagedPath {
            path: project_root.to_path_buf(),
        });
    }
    for entry in entries {
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|error| io_error(entry.path(), error))?;
        let valid = if entry.file_name() == SCOPE_RECORD_FILE
            || entry.file_name() == DEPENDENCY_LOCK_FILE
        {
            metadata.is_file() && !metadata.file_type().is_symlink()
        } else if entry.file_name() == SOURCE_DIRECTORY {
            metadata.is_dir() && !metadata.file_type().is_symlink()
        } else {
            false
        };
        if !valid {
            return Err(AuthoringError::UnsafeManagedPath { path: entry.path() });
        }
    }
    Ok(())
}

fn read_regular_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, AuthoringError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > limit {
        return Err(AuthoringError::UnsafeManagedPath {
            path: path.to_path_buf(),
        });
    }
    fs::read(path).map_err(|error| io_error(path, error))
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), AuthoringError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| io_error(path, error))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(path, error))
}

fn cleanup_owned_staging(staging_parent: &Path, target: &Path) {
    if target.parent() != Some(staging_parent) {
        return;
    }
    let Some(name) = target.file_name().and_then(|value| value.to_str()) else {
        return;
    };
    let Some(uuid) = name
        .strip_prefix("create-")
        .or_else(|| name.strip_prefix("build-"))
        .or_else(|| name.strip_prefix("edit-"))
    else {
        return;
    };
    if Uuid::parse_str(uuid).is_err() {
        return;
    }
    let Ok(metadata) = fs::symlink_metadata(target) else {
        return;
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return;
    }
    let _ = fs::remove_dir_all(target);
    let _ = sync_directory_if_supported(staging_parent);
}

#[cfg(unix)]
fn sync_directory_if_supported(path: &Path) -> Result<(), AuthoringError> {
    match std::fs::File::open(path).and_then(|file| file.sync_all()) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::InvalidInput | std::io::ErrorKind::Unsupported
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(io_error(path, error)),
    }
}

#[cfg(not(unix))]
fn sync_directory_if_supported(_path: &Path) -> Result<(), AuthoringError> {
    Ok(())
}
