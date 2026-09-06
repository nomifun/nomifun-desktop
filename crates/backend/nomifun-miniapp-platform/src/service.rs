use std::collections::BTreeMap;
use std::sync::Arc;

use nomifun_agent_contracts::{
    CanonicalErrorCode, DigestHex, MiniAppId, MiniAppPointerExpectation,
    MiniAppProductLifecycleState, MiniAppProjectId, MiniAppPublishAuthorization,
    MiniAppPublishRequest, MiniAppRollbackRequest, MiniAppServiceStorageDescriptor,
    MiniAppUiOnlyAutoPublishAuthorization, OperationId, ResolvedMiniAppServiceSpec,
};
use tokio::sync::{Mutex, OwnedMutexGuard};
use uuid::Uuid;

use crate::{
    BeginMiniAppDelete, CommitMiniAppAutoPublish, CommitMiniAppLifecycle, CommitMiniAppRelease,
    CompleteReadyReleaseCommit, CreateMiniAppCommit, DurableMiniAppOperation, FailMiniAppDelete,
    FinalizeMiniAppDelete, MiniAppDataRoot, MiniAppKind, MiniAppLifecycleCommand,
    MiniAppLifecyclePlan, MiniAppManagedDataPort, MiniAppMutationExpectation,
    MiniAppOperationKind, MiniAppPlatformError, MiniAppPlatformResult, MiniAppReleaseCommand,
    MiniAppReleaseCutoverKind, MiniAppReleaseCutoverPlan, MiniAppRepository,
    MiniAppRepositorySnapshot, MiniAppRuntimePort, RestartMiniAppDelete,
};

#[derive(Default)]
pub struct MiniAppOwnerMutationCoordinator {
    locks: Mutex<BTreeMap<MiniAppId, Arc<Mutex<()>>>>,
}

impl MiniAppOwnerMutationCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn acquire(&self, miniapp_id: &MiniAppId) -> OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.locks.lock().await;
            locks
                .entry(miniapp_id.clone())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        lock.lock_owned().await
    }
}

pub struct MiniAppApplicationService {
    repository: Arc<dyn MiniAppRepository>,
    runtime: Arc<dyn MiniAppRuntimePort>,
    managed_data: Arc<dyn MiniAppManagedDataPort>,
    mutations: Arc<MiniAppOwnerMutationCoordinator>,
}

pub struct MiniAppApplicationDependencies {
    pub repository: Arc<dyn MiniAppRepository>,
    pub runtime: Arc<dyn MiniAppRuntimePort>,
    pub managed_data: Arc<dyn MiniAppManagedDataPort>,
    pub mutations: Arc<MiniAppOwnerMutationCoordinator>,
}

