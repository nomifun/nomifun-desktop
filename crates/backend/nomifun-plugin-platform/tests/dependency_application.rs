use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use nomifun_agent_contracts::PluginHostCommitFence;
use nomifun_api_types::{
    CreatePluginProjectRequest, PluginProjectLanguageDto, UpdatePluginDependenciesRequest,
};
use nomifun_db::{
    BeginPluginDependencyMutationParams, FinalizePluginDependencyMutationParams,
    installation_owner_id,
};
use nomifun_js_authoring::{
    ContentAddressedNpmCache, NormalizedSourcePath, NpmRegistryPort,
    OperationCancellation, RegistryPackageRelease, SourceStoreLimits,
};
use nomifun_plugin_platform::OwnerMutationCoordinator;
use nomifun_plugin_platform::application::{
    CreateProjectInput, DbPluginRepositoryAdapter,
    FsPluginSourceStore, ImportedPluginArtifact, PluginApplicationService,
    PluginArtifactStorePort, PluginHostCoordinator, PluginMountDataStore,
    PluginOperationCancellation, PluginRegistryPublisher, PluginRepository,
    PluginServiceDependencies, PluginServiceError, PluginServicePaths,
    PluginSourceStorePort, UnconfiguredPluginBuildExecutor,
    UnconfiguredPluginCandidateTestExecutor,
};
use uuid::Uuid;

#[derive(Clone)]
struct FakeRegistry;

impl NpmRegistryPort for FakeRegistry {
    fn resolve(
        &self,
        package_name: &str,
        _requirement: &str,
        cancellation: &dyn OperationCancellation,
    ) -> Result<RegistryPackageRelease, nomifun_js_authoring::AuthoringError> {
        if cancellation.is_cancelled() {
            return Err(nomifun_js_authoring::AuthoringError::Canceled);
        }
        let package_json = serde_json::to_vec(&serde_json::json!({
            "name": package_name,
            "version": "1.2.0",
            "type": "module",
            "main": "index.js",
            "dependencies": {},
            "license": "MIT",
            "scripts": {"test": "node test.js"}
        }))
        .unwrap();
        RegistryPackageRelease::new(
            package_name,
            "1.2.0",
            "sha512-YWJjZA==",
            [
                (
                    NormalizedSourcePath::parse("package.json").unwrap(),
                    package_json,
                ),
                (
                    NormalizedSourcePath::parse("index.js").unwrap(),
                    b"export default 1;\n".to_vec(),
                ),
            ],
        )
    }
}

#[derive(Clone)]
struct CancelAwareBlockingRegistry {
    started: Arc<AtomicBool>,
    canceled: Arc<AtomicBool>,
}

impl NpmRegistryPort for CancelAwareBlockingRegistry {
    fn resolve(
        &self,
        _package_name: &str,
        _requirement: &str,
        cancellation: &dyn OperationCancellation,
    ) -> Result<RegistryPackageRelease, nomifun_js_authoring::AuthoringError> {
        self.started.store(true, Ordering::Release);
        while !cancellation.is_cancelled() {
            std::thread::sleep(Duration::from_millis(5));
        }
        self.canceled.store(true, Ordering::Release);
        Err(nomifun_js_authoring::AuthoringError::Canceled)
    }
}

#[derive(Default)]
struct UnusedArtifactStore;

#[async_trait]
impl PluginArtifactStorePort for UnusedArtifactStore {
    async fn import_directory(
        &self,
        _source: &Path,
    ) -> Result<ImportedPluginArtifact, PluginServiceError> {
        Err(PluginServiceError::integration("unused artifact store"))
    }

    async fn import_zip(
        &self,
        _source: &Path,
    ) -> Result<ImportedPluginArtifact, PluginServiceError> {
        Err(PluginServiceError::integration("unused artifact store"))
    }

    async fn verify(
        &self,
        _artifact: &nomifun_db::PluginArtifactRow,
    ) -> Result<(), PluginServiceError> {
        Ok(())
    }

    async fn load_for_share(
        &self,
        _artifact: &nomifun_db::PluginArtifactRow,
    ) -> Result<nomifun_plugin_platform::StoredPluginArtifact, PluginServiceError> {
        Err(PluginServiceError::integration("unused artifact store"))
    }
}

#[derive(Default)]
struct NoopHost;

#[async_trait]
impl PluginHostCoordinator for NoopHost {
    async fn commit_fence(
        &self,
        _mount_id: &str,
    ) -> Result<PluginHostCommitFence, PluginServiceError> {
        Ok(PluginHostCommitFence::NotResident)
    }
}

#[derive(Default)]
struct NoopRegistryPublisher;

#[async_trait]
impl PluginRegistryPublisher for NoopRegistryPublisher {
    async fn reconcile_mount(
        &self,
        _owner_user_id: &str,
        _mount: &nomifun_db::PluginMountRow,
    ) -> Result<(), PluginServiceError> {
        Ok(())
    }
}

