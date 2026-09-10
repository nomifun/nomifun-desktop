use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::canonical::{canonical_json_bytes, strict_json_from_slice};
use crate::dependency::{
    DependencyRequestSet, ExactDependencyLock, package_json_with_dependencies,
};
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
const DEPENDENCY_MUTATION_RECORD_FILE: &str = "mutation.json";
const DEPENDENCY_MUTATION_JOURNAL_FILE: &str = ".dependency-mutation.json";
const DEPENDENCY_MUTATION_FORMAT_VERSION: &str = "1.0.0";
const NEXT_DEPENDENCY_LOCK_FILE: &str = "next-dependency-lock.json";
const PREVIOUS_PACKAGE_JSON_FILE: &str = "previous-package.json";
const PREVIOUS_DEPENDENCY_LOCK_FILE: &str = "previous-dependency-lock.json";
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
        self.load_project_internal(scope, false)
    }

    fn load_project_internal(
        &self,
        scope: &SourceScope,
        allow_dependency_journal: bool,
    ) -> Result<StoredSourceProject, AuthoringError> {
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
        verify_project_inventory(&canonical_project_root, allow_dependency_journal)?;

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

    pub fn dependency_state(
        &self,
        scope: &SourceScope,
        cancellation: &dyn OperationCancellation,
    ) -> Result<DependencyState, AuthoringError> {
        let _mutation = self.lock_mutation()?;
        check_canceled(cancellation)?;
        let project = self.load_project(scope)?;
        let capture = capture_source_tree(&project.source_root, self.limits, cancellation)?;
        let lock = self.read_dependency_lock_raw_unlocked(&project)?;
        lock.validate_against(capture.dependency_requests())?;
        Ok(DependencyState {
            source_digest: capture.snapshot().digest().clone(),
            lock_digest: lock.digest()?,
            direct_dependencies: capture.dependency_requests().dependencies().clone(),
        })
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

    pub fn prepare_dependency_mutation(
        &self,
        scope: &SourceScope,
        mutation_id: &str,
        expected_project_revision: u64,
        expected_build_generation: u64,
        expected_source_digest: &crate::DigestHex,
        expected_lock_digest: &crate::DigestHex,
        requests: &DependencyRequestSet,
        lock: &ExactDependencyLock,
        cancellation: &dyn OperationCancellation,
    ) -> Result<PreparedDependencyMutation, AuthoringError> {
        validate_dependency_mutation_id(mutation_id)?;
        check_canceled(cancellation)?;
        let _mutation = self.lock_mutation()?;
        lock.validate_against(requests)?;
        let next_lock_digest = lock.digest()?;
        let lock_bytes = canonical_json_bytes(lock)?;
        require_dependency_lock_size(&lock_bytes)?;

        let project = self.load_project(scope)?;
        let current = capture_source_tree(&project.source_root, self.limits, cancellation)?;
        require_source_digest(expected_source_digest, current.snapshot().digest())?;
        let current_lock =
            self.read_dependency_lock_unlocked(&project, current.dependency_requests())?;
        require_lock_digest(expected_lock_digest, &current_lock.digest()?)?;

        let staging = self.allocate_dependency_staging(mutation_id)?;
        let staged_source_root = staging.path().join(SOURCE_DIRECTORY);
        fs::create_dir(&staged_source_root)
            .map_err(|error| io_error(&staged_source_root, error))?;
        copy_snapshot_to_staging(
            &project.source_root,
            &staged_source_root,
            current.snapshot(),
            cancellation,
        )?;
        if current.dependency_requests() != requests {
            let staged_package_json = staged_source_root.join("package.json");
            let package_json =
                read_regular_bounded(&staged_package_json, self.limits.max_package_json_bytes)?;
            let next_package_json = package_json_with_dependencies(&package_json, requests)?;
            if u64::try_from(next_package_json.len()).unwrap_or(u64::MAX)
                > self.limits.max_package_json_bytes
            {
                return Err(AuthoringError::FileTooLarge {
                    path: "package.json".into(),
                    observed: u64::try_from(next_package_json.len()).unwrap_or(u64::MAX),
                    limit: self.limits.max_package_json_bytes,
                });
            }
            fs::remove_file(&staged_package_json)
                .map_err(|error| io_error(&staged_package_json, error))?;
            write_new_synced(&staged_package_json, &next_package_json)?;
        }
        let staged_lock = staging.path().join(NEXT_DEPENDENCY_LOCK_FILE);
        write_new_synced(&staged_lock, &lock_bytes)?;

        let next = capture_source_tree(&staged_source_root, self.limits, cancellation)?;
        if next.dependency_requests() != requests {
            return Err(AuthoringError::DependencyMutationConflict(
                "staged package.json does not contain the resolved request set".into(),
            ));
        }
        check_canceled(cancellation)?;
        let facts = DependencyMutationFacts::new(
            mutation_id,
            scope.clone(),
            expected_project_revision,
            expected_build_generation,
            expected_source_digest.clone(),
            expected_lock_digest.clone(),
            next.snapshot().digest().clone(),
            next_lock_digest,
        )?;
        let record = DependencyMutationRecord {
            format_version: DEPENDENCY_MUTATION_FORMAT_VERSION.into(),
            facts: facts.clone(),
        };
        write_new_synced(
            &staging.path().join(DEPENDENCY_MUTATION_RECORD_FILE),
            &canonical_json_bytes(&record)?,
        )?;
        sync_directory_if_supported(staging.path())?;
        Ok(PreparedDependencyMutation { facts, staging })
    }

    pub fn commit_dependency_mutation(
        &self,
        mutation: &DurableDependencyMutation,
    ) -> Result<(), AuthoringError> {
        validate_dependency_operation_root(
            &self.staging_root,
            &mutation.operation_root,
            mutation.facts.mutation_id(),
        )?;
        let record = read_dependency_mutation_record(&mutation.operation_root)?;
        require_dependency_facts(&mutation.facts, &record.facts)?;

        let _mutation = self.lock_mutation()?;
        let project = self.load_project(mutation.facts.scope())?;
        require_project_dependency_facts(
            &project,
            self.limits,
            mutation.facts.expected_source_digest(),
            mutation.facts.expected_lock_digest(),
        )?;
        let journal = project.project_root.join(DEPENDENCY_MUTATION_JOURNAL_FILE);
        write_new_synced(&journal, &canonical_json_bytes(&record)?)?;
        sync_directory_if_supported(&project.project_root)?;

        let live_package = project.source_root.join("package.json");
        let live_lock = project.project_root.join(DEPENDENCY_LOCK_FILE);
        let staged_package = mutation
            .operation_root
            .join(SOURCE_DIRECTORY)
            .join("package.json");
        let staged_lock = mutation.operation_root.join(NEXT_DEPENDENCY_LOCK_FILE);
        let previous_package = mutation.operation_root.join(PREVIOUS_PACKAGE_JSON_FILE);
        let previous_lock = mutation.operation_root.join(PREVIOUS_DEPENDENCY_LOCK_FILE);
        move_regular_file(&live_package, &previous_package)?;
        move_regular_file(&live_lock, &previous_lock)?;
        move_regular_file(&staged_package, &live_package)?;
        move_regular_file(&staged_lock, &live_lock)?;
        sync_directory_if_supported(&project.source_root)?;
        sync_directory_if_supported(&project.project_root)?;

        let committed = self.load_project_internal(mutation.facts.scope(), true)?;
        require_project_dependency_facts(
            &committed,
            self.limits,
            mutation.facts.next_source_digest(),
            mutation.facts.next_lock_digest(),
        )
        .map_err(|error| {
            AuthoringError::DependencyMutationNeedsRecovery(format!(
                "committed dependency files do not match the durable intent: {error}"
            ))
        })
    }

    pub fn finish_dependency_mutation(
        &self,
        facts: &DependencyMutationFacts,
    ) -> Result<(), AuthoringError> {
        validate_dependency_mutation_id(facts.mutation_id())?;
        let _mutation = self.lock_mutation()?;
        let project = self.load_project_internal(facts.scope(), true)?;
        require_dependency_journal(&project, facts)?;
        require_project_dependency_facts(
            &project,
            self.limits,
            facts.next_source_digest(),
            facts.next_lock_digest(),
        )?;
        remove_dependency_operation_root(&self.staging_root, facts.mutation_id())?;
        remove_regular_file_if_present(
            &project.project_root.join(DEPENDENCY_MUTATION_JOURNAL_FILE),
        )?;
        sync_directory_if_supported(&project.project_root)
    }

    pub fn rollback_dependency_mutation(
        &self,
        facts: &DependencyMutationFacts,
    ) -> Result<(), AuthoringError> {
        validate_dependency_mutation_id(facts.mutation_id())?;
        let _mutation = self.lock_mutation()?;
        let project = self.load_project_internal(facts.scope(), true)?;
        let journal_path = project.project_root.join(DEPENDENCY_MUTATION_JOURNAL_FILE);
        if journal_path.exists() {
            require_dependency_journal(&project, facts)?;
        }
        let operation_root = dependency_operation_root(&self.staging_root, facts.mutation_id());
        validate_dependency_operation_root(
            &self.staging_root,
            &operation_root,
            facts.mutation_id(),
        )?;
        restore_previous_file(
            &project.source_root.join("package.json"),
            &operation_root.join(PREVIOUS_PACKAGE_JSON_FILE),
        )?;
        restore_previous_file(
            &project.project_root.join(DEPENDENCY_LOCK_FILE),
            &operation_root.join(PREVIOUS_DEPENDENCY_LOCK_FILE),
        )?;
        require_project_dependency_facts(
            &project,
            self.limits,
            facts.expected_source_digest(),
            facts.expected_lock_digest(),
        )
        .map_err(|error| {
            AuthoringError::DependencyMutationNeedsRecovery(format!(
                "dependency rollback could not restore the previous facts: {error}"
            ))
        })?;
        remove_dependency_operation_root(&self.staging_root, facts.mutation_id())?;
        remove_regular_file_if_present(&journal_path)?;
        sync_directory_if_supported(&project.project_root)
    }

    pub fn list_dependency_mutation_journals(
        &self,
    ) -> Result<Vec<DependencyMutationFacts>, AuthoringError> {
        let _mutation = self.lock_mutation()?;
        let mut facts = Vec::new();
        for owner in sorted_directory_entries(&self.sources_root)? {
            require_plain_directory(&owner)?;
            let projects = owner.path().join(PROJECTS_DIRECTORY);
            if !projects.exists() {
                continue;
            }
            require_plain_directory_path(&projects)?;
            for project in sorted_directory_entries(&projects)? {
                require_plain_directory(&project)?;
                let journal = project.path().join(DEPENDENCY_MUTATION_JOURNAL_FILE);
                if !journal.exists() {
                    continue;
                }
                let record = read_dependency_journal_path(&journal)?;
                let owner_id = owner.file_name().to_string_lossy().into_owned();
                let project_id = project.file_name().to_string_lossy().into_owned();
                if record.facts.scope().owner_id().as_ref() != owner_id
                    || record.facts.scope().project_id().as_ref() != project_id
                {
                    return Err(AuthoringError::DependencyMutationNeedsRecovery(
                        "dependency journal scope differs from its managed path".into(),
                    ));
                }
                facts.push(record.facts);
            }
        }
        facts.sort_by(|left, right| left.mutation_id.cmp(&right.mutation_id));
        Ok(facts)
    }

    pub fn dependency_mutation_journal(
        &self,
        scope: &SourceScope,
    ) -> Result<Option<DependencyMutationFacts>, AuthoringError> {
        let _mutation = self.lock_mutation()?;
        let journal = self
            .project_root(scope)
            .join(DEPENDENCY_MUTATION_JOURNAL_FILE);
        if !journal.exists() {
            return Ok(None);
        }
        let project = self.load_project_internal(scope, true)?;
        let journal = project.project_root.join(DEPENDENCY_MUTATION_JOURNAL_FILE);
        let record = read_dependency_journal_path(&journal)?;
        if record.facts.scope() != scope {
            return Err(AuthoringError::DependencyMutationNeedsRecovery(
                "dependency journal scope differs from the requested Project".into(),
            ));
        }
        Ok(Some(record.facts))
    }

    pub fn cleanup_orphan_dependency_staging(
        &self,
        retained_mutation_ids: &BTreeSet<String>,
    ) -> Result<(), AuthoringError> {
        let _mutation = self.lock_mutation()?;
        for entry in sorted_directory_entries(&self.staging_root)? {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(mutation_id) = name.strip_prefix("dependency-") else {
                continue;
            };
            validate_dependency_mutation_id(mutation_id)?;
            if !retained_mutation_ids.contains(mutation_id) {
                remove_dependency_operation_root(&self.staging_root, mutation_id)?;
            }
        }
        Ok(())
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
        let lock = self.read_dependency_lock_raw_unlocked(project)?;
        lock.validate_against(requests)?;
        Ok(lock)
    }

    fn read_dependency_lock_raw_unlocked(
        &self,
        project: &StoredSourceProject,
    ) -> Result<ExactDependencyLock, AuthoringError> {
        let path = project.project_root.join(DEPENDENCY_LOCK_FILE);
        let bytes = read_regular_bounded(&path, MAX_DEPENDENCY_LOCK_BYTES)?;
        let lock: ExactDependencyLock = strict_json_from_slice(&bytes)?;
        if canonical_json_bytes(&lock)? != bytes {
            return Err(AuthoringError::InvalidDependencyLock(
                "dependency lock must use canonical JSON".into(),
            ));
        }
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

    fn allocate_dependency_staging(
        &self,
        mutation_id: &str,
    ) -> Result<StagingGuard, AuthoringError> {
        validate_dependency_mutation_id(mutation_id)?;
        let path = dependency_operation_root(&self.staging_root, mutation_id);
        match fs::create_dir(&path) {
            Ok(()) => Ok(StagingGuard {
                staging_parent: self.staging_root.clone(),
                path: Some(path),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(AuthoringError::DependencyMutationConflict(format!(
                    "dependency mutation staging already exists for {mutation_id}"
                )))
            }
            Err(error) => Err(io_error(&path, error)),
        }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencyState {
    source_digest: crate::DigestHex,
    lock_digest: crate::DigestHex,
    direct_dependencies: std::collections::BTreeMap<String, String>,
}

impl DependencyState {
    pub fn new(
        source_digest: crate::DigestHex,
        lock_digest: crate::DigestHex,
        direct_dependencies: std::collections::BTreeMap<String, String>,
    ) -> Result<Self, AuthoringError> {
        validate_dependency_digest(&source_digest)?;
        validate_dependency_digest(&lock_digest)?;
        DependencyRequestSet::new(direct_dependencies.clone())?;
        Ok(Self {
            source_digest,
            lock_digest,
            direct_dependencies,
        })
    }

    pub fn source_digest(&self) -> &crate::DigestHex {
        &self.source_digest
    }

    pub fn lock_digest(&self) -> &crate::DigestHex {
        &self.lock_digest
    }

    pub fn direct_dependencies(&self) -> &std::collections::BTreeMap<String, String> {
        &self.direct_dependencies
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyMutationFacts {
    mutation_id: String,
    scope: SourceScope,
    expected_project_revision: u64,
    expected_build_generation: u64,
    expected_source_digest: crate::DigestHex,
    expected_lock_digest: crate::DigestHex,
    next_source_digest: crate::DigestHex,
    next_lock_digest: crate::DigestHex,
}

impl DependencyMutationFacts {
    pub fn new(
        mutation_id: impl Into<String>,
        scope: SourceScope,
        expected_project_revision: u64,
        expected_build_generation: u64,
        expected_source_digest: crate::DigestHex,
        expected_lock_digest: crate::DigestHex,
        next_source_digest: crate::DigestHex,
        next_lock_digest: crate::DigestHex,
    ) -> Result<Self, AuthoringError> {
        let facts = Self {
            mutation_id: mutation_id.into(),
            scope,
            expected_project_revision,
            expected_build_generation,
            expected_source_digest,
            expected_lock_digest,
            next_source_digest,
            next_lock_digest,
        };
        validate_dependency_facts(&facts)?;
        Ok(facts)
    }

    pub fn mutation_id(&self) -> &str {
        &self.mutation_id
    }

    pub fn scope(&self) -> &SourceScope {
        &self.scope
    }

    pub fn expected_project_revision(&self) -> u64 {
        self.expected_project_revision
    }

    pub fn expected_build_generation(&self) -> u64 {
        self.expected_build_generation
    }

    pub fn expected_source_digest(&self) -> &crate::DigestHex {
        &self.expected_source_digest
    }

    pub fn expected_lock_digest(&self) -> &crate::DigestHex {
        &self.expected_lock_digest
    }

    pub fn next_source_digest(&self) -> &crate::DigestHex {
        &self.next_source_digest
    }

    pub fn next_lock_digest(&self) -> &crate::DigestHex {
        &self.next_lock_digest
    }

    pub fn is_noop(&self) -> bool {
        self.expected_source_digest == self.next_source_digest
            && self.expected_lock_digest == self.next_lock_digest
    }
}

#[derive(Debug)]
pub struct PreparedDependencyMutation {
    facts: DependencyMutationFacts,
    staging: StagingGuard,
}

impl PreparedDependencyMutation {
    pub fn facts(&self) -> &DependencyMutationFacts {
        &self.facts
    }

    pub fn persist(mut self) -> DurableDependencyMutation {
        DurableDependencyMutation {
            facts: self.facts.clone(),
            operation_root: self.staging.take_path(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DurableDependencyMutation {
    facts: DependencyMutationFacts,
    operation_root: PathBuf,
}

impl DurableDependencyMutation {
    pub fn facts(&self) -> &DependencyMutationFacts {
        &self.facts
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DependencyMutationRecord {
    format_version: String,
    facts: DependencyMutationFacts,
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

#[derive(Debug)]
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

fn validate_dependency_mutation_id(mutation_id: &str) -> Result<(), AuthoringError> {
    let parsed = Uuid::parse_str(mutation_id).map_err(|error| {
        AuthoringError::DependencyMutationConflict(format!(
            "mutation_id must be a canonical UUIDv7: {error}"
        ))
    })?;
    if parsed.get_version_num() != 7 || parsed.to_string() != mutation_id {
        return Err(AuthoringError::DependencyMutationConflict(
            "mutation_id must be a lowercase canonical UUIDv7".into(),
        ));
    }
    Ok(())
}

fn validate_dependency_digest(digest: &crate::DigestHex) -> Result<(), AuthoringError> {
    if digest.as_ref().len() == 64
        && digest
            .as_ref()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(AuthoringError::InvalidDigest {
            value: digest.as_ref().to_owned(),
        })
    }
}

fn validate_dependency_facts(facts: &DependencyMutationFacts) -> Result<(), AuthoringError> {
    validate_dependency_mutation_id(facts.mutation_id())?;
    SourceScope::new(
        facts.scope().owner_id().clone(),
        facts.scope().project_id().clone(),
    )?;
    for digest in [
        facts.expected_source_digest(),
        facts.expected_lock_digest(),
        facts.next_source_digest(),
        facts.next_lock_digest(),
    ] {
        validate_dependency_digest(digest)?;
    }
    Ok(())
}

fn require_source_digest(
    expected: &crate::DigestHex,
    observed: &crate::DigestHex,
) -> Result<(), AuthoringError> {
    validate_dependency_digest(expected)?;
    if expected == observed {
        Ok(())
    } else {
        Err(AuthoringError::SourceChanged {
            expected: expected.as_ref().to_owned(),
            observed: observed.as_ref().to_owned(),
        })
    }
}

fn require_lock_digest(
    expected: &crate::DigestHex,
    observed: &crate::DigestHex,
) -> Result<(), AuthoringError> {
    validate_dependency_digest(expected)?;
    if expected == observed {
        Ok(())
    } else {
        Err(AuthoringError::DependencyLockChanged {
            expected: expected.as_ref().to_owned(),
            observed: observed.as_ref().to_owned(),
        })
    }
}

fn require_dependency_facts(
    expected: &DependencyMutationFacts,
    observed: &DependencyMutationFacts,
) -> Result<(), AuthoringError> {
    validate_dependency_facts(observed)?;
    if expected == observed {
        Ok(())
    } else {
        Err(AuthoringError::DependencyMutationConflict(
            "dependency mutation facts do not match the durable record".into(),
        ))
    }
}

fn require_dependency_lock_size(bytes: &[u8]) -> Result<(), AuthoringError> {
    let observed = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if observed <= MAX_DEPENDENCY_LOCK_BYTES {
        Ok(())
    } else {
        Err(AuthoringError::FileTooLarge {
            path: DEPENDENCY_LOCK_FILE.into(),
            observed,
            limit: MAX_DEPENDENCY_LOCK_BYTES,
        })
    }
}

fn dependency_operation_root(staging_root: &Path, mutation_id: &str) -> PathBuf {
    staging_root.join(format!("dependency-{mutation_id}"))
}

fn validate_dependency_operation_root(
    staging_root: &Path,
    operation_root: &Path,
    mutation_id: &str,
) -> Result<(), AuthoringError> {
    validate_dependency_mutation_id(mutation_id)?;
    if operation_root != dependency_operation_root(staging_root, mutation_id)
        || operation_root.parent() != Some(staging_root)
    {
        return Err(AuthoringError::UnsafeManagedPath {
            path: operation_root.to_path_buf(),
        });
    }
    match fs::symlink_metadata(operation_root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(AuthoringError::UnsafeManagedPath {
                path: operation_root.to_path_buf(),
            })
        }
        Ok(_) => {
            let canonical = fs::canonicalize(operation_root)
                .map_err(|error| io_error(operation_root, error))?;
            if canonical.parent() == Some(staging_root) {
                Ok(())
            } else {
                Err(AuthoringError::UnsafeManagedPath { path: canonical })
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(operation_root, error)),
    }
}

fn read_dependency_mutation_record(
    operation_root: &Path,
) -> Result<DependencyMutationRecord, AuthoringError> {
    let path = operation_root.join(DEPENDENCY_MUTATION_RECORD_FILE);
    let bytes = read_regular_bounded(&path, 64 * 1024)?;
    let record: DependencyMutationRecord = strict_json_from_slice(&bytes)?;
    if canonical_json_bytes(&record)? != bytes
        || record.format_version != DEPENDENCY_MUTATION_FORMAT_VERSION
    {
        return Err(AuthoringError::DependencyMutationConflict(
            "dependency mutation record is non-canonical or unsupported".into(),
        ));
    }
    validate_dependency_facts(&record.facts)?;
    Ok(record)
}

fn require_dependency_journal(
    project: &StoredSourceProject,
    expected: &DependencyMutationFacts,
) -> Result<(), AuthoringError> {
    let path = project.project_root.join(DEPENDENCY_MUTATION_JOURNAL_FILE);
    let record = read_dependency_journal_path(&path)?;
    require_dependency_facts(expected, &record.facts)
}

fn read_dependency_journal_path(
    path: &Path,
) -> Result<DependencyMutationRecord, AuthoringError> {
    let bytes = read_regular_bounded(path, 64 * 1024)?;
    let record: DependencyMutationRecord = strict_json_from_slice(&bytes)?;
    if canonical_json_bytes(&record)? != bytes
        || record.format_version != DEPENDENCY_MUTATION_FORMAT_VERSION
    {
        return Err(AuthoringError::DependencyMutationNeedsRecovery(
            "dependency mutation journal is non-canonical or unsupported".into(),
        ));
    }
    validate_dependency_facts(&record.facts)?;
    Ok(record)
}

fn sorted_directory_entries(path: &Path) -> Result<Vec<fs::DirEntry>, AuthoringError> {
    let mut entries = fs::read_dir(path)
        .map_err(|error| io_error(path, error))?
        .map(|entry| entry.map_err(|error| io_error(path, error)))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn require_plain_directory(entry: &fs::DirEntry) -> Result<(), AuthoringError> {
    require_plain_directory_path(&entry.path())
}

fn require_plain_directory_path(path: &Path) -> Result<(), AuthoringError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        Err(AuthoringError::UnsafeManagedPath {
            path: path.to_path_buf(),
        })
    } else {
        Ok(())
    }
}

fn read_dependency_lock_raw(
    project: &StoredSourceProject,
) -> Result<ExactDependencyLock, AuthoringError> {
    let path = project.project_root.join(DEPENDENCY_LOCK_FILE);
    let bytes = read_regular_bounded(&path, MAX_DEPENDENCY_LOCK_BYTES)?;
    let lock: ExactDependencyLock = strict_json_from_slice(&bytes)?;
    if canonical_json_bytes(&lock)? != bytes {
        return Err(AuthoringError::InvalidDependencyLock(
            "dependency lock must use canonical JSON".into(),
        ));
    }
    Ok(lock)
}

fn require_project_dependency_facts(
    project: &StoredSourceProject,
    limits: SourceStoreLimits,
    source_digest: &crate::DigestHex,
    lock_digest: &crate::DigestHex,
) -> Result<(), AuthoringError> {
    let capture = capture_source_tree(&project.source_root, limits, &NeverCancel)?;
    require_source_digest(source_digest, capture.snapshot().digest())?;
    let lock = read_dependency_lock_raw(project)?;
    lock.validate_against(capture.dependency_requests())?;
    require_lock_digest(lock_digest, &lock.digest()?)
}

fn move_regular_file(source: &Path, destination: &Path) -> Result<(), AuthoringError> {
    let source_metadata = fs::symlink_metadata(source).map_err(|error| io_error(source, error))?;
    if source_metadata.file_type().is_symlink() || !source_metadata.is_file() {
        return Err(AuthoringError::UnsafeManagedPath {
            path: source.to_path_buf(),
        });
    }
    if fs::symlink_metadata(destination).is_ok() {
        return Err(AuthoringError::DependencyMutationConflict(format!(
            "dependency mutation destination already exists: {}",
            destination.display()
        )));
    }
    fs::rename(source, destination).map_err(|error| io_error(destination, error))
}

fn restore_previous_file(live: &Path, previous: &Path) -> Result<(), AuthoringError> {
    let previous_metadata = match fs::symlink_metadata(previous) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error(previous, error)),
    };
    if previous_metadata.file_type().is_symlink() || !previous_metadata.is_file() {
        return Err(AuthoringError::UnsafeManagedPath {
            path: previous.to_path_buf(),
        });
    }
    match fs::symlink_metadata(live) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(AuthoringError::UnsafeManagedPath {
                path: live.to_path_buf(),
            });
        }
        Ok(_) => fs::remove_file(live).map_err(|error| io_error(live, error))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(live, error)),
    }
    fs::rename(previous, live).map_err(|error| io_error(live, error))
}

fn remove_dependency_operation_root(
    staging_root: &Path,
    mutation_id: &str,
) -> Result<(), AuthoringError> {
    let operation_root = dependency_operation_root(staging_root, mutation_id);
    validate_dependency_operation_root(staging_root, &operation_root, mutation_id)?;
    let metadata = match fs::symlink_metadata(&operation_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error(&operation_root, error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AuthoringError::UnsafeManagedPath {
            path: operation_root,
        });
    }
    for entry in walkdir::WalkDir::new(&operation_root).follow_links(false) {
        let entry = entry.map_err(|error| {
            AuthoringError::DependencyMutationNeedsRecovery(error.to_string())
        })?;
        let file_type = entry.file_type();
        if file_type.is_symlink() || !(file_type.is_file() || file_type.is_dir()) {
            return Err(AuthoringError::UnsafeManagedPath {
                path: entry.path().to_path_buf(),
            });
        }
    }
    fs::remove_dir_all(&operation_root).map_err(|error| io_error(&operation_root, error))?;
    sync_directory_if_supported(staging_root)
}

fn remove_regular_file_if_present(path: &Path) -> Result<(), AuthoringError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(AuthoringError::UnsafeManagedPath {
                path: path.to_path_buf(),
            })
        }
        Ok(_) => fs::remove_file(path).map_err(|error| io_error(path, error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(path, error)),
    }
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

fn verify_project_inventory(
    project_root: &Path,
    allow_dependency_journal: bool,
) -> Result<(), AuthoringError> {
    let mut entries = fs::read_dir(project_root)
        .map_err(|error| io_error(project_root, error))?
        .map(|entry| entry.map_err(|error| io_error(project_root, error)))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    let maximum_entries = if allow_dependency_journal { 4 } else { 3 };
    if entries.len() < 2 || entries.len() > maximum_entries {
        return Err(AuthoringError::UnsafeManagedPath {
            path: project_root.to_path_buf(),
        });
    }
    for entry in entries {
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|error| io_error(entry.path(), error))?;
        let valid = if entry.file_name() == SCOPE_RECORD_FILE
            || entry.file_name() == DEPENDENCY_LOCK_FILE
            || (allow_dependency_journal
                && entry.file_name() == DEPENDENCY_MUTATION_JOURNAL_FILE)
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
        .or_else(|| name.strip_prefix("dependency-"))
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
