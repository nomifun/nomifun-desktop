use std::collections::BTreeMap;
use std::sync::RwLock;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    CanonicalErrorCode, DigestHex, MiniAppDeletingIntent, MiniAppId, MiniAppProductLifecycleState,
    MiniAppPublishRequest, MiniAppRollbackRequest, MiniAppUiOnlyAutoPublishAuthorization,
    OperationId,
};

use crate::runtime::{
    DurablePluginRuntimeOperation, PluginRuntimeCatalogRecord, PluginRuntimeDataRoot, PluginRuntimeDeletionRecord,
    PluginRuntimeOperationKind, PluginRuntimeOperationState, PluginRuntimePlatformError, PluginRuntimePlatformResult,
    PluginRuntimeRepositorySnapshot, StoredPluginRuntimeRelease, new_delete_record,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeMutationExpectation {
    pub miniapp_id: MiniAppId,
    pub product_revision: u64,
    pub pointer_revision: u64,
    pub active_release_epoch: u64,
    pub lifecycle: MiniAppProductLifecycleState,
    pub ready_release_digest: Option<DigestHex>,
    pub active_release_digest: Option<DigestHex>,
    pub previous_release_digest: Option<DigestHex>,
}

impl PluginRuntimeMutationExpectation {
    pub fn from_snapshot(snapshot: &PluginRuntimeRepositorySnapshot) -> Self {
        let product = &snapshot.root.product;
        Self {
            miniapp_id: product.miniapp_id.clone(),
            product_revision: product.product_revision,
            pointer_revision: product.pointers.pointer_revision,
            active_release_epoch: product.pointers.active_release_epoch,
            lifecycle: product.lifecycle,
            ready_release_digest: product
                .pointers
                .ready_release
                .as_ref()
                .map(|release| release.release_digest.clone()),
            active_release_digest: product
                .pointers
                .active_release
                .as_ref()
                .map(|release| release.release_digest.clone()),
            previous_release_digest: product
                .pointers
                .previous_release
                .as_ref()
                .map(|release| release.release_digest.clone()),
        }
    }

    pub fn validate(&self, snapshot: &PluginRuntimeRepositorySnapshot) -> PluginRuntimePlatformResult<()> {
        let observed = Self::from_snapshot(snapshot);
        if &observed == self {
            Ok(())
        } else {
            Err(PluginRuntimePlatformError::CompareAndSwapConflict)
        }
    }
}

#[derive(Clone, Debug)]
pub struct CreatePluginRuntimeCommit {
    pub expected_library_revision: u64,
    pub root: PluginRuntimeDataRoot,
}

#[derive(Clone, Debug)]
pub struct CompleteReadyReleaseCommit {
    pub expected: PluginRuntimeMutationExpectation,
    pub release: StoredPluginRuntimeRelease,
    pub completed_operation: DurablePluginRuntimeOperation,
    pub now_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginRuntimeReleaseCommand {
    Publish(MiniAppPublishRequest),
    Rollback(MiniAppRollbackRequest),
}

#[derive(Clone, Debug)]
pub struct CommitPluginRuntimeRelease {
    pub expected: PluginRuntimeMutationExpectation,
    pub command: PluginRuntimeReleaseCommand,
    pub now_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginRuntimeLifecycleCommand {
    Enable,
    Disable,
    Trash,
    Restore,
}

#[derive(Clone, Debug)]
pub struct CommitPluginRuntimeLifecycle {
    pub expected: PluginRuntimeMutationExpectation,
    pub command: PluginRuntimeLifecycleCommand,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct CommitPluginRuntimeAutoPublish {
    pub expected: PluginRuntimeMutationExpectation,
    pub authorization: Option<MiniAppUiOnlyAutoPublishAuthorization>,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct BeginPluginRuntimeDelete {
    pub expected: PluginRuntimeMutationExpectation,
    pub operation_id: OperationId,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct FailPluginRuntimeDelete {
    pub miniapp_id: MiniAppId,
    pub operation_id: OperationId,
    pub expected_operation_revision: u64,
    pub error: CanonicalErrorCode,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct RestartPluginRuntimeDelete {
    pub miniapp_id: MiniAppId,
    pub expected_operation_id: OperationId,
    pub operation_id: OperationId,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct FinalizePluginRuntimeDelete {
    pub miniapp_id: MiniAppId,
    pub operation_id: OperationId,
    pub expected_operation_revision: u64,
    pub now_ms: i64,
}

/// Persistence port for the clean-start M1 data root.
///
/// Implementations must execute each mutating method in one database
/// transaction. `commit_release` and `commit_lifecycle` also own the Catalog
/// rows for that MiniApp, so product state and published capabilities cannot
/// become half-committed.
#[async_trait]
pub trait PluginRuntimeRepository: Send + Sync {
    async fn library_revision(&self) -> PluginRuntimePlatformResult<u64>;
    async fn list(&self) -> PluginRuntimePlatformResult<Vec<PluginRuntimeRepositorySnapshot>>;
    async fn get(&self, miniapp_id: &MiniAppId) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot>;

    async fn create(
        &self,
        commit: CreatePluginRuntimeCommit,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot>;
    async fn start_operation(
        &self,
        operation: DurablePluginRuntimeOperation,
    ) -> PluginRuntimePlatformResult<DurablePluginRuntimeOperation>;
    async fn finish_operation(
        &self,
        expected_revision: u64,
        operation: DurablePluginRuntimeOperation,
    ) -> PluginRuntimePlatformResult<DurablePluginRuntimeOperation>;
    async fn complete_ready_release(
        &self,
        commit: CompleteReadyReleaseCommit,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot>;
    async fn commit_release(
        &self,
        commit: CommitPluginRuntimeRelease,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot>;
    async fn commit_lifecycle(
        &self,
        commit: CommitPluginRuntimeLifecycle,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot>;
    async fn commit_auto_publish(
        &self,
        commit: CommitPluginRuntimeAutoPublish,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot>;
    async fn begin_delete(
        &self,
        command: BeginPluginRuntimeDelete,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot>;
    async fn fail_delete(
        &self,
        command: FailPluginRuntimeDelete,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot>;
    async fn restart_delete(
        &self,
        command: RestartPluginRuntimeDelete,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot>;
    async fn finalize_delete(
        &self,
        command: FinalizePluginRuntimeDelete,
    ) -> PluginRuntimePlatformResult<u64>;
    async fn get_operation(
        &self,
        operation_id: &OperationId,
    ) -> PluginRuntimePlatformResult<DurablePluginRuntimeOperation>;
    async fn list_operations(
        &self,
        miniapp_id: &MiniAppId,
    ) -> PluginRuntimePlatformResult<Vec<DurablePluginRuntimeOperation>>;
}

#[derive(Default)]
struct MemoryState {
    library_revision: u64,
    roots: BTreeMap<MiniAppId, PluginRuntimeDataRoot>,
    catalog: BTreeMap<MiniAppId, PluginRuntimeCatalogRecord>,
    deleting_intents: BTreeMap<MiniAppId, MiniAppDeletingIntent>,
    operations: BTreeMap<OperationId, DurablePluginRuntimeOperation>,
}

#[derive(Default)]
pub struct InMemoryPluginRuntimeRepository {
    state: RwLock<MemoryState>,
}

impl InMemoryPluginRuntimeRepository {
    pub fn new() -> Self {
        Self::default()
    }

    fn read_state(&self) -> PluginRuntimePlatformResult<std::sync::RwLockReadGuard<'_, MemoryState>> {
        self.state
            .read()
            .map_err(|_| PluginRuntimePlatformError::Repository("repository lock poisoned".into()))
    }

    fn write_state(&self) -> PluginRuntimePlatformResult<std::sync::RwLockWriteGuard<'_, MemoryState>> {
        self.state
            .write()
            .map_err(|_| PluginRuntimePlatformError::Repository("repository lock poisoned".into()))
    }
}

#[async_trait]
impl PluginRuntimeRepository for InMemoryPluginRuntimeRepository {
    async fn library_revision(&self) -> PluginRuntimePlatformResult<u64> {
        Ok(self.read_state()?.library_revision)
    }

    async fn list(&self) -> PluginRuntimePlatformResult<Vec<PluginRuntimeRepositorySnapshot>> {
        let state = self.read_state()?;
        state
            .roots
            .keys()
            .map(|miniapp_id| snapshot(&state, miniapp_id))
            .collect()
    }

    async fn get(&self, miniapp_id: &MiniAppId) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        let state = self.read_state()?;
        snapshot(&state, miniapp_id)
    }

    async fn create(
        &self,
        commit: CreatePluginRuntimeCommit,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        commit.root.validate_structure()?;
        if commit.root.product.lifecycle != MiniAppProductLifecycleState::Disabled
            || commit.root.product.pointers.active_release.is_some()
        {
            return Err(PluginRuntimePlatformError::InvalidState(
                "new Plugin data root must start disabled without an Active Release".into(),
            ));
        }
        let miniapp_id = commit.root.product.miniapp_id.clone();
        let mut state = self.write_state()?;
        if state.library_revision != commit.expected_library_revision {
            return Err(PluginRuntimePlatformError::CompareAndSwapConflict);
        }
        if state.roots.contains_key(&miniapp_id) {
            return Err(PluginRuntimePlatformError::AlreadyExists(miniapp_id.0));
        }
        state.roots.insert(miniapp_id.clone(), commit.root);
        increment_library_revision(&mut state)?;
        let result = snapshot(&state, &miniapp_id)?;
        result.validate()?;
        Ok(result)
    }

    async fn start_operation(
        &self,
        operation: DurablePluginRuntimeOperation,
    ) -> PluginRuntimePlatformResult<DurablePluginRuntimeOperation> {
        operation.validate()?;
        if operation.state != PluginRuntimeOperationState::Running {
            return Err(PluginRuntimePlatformError::InvalidState(
                "new durable operation must start in running state".into(),
            ));
        }
        let mut state = self.write_state()?;
        let root = state
            .roots
            .get(&operation.miniapp_id)
            .ok_or_else(|| PluginRuntimePlatformError::NotFound(operation.miniapp_id.0.clone()))?;
        if matches!(
            root.product.lifecycle,
            MiniAppProductLifecycleState::Trashed | MiniAppProductLifecycleState::Deleting
        ) || operation.kind == PluginRuntimeOperationKind::PermanentDelete
        {
            return Err(PluginRuntimePlatformError::LifecycleConflict(format!(
                "{:?}",
                root.product.lifecycle
            )));
        }
        ensure_owner_idle(&state, &operation.miniapp_id, None)?;
        if state.operations.contains_key(&operation.operation_id) {
            return Err(PluginRuntimePlatformError::AlreadyExists(
                operation.operation_id.0,
            ));
        }
        state
            .operations
            .insert(operation.operation_id.clone(), operation.clone());
        Ok(operation)
    }

    async fn finish_operation(
        &self,
        expected_revision: u64,
        operation: DurablePluginRuntimeOperation,
    ) -> PluginRuntimePlatformResult<DurablePluginRuntimeOperation> {
        operation.validate()?;
        if operation.state == PluginRuntimeOperationState::Running {
            return Err(PluginRuntimePlatformError::InvalidState(
                "finish_operation requires a terminal state".into(),
            ));
        }
        let mut state = self.write_state()?;
        let observed = state
            .operations
            .get(&operation.operation_id)
            .ok_or_else(|| PluginRuntimePlatformError::UnknownOperation(operation.operation_id.0.clone()))?;
        validate_operation_transition(observed, expected_revision, &operation)?;
        state
            .operations
            .insert(operation.operation_id.clone(), operation.clone());
        Ok(operation)
    }

    async fn complete_ready_release(
        &self,
        commit: CompleteReadyReleaseCommit,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        commit.release.validate()?;
        commit.completed_operation.validate()?;
        if !matches!(
            commit.completed_operation.kind,
            PluginRuntimeOperationKind::Build | PluginRuntimeOperationKind::Import
        ) || commit.completed_operation.state != PluginRuntimeOperationState::Succeeded
            || commit.release.ready.origin_operation_id
                != commit.completed_operation.operation_id
            || commit.release.miniapp_id != commit.expected.miniapp_id
            || commit.completed_operation.miniapp_id != commit.expected.miniapp_id
            || !commit
                .completed_operation
                .result_artifact_digests
                .values()
                .any(|digest| digest == &commit.release.artifact.artifact_digest)
        {
            return Err(PluginRuntimePlatformError::InvalidState(
                "Ready Release must atomically complete its exact Build or Import operation"
                    .into(),
            ));
        }

        let mut state = self.write_state()?;
        let current = snapshot(&state, &commit.expected.miniapp_id)?;
        commit.expected.validate(&current)?;
        ensure_owner_idle(
            &state,
            &commit.expected.miniapp_id,
            Some(&commit.completed_operation.operation_id),
        )?;
        let observed_operation = state
            .operations
            .get(&commit.completed_operation.operation_id)
            .ok_or_else(|| {
                PluginRuntimePlatformError::UnknownOperation(
                    commit.completed_operation.operation_id.0.clone(),
                )
            })?;
        validate_operation_transition(
            observed_operation,
            observed_operation.revision,
            &commit.completed_operation,
        )?;

        let mut root = current.root;
        root.replace_ready(commit.release, commit.now_ms)?;
        state.roots.insert(commit.expected.miniapp_id.clone(), root);
        state.operations.insert(
            commit.completed_operation.operation_id.clone(),
            commit.completed_operation,
        );
        increment_library_revision(&mut state)?;
        let result = snapshot(&state, &commit.expected.miniapp_id)?;
        result.validate()?;
        Ok(result)
    }

    async fn commit_release(
        &self,
        commit: CommitPluginRuntimeRelease,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        let miniapp_id = commit.expected.miniapp_id.clone();
        let mut state = self.write_state()?;
        let current = snapshot(&state, &miniapp_id)?;
        commit.expected.validate(&current)?;
        ensure_owner_idle(&state, &miniapp_id, None)?;
        current.root.ensure_release_mutable()?;

        let mut root = current.root;
        let next = match &commit.command {
            PluginRuntimeReleaseCommand::Publish(request) => {
                if request.miniapp_id != miniapp_id {
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "Publish request belongs to another Plugin".into(),
                    ));
                }
                let ready = root.ready_release()?;
                if ready.release_ref() != &request.target_ready_release {
                    return Err(PluginRuntimePlatformError::CompareAndSwapConflict);
                }
                request.next_state(&root.product.pointers)?
            }
            PluginRuntimeReleaseCommand::Rollback(request) => {
                if request.miniapp_id != miniapp_id {
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "Rollback request belongs to another Plugin".into(),
                    ));
                }
                let previous = root.previous_release()?;
                if previous.release_ref() != &request.rollback_target {
                    return Err(PluginRuntimePlatformError::CompareAndSwapConflict);
                }
                request.next_state(&root.product.pointers)?
            }
        };
        root.apply_pointer_state(next, commit.now_ms)?;
        state.roots.insert(miniapp_id.clone(), root);
        synchronize_catalog(&mut state, &miniapp_id)?;
        increment_library_revision(&mut state)?;
        let result = snapshot(&state, &miniapp_id)?;
        result.validate()?;
        Ok(result)
    }

    async fn commit_lifecycle(
        &self,
        commit: CommitPluginRuntimeLifecycle,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        let miniapp_id = commit.expected.miniapp_id.clone();
        let mut state = self.write_state()?;
        let current = snapshot(&state, &miniapp_id)?;
        commit.expected.validate(&current)?;
        ensure_owner_idle(&state, &miniapp_id, None)?;
        let target = lifecycle_target(current.root.product.lifecycle, commit.command)?;

        let mut root = current.root;
        root.apply_lifecycle(target, commit.now_ms)?;
        state.roots.insert(miniapp_id.clone(), root);
        synchronize_catalog(&mut state, &miniapp_id)?;
        increment_library_revision(&mut state)?;
        let result = snapshot(&state, &miniapp_id)?;
        result.validate()?;
        Ok(result)
    }

    async fn commit_auto_publish(
        &self,
        commit: CommitPluginRuntimeAutoPublish,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        let miniapp_id = commit.expected.miniapp_id.clone();
        let mut state = self.write_state()?;
        let current = snapshot(&state, &miniapp_id)?;
        commit.expected.validate(&current)?;
        ensure_owner_idle(&state, &miniapp_id, None)?;
        let mut root = current.root;
        root.set_auto_publish(commit.authorization, commit.now_ms)?;
        state.roots.insert(miniapp_id.clone(), root);
        increment_library_revision(&mut state)?;
        let result = snapshot(&state, &miniapp_id)?;
        result.validate()?;
        Ok(result)
    }

    async fn begin_delete(
        &self,
        command: BeginPluginRuntimeDelete,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        let miniapp_id = command.expected.miniapp_id.clone();
        let mut state = self.write_state()?;
        let current = snapshot(&state, &miniapp_id)?;
        command.expected.validate(&current)?;
        ensure_owner_idle(&state, &miniapp_id, None)?;
        if current.root.product.lifecycle != MiniAppProductLifecycleState::Trashed {
            return Err(PluginRuntimePlatformError::LifecycleConflict(format!(
                "{:?}",
                current.root.product.lifecycle
            )));
        }
        if state.deleting_intents.contains_key(&miniapp_id)
            || state.operations.contains_key(&command.operation_id)
        {
            return Err(PluginRuntimePlatformError::AlreadyExists(
                command.operation_id.0,
            ));
        }

        let deletion =
            new_delete_record(miniapp_id.clone(), command.operation_id, command.now_ms)?;
        let mut root = current.root;
        root.apply_lifecycle(MiniAppProductLifecycleState::Deleting, command.now_ms)?;
        state.roots.insert(miniapp_id.clone(), root);
        state
            .deleting_intents
            .insert(miniapp_id.clone(), deletion.intent);
        state.operations.insert(
            deletion.operation.operation_id.clone(),
            deletion.operation,
        );
        state.catalog.remove(&miniapp_id);
        increment_library_revision(&mut state)?;
        let result = snapshot(&state, &miniapp_id)?;
        result.validate()?;
        Ok(result)
    }

    async fn fail_delete(
        &self,
        command: FailPluginRuntimeDelete,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        let mut state = self.write_state()?;
        require_deleting_operation(&state, &command.miniapp_id, &command.operation_id)?;
        let mut operation = state
            .operations
            .get(&command.operation_id)
            .cloned()
            .ok_or_else(|| PluginRuntimePlatformError::UnknownOperation(command.operation_id.0.clone()))?;
        operation.fail(
            command.expected_operation_revision,
            command.now_ms,
            command.error.clone(),
        )?;
        state
            .operations
            .insert(command.operation_id.clone(), operation);
        state
            .deleting_intents
            .get_mut(&command.miniapp_id)
            .expect("deleting operation was checked above")
            .last_error = Some(command.error);
        increment_library_revision(&mut state)?;
        let result = snapshot(&state, &command.miniapp_id)?;
        result.validate()?;
        Ok(result)
    }

    async fn restart_delete(
        &self,
        command: RestartPluginRuntimeDelete,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        let mut state = self.write_state()?;
        require_deleting_operation(
            &state,
            &command.miniapp_id,
            &command.expected_operation_id,
        )?;
        let observed = state
            .operations
            .get(&command.expected_operation_id)
            .expect("deleting operation was checked above");
        if observed.state != PluginRuntimeOperationState::Failed {
            return Err(PluginRuntimePlatformError::OperationConflict);
        }
        if state.operations.contains_key(&command.operation_id) {
            return Err(PluginRuntimePlatformError::AlreadyExists(
                command.operation_id.0,
            ));
        }
        let deletion = new_delete_record(
            command.miniapp_id.clone(),
            command.operation_id,
            command.now_ms,
        )?;
        state
            .deleting_intents
            .insert(command.miniapp_id.clone(), deletion.intent);
        state.operations.insert(
            deletion.operation.operation_id.clone(),
            deletion.operation,
        );
        increment_library_revision(&mut state)?;
        let result = snapshot(&state, &command.miniapp_id)?;
        result.validate()?;
        Ok(result)
    }

    async fn finalize_delete(
        &self,
        command: FinalizePluginRuntimeDelete,
    ) -> PluginRuntimePlatformResult<u64> {
        let mut state = self.write_state()?;
        require_deleting_operation(&state, &command.miniapp_id, &command.operation_id)?;
        let mut operation = state
            .operations
            .get(&command.operation_id)
            .cloned()
            .ok_or_else(|| PluginRuntimePlatformError::UnknownOperation(command.operation_id.0.clone()))?;
        operation.succeed(
            command.expected_operation_revision,
            command.now_ms,
            BTreeMap::new(),
        )?;
        state
            .operations
            .insert(command.operation_id.clone(), operation);
        state.roots.remove(&command.miniapp_id);
        state.catalog.remove(&command.miniapp_id);
        state.deleting_intents.remove(&command.miniapp_id);
        increment_library_revision(&mut state)?;
        Ok(state.library_revision)
    }

    async fn get_operation(
        &self,
        operation_id: &OperationId,
    ) -> PluginRuntimePlatformResult<DurablePluginRuntimeOperation> {
        self.read_state()?
            .operations
            .get(operation_id)
            .cloned()
            .ok_or_else(|| PluginRuntimePlatformError::UnknownOperation(operation_id.0.clone()))
    }

    async fn list_operations(
        &self,
        miniapp_id: &MiniAppId,
    ) -> PluginRuntimePlatformResult<Vec<DurablePluginRuntimeOperation>> {
        Ok(self
            .read_state()?
            .operations
            .values()
            .filter(|operation| &operation.miniapp_id == miniapp_id)
            .cloned()
            .collect())
    }
}

fn snapshot(
    state: &MemoryState,
    miniapp_id: &MiniAppId,
) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
    let root = state
        .roots
        .get(miniapp_id)
        .cloned()
        .ok_or_else(|| PluginRuntimePlatformError::NotFound(miniapp_id.0.clone()))?;
    let deletion = match state.deleting_intents.get(miniapp_id) {
        Some(intent) => {
            let operation = state
                .operations
                .get(&intent.operation_id)
                .cloned()
                .ok_or_else(|| {
                    PluginRuntimePlatformError::InvalidState(
                        "deleting intent references a missing durable operation".into(),
                    )
                })?;
            Some(PluginRuntimeDeletionRecord {
                intent: intent.clone(),
                operation,
            })
        }
        None => None,
    };
    let value = PluginRuntimeRepositorySnapshot {
        library_revision: state.library_revision,
        root,
        catalog: state.catalog.get(miniapp_id).cloned(),
        deletion,
    };
    value.validate()?;
    Ok(value)
}

fn lifecycle_target(
    current: MiniAppProductLifecycleState,
    command: PluginRuntimeLifecycleCommand,
) -> PluginRuntimePlatformResult<MiniAppProductLifecycleState> {
    match (current, command) {
        (MiniAppProductLifecycleState::Disabled, PluginRuntimeLifecycleCommand::Enable) => {
            Ok(MiniAppProductLifecycleState::Enabled)
        }
        (MiniAppProductLifecycleState::Enabled, PluginRuntimeLifecycleCommand::Disable) => {
            Ok(MiniAppProductLifecycleState::Disabled)
        }
        (
            MiniAppProductLifecycleState::Enabled | MiniAppProductLifecycleState::Disabled,
            PluginRuntimeLifecycleCommand::Trash,
        ) => Ok(MiniAppProductLifecycleState::Trashed),
        (MiniAppProductLifecycleState::Trashed, PluginRuntimeLifecycleCommand::Restore) => {
            Ok(MiniAppProductLifecycleState::Disabled)
        }
        _ => Err(PluginRuntimePlatformError::LifecycleConflict(format!(
            "{current:?}"
        ))),
    }
}

fn synchronize_catalog(
    state: &mut MemoryState,
    miniapp_id: &MiniAppId,
) -> PluginRuntimePlatformResult<()> {
    let root = state
        .roots
        .get(miniapp_id)
        .ok_or_else(|| PluginRuntimePlatformError::NotFound(miniapp_id.0.clone()))?;
    if root.product.lifecycle == MiniAppProductLifecycleState::Enabled {
        state
            .catalog
            .insert(miniapp_id.clone(), PluginRuntimeCatalogRecord::for_root(root)?);
    } else {
        state.catalog.remove(miniapp_id);
    }
    Ok(())
}

fn validate_operation_transition(
    observed: &DurablePluginRuntimeOperation,
    expected_revision: u64,
    next: &DurablePluginRuntimeOperation,
) -> PluginRuntimePlatformResult<()> {
    if observed.revision != expected_revision
        || observed.state != PluginRuntimeOperationState::Running
        || observed.operation_id != next.operation_id
        || observed.miniapp_id != next.miniapp_id
        || observed.kind != next.kind
        || observed.cancelable != next.cancelable
        || next.revision != expected_revision.saturating_add(1)
    {
        return Err(PluginRuntimePlatformError::OperationConflict);
    }
    Ok(())
}

fn require_deleting_operation(
    state: &MemoryState,
    miniapp_id: &MiniAppId,
    operation_id: &OperationId,
) -> PluginRuntimePlatformResult<()> {
    let root = state
        .roots
        .get(miniapp_id)
        .ok_or_else(|| PluginRuntimePlatformError::NotFound(miniapp_id.0.clone()))?;
    let intent = state
        .deleting_intents
        .get(miniapp_id)
        .ok_or_else(|| PluginRuntimePlatformError::InvalidState("deleting intent is missing".into()))?;
    if root.product.lifecycle != MiniAppProductLifecycleState::Deleting
        || &intent.operation_id != operation_id
    {
        return Err(PluginRuntimePlatformError::OperationConflict);
    }
    Ok(())
}

fn increment_library_revision(state: &mut MemoryState) -> PluginRuntimePlatformResult<()> {
    state.library_revision = state
        .library_revision
        .checked_add(1)
        .ok_or_else(|| PluginRuntimePlatformError::InvalidState("library revision overflow".into()))?;
    Ok(())
}

fn ensure_owner_idle(
    state: &MemoryState,
    miniapp_id: &MiniAppId,
    allowed_operation: Option<&OperationId>,
) -> PluginRuntimePlatformResult<()> {
    if let Some(operation) = state.operations.values().find(|operation| {
        &operation.miniapp_id == miniapp_id
            && operation.state == PluginRuntimeOperationState::Running
            && allowed_operation != Some(&operation.operation_id)
    }) {
        return Err(PluginRuntimePlatformError::OwnerBusy(
            operation.operation_id.0.clone(),
        ));
    }
    Ok(())
}