#[derive(Default)]
struct NoopDataStore;

#[async_trait]
impl PluginMountDataStore for NoopDataStore {
    async fn delete_mount_data(
        &self,
        _mount_id: &str,
        _managed_relative_path: &str,
    ) -> Result<(), PluginServiceError> {
        Ok(())
    }
}

#[derive(Default)]
struct NoopCancellation;

#[async_trait]
impl PluginOperationCancellation for NoopCancellation {
    async fn cancel(
        &self,
        _operation: &nomifun_db::ProductOperationRow,
    ) -> Result<(), PluginServiceError> {
        Ok(())
    }
}

struct Fixture {
    _database: nomifun_db::Database,
    owner_user_id: String,
    repository: Arc<DbPluginRepositoryAdapter>,
    source_store: Arc<FsPluginSourceStore>,
    service: PluginApplicationService,
    _temp: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    fixture_with_registry(Arc::new(FakeRegistry)).await
}

async fn fixture_with_registry(registry: Arc<dyn NpmRegistryPort>) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let owner_user_id = installation_owner_id(database.pool()).await.unwrap();
    let repository = Arc::new(DbPluginRepositoryAdapter::new(database.pool().clone()));
    let source_store = Arc::new(
        FsPluginSourceStore::new(
            temp.path().join("authoring"),
            SourceStoreLimits::default(),
        )
        .unwrap()
        .with_npm_registry(
            registry,
            ContentAddressedNpmCache::new(temp.path().join("npm-cache")).unwrap(),
        ),
    );
    let service = PluginApplicationService::new(PluginServiceDependencies {
        repository: repository.clone(),
        artifacts: Arc::new(UnusedArtifactStore),
        host: Arc::new(NoopHost),
        registry: Arc::new(NoopRegistryPublisher),
        mutation_coordinator: Arc::new(OwnerMutationCoordinator::new()),
        builder: Arc::new(UnconfiguredPluginBuildExecutor),
        tester: Arc::new(UnconfiguredPluginCandidateTestExecutor),
        operation_cancellation: Arc::new(NoopCancellation),
        source_store: source_store.clone(),
        data_store: Arc::new(NoopDataStore),
        paths: PluginServicePaths {
            mount_data_relative_root: "plugin-mount-data".into(),
        },
    });
    Fixture {
        _database: database,
        owner_user_id,
        repository,
        source_store,
        service,
        _temp: temp,
    }
}

async fn create_project(fixture: &Fixture) -> nomifun_api_types::PluginProjectDetailDto {
    fixture
        .service
        .create_project(CreateProjectInput {
            owner_user_id: fixture.owner_user_id.clone(),
            request: CreatePluginProjectRequest {
                expected_library_revision: 0,
                package_id: "example.dependency-application".into(),
                package_version: "1.0.0".into(),
                display_name: "Dependency Application".into(),
                description: "Durable dependency mutation fixture.".into(),
                language: PluginProjectLanguageDto::JavaScript,
                linked_mount_id: None,
                expected_linked_mount_revision: None,
                expected_linked_target_digest: None,
            },
        })
        .await
        .unwrap()
}

fn update_request(
    project: &nomifun_api_types::PluginProjectDetailDto,
) -> UpdatePluginDependenciesRequest {
    UpdatePluginDependenciesRequest {
        project_id: project.summary.project_id.clone(),
        expected_project_revision: project.summary.project_revision,
        expected_build_generation: project.summary.build_generation,
        expected_source_snapshot_digest: project.source_snapshot_digest.clone().unwrap(),
        expected_dependency_lock_digest: project.dependency_lock_digest.clone().unwrap(),
        dependencies: BTreeMap::from([("alpha".into(), "^1.0.0".into())]),
    }
}

