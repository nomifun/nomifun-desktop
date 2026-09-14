use std::collections::BTreeMap;
use std::sync::Arc;

use nomifun_agent_contracts::{
    CanonicalErrorCode, DigestHex, PluginProductId, PluginPointerExpectation,
    PluginProductLifecycleState, PluginProjectId, PluginPublishAuthorization,
    PluginPublishRequest, PluginRollbackRequest, PluginServiceStorageDescriptor,
    PluginUiOnlyAutoPublishAuthorization, OperationId, ResolvedPluginServiceSpec,
};
use tokio::sync::{Mutex, OwnedMutexGuard};
use uuid::Uuid;

use crate::runtime::{
    BeginPluginRuntimeDelete, CommitPluginRuntimeAutoPublish, CommitPluginRuntimeLifecycle, CommitPluginRuntimeRelease,
    CompleteReadyReleaseCommit, CreatePluginRuntimeCommit, DurablePluginRuntimeOperation, FailPluginRuntimeDelete,
    FinalizePluginRuntimeDelete, PluginRuntimeDataRoot, PluginRuntimeKind, PluginRuntimeLifecycleCommand,
    PluginRuntimeLifecyclePlan, PluginRuntimeManagedDataPort, PluginRuntimeMutationExpectation,
    PluginRuntimeOperationKind, PluginRuntimePlatformError, PluginRuntimePlatformResult, PluginRuntimeReleaseCommand,
    PluginRuntimeReleaseCutoverKind, PluginRuntimeReleaseCutoverPlan, PluginRuntimeRepository,
    PluginRuntimeRepositorySnapshot, PluginRuntimeRuntimePort, RestartPluginRuntimeDelete,
};

#[derive(Default)]
pub struct PluginRuntimeOwnerMutationCoordinator {
    locks: Mutex<BTreeMap<PluginProductId, Arc<Mutex<()>>>>,
}

impl PluginRuntimeOwnerMutationCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn acquire(&self, plugin_product_id: &PluginProductId) -> OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.locks.lock().await;
            locks
                .entry(plugin_product_id.clone())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        lock.lock_owned().await
    }
}

pub struct PluginRuntimeMutationService {
    repository: Arc<dyn PluginRuntimeRepository>,
    runtime: Arc<dyn PluginRuntimeRuntimePort>,
    managed_data: Arc<dyn PluginRuntimeManagedDataPort>,
    mutations: Arc<PluginRuntimeOwnerMutationCoordinator>,
}

pub struct PluginRuntimeApplicationDependencies {
    pub repository: Arc<dyn PluginRuntimeRepository>,
    pub runtime: Arc<dyn PluginRuntimeRuntimePort>,
    pub managed_data: Arc<dyn PluginRuntimeManagedDataPort>,
    pub mutations: Arc<PluginRuntimeOwnerMutationCoordinator>,
}