#[derive(Clone, Debug)]
pub struct CreateMiniApp {
    pub expected_library_revision: u64,
    pub miniapp_id: MiniAppId,
    pub project_id: MiniAppProjectId,
    pub display_name: String,
    pub description: Option<String>,
    pub kind: MiniAppKind,
    pub storage: MiniAppServiceStorageDescriptor,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct PublishMiniApp {
    pub expected: MiniAppMutationExpectation,
    pub authorization: MiniAppPublishAuthorization,
    pub target_catalog_digest: DigestHex,
    pub current_service_spec: Option<ResolvedMiniAppServiceSpec>,
    pub target_service_spec: Option<ResolvedMiniAppServiceSpec>,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct RollbackMiniApp {
    pub expected: MiniAppMutationExpectation,
    pub actor_id: String,
    pub target_catalog_digest: DigestHex,
    pub current_service_spec: Option<ResolvedMiniAppServiceSpec>,
    pub target_service_spec: Option<ResolvedMiniAppServiceSpec>,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct ChangeMiniAppLifecycle {
    pub expected: MiniAppMutationExpectation,
    pub command: MiniAppLifecycleCommand,
    pub active_service_spec: Option<ResolvedMiniAppServiceSpec>,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct SetMiniAppAutoPublish {
    pub expected: MiniAppMutationExpectation,
    pub authorization: Option<MiniAppUiOnlyAutoPublishAuthorization>,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct BeginPermanentDelete {
    pub expected: MiniAppMutationExpectation,
    pub operation_id: Option<OperationId>,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct RunPermanentDelete {
    pub miniapp_id: MiniAppId,
    pub operation_id: OperationId,
    pub expected_operation_revision: u64,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct RetryPermanentDelete {
    pub miniapp_id: MiniAppId,
    pub failed_operation_id: OperationId,
    pub operation_id: Option<OperationId>,
    pub now_ms: i64,
}

impl MiniAppApplicationService {
    pub fn new(dependencies: MiniAppApplicationDependencies) -> Self {
        Self {
            repository: dependencies.repository,
            runtime: dependencies.runtime,
            managed_data: dependencies.managed_data,
            mutations: dependencies.mutations,
        }
    }

    pub async fn list(&self) -> MiniAppPlatformResult<Vec<MiniAppRepositorySnapshot>> {
        self.repository.list().await
    }

    pub async fn get(
        &self,
        miniapp_id: &MiniAppId,
    ) -> MiniAppPlatformResult<MiniAppRepositorySnapshot> {
        self.repository.get(miniapp_id).await
    }

    pub async fn create(
        &self,
        command: CreateMiniApp,
    ) -> MiniAppPlatformResult<MiniAppRepositorySnapshot> {
        let _guard = self.mutations.acquire(&command.miniapp_id).await;
        let root = MiniAppDataRoot::new(
            command.miniapp_id,
            command.project_id,
            command.display_name,
            command.description,
            command.kind,
            command.storage,
            command.now_ms,
        )?;
        self.repository
            .create(CreateMiniAppCommit {
                expected_library_revision: command.expected_library_revision,
                root,
            })
            .await
    }

    pub async fn start_operation(
        &self,
        miniapp_id: &MiniAppId,
        operation_id: OperationId,
        kind: MiniAppOperationKind,
        now_ms: i64,
    ) -> MiniAppPlatformResult<DurableMiniAppOperation> {
        let _guard = self.mutations.acquire(miniapp_id).await;
        if kind == MiniAppOperationKind::PermanentDelete {
            return Err(MiniAppPlatformError::InvalidState(
                "Permanent Delete must start through begin_delete".into(),
            ));
        }
        self.repository
            .start_operation(DurableMiniAppOperation::running(
                operation_id,
                miniapp_id.clone(),
                kind,
                true,
                now_ms,
            )?)
            .await
    }

    pub async fn complete_ready_release(
        &self,
        commit: CompleteReadyReleaseCommit,
    ) -> MiniAppPlatformResult<MiniAppRepositorySnapshot> {
        let _guard = self.mutations.acquire(&commit.expected.miniapp_id).await;
        self.repository.complete_ready_release(commit).await
    }

    pub async fn publish(
        &self,
        command: PublishMiniApp,
    ) -> MiniAppPlatformResult<MiniAppRepositorySnapshot> {
        let _guard = self.mutations.acquire(&command.expected.miniapp_id).await;
        let current = self.load_expected(&command.expected).await?;
        if let MiniAppPublishAuthorization::AutoUiOnly { authorization, .. } =
            &command.authorization
            && current.root.product.auto_publish.as_ref() != Some(authorization)
        {
            return Err(MiniAppPlatformError::InvalidState(
                "auto Publish requires the exact persisted user authorization".into(),
            ));
        }
        let target = current.root.ready_release()?.clone();
        let request = MiniAppPublishRequest {
            miniapp_id: command.expected.miniapp_id.clone(),
            expected: MiniAppPointerExpectation::from_state(&current.root.product.pointers),
            target_ready_release: target.release_ref().clone(),
            target_catalog_digest: command.target_catalog_digest,
            authorization: command.authorization,
        };
        let repository_command = MiniAppReleaseCommand::Publish(request);
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
        command: SetMiniAppAutoPublish,
    ) -> MiniAppPlatformResult<MiniAppRepositorySnapshot> {
        let _guard = self.mutations.acquire(&command.expected.miniapp_id).await;
        self.load_expected(&command.expected).await?;
        self.repository
            .commit_auto_publish(CommitMiniAppAutoPublish {
                expected: command.expected,
                authorization: command.authorization,
                now_ms: command.now_ms,
            })
            .await
    }

    pub async fn rollback(
        &self,
        command: RollbackMiniApp,
    ) -> MiniAppPlatformResult<MiniAppRepositorySnapshot> {
        let _guard = self.mutations.acquire(&command.expected.miniapp_id).await;
        let current = self.load_expected(&command.expected).await?;
        let target = current.root.previous_release()?.clone();
        let request = MiniAppRollbackRequest {
            miniapp_id: command.expected.miniapp_id.clone(),
            expected: MiniAppPointerExpectation::from_state(&current.root.product.pointers),
            rollback_target: target.release_ref().clone(),
            target_catalog_digest: command.target_catalog_digest,
            actor_id: command.actor_id,
        };
        let repository_command = MiniAppReleaseCommand::Rollback(request);
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
        command: ChangeMiniAppLifecycle,
    ) -> MiniAppPlatformResult<MiniAppRepositorySnapshot> {
        let _guard = self.mutations.acquire(&command.expected.miniapp_id).await;
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
        let plan = MiniAppLifecyclePlan {
            miniapp_id: command.expected.miniapp_id.clone(),
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
            .commit_lifecycle(CommitMiniAppLifecycle {
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
                        MiniAppPlatformError::ReconcileRequired(error.to_string())
                    })?;
                Ok(snapshot)
            }
            Err(error) => {
                self.runtime
                    .abort_lifecycle(ticket, &current)
                    .await
                    .map_err(|recovery| {
                        MiniAppPlatformError::RuntimeRecovery(format!(
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
    ) -> MiniAppPlatformResult<MiniAppRepositorySnapshot> {
        let _guard = self.mutations.acquire(&command.expected.miniapp_id).await;
        let current = self.load_expected(&command.expected).await?;
        if current.root.product.lifecycle != MiniAppProductLifecycleState::Trashed {
            return Err(MiniAppPlatformError::LifecycleConflict(format!(
                "{:?}",
                current.root.product.lifecycle
            )));
        }
        self.runtime.prepare_delete(&current).await?;
        self.repository
            .begin_delete(BeginMiniAppDelete {
                expected: command.expected,
                operation_id: command
                    .operation_id
                    .unwrap_or_else(|| new_operation_id("miniapp-delete")),
                now_ms: command.now_ms,
            })
            .await
    }

    pub async fn run_delete(&self, command: RunPermanentDelete) -> MiniAppPlatformResult<u64> {
        let _guard = self.mutations.acquire(&command.miniapp_id).await;
        self.run_delete_locked(command).await
    }

    pub async fn retry_delete(
        &self,
        command: RetryPermanentDelete,
    ) -> MiniAppPlatformResult<u64> {
        let _guard = self.mutations.acquire(&command.miniapp_id).await;
        let snapshot = self.repository.get(&command.miniapp_id).await?;
        let failed = snapshot
            .deletion
            .as_ref()
            .ok_or_else(|| MiniAppPlatformError::InvalidState("deleting intent is missing".into()))?;
        if failed.operation.operation_id != command.failed_operation_id
            || failed.operation.state != crate::MiniAppOperationState::Failed
        {
            return Err(MiniAppPlatformError::OperationConflict);
        }
        let operation_id = command
            .operation_id
            .unwrap_or_else(|| new_operation_id("miniapp-delete"));
        let restarted = self
            .repository
            .restart_delete(RestartMiniAppDelete {
                miniapp_id: command.miniapp_id.clone(),
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
            miniapp_id: command.miniapp_id,
            operation_id,
            expected_operation_revision: operation.revision,
            now_ms: command.now_ms,
        })
        .await
    }

    async fn execute_release_cutover(
        &self,
        current: MiniAppRepositorySnapshot,
        command: MiniAppReleaseCommand,
        plan: MiniAppReleaseCutoverPlan,
        now_ms: i64,
    ) -> MiniAppPlatformResult<MiniAppRepositorySnapshot> {
        plan.validate()?;
        let ticket = self.runtime.prepare_release_cutover(&plan).await?;
        let committed = self
            .repository
            .commit_release(CommitMiniAppRelease {
                expected: MiniAppMutationExpectation::from_snapshot(&current),
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
                        MiniAppPlatformError::ReconcileRequired(error.to_string())
                    })?;
                Ok(snapshot)
            }
            Err(error) => {
                self.runtime
                    .abort_release_cutover(ticket, &current)
                    .await
                    .map_err(|recovery| {
                        MiniAppPlatformError::RuntimeRecovery(format!(
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
    ) -> MiniAppPlatformResult<u64> {
        let snapshot = self.repository.get(&command.miniapp_id).await?;
        let deletion = snapshot
            .deletion
            .as_ref()
            .ok_or_else(|| MiniAppPlatformError::InvalidState("deleting intent is missing".into()))?;
        if deletion.operation.operation_id != command.operation_id
            || deletion.operation.revision != command.expected_operation_revision
            || deletion.operation.state != crate::MiniAppOperationState::Running
        {
            return Err(MiniAppPlatformError::OperationConflict);
        }

        let cleanup = async {
            self.runtime.prepare_delete(&snapshot).await?;
            self.managed_data.purge_for_delete(&snapshot).await
        }
        .await;
        if let Err(error) = cleanup {
            self.repository
                .fail_delete(FailMiniAppDelete {
                    miniapp_id: command.miniapp_id,
                    operation_id: command.operation_id,
                    expected_operation_revision: command.expected_operation_revision,
                    error: CanonicalErrorCode::from("miniapp_delete_failed"),
                    now_ms: command.now_ms,
                })
                .await
                .map_err(|record_error| {
                    MiniAppPlatformError::RuntimeRecovery(format!(
                        "delete cleanup failed with {error}; recording failure failed with {record_error}"
                    ))
                })?;
            return Err(error);
        }
        self.repository
            .finalize_delete(FinalizeMiniAppDelete {
                miniapp_id: command.miniapp_id,
                operation_id: command.operation_id,
                expected_operation_revision: command.expected_operation_revision,
                now_ms: command.now_ms,
            })
            .await
    }

    async fn load_expected(
        &self,
        expected: &MiniAppMutationExpectation,
    ) -> MiniAppPlatformResult<MiniAppRepositorySnapshot> {
        let snapshot = self.repository.get(&expected.miniapp_id).await?;
        expected.validate(&snapshot)?;
        Ok(snapshot)
    }
}

fn release_cutover_plan(
    snapshot: &MiniAppRepositorySnapshot,
    command: &MiniAppReleaseCommand,
    current_service_spec: Option<ResolvedMiniAppServiceSpec>,
    target_service_spec: Option<ResolvedMiniAppServiceSpec>,
) -> MiniAppPlatformResult<MiniAppReleaseCutoverPlan> {
    let current_release = snapshot
        .root
        .product
        .pointers
        .active_release
        .as_ref()
        .map(|_| snapshot.root.active_release().cloned())
        .transpose()?;
    let (kind, target_release, target_migrations) = match command {
        MiniAppReleaseCommand::Publish(request) => {
            let target = snapshot.root.ready_release()?.clone();
            if target.release_ref() != &request.target_ready_release {
                return Err(MiniAppPlatformError::CompareAndSwapConflict);
            }
            (
                MiniAppReleaseCutoverKind::Publish,
                target.clone(),
                target.artifact.manifest.payload.migrations.clone(),
            )
        }
        MiniAppReleaseCommand::Rollback(request) => {
            let target = snapshot.root.previous_release()?.clone();
            if target.release_ref() != &request.rollback_target {
                return Err(MiniAppPlatformError::CompareAndSwapConflict);
            }
            (MiniAppReleaseCutoverKind::Rollback, target, Vec::new())
        }
    };
    Ok(MiniAppReleaseCutoverPlan {
        kind,
        miniapp_id: snapshot.root.product.miniapp_id.clone(),
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
                MiniAppPlatformError::InvalidState("active Release epoch overflow".into())
            })?,
        current_service_spec,
        target_service_spec,
        target_migrations,
    })
}

fn lifecycle_target(
    current: MiniAppProductLifecycleState,
    command: MiniAppLifecycleCommand,
) -> MiniAppPlatformResult<MiniAppProductLifecycleState> {
    match (current, command) {
        (MiniAppProductLifecycleState::Disabled, MiniAppLifecycleCommand::Enable) => {
            Ok(MiniAppProductLifecycleState::Enabled)
        }
        (MiniAppProductLifecycleState::Enabled, MiniAppLifecycleCommand::Disable) => {
            Ok(MiniAppProductLifecycleState::Disabled)
        }
        (
            MiniAppProductLifecycleState::Enabled | MiniAppProductLifecycleState::Disabled,
            MiniAppLifecycleCommand::Trash,
        ) => Ok(MiniAppProductLifecycleState::Trashed),
        (MiniAppProductLifecycleState::Trashed, MiniAppLifecycleCommand::Restore) => {
            Ok(MiniAppProductLifecycleState::Disabled)
        }
        _ => Err(MiniAppPlatformError::LifecycleConflict(format!(
            "{current:?}"
        ))),
    }
}

fn new_operation_id(prefix: &str) -> OperationId {
    OperationId::from(format!("{prefix}-{}", Uuid::now_v7()))
}
