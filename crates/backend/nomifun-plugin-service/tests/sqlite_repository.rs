use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::PluginHostCommitFence;
use nomifun_api_types::{DurableOperationStateDto, UninstallPluginRequest};
use nomifun_db::{
    ApplyPluginCandidateParams, CreatePluginArtifactParams, CreatePluginProjectParams,
    FinishProductOperationParams, ProductOperationKind, ProductOperationState,
    RecordPluginReadyCandidateParams, StartProductOperationParams, UninstallPluginMountParams,
};
use nomifun_plugin_platform::OwnerMutationCoordinator;
use nomifun_js_authoring::{
    DependencyMutationFacts, DependencyState, DurableDependencyMutation,
    PreparedDependencyMutation,
};
use nomifun_plugin_service::{
    AppliedPluginSource, ApplyPluginSourceEditRequest, CreatedPluginSource,
    DbPluginRepositoryAdapter, ImportedPluginArtifact, PluginApplicationService,
    PluginArtifactStorePort, PluginHostCoordinator, PluginMountDataStore,
    PluginOperationCancellation, PluginRegistryPublisher, PluginRepository,
    PluginServiceDependencies, PluginServiceError, PluginServicePaths,
    PluginSourceStorePort,
    UnconfiguredPluginBuildExecutor, UnconfiguredPluginCandidateTestExecutor, ERR_FORBIDDEN,
    ERR_INTEGRATION, ERR_NOT_FOUND, ERR_RECONCILE_REQUIRED, ERR_STALE,
};
use serde_json::json;
use uuid::Uuid;

fn id() -> String {
    Uuid::now_v7().to_string()
}

fn digest(character: char) -> String {
    character.to_string().repeat(64)
}

struct Fixture {
    _database: nomifun_db::Database,
    owner_user_id: String,
    repository: Arc<DbPluginRepositoryAdapter>,
    project_id: String,
    mount_id: String,
    artifact_id: String,
    artifact_digest: String,
}

async fn active_fixture() -> Fixture {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let owner_user_id = nomifun_db::installation_owner_id(database.pool())
        .await
        .unwrap();
    let repository = Arc::new(DbPluginRepositoryAdapter::new(database.pool().clone()));
    let project_id = id();
    repository
        .create_project(&CreatePluginProjectParams {
            project_id: project_id.clone(),
            owner_user_id: owner_user_id.clone(),
            package_id: "dev.nomifun.sqlite-fixture".into(),
            display_name: "SQLite Fixture".into(),
            description: "SQLite repository test Plugin.".into(),
            managed_source_path: None,
            source_head_digest: None,
            dependency_lock_digest: None,
            initial_build_generation: 0,
            created_at: 10,
        })
        .await
        .unwrap();
    let artifact_id = id();
    let artifact_digest = digest('a');
    repository
        .put_artifact(&CreatePluginArtifactParams {
            artifact_id: artifact_id.clone(),
            artifact_digest: artifact_digest.clone(),
            package_id: "dev.nomifun.sqlite-fixture".into(),
            package_version: "1.0.0".into(),
            manifest_digest: digest('b'),
            manifest: json!({"schema_version": "plugin-package-v1"}),
            managed_path: format!("plugin-artifacts/{artifact_digest}"),
            created_at: 11,
        })
        .await
        .unwrap();
    let origin_operation_id = id();
    repository
        .start_operation(&StartProductOperationParams {
            operation_id: origin_operation_id.clone(),
            kind: ProductOperationKind::Import,
            owner_kind: "plugin_project".into(),
            owner_id: project_id.clone(),
            progress_percent: Some(0),
            bounded_log_tail: vec!["import started".into()],
            started_at_ms: 12,
        })
        .await
        .unwrap();
    repository
        .finish_operation(&FinishProductOperationParams {
            operation_id: origin_operation_id.clone(),
            state: ProductOperationState::Succeeded,
            progress_percent: Some(100),
            last_error_code: None,
            bounded_log_tail: vec!["import succeeded".into()],
            finished_at_ms: 13,
        })
        .await
        .unwrap();
    let candidate_id = id();
    repository
        .record_candidate(&RecordPluginReadyCandidateParams {
            candidate_id: candidate_id.clone(),
            project_id: project_id.clone(),
            candidate_digest: digest('c'),
            origin: nomifun_db::PluginCandidateOrigin::Import,
            artifact_id: artifact_id.clone(),
            artifact_digest: artifact_digest.clone(),
            base_target_digest: None,
            source_snapshot_digest: None,
            dependency_lock_digest: None,
            contract_diff: json!({"compatibility": "compatible", "changes": []}),
            origin_operation_id,
            expected_generation: 0,
            created_at: 14,
        })
        .await
        .unwrap();
    let mount_id = id();
    repository
        .apply_candidate(&ApplyPluginCandidateParams {
            project_id: project_id.clone(),
            candidate_id,
            expected_project_generation: 0,
            expected_mount_revision: Some(0),
            expected_current_artifact_digest: None,
            new_mount_id: Some(mount_id.clone()),
            new_data_dir_path: Some(format!("plugin-mount-data/{mount_id}")),
            config_schema_digest: digest('d'),
            initial_config: json!({}),
            applied_at: 15,
        })
        .await
        .unwrap();

    Fixture {
        _database: database,
        owner_user_id,
        repository,
        project_id,
        mount_id,
        artifact_id,
        artifact_digest,
    }
}