#[derive(Clone, Debug)]
pub struct CreatePluginRuntime {
    pub expected_library_revision: u64,
    pub plugin_product_id: PluginProductId,
    pub project_id: PluginProjectId,
    pub display_name: String,
    pub description: Option<String>,
    pub kind: PluginRuntimeKind,
    pub storage: PluginServiceStorageDescriptor,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct PublishPluginRuntime {
    pub expected: PluginRuntimeMutationExpectation,
    pub authorization: PluginPublishAuthorization,
    pub target_catalog_digest: DigestHex,
    pub current_service_spec: Option<ResolvedPluginServiceSpec>,
    pub target_service_spec: Option<ResolvedPluginServiceSpec>,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct RollbackPluginRuntime {
    pub expected: PluginRuntimeMutationExpectation,
    pub actor_id: String,
    pub target_catalog_digest: DigestHex,
    pub current_service_spec: Option<ResolvedPluginServiceSpec>,
    pub target_service_spec: Option<ResolvedPluginServiceSpec>,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct ChangePluginRuntimeLifecycle {
    pub expected: PluginRuntimeMutationExpectation,
    pub command: PluginRuntimeLifecycleCommand,
    pub active_service_spec: Option<ResolvedPluginServiceSpec>,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct SetPluginRuntimeAutoPublish {
    pub expected: PluginRuntimeMutationExpectation,
    pub authorization: Option<PluginUiOnlyAutoPublishAuthorization>,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct BeginPermanentDelete {
    pub expected: PluginRuntimeMutationExpectation,
    pub operation_id: Option<OperationId>,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct RunPermanentDelete {
    pub plugin_product_id: PluginProductId,
    pub operation_id: OperationId,
    pub expected_operation_revision: u64,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct RetryPermanentDelete {
    pub plugin_product_id: PluginProductId,
    pub failed_operation_id: OperationId,
    pub operation_id: Option<OperationId>,
    pub now_ms: i64,
}

impl PluginRuntimeMutationService {
    pub fn new(dependencies: PluginRuntimeApplicationDependencies) -> Self {
        Self {
            repository: dependencies.repository,
            runtime: dependencies.runtime,
            managed_data: dependencies.managed_data,
            mutations: dependencies.mutations,
        }
    }

    pub async fn list(&self) -> PluginRuntimePlatformResult<Vec<PluginRuntimeRepositorySnapshot>> {
        self.repository.list().await
    }

    pub async fn get(
        &self,
        plugin_product_id: &PluginProductId,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        self.repository.get(plugin_product_id).await
    }

    pub async fn create(
        &self,
        command: CreatePluginRuntime,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        let _guard = self.mutations.acquire(&command.plugin_product_id).await;
        let root = PluginRuntimeDataRoot::new(
            command.plugin_product_id,
            command.project_id,
            command.display_name,
            command.description,
            command.kind,
            command.storage,
            command.now_ms,
        )?;
        self.repository
            .create(CreatePluginRuntimeCommit {
                expected_library_revision: command.expected_library_revision,
                root,
            })
            .await
    }

    pub async fn start_operation(
        &self,
        plugin_product_id: &PluginProductId,
        operation_id: OperationId,
        kind: PluginRuntimeOperationKind,
        now_ms: i64,
    ) -> PluginRuntimePlatformResult<DurablePluginRuntimeOperation> {
        let _guard = self.mutations.acquire(plugin_product_id).await;
        if kind == PluginRuntimeOperationKind::PermanentDelete {
            return Err(PluginRuntimePlatformError::InvalidState(
                "Permanent Delete must start through begin_delete".into(),
            ));
        }
        self.repository
            .start_operation(DurablePluginRuntimeOperation::running(
                operation_id,
                plugin_product_id.clone(),
                kind,
                true,
                now_ms,
            )?)
            .await
    }

    pub async fn complete_ready_release(
        &self,
        commit: CompleteReadyReleaseCommit,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        let _guard = self.mutations.acquire(&commit.expected.plugin_product_id).await;
        self.repository.complete_ready_release(commit).await
    }

    pub async fn publish(
        &self,
        command: PublishPluginRuntime,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        let _guard = self.mutations.acquire(&command.expected.plugin_product_id).await;
        let current = self.load_expected(&command.expected).await?;
        if let PluginPublishAuthorization::AutoUiOnly { authorization, .. } =
            &command.authorization
            && current.root.product.auto_publish.as_ref() != Some(authorization)
        {
            return Err(PluginRuntimePlatformError::InvalidState(
                "auto Publish requires the exact persisted user authorization".into(),
            ));
        }
        let target = current.root.ready_release()?.clone();
        let request = PluginPublishRequest {
            plugin_product_id: command.expected.plugin_product_id.clone(),
            expected: PluginPointerExpectation::from_state(&current.root.product.pointers),
            target_ready_release: target.release_ref().clone(),
            target_catalog_digest: command.target_catalog_digest,
            authorization: command.authorization,
        };
        let repository_command = PluginRuntimeReleaseCommand::Publish(request);
        let plan = release_cutover_plan(
            &current,
            &repository_command,
            command.current_service_spec,
            command.target_service_spec,
        )?;
        self.execute_release_cutover(current, repository_command, plan, command.now_ms)
            .await
    }

    pub async fn set_auto_publish(
        &self,
        command: SetPluginRuntimeAutoPublish,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        let _guard = self.mutations.acquire(&command.expected.plugin_product_id).await;
        self.load_expected(&command.expected).await?;
        self.repository
            .commit_auto_publish(CommitPluginRuntimeAutoPublish {
                expected: command.expected,
                authorization: command.authorization,
                now_ms: command.now_ms,
            })
            .await
    }

    pub async fn rollback(
        &self,
        command: RollbackPluginRuntime,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        let _guard = self.mutations.acquire(&command.expected.plugin_product_id).await;
        let current = self.load_expected(&command.expected).await?;
        let target = current.root.previous_release()?.clone();
        let request = PluginRollbackRequest {
            plugin_product_id: command.expected.plugin_product_id.clone(),
            expected: PluginPointerExpectation::from_state(&current.root.product.pointers),
            rollback_target: target.release_ref().clone(),
            target_catalog_digest: command.target_catalog_digest,
            actor_id: command.actor_id,
        };
        let repository_command = PluginRuntimeReleaseCommand::Rollback(request);
        let plan = release_cutover_plan(
            &current,
            &repository_command,
            command.current_service_spec,
            command.target_service_spec,
        )?;
        self.execute_release_cutover(current, repository_command, plan, command.now_ms)
            .await
    }

    pub async fn change_lifecycle(
        &self,
        command: ChangePluginRuntimeLifecycle,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        let _guard = self.mutations.acquire(&command.expected.plugin_product_id).await;
        let current = self.load_expected(&command.expected).await?;
        let target = lifecycle_target(current.root.product.lifecycle, command.command)?;
        let active_release = current
            .root
            .product
            .pointers
            .active_release
            .as_ref()
            .map(|_| current.root.active_release().cloned())
            .transpose()?;
        let plan = PluginRuntimeLifecyclePlan {
            plugin_product_id: command.expected.plugin_product_id.clone(),
            from: current.root.product.lifecycle,
            to: target,
            active_release_epoch: current.root.product.pointers.active_release_epoch,
            active_release,
            active_service_spec: command.active_service_spec,
        };
        plan.validate()?;
        let ticket = self.runtime.prepare_lifecycle(&plan).await?;
        let committed = self
            .repository
            .commit_lifecycle(CommitPluginRuntimeLifecycle {
                expected: command.expected,
                command: command.command,
                now_ms: command.now_ms,
            })
            .await;
        match committed {
            Ok(snapshot) => {
                self.runtime
                    .complete_lifecycle(ticket, &snapshot)
                    .await
                    .map_err(|error| {
                        PluginRuntimePlatformError::ReconcileRequired(error.to_string())
                    })?;
                Ok(snapshot)
            }
            Err(error) => {
                self.runtime
                    .abort_lifecycle(ticket, &current)
                    .await
                    .map_err(|recovery| {
                        PluginRuntimePlatformError::RuntimeRecovery(format!(
                            "commit failed with {error}; runtime recovery failed with {recovery}"
                        ))
                    })?;
                Err(error)
            }
        }
    }

    pub async fn begin_delete(
        &self,
        command: BeginPermanentDelete,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        let _guard = self.mutations.acquire(&command.expected.plugin_product_id).await;
        let current = self.load_expected(&command.expected).await?;
        if current.root.product.lifecycle != PluginProductLifecycleState::Trashed {
            return Err(PluginRuntimePlatformError::LifecycleConflict(format!(
                "{:?}",
                current.root.product.lifecycle
            )));
        }
        self.runtime.prepare_delete(&current).await?;
        self.repository
            .begin_delete(BeginPluginRuntimeDelete {
                expected: command.expected,
                operation_id: command
                    .operation_id
                    .unwrap_or_else(|| new_operation_id("plugin-delete")),
                now_ms: command.now_ms,
            })
            .await
    }

    pub async fn run_delete(&self, command: RunPermanentDelete) -> PluginRuntimePlatformResult<u64> {
        let _guard = self.mutations.acquire(&command.plugin_product_id).await;
        self.run_delete_locked(command).await
    }

    pub async fn retry_delete(
        &self,
        command: RetryPermanentDelete,
    ) -> PluginRuntimePlatformResult<u64> {
        let _guard = self.mutations.acquire(&command.plugin_product_id).await;
        let snapshot = self.repository.get(&command.plugin_product_id).await?;
        let failed = snapshot
            .deletion
            .as_ref()
            .ok_or_else(|| PluginRuntimePlatformError::InvalidState("deleting intent is missing".into()))?;
        if failed.operation.operation_id != command.failed_operation_id
            || failed.operation.state != crate::runtime::PluginRuntimeOperationState::Failed
        {
            return Err(PluginRuntimePlatformError::OperationConflict);
        }
        let operation_id = command
            .operation_id
            .unwrap_or_else(|| new_operation_id("plugin-delete"));
        let restarted = self
            .repository
            .restart_delete(RestartPluginRuntimeDelete {
                plugin_product_id: command.plugin_product_id.clone(),
                expected_operation_id: command.failed_operation_id,
                operation_id: operation_id.clone(),
                now_ms: command.now_ms,
            })
            .await?;
        let operation = &restarted
            .deletion
            .as_ref()
            .expect("restart_delete returns a deleting snapshot")
            .operation;
        self.run_delete_locked(RunPermanentDelete {
            plugin_product_id: command.plugin_product_id,
            operation_id,
            expected_operation_revision: operation.revision,
            now_ms: command.now_ms,
        })
        .await
    }

    async fn execute_release_cutover(
        &self,
        current: PluginRuntimeRepositorySnapshot,
        command: PluginRuntimeReleaseCommand,
        plan: PluginRuntimeReleaseCutoverPlan,
        now_ms: i64,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        plan.validate()?;
        let ticket = self.runtime.prepare_release_cutover(&plan).await?;
        let committed = self
            .repository
            .commit_release(CommitPluginRuntimeRelease {
                expected: PluginRuntimeMutationExpectation::from_snapshot(&current),
                command,
                now_ms,
            })
            .await;
        match committed {
            Ok(snapshot) => {
                self.runtime
                    .complete_release_cutover(ticket, &snapshot)
                    .await
                    .map_err(|error| {
                        PluginRuntimePlatformError::ReconcileRequired(error.to_string())
                    })?;
                Ok(snapshot)
            }
            Err(error) => {
                self.runtime
                    .abort_release_cutover(ticket, &current)
                    .await
                    .map_err(|recovery| {
                        PluginRuntimePlatformError::RuntimeRecovery(format!(
                            "commit failed with {error}; runtime recovery failed with {recovery}"
                        ))
                    })?;
                Err(error)
            }
        }
    }

    async fn run_delete_locked(
        &self,
        command: RunPermanentDelete,
    ) -> PluginRuntimePlatformResult<u64> {
        let snapshot = self.repository.get(&command.plugin_product_id).await?;
        let deletion = snapshot
            .deletion
            .as_ref()
            .ok_or_else(|| PluginRuntimePlatformError::InvalidState("deleting intent is missing".into()))?;
        if deletion.operation.operation_id != command.operation_id
            || deletion.operation.revision != command.expected_operation_revision
            || deletion.operation.state != crate::runtime::PluginRuntimeOperationState::Running
        {
            return Err(PluginRuntimePlatformError::OperationConflict);
        }

        let cleanup = async {
            self.runtime.prepare_delete(&snapshot).await?;
            self.managed_data.purge_for_delete(&snapshot).await
        }
        .await;
        if let Err(error) = cleanup {
            self.repository
                .fail_delete(FailPluginRuntimeDelete {
                    plugin_product_id: command.plugin_product_id,
                    operation_id: command.operation_id,
                    expected_operation_revision: command.expected_operation_revision,
                    error: CanonicalErrorCode::from("plugin_delete_failed"),
                    now_ms: command.now_ms,
                })
                .await
                .map_err(|record_error| {
                    PluginRuntimePlatformError::RuntimeRecovery(format!(
                        "delete cleanup failed with {error}; recording failure failed with {record_error}"
                    ))
                })?;
            return Err(error);
        }
        self.repository
            .finalize_delete(FinalizePluginRuntimeDelete {
                plugin_product_id: command.plugin_product_id,
                operation_id: command.operation_id,
                expected_operation_revision: command.expected_operation_revision,
                now_ms: command.now_ms,
            })
            .await
    }

    async fn load_expected(
        &self,
        expected: &PluginRuntimeMutationExpectation,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
        let snapshot = self.repository.get(&expected.plugin_product_id).await?;
        expected.validate(&snapshot)?;
        Ok(snapshot)
    }
}

fn release_cutover_plan(
    snapshot: &PluginRuntimeRepositorySnapshot,
    command: &PluginRuntimeReleaseCommand,
    current_service_spec: Option<ResolvedPluginServiceSpec>,
    target_service_spec: Option<ResolvedPluginServiceSpec>,
) -> PluginRuntimePlatformResult<PluginRuntimeReleaseCutoverPlan> {
    let current_release = snapshot
        .root
        .product
        .pointers
        .active_release
        .as_ref()
        .map(|_| snapshot.root.active_release().cloned())
        .transpose()?;
    let (kind, target_release, target_migrations) = match command {
        PluginRuntimeReleaseCommand::Publish(request) => {
            let target = snapshot.root.ready_release()?.clone();
            if target.release_ref() != &request.target_ready_release {
                return Err(PluginRuntimePlatformError::CompareAndSwapConflict);
            }
            (
                PluginRuntimeReleaseCutoverKind::Publish,
                target.clone(),
                target.artifact.manifest.payload.migrations.clone(),
            )
        }
        PluginRuntimeReleaseCommand::Rollback(request) => {
            let target = snapshot.root.previous_release()?.clone();
            if target.release_ref() != &request.rollback_target {
                return Err(PluginRuntimePlatformError::CompareAndSwapConflict);
            }
            (PluginRuntimeReleaseCutoverKind::Rollback, target, Vec::new())
        }
    };
    Ok(PluginRuntimeReleaseCutoverPlan {
        kind,
        plugin_product_id: snapshot.root.product.plugin_product_id.clone(),
        lifecycle: snapshot.root.product.lifecycle,
        current_release,
        target_release,
        current_active_epoch: snapshot.root.product.pointers.active_release_epoch,
        target_active_epoch: snapshot
            .root
            .product
            .pointers
            .active_release_epoch
            .checked_add(1)
            .ok_or_else(|| {
                PluginRuntimePlatformError::InvalidState("active Release epoch overflow".into())
            })?,
        current_service_spec,
        target_service_spec,
        target_migrations,
    })
}

fn lifecycle_target(
    current: PluginProductLifecycleState,
    command: PluginRuntimeLifecycleCommand,
) -> PluginRuntimePlatformResult<PluginProductLifecycleState> {
    match (current, command) {
        (PluginProductLifecycleState::Disabled, PluginRuntimeLifecycleCommand::Enable) => {
            Ok(PluginProductLifecycleState::Enabled)
        }
        (PluginProductLifecycleState::Enabled, PluginRuntimeLifecycleCommand::Disable) => {
            Ok(PluginProductLifecycleState::Disabled)
        }
        (
            PluginProductLifecycleState::Enabled | PluginProductLifecycleState::Disabled,
            PluginRuntimeLifecycleCommand::Trash,
        ) => Ok(PluginProductLifecycleState::Trashed),
        (PluginProductLifecycleState::Trashed, PluginRuntimeLifecycleCommand::Restore) => {
            Ok(PluginProductLifecycleState::Disabled)
        }
        _ => Err(PluginRuntimePlatformError::LifecycleConflict(format!(
            "{current:?}"
        ))),
    }
}

fn new_operation_id(prefix: &str) -> OperationId {
    OperationId::from(format!("{prefix}-{}", Uuid::now_v7()))
}