#[tokio::test]
async fn dependency_application_commits_source_lock_and_sqlite_generation() {
    let fixture = fixture().await;
    let project = create_project(&fixture).await;
    let updated = fixture
        .service
        .update_dependencies(&fixture.owner_user_id, update_request(&project))
        .await
        .unwrap();

    assert_eq!(updated.summary.build_generation, project.summary.build_generation + 1);
    assert_eq!(updated.direct_dependencies.get("alpha"), Some(&"^1.0.0".into()));
    assert_ne!(updated.source_snapshot_digest, project.source_snapshot_digest);
    assert_ne!(updated.dependency_lock_digest, project.dependency_lock_digest);
    assert!(
        fixture
            .repository
            .list_dependency_mutation_intents()
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .source_store
            .list_dependency_mutation_journals()
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        std::fs::read_dir(fixture.source_store.store().managed_root().join(".staging"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn startup_recovery_rolls_back_files_when_db_finalize_never_ran() {
    let fixture = fixture().await;
    let project = create_project(&fixture).await;
    let request = update_request(&project);
    let mutation_id = Uuid::now_v7().to_string();
    let prepared = fixture
        .source_store
        .prepare_dependency_mutation(&fixture.owner_user_id, &mutation_id, &request)
        .await
        .unwrap();
    let facts = prepared.facts().clone();
    fixture
        .repository
        .begin_dependency_mutation(&BeginPluginDependencyMutationParams {
            intent_id: mutation_id,
            project_id: request.project_id.clone(),
            owner_user_id: fixture.owner_user_id.clone(),
            expected_project_updated_at: request.expected_project_revision as i64,
            expected_build_generation: request.expected_build_generation as i64,
            expected_source_digest: request.expected_source_snapshot_digest.clone(),
            expected_lock_digest: request.expected_dependency_lock_digest.clone(),
            next_source_digest: facts.next_source_digest().as_ref().to_owned(),
            next_lock_digest: facts.next_lock_digest().as_ref().to_owned(),
            created_at: 1,
        })
        .await
        .unwrap();
    let durable = prepared.persist();
    fixture
        .source_store
        .commit_dependency_mutation(&durable)
        .await
        .unwrap();

    fixture.service.reconcile_dependency_mutations().await.unwrap();
    let recovered = fixture
        .service
        .get_project(&fixture.owner_user_id, &request.project_id)
        .await
        .unwrap();
    assert_eq!(recovered.source_snapshot_digest, project.source_snapshot_digest);
    assert_eq!(recovered.dependency_lock_digest, project.dependency_lock_digest);
    assert!(recovered.direct_dependencies.is_empty());
}

#[tokio::test]
async fn startup_recovery_finishes_journal_after_db_finalize() {
    let fixture = fixture().await;
    let project = create_project(&fixture).await;
    let request = update_request(&project);
    let mutation_id = Uuid::now_v7().to_string();
    let prepared = fixture
        .source_store
        .prepare_dependency_mutation(&fixture.owner_user_id, &mutation_id, &request)
        .await
        .unwrap();
    let facts = prepared.facts().clone();
    fixture
        .repository
        .begin_dependency_mutation(&BeginPluginDependencyMutationParams {
            intent_id: mutation_id.clone(),
            project_id: request.project_id.clone(),
            owner_user_id: fixture.owner_user_id.clone(),
            expected_project_updated_at: request.expected_project_revision as i64,
            expected_build_generation: request.expected_build_generation as i64,
            expected_source_digest: request.expected_source_snapshot_digest,
            expected_lock_digest: request.expected_dependency_lock_digest,
            next_source_digest: facts.next_source_digest().as_ref().to_owned(),
            next_lock_digest: facts.next_lock_digest().as_ref().to_owned(),
            created_at: 1,
        })
        .await
        .unwrap();
    let durable = prepared.persist();
    fixture
        .source_store
        .commit_dependency_mutation(&durable)
        .await
        .unwrap();
    fixture
        .repository
        .finalize_dependency_mutation(&FinalizePluginDependencyMutationParams {
            intent_id: mutation_id,
            project_id: request.project_id.clone(),
            owner_user_id: fixture.owner_user_id.clone(),
            updated_at: project.summary.project_revision as i64 + 1,
        })
        .await
        .unwrap();

    fixture.service.reconcile_dependency_mutations().await.unwrap();
    let recovered = fixture
        .service
        .get_project(&fixture.owner_user_id, &request.project_id)
        .await
        .unwrap();
    assert_eq!(recovered.summary.build_generation, project.summary.build_generation + 1);
    assert_eq!(recovered.direct_dependencies.get("alpha"), Some(&"^1.0.0".into()));
    assert!(
        fixture
            .source_store
            .list_dependency_mutation_journals()
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        std::fs::read_dir(fixture.source_store.store().managed_root().join(".staging"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn canceled_request_stops_resolution_before_intent_or_source_commit() {
    let started = Arc::new(AtomicBool::new(false));
    let canceled = Arc::new(AtomicBool::new(false));
    let fixture = fixture_with_registry(Arc::new(CancelAwareBlockingRegistry {
        started: Arc::clone(&started),
        canceled: Arc::clone(&canceled),
    }))
    .await;
    let project = create_project(&fixture).await;
    let request = update_request(&project);
    let source_store = Arc::clone(&fixture.source_store);
    let owner_user_id = fixture.owner_user_id.clone();
    let mutation_id = Uuid::now_v7().to_string();
    let task = tokio::spawn(async move {
        source_store
            .prepare_dependency_mutation(
                &owner_user_id,
                &mutation_id,
                &request,
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !started.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while !canceled.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();

    assert!(
        fixture
            .repository
            .list_dependency_mutation_intents()
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .source_store
            .list_dependency_mutation_journals()
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        std::fs::read_dir(fixture.source_store.store().managed_root().join(".staging"))
            .unwrap()
            .count(),
        0
    );
}