#[derive(Default)]
struct UnavailableArtifactStore;

#[async_trait]
impl PluginArtifactStorePort for UnavailableArtifactStore {
    async fn import_directory(
        &self,
        _source: &Path,
    ) -> Result<ImportedPluginArtifact, PluginServiceError> {
        Err(PluginServiceError::integration(
            "artifact import is outside this test",
        ))
    }

    async fn import_zip(
        &self,
        _source: &Path,
    ) -> Result<ImportedPluginArtifact, PluginServiceError> {
        Err(PluginServiceError::integration(
            "artifact import is outside this test",
        ))
    }

    async fn verify(
        &self,
        _artifact: &nomifun_db::PluginArtifactRow,
    ) -> Result<(), PluginServiceError> {
        Ok(())
    }
}

#[derive(Default)]
struct UnavailableRuntime;

#[async_trait]
impl PluginHostCoordinator for UnavailableRuntime {
    async fn commit_fence(
        &self,
        _mount_id: &str,
    ) -> Result<PluginHostCommitFence, PluginServiceError> {
        Ok(PluginHostCommitFence::NotResident)
    }

}

#[derive(Default)]
struct NoopMountDataStore;

#[async_trait]
impl PluginMountDataStore for NoopMountDataStore {
    async fn delete_mount_data(
        &self,
        _mount_id: &str,
        _managed_relative_path: &str,
    ) -> Result<(), PluginServiceError> {
        Ok(())
    }
}

#[derive(Default)]
struct UnavailableSourceStore;

#[async_trait]
impl PluginSourceStorePort for UnavailableSourceStore {
    async fn create_project(
        &self,
        _owner_user_id: &str,
        _project_id: &str,
        _request: &nomifun_api_types::CreatePluginProjectRequest,
    ) -> Result<CreatedPluginSource, PluginServiceError> {
        Err(PluginServiceError::integration(
            "Source authoring is outside this test",
        ))
    }

    async fn delete_project(
        &self,
        _owner_user_id: &str,
        _project_id: &str,
    ) -> Result<(), PluginServiceError> {
        Ok(())
    }

    async fn apply_source_edit(
        &self,
        _owner_user_id: &str,
        _request: &ApplyPluginSourceEditRequest,
    ) -> Result<AppliedPluginSource, PluginServiceError> {
        Err(PluginServiceError::integration(
            "Source authoring is outside this test",
        ))
    }

    async fn dependency_state(
        &self,
        _owner_user_id: &str,
        _project_id: &str,
    ) -> Result<DependencyState, PluginServiceError> {
        Err(PluginServiceError::integration(
            "Source authoring is outside this test",
        ))
    }

    async fn prepare_dependency_mutation(
        &self,
        _owner_user_id: &str,
        _mutation_id: &str,
        _request: &nomifun_api_types::UpdatePluginDependenciesRequest,
    ) -> Result<PreparedDependencyMutation, PluginServiceError> {
        Err(PluginServiceError::integration(
            "Source authoring is outside this test",
        ))
    }

    async fn commit_dependency_mutation(
        &self,
        _mutation: &DurableDependencyMutation,
    ) -> Result<(), PluginServiceError> {
        Err(PluginServiceError::integration(
            "Source authoring is outside this test",
        ))
    }

    async fn finish_dependency_mutation(
        &self,
        _facts: &DependencyMutationFacts,
    ) -> Result<(), PluginServiceError> {
        Ok(())
    }

    async fn rollback_dependency_mutation(
        &self,
        _facts: &DependencyMutationFacts,
    ) -> Result<(), PluginServiceError> {
        Ok(())
    }

    async fn list_dependency_mutation_journals(
        &self,
    ) -> Result<Vec<DependencyMutationFacts>, PluginServiceError> {
        Ok(Vec::new())
    }

    async fn dependency_mutation_journal(
        &self,
        _owner_user_id: &str,
        _project_id: &str,
    ) -> Result<Option<DependencyMutationFacts>, PluginServiceError> {
        Ok(None)
    }

    async fn cleanup_orphan_dependency_staging(
        &self,
        _retained_mutation_ids: &BTreeSet<String>,
    ) -> Result<(), PluginServiceError> {
        Ok(())
    }
}

#[derive(Default)]
struct NoopOperationCancellation;

#[async_trait]
impl PluginOperationCancellation for NoopOperationCancellation {
    async fn cancel(
        &self,
        _operation: &nomifun_db::ProductOperationRow,
    ) -> Result<(), PluginServiceError> {
        Ok(())
    }
}

#[derive(Default)]
struct UnavailableRegistryPublisher;

#[async_trait]
impl PluginRegistryPublisher for UnavailableRegistryPublisher {
    async fn reconcile_mount(
        &self,
        _owner_user_id: &str,
        _mount: &nomifun_db::PluginMountRow,
    ) -> Result<(), PluginServiceError> {
        Err(PluginServiceError::integration(
            "Kernel publication is outside this test",
        ))
    }
}

fn application_service(
    repository: Arc<DbPluginRepositoryAdapter>,
) -> PluginApplicationService {
    application_service_with_cancellation(repository, Arc::new(NoopOperationCancellation))
}

fn application_service_with_cancellation(
    repository: Arc<DbPluginRepositoryAdapter>,
    operation_cancellation: Arc<dyn PluginOperationCancellation>,
) -> PluginApplicationService {
    PluginApplicationService::new(PluginServiceDependencies {
        repository,
        artifacts: Arc::new(UnavailableArtifactStore),
        host: Arc::new(UnavailableRuntime),
        registry: Arc::new(UnavailableRegistryPublisher),
        mutation_coordinator: Arc::new(OwnerMutationCoordinator::new()),
        builder: Arc::new(UnconfiguredPluginBuildExecutor),
        tester: Arc::new(UnconfiguredPluginCandidateTestExecutor),
        operation_cancellation,
        source_store: Arc::new(UnavailableSourceStore),
        data_store: Arc::new(NoopMountDataStore),
        paths: PluginServicePaths {
            mount_data_relative_root: "plugin-mount-data".into(),
        },
    })
}

#[derive(Default)]
struct FailingOperationCancellation;

#[async_trait]
impl PluginOperationCancellation for FailingOperationCancellation {
    async fn cancel(
        &self,
        _operation: &nomifun_db::ProductOperationRow,
    ) -> Result<(), PluginServiceError> {
        Err(PluginServiceError::integration(
            "operation worker did not stop",
        ))
    }
}

#[tokio::test]
async fn artifact_query_is_exact_and_returns_the_persisted_managed_relative_path() {
    let fixture = active_fixture().await;

    let artifact = fixture
        .repository
        .get_artifact(&fixture.artifact_digest)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(artifact.artifact_id, fixture.artifact_id);
    assert_eq!(
        artifact.managed_path,
        format!("plugin-artifacts/{}", fixture.artifact_digest)
    );
    assert!(
        fixture
            .repository
            .get_artifact(&digest('f'))
            .await
            .unwrap()
            .is_none()
    );

    let inventory = fixture
        .repository
        .inventory(&fixture.owner_user_id)
        .await
        .unwrap();
    assert_eq!(inventory.artifacts, vec![artifact]);
    assert_ne!(inventory.library_revision, 0);
}

#[tokio::test]
async fn application_service_rejects_cross_owner_project_and_mount_mutations() {
    let fixture = active_fixture().await;
    let service = application_service(Arc::clone(&fixture.repository));
    let other_owner = id();

    service
        .get_project(&fixture.owner_user_id, &fixture.project_id)
        .await
        .unwrap();
    let project_error = service
        .get_project(&other_owner, &fixture.project_id)
        .await
        .unwrap_err();
    assert_eq!(project_error.code(), ERR_FORBIDDEN);

    let mount_error = service
        .uninstall(
            &other_owner,
            UninstallPluginRequest {
                mount_id: fixture.mount_id,
                expected_mount_revision: 1,
                expected_current_target_digest: fixture.artifact_digest,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(mount_error.code(), ERR_FORBIDDEN);
}

#[tokio::test]
async fn uninstall_keeps_the_stable_mount_and_linked_project_as_retained_data() {
    let fixture = active_fixture().await;

    let retained = fixture
        .repository
        .uninstall_retain_data(&UninstallPluginMountParams {
            mount_id: fixture.mount_id.clone(),
            expected_revision: 1,
            expected_current_artifact_digest: fixture.artifact_digest,
            uninstalled_at: 16,
        })
        .await
        .unwrap();
    assert!(retained.retained);
    assert!(!retained.enabled);
    assert_eq!(retained.revision, 2);
    assert!(retained.current_artifact_digest.is_none());
    assert!(retained.previous_artifact_digest.is_none());
    assert_eq!(
        retained.data_dir_path,
        format!("plugin-mount-data/{}", fixture.mount_id)
    );

    let project = fixture
        .repository
        .get_project(&fixture.project_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(project.linked_mount_id.as_deref(), Some(fixture.mount_id.as_str()));
    let inventory = fixture
        .repository
        .inventory(&fixture.owner_user_id)
        .await
        .unwrap();
    assert_eq!(inventory.mounts, vec![retained]);
    assert_eq!(inventory.projects, vec![project]);
}

#[tokio::test]
async fn committed_mount_mutation_reports_reconcile_required_without_rolling_back() {
    let fixture = active_fixture().await;
    let service = application_service(Arc::clone(&fixture.repository));

    let error = service
        .uninstall(
            &fixture.owner_user_id,
            UninstallPluginRequest {
                mount_id: fixture.mount_id.clone(),
                expected_mount_revision: 1,
                expected_current_target_digest: fixture.artifact_digest,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), ERR_RECONCILE_REQUIRED);

    let retained = fixture
        .repository
        .get_mount(&fixture.mount_id)
        .await
        .unwrap()
        .unwrap();
    assert!(retained.retained);
    assert!(retained.current_artifact_digest.is_none());
}

#[tokio::test]
async fn operation_cancel_is_owner_scoped_and_uses_terminal_revision_cas() {
    let fixture = active_fixture().await;
    let service = application_service(Arc::clone(&fixture.repository));
    let operation_id = id();
    fixture
        .repository
        .start_operation(&StartProductOperationParams {
            operation_id: operation_id.clone(),
            kind: ProductOperationKind::Export,
            owner_kind: "plugin_project".into(),
            owner_id: fixture.project_id,
            progress_percent: Some(25),
            bounded_log_tail: vec!["export started".into()],
            started_at_ms: 20,
        })
        .await
        .unwrap();
    let other_owner = id();

    assert!(
        fixture
            .repository
            .get_operation(&other_owner, &operation_id)
            .await
            .unwrap()
            .is_none()
    );
    let foreign_error = service
        .cancel_operation(&other_owner, &operation_id, 1)
        .await
        .unwrap_err();
    assert_eq!(foreign_error.code(), ERR_NOT_FOUND);

    let canceled = service
        .cancel_operation(&fixture.owner_user_id, &operation_id, 1)
        .await
        .unwrap();
    assert_eq!(canceled.state, DurableOperationStateDto::Canceled);
    assert_eq!(canceled.operation_revision, 2);
    assert!(!canceled.cancelable);

    let stale = service
        .cancel_operation(&fixture.owner_user_id, &operation_id, 1)
        .await
        .unwrap_err();
    assert_eq!(stale.code(), ERR_STALE);
}

#[tokio::test]
async fn operation_cancel_failure_keeps_the_durable_operation_running() {
    let fixture = active_fixture().await;
    let service = application_service_with_cancellation(
        Arc::clone(&fixture.repository),
        Arc::new(FailingOperationCancellation),
    );
    let operation_id = id();
    fixture
        .repository
        .start_operation(&StartProductOperationParams {
            operation_id: operation_id.clone(),
            kind: ProductOperationKind::Export,
            owner_kind: "plugin_project".into(),
            owner_id: fixture.project_id,
            progress_percent: Some(10),
            bounded_log_tail: vec!["export started".into()],
            started_at_ms: 20,
        })
        .await
        .unwrap();

    let error = service
        .cancel_operation(&fixture.owner_user_id, &operation_id, 1)
        .await
        .unwrap_err();
    assert_eq!(error.code(), ERR_INTEGRATION);
    let operation = fixture
        .repository
        .get_operation(&fixture.owner_user_id, &operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(operation.state, "running");
    assert!(operation.finished_at_ms.is_none());
}
