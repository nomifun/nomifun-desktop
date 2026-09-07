use std::collections::BTreeMap;
use std::path::{Component, Path};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    DigestHex, PluginMountId, PluginProjectId, RuntimeTarget, UserId,
};
use nomifun_js_authoring::{
    AuthoringError, ExactDependencyLock, FixedPluginPacker, NeverCancel,
    NpmResolverIdentity, OperationCancellation, PluginLanguage,
    PluginPackageBuildOptions, PluginScaffoldRequest, SourceScope, SourceStore,
    SourceStoreLimits,
};
use nomifun_db::{
    ApplyPluginCandidateParams, CreatePluginArtifactParams, CreatePluginProjectParams,
    DeletePluginProjectParams,
    FinishProductOperationParams, IPluginN1Repository, ListPluginCredentialBindingsParams,
    PluginArtifactRow, PluginCandidateTestReceiptRow, PluginMountRow, PluginProjectRow,
    PluginReadyCandidateRow, ProductOperationRow, ProductOperationState,
    RecordPluginCandidateTestReceiptParams, RecordPluginReadyCandidateParams,
    ReplacePluginCredentialBindingsParams, RestorePluginMountParams, SqlitePluginN1Repository,
    SqlitePool, StartProductOperationParams, UninstallPluginMountParams,
    UpdatePluginMountConfigParams,
};
use nomifun_js_host::{ExtensionHostSupervisor, JavaScriptHostError};
use nomifun_plugin_platform::{
    ArtifactStoreLimits, ImportCancellation, PluginArtifactStore,
    PluginArtifactStoreError,
};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Notify};

use crate::error::PluginServiceError;
use crate::types::{
    BuildOutput, CandidateTestOutput, CreatedPluginSource, ImportedPluginArtifact,
    LinkPluginProjectParams, PluginInventory,
};

#[async_trait]
pub trait PluginRepository: Send + Sync {
    async fn inventory(&self, owner_user_id: &str) -> Result<PluginInventory, PluginServiceError>;
    async fn get_project(&self, project_id: &str)
        -> Result<Option<PluginProjectRow>, PluginServiceError>;
    async fn get_project_for_mount(
        &self,
        mount_id: &str,
    ) -> Result<Option<PluginProjectRow>, PluginServiceError>;
    async fn mount_owner_user_id(
        &self,
        mount_id: &str,
    ) -> Result<Option<String>, PluginServiceError>;
    async fn get_mount(&self, mount_id: &str)
        -> Result<Option<PluginMountRow>, PluginServiceError>;
    async fn get_candidate(&self, project_id: &str)
        -> Result<Option<PluginReadyCandidateRow>, PluginServiceError>;
    async fn get_artifact(
        &self,
        artifact_digest: &str,
    ) -> Result<Option<PluginArtifactRow>, PluginServiceError>;
    async fn get_test_receipt(
        &self,
        candidate_id: &str,
    ) -> Result<Option<PluginCandidateTestReceiptRow>, PluginServiceError>;
    async fn create_project(
        &self,
        params: &CreatePluginProjectParams,
    ) -> Result<PluginProjectRow, PluginServiceError>;
    async fn delete_project_cas(
        &self,
        params: &DeletePluginProjectParams,
    ) -> Result<bool, PluginServiceError>;
    async fn link_project(
        &self,
        params: &LinkPluginProjectParams,
    ) -> Result<PluginProjectRow, PluginServiceError>;
    async fn put_artifact(
        &self,
        params: &CreatePluginArtifactParams,
    ) -> Result<PluginArtifactRow, PluginServiceError>;
    async fn start_operation(
        &self,
        params: &StartProductOperationParams,
    ) -> Result<ProductOperationRow, PluginServiceError>;
    async fn finish_operation(
        &self,
        params: &FinishProductOperationParams,
    ) -> Result<ProductOperationRow, PluginServiceError>;
    async fn record_candidate(
        &self,
        params: &RecordPluginReadyCandidateParams,
    ) -> Result<PluginReadyCandidateRow, PluginServiceError>;
    async fn record_test_receipt(
        &self,
        params: &RecordPluginCandidateTestReceiptParams,
    ) -> Result<PluginCandidateTestReceiptRow, PluginServiceError>;
    async fn apply_candidate(
        &self,
        params: &ApplyPluginCandidateParams,
    ) -> Result<PluginMountRow, PluginServiceError>;
    async fn restore_previous(
        &self,
        params: &RestorePluginMountParams,
    ) -> Result<PluginMountRow, PluginServiceError>;
    async fn uninstall_retain_data(
        &self,
        params: &UninstallPluginMountParams,
    ) -> Result<PluginMountRow, PluginServiceError>;
    async fn mark_delete_pending(
        &self,
        mount_id: &str,
        expected_revision: i64,
        updated_at: i64,
    ) -> Result<PluginMountRow, PluginServiceError>;
    async fn complete_data_delete(&self, mount_id: &str)
        -> Result<bool, PluginServiceError>;
    async fn update_config(
        &self,
        params: &UpdatePluginMountConfigParams,
    ) -> Result<PluginMountRow, PluginServiceError>;
    async fn replace_credentials(
        &self,
        params: &ReplacePluginCredentialBindingsParams,
    ) -> Result<nomifun_db::PluginCredentialBindingSnapshot, PluginServiceError>;
    async fn list_credentials(
        &self,
        params: &ListPluginCredentialBindingsParams,
    ) -> Result<nomifun_db::PluginCredentialBindingSnapshot, PluginServiceError>;

    async fn set_enabled(
        &self,
        mount_id: &str,
        expected_revision: i64,
        expected_digest: &str,
        enabled: bool,
        updated_at: i64,
    ) -> Result<PluginMountRow, PluginServiceError>;

    async fn retry_mount(
        &self,
        mount_id: &str,
        expected_revision: i64,
        expected_digest: &str,
        updated_at: i64,
    ) -> Result<PluginMountRow, PluginServiceError>;

    async fn list_operations(
        &self,
        owner_user_id: &str,
    ) -> Result<Vec<ProductOperationRow>, PluginServiceError>;

    async fn get_operation(
        &self,
        owner_user_id: &str,
        operation_id: &str,
    ) -> Result<Option<ProductOperationRow>, PluginServiceError>;

    async fn cancel_operation(
        &self,
        owner_user_id: &str,
        operation_id: &str,
        expected_revision: u64,
        finished_at_ms: i64,
    ) -> Result<ProductOperationRow, PluginServiceError>;
}

#[async_trait]
pub trait PluginArtifactStorePort: Send + Sync {
    async fn import_directory(
        &self,
        source: &Path,
    ) -> Result<ImportedPluginArtifact, PluginServiceError>;
    async fn import_zip(&self, source: &Path)
        -> Result<ImportedPluginArtifact, PluginServiceError>;
    async fn verify(&self, artifact: &PluginArtifactRow) -> Result<(), PluginServiceError>;
}

#[async_trait]
pub trait PluginMountDataStore: Send + Sync {
    async fn delete_mount_data(
        &self,
        mount_id: &str,
        managed_relative_path: &str,
    ) -> Result<(), PluginServiceError>;
}

#[async_trait]
pub trait PluginSourceStorePort: Send + Sync {
    async fn create_project(
        &self,
        owner_user_id: &str,
        project_id: &str,
        request: &nomifun_api_types::CreatePluginProjectRequest,
    ) -> Result<CreatedPluginSource, PluginServiceError>;

    async fn delete_project(
        &self,
        owner_user_id: &str,
        project_id: &str,
    ) -> Result<(), PluginServiceError>;
}

pub struct FsPluginArtifactStore {
    store: PluginArtifactStore,
}

pub struct FsPluginMountDataStore {
    root: std::path::PathBuf,
}

pub struct FsPluginSourceStore {
    store: SourceStore,
    resolver: NpmResolverIdentity,
}

pub struct FsPluginBuildExecutor {
    source_store: SourceStore,
    artifact_store: PluginArtifactStore,
    packer: FixedPluginPacker,
    options: PluginPackageBuildOptions,
    active: Mutex<BTreeMap<String, Arc<BuildCancellation>>>,
}

struct BuildCancellation {
    project_id: String,
    canceled: AtomicBool,
    completed: AtomicBool,
    completion: Notify,
}

impl BuildCancellation {
    fn new(project_id: String) -> Self {
        Self {
            project_id,
            canceled: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            completion: Notify::new(),
        }
    }

    fn cancel(&self) {
        self.canceled.store(true, Ordering::Release);
    }

    fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Acquire)
    }

    fn complete(&self) {
        self.completed.store(true, Ordering::Release);
        self.completion.notify_waiters();
    }

    async fn wait_for_completion(&self) -> Result<(), PluginServiceError> {
        tokio::time::timeout(Duration::from_secs(30), async {
            while !self.completed.load(Ordering::Acquire) {
                let notified = self.completion.notified();
                if self.completed.load(Ordering::Acquire) {
                    break;
                }
                notified.await;
            }
        })
        .await
        .map_err(|_| {
            PluginServiceError::integration(
                "Plugin Build cancellation could not prove process and staging cleanup",
            )
        })
    }
}

impl OperationCancellation for BuildCancellation {
    fn is_cancelled(&self) -> bool {
        self.canceled.load(Ordering::Acquire)
    }
}

impl ImportCancellation for BuildCancellation {
    fn is_cancelled(&self) -> bool {
        self.canceled.load(Ordering::Acquire)
    }
}

impl FsPluginSourceStore {
    pub fn new(
        root: impl AsRef<Path>,
        limits: SourceStoreLimits,
    ) -> Result<Self, PluginServiceError> {
        let store = SourceStore::new(root, limits).map_err(|error| {
            PluginServiceError::integration(format!(
                "cannot initialize Plugin Source Store: {error}"
            ))
        })?;
        let resolver = NpmResolverIdentity::new("nomifun-npm", "1.0.0").map_err(|error| {
            PluginServiceError::integration(format!(
                "cannot initialize Plugin dependency resolver identity: {error}"
            ))
        })?;
        Ok(Self { store, resolver })
    }

    pub fn store(&self) -> &SourceStore {
        &self.store
    }
}

impl FsPluginBuildExecutor {
    pub fn new(
        source_store: &FsPluginSourceStore,
        artifact_store: &FsPluginArtifactStore,
        packer: FixedPluginPacker,
        runtime_target: RuntimeTarget,
    ) -> Result<Self, PluginServiceError> {
        let source_root = source_store.store.managed_root();
        let artifact_root = artifact_store.store.managed_root();
        if source_root.starts_with(artifact_root) || artifact_root.starts_with(source_root) {
            return Err(PluginServiceError::integration(
                "Plugin Source and Artifact stores must use disjoint managed roots",
            ));
        }
        Ok(Self {
            source_store: source_store.store.clone(),
            artifact_store: artifact_store.store.clone(),
            packer,
            options: PluginPackageBuildOptions::for_target(runtime_target),
            active: Mutex::new(BTreeMap::new()),
        })
    }
}

#[async_trait]
impl PluginBuildExecutor for FsPluginBuildExecutor {
    async fn build(
        &self,
        operation_id: &str,
        project: &PluginProjectRow,
        request: &nomifun_api_types::BuildPluginProjectRequest,
    ) -> Result<BuildOutput, PluginServiceError> {
        if operation_id.trim().is_empty() {
            return Err(PluginServiceError::invalid(
                "Build operation identity must be non-empty",
            ));
        }
        let cancellation = Arc::new(BuildCancellation::new(project.project_id.clone()));
        {
            let mut active = self.active.lock().await;
            if active.contains_key(operation_id) {
                return Err(PluginServiceError::conflict(
                    "Build operation identity is already active",
                ));
            }
            active.insert(operation_id.to_owned(), Arc::clone(&cancellation));
        }

        let source_store = self.source_store.clone();
        let artifact_store = self.artifact_store.clone();
        let packer = self.packer.clone();
        let options = self.options.clone();
        let project = project.clone();
        let request = request.clone();
        let worker_cancellation = Arc::clone(&cancellation);
        let result = tokio::task::spawn_blocking(move || {
            execute_plugin_build(
                &source_store,
                &artifact_store,
                &packer,
                &options,
                &project,
                &request,
                worker_cancellation.as_ref(),
            )
        })
        .await
        .map_err(|error| {
            PluginServiceError::integration(format!(
                "Plugin Build worker terminated unexpectedly: {error}"
            ))
        });
        let canceled = {
            let mut active = self.active.lock().await;
            let Some(active_build) = active.remove(operation_id) else {
                cancellation.complete();
                return Err(PluginServiceError::integration(
                    "Plugin Build cancellation registry lost the active operation",
                ));
            };
            Arc::ptr_eq(&active_build, &cancellation) && cancellation.is_canceled()
        };
        cancellation.complete();
        match result? {
            Ok(_) if canceled => Err(PluginServiceError::operation_canceled(
                "Plugin Build was canceled",
            )),
            output => output,
        }
    }
}

#[async_trait]
impl PluginOperationCancellation for FsPluginBuildExecutor {
    async fn cancel(&self, operation: &ProductOperationRow) -> Result<(), PluginServiceError> {
        if operation.kind != "build" || operation.owner_kind != "plugin_project" {
            return Err(PluginServiceError::invalid(
                "only active Plugin Project Build operations are cancelable here",
            ));
        }
        let cancellation = {
            let active = self.active.lock().await;
            let cancellation = active.get(&operation.operation_id).ok_or_else(|| {
                PluginServiceError::conflict(
                    "Plugin Build has already crossed its cancellation boundary",
                )
            })?;
            if cancellation.project_id != operation.owner_id {
                return Err(PluginServiceError::stale(
                    "Plugin Build operation owner changed",
                ));
            }
            cancellation.cancel();
            Arc::clone(cancellation)
        };
        cancellation.wait_for_completion().await
    }
}

fn execute_plugin_build(
    source_store: &SourceStore,
    artifact_store: &PluginArtifactStore,
    packer: &FixedPluginPacker,
    options: &PluginPackageBuildOptions,
    project: &PluginProjectRow,
    request: &nomifun_api_types::BuildPluginProjectRequest,
    cancellation: &BuildCancellation,
) -> Result<BuildOutput, PluginServiceError> {
    let scope = SourceScope::new(
        UserId::from(project.owner_user_id.clone()),
        PluginProjectId::from(project.project_id.clone()),
    )
    .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
    let source = source_store
        .snapshot(&scope, cancellation)
        .map_err(map_authoring_build_error)?;
    if source.snapshot().digest().as_ref() != request.expected_source_snapshot_digest {
        return Err(PluginServiceError::stale(
            "Build source snapshot differs from the requested Project head",
        ));
    }
    let dependency_lock = source_store
        .load_dependency_lock(&scope, cancellation)
        .map_err(map_authoring_build_error)?;
    let dependency_lock_digest = dependency_lock
        .digest()
        .map_err(map_authoring_build_error)?;
    if dependency_lock_digest.as_ref() != request.expected_dependency_lock_digest {
        return Err(PluginServiceError::stale(
            "Build dependency lock differs from the requested Project lock",
        ));
    }
    let staged = source_store
        .stage_snapshot(&scope, source.snapshot(), cancellation)
        .map_err(map_authoring_build_error)?;
    let packed = packer
        .pack(staged, &dependency_lock, options, cancellation)
        .map_err(map_authoring_build_error)?;
    let imported = artifact_store
        .import_directory(packed.package_root(), cancellation)
        .map_err(map_artifact_build_error)?;
    Ok(BuildOutput {
        artifact: imported.stored.artifact,
        managed_relative_path: imported.stored.managed_relative_path,
        source_snapshot_digest: source.snapshot().digest().as_ref().to_owned(),
        dependency_lock_digest: dependency_lock_digest.as_ref().to_owned(),
    })
}

fn map_authoring_build_error(error: AuthoringError) -> PluginServiceError {
    match error {
        AuthoringError::Canceled => {
            PluginServiceError::operation_canceled("Plugin Build was canceled")
        }
        AuthoringError::SourceChanged { .. }
        | AuthoringError::ProjectNotFound
        | AuthoringError::ScopeMismatch => PluginServiceError::stale(error.to_string()),
        AuthoringError::BuildHostUnavailable(_)
        | AuthoringError::BuildHostTimeout(_)
        | AuthoringError::BuildHostFailed { .. } => PluginServiceError::Coded {
            code: crate::ERR_RUNTIME,
            message: error.to_string(),
        },
        _ => PluginServiceError::Coded {
            code: crate::ERR_OPERATION,
            message: error.to_string(),
        },
    }
}

fn map_artifact_build_error(error: PluginArtifactStoreError) -> PluginServiceError {
    match error {
        PluginArtifactStoreError::Canceled => {
            PluginServiceError::operation_canceled("Plugin Build was canceled")
        }
        other => other.into(),
    }
}

#[async_trait]
impl PluginSourceStorePort for FsPluginSourceStore {
    async fn create_project(
        &self,
        owner_user_id: &str,
        project_id: &str,
        request: &nomifun_api_types::CreatePluginProjectRequest,
    ) -> Result<CreatedPluginSource, PluginServiceError> {
        let language = match request.language {
            nomifun_api_types::PluginProjectLanguageDto::JavaScript => {
                PluginLanguage::JavaScript
            }
            nomifun_api_types::PluginProjectLanguageDto::TypeScript => {
                PluginLanguage::TypeScript
            }
        };
        let scope = SourceScope::new(
            UserId::from(owner_user_id.to_owned()),
            PluginProjectId::from(project_id.to_owned()),
        )
        .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
        let scaffold = self
            .store
            .create_plugin_project(
                scope.clone(),
                &PluginScaffoldRequest {
                    package_id: request.package_id.clone(),
                    package_version: request.package_version.clone(),
                    display_name: request.display_name.clone(),
                    description: request.description.clone(),
                    language,
                },
                &NeverCancel,
            )
            .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
        let capture = scaffold.capture();
        let prepared = (|| {
            if !capture.dependency_requests().dependencies().is_empty() {
                return Err(PluginServiceError::integration(
                    "initial Plugin scaffold cannot contain dependencies before the N1 resolver is wired",
                ));
            }
            let lock = ExactDependencyLock::empty(
                capture.dependency_requests(),
                self.resolver.clone(),
            )
            .map_err(|error| PluginServiceError::integration(error.to_string()))?;
            let lock_digest = self
                .store
                .write_initial_dependency_lock(
                    &scope,
                    capture.snapshot(),
                    &lock,
                    &NeverCancel,
                )
                .map_err(|error| PluginServiceError::integration(error.to_string()))?;
            Ok(lock_digest)
        })();
        let lock_digest = match prepared {
            Ok(lock_digest) => lock_digest,
            Err(error) => {
                self.store.delete_project(&scope).map_err(|cleanup| {
                    PluginServiceError::integration(format!(
                        "Plugin Source preparation failed with {error}; cleanup failed with {cleanup}"
                    ))
                })?;
                return Err(error);
            }
        };
        Ok(CreatedPluginSource {
            managed_relative_path: scaffold.project().managed_relative_path().to_owned(),
            source_snapshot_digest: capture.snapshot().digest().as_ref().to_owned(),
            dependency_lock_digest: lock_digest.as_ref().to_owned(),
        })
    }

    async fn delete_project(
        &self,
        owner_user_id: &str,
        project_id: &str,
    ) -> Result<(), PluginServiceError> {
        let scope = SourceScope::new(
            UserId::from(owner_user_id.to_owned()),
            PluginProjectId::from(project_id.to_owned()),
        )
        .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
        self.store
            .delete_project(&scope)
            .map_err(|error| PluginServiceError::integration(error.to_string()))
    }
}

impl FsPluginMountDataStore {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, PluginServiceError> {
        let root = std::fs::canonicalize(root.as_ref()).map_err(|error| {
            PluginServiceError::integration(format!(
                "cannot canonicalize Plugin data root {}: {error}",
                root.as_ref().display()
            ))
        })?;
        Ok(Self { root })
    }
}

#[async_trait]
impl PluginMountDataStore for FsPluginMountDataStore {
    async fn delete_mount_data(
        &self,
        mount_id: &str,
        managed_relative_path: &str,
    ) -> Result<(), PluginServiceError> {
        if mount_id.trim().is_empty() {
            return Err(PluginServiceError::invalid("mount_id must not be empty"));
        }
        let relative = Path::new(managed_relative_path);
        if managed_relative_path.is_empty()
            || managed_relative_path.contains('\\')
            || relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(PluginServiceError::invalid(
                "Plugin data path must be a managed relative path",
            ));
        }
        let data_dir = self.root.join(relative);
        if !data_dir.exists() {
            return Ok(());
        }
        let resolved = std::fs::canonicalize(&data_dir).map_err(|error| {
            PluginServiceError::integration(format!(
                "cannot canonicalize Plugin data directory {}: {error}",
                data_dir.display()
            ))
        })?;
        if resolved == self.root || !resolved.starts_with(&self.root) {
            return Err(PluginServiceError::conflict(
                "Plugin data deletion escaped the configured data root",
            ));
        }
        if resolved.file_name().and_then(|value| value.to_str()) != Some(mount_id) {
            return Err(PluginServiceError::conflict(
                "Plugin data directory does not match the stable Mount identity",
            ));
        }
        std::fs::remove_dir_all(&resolved).map_err(|error| {
            PluginServiceError::integration(format!(
                "cannot delete Plugin data directory {}: {error}",
                resolved.display()
            ))
        })
    }
}

impl FsPluginArtifactStore {
    pub fn new(
        managed_root: impl AsRef<Path>,
        limits: ArtifactStoreLimits,
    ) -> Result<Self, PluginServiceError> {
        Ok(Self {
            store: PluginArtifactStore::new(managed_root, limits)?,
        })
    }

    pub fn store(&self) -> &PluginArtifactStore {
        &self.store
    }
}

#[async_trait]
impl PluginArtifactStorePort for FsPluginArtifactStore {
    async fn import_directory(
        &self,
        source: &Path,
    ) -> Result<ImportedPluginArtifact, PluginServiceError> {
        let result = self
            .store
            .import_directory(source, &nomifun_plugin_platform::NeverCancel)?;
        Ok(ImportedPluginArtifact {
            artifact: result.stored.artifact,
            managed_relative_path: result.stored.managed_relative_path,
            package_root: result.stored.package_root,
            already_present: result.already_present,
        })
    }

    async fn import_zip(&self, source: &Path) -> Result<ImportedPluginArtifact, PluginServiceError> {
        let result = self
            .store
            .import_zip(source, &nomifun_plugin_platform::NeverCancel)?;
        Ok(ImportedPluginArtifact {
            artifact: result.stored.artifact,
            managed_relative_path: result.stored.managed_relative_path,
            package_root: result.stored.package_root,
            already_present: result.already_present,
        })
    }

    async fn verify(&self, artifact: &PluginArtifactRow) -> Result<(), PluginServiceError> {
        let stored = self
            .store
            .load(&DigestHex::from(artifact.artifact_digest.clone()))?;
        let package = &stored.artifact.manifest.payload.package;
        if stored.artifact.artifact_id.as_ref() != artifact.artifact_id
            || stored.artifact.artifact_digest.as_ref() != artifact.artifact_digest
            || stored.artifact.manifest.payload_digest.as_ref() != artifact.manifest_digest
            || package.package_id.as_ref() != artifact.package_id
            || package.package_version.as_ref() != artifact.package_version
            || stored.managed_relative_path != artifact.managed_path
        {
            return Err(PluginServiceError::Coded {
                code: crate::ERR_ARTIFACT,
                message: "persisted Plugin artifact metadata differs from the verified store"
                    .into(),
            });
        }
        Ok(())
    }
}

#[async_trait]
pub trait PluginHostCoordinator: Send + Sync {
    async fn commit_fence(
        &self,
        mount_id: &str,
    ) -> Result<nomifun_agent_contracts::PluginHostCommitFence, PluginServiceError>;
}

pub struct SharedJsHostCoordinator {
    host: Arc<ExtensionHostSupervisor>,
}

impl SharedJsHostCoordinator {
    pub fn new(host: Arc<ExtensionHostSupervisor>) -> Self {
        Self { host }
    }

    pub fn host(&self) -> &Arc<ExtensionHostSupervisor> {
        &self.host
    }
}

#[async_trait]
impl PluginHostCoordinator for SharedJsHostCoordinator {
    async fn commit_fence(
        &self,
        mount_id: &str,
    ) -> Result<nomifun_agent_contracts::PluginHostCommitFence, PluginServiceError> {
        let mount_id = PluginMountId::from(mount_id.to_owned());
        match self.host.commit_fence_for_mount(&mount_id).await {
            Ok(fence) => Ok(fence),
            Err(JavaScriptHostError::NotQuiescent { generation }) => {
                Ok(self.host.stop_generation(generation).await?)
            }
            Err(error) => Err(error.into()),
        }
    }
}

#[async_trait]
pub trait PluginRegistryPublisher: Send + Sync {
    async fn reconcile_mount(
        &self,
        owner_user_id: &str,
        mount: &PluginMountRow,
    ) -> Result<(), PluginServiceError>;
}

#[async_trait]
pub trait PluginBuildExecutor: Send + Sync {
    async fn build(
        &self,
        operation_id: &str,
        project: &PluginProjectRow,
        request: &nomifun_api_types::BuildPluginProjectRequest,
    ) -> Result<BuildOutput, PluginServiceError>;
}

#[async_trait]
pub trait PluginCandidateTestExecutor: Send + Sync {
    async fn test(
        &self,
        project: &PluginProjectRow,
        candidate: &PluginReadyCandidateRow,
        request: &nomifun_api_types::TestPluginCandidateRequest,
    ) -> Result<CandidateTestOutput, PluginServiceError>;
}

#[async_trait]
pub trait PluginOperationCancellation: Send + Sync {
    async fn cancel(&self, operation: &ProductOperationRow) -> Result<(), PluginServiceError>;
}

#[derive(Default)]
pub struct UnconfiguredPluginBuildExecutor;

#[async_trait]
impl PluginBuildExecutor for UnconfiguredPluginBuildExecutor {
    async fn build(
        &self,
        _operation_id: &str,
        _project: &PluginProjectRow,
        _request: &nomifun_api_types::BuildPluginProjectRequest,
    ) -> Result<BuildOutput, PluginServiceError> {
        Err(PluginServiceError::integration(
            "Plugin Build Host is not wired into App composition",
        ))
    }
}

#[derive(Default)]
pub struct UnconfiguredPluginCandidateTestExecutor;

#[async_trait]
impl PluginCandidateTestExecutor for UnconfiguredPluginCandidateTestExecutor {
    async fn test(
        &self,
        _project: &PluginProjectRow,
        _candidate: &PluginReadyCandidateRow,
        _request: &nomifun_api_types::TestPluginCandidateRequest,
    ) -> Result<CandidateTestOutput, PluginServiceError> {
        Err(PluginServiceError::integration(
            "Plugin Candidate Test Host is not wired into App composition",
        ))
    }
}

#[derive(Default)]
pub struct UnconfiguredPluginOperationCancellation;

#[async_trait]
impl PluginOperationCancellation for UnconfiguredPluginOperationCancellation {
    async fn cancel(&self, _operation: &ProductOperationRow) -> Result<(), PluginServiceError> {
        Err(PluginServiceError::integration(
            "Plugin operation cancellation is not wired into App composition",
        ))
    }
}

#[derive(Default)]
pub struct UnconfiguredPluginRegistryPublisher;

#[async_trait]
impl PluginRegistryPublisher for UnconfiguredPluginRegistryPublisher {
    async fn reconcile_mount(
        &self,
        _owner_user_id: &str,
        _mount: &PluginMountRow,
    ) -> Result<(), PluginServiceError> {
        Err(PluginServiceError::integration(
            "Kernel Registry publication is not wired into App composition",
        ))
    }
}

pub struct DbPluginRepositoryAdapter {
    inner: SqlitePluginN1Repository,
    pool: SqlitePool,
}

impl DbPluginRepositoryAdapter {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            inner: SqlitePluginN1Repository::new(pool.clone()),
            pool,
        }
    }

    pub fn sqlite_repository(&self) -> &SqlitePluginN1Repository {
        &self.inner
    }
}

#[async_trait]
impl PluginRepository for DbPluginRepositoryAdapter {
    async fn inventory(&self, owner_user_id: &str) -> Result<PluginInventory, PluginServiceError> {
        let projects = nomifun_db::sqlx::query_as::<_, PluginProjectRow>(
            "SELECT * FROM plugin_projects
             WHERE owner_user_id = ?
             ORDER BY created_at ASC, project_id ASC",
        )
        .bind(owner_user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(query_error)?;
        let mounts = nomifun_db::sqlx::query_as::<_, PluginMountRow>(
            "SELECT mount.*
             FROM plugin_mounts mount
             WHERE EXISTS (
                 SELECT 1 FROM plugin_projects project
                 WHERE project.linked_mount_id = mount.mount_id
                   AND project.owner_user_id = ?
             )
             OR (
                 NOT EXISTS (
                     SELECT 1 FROM plugin_projects project
                     WHERE project.linked_mount_id = mount.mount_id
                 )
                 AND EXISTS (
                     SELECT 1 FROM installation_identity identity
                     WHERE identity.singleton_key = 'installation'
                       AND identity.owner_user_id = ?
                 )
             )
             ORDER BY mount.created_at ASC, mount.mount_id ASC",
        )
        .bind(owner_user_id)
        .bind(owner_user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(query_error)?;
        let candidates = nomifun_db::sqlx::query_as::<_, PluginReadyCandidateRow>(
            "SELECT candidate.*,
                    artifact.package_id AS target_package_id,
                    artifact.package_version AS target_package_version,
                    artifact.manifest_digest AS target_manifest_digest
             FROM plugin_ready_candidates candidate
             JOIN plugin_artifacts artifact
               ON artifact.artifact_id = candidate.artifact_id
              AND artifact.artifact_digest = candidate.artifact_digest
             JOIN plugin_projects project
               ON project.project_id = candidate.project_id
              AND project.ready_candidate_id = candidate.candidate_id
             WHERE project.owner_user_id = ?
             ORDER BY candidate.created_at ASC, candidate.candidate_id ASC",
        )
        .bind(owner_user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(query_error)?;
        let artifacts = nomifun_db::sqlx::query_as::<_, PluginArtifactRow>(
            "SELECT artifact.*
             FROM plugin_artifacts artifact
             WHERE artifact.artifact_digest IN (
                 SELECT candidate.artifact_digest
                 FROM plugin_ready_candidates candidate
                 JOIN plugin_projects project
                   ON project.project_id = candidate.project_id
                  AND project.ready_candidate_id = candidate.candidate_id
                 WHERE project.owner_user_id = ?
                 UNION
                 SELECT mount.current_artifact_digest
                 FROM plugin_mounts mount
                 WHERE mount.current_artifact_digest IS NOT NULL
                   AND (
                       EXISTS (
                           SELECT 1 FROM plugin_projects project
                           WHERE project.linked_mount_id = mount.mount_id
                             AND project.owner_user_id = ?
                       )
                       OR (
                           NOT EXISTS (
                               SELECT 1 FROM plugin_projects project
                               WHERE project.linked_mount_id = mount.mount_id
                           )
                           AND EXISTS (
                               SELECT 1 FROM installation_identity identity
                               WHERE identity.singleton_key = 'installation'
                                 AND identity.owner_user_id = ?
                           )
                       )
                   )
                 UNION
                 SELECT mount.previous_artifact_digest
                 FROM plugin_mounts mount
                 WHERE mount.previous_artifact_digest IS NOT NULL
                   AND (
                       EXISTS (
                           SELECT 1 FROM plugin_projects project
                           WHERE project.linked_mount_id = mount.mount_id
                             AND project.owner_user_id = ?
                       )
                       OR (
                           NOT EXISTS (
                               SELECT 1 FROM plugin_projects project
                               WHERE project.linked_mount_id = mount.mount_id
                           )
                           AND EXISTS (
                               SELECT 1 FROM installation_identity identity
                               WHERE identity.singleton_key = 'installation'
                                 AND identity.owner_user_id = ?
                           )
                       )
                   )
             )
             ORDER BY artifact.created_at ASC, artifact.artifact_digest ASC",
        )
        .bind(owner_user_id)
        .bind(owner_user_id)
        .bind(owner_user_id)
        .bind(owner_user_id)
        .bind(owner_user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(query_error)?;
        let receipts = nomifun_db::sqlx::query_as::<_, PluginCandidateTestReceiptRow>(
            "SELECT receipt.*
             FROM plugin_candidate_test_receipts receipt
             JOIN plugin_ready_candidates candidate
               ON candidate.candidate_id = receipt.candidate_id
              AND candidate.candidate_digest = receipt.candidate_digest
             JOIN plugin_projects project
               ON project.project_id = candidate.project_id
              AND project.ready_candidate_id = candidate.candidate_id
             WHERE project.owner_user_id = ?
             ORDER BY receipt.tested_at ASC, receipt.receipt_id ASC",
        )
        .bind(owner_user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(query_error)?;
        let operations = self.list_operations(owner_user_id).await?;
        let library_revision =
            inventory_revision(&artifacts, &projects, &mounts, &candidates, &receipts)?;
        Ok(PluginInventory {
            library_revision,
            artifacts,
            projects,
            mounts,
            candidates,
            receipts,
            operations,
        })
    }

    async fn get_project(
        &self,
        project_id: &str,
    ) -> Result<Option<PluginProjectRow>, PluginServiceError> {
        Ok(self.inner.get_project(project_id).await?)
    }

    async fn get_project_for_mount(
        &self,
        mount_id: &str,
    ) -> Result<Option<PluginProjectRow>, PluginServiceError> {
        nomifun_db::sqlx::query_as(
            "SELECT * FROM plugin_projects WHERE linked_mount_id = ?",
        )
        .bind(mount_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(query_error)
    }

    async fn mount_owner_user_id(
        &self,
        mount_id: &str,
    ) -> Result<Option<String>, PluginServiceError> {
        nomifun_db::sqlx::query_scalar(
            "SELECT COALESCE(
                 (
                     SELECT project.owner_user_id
                     FROM plugin_projects project
                     WHERE project.linked_mount_id = mount.mount_id
                 ),
                 (
                     SELECT identity.owner_user_id
                     FROM installation_identity identity
                     WHERE identity.singleton_key = 'installation'
                 )
             )
             FROM plugin_mounts mount
             WHERE mount.mount_id = ?",
        )
        .bind(mount_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(query_error)
    }

    async fn get_mount(
        &self,
        mount_id: &str,
    ) -> Result<Option<PluginMountRow>, PluginServiceError> {
        Ok(self.inner.get_mount(mount_id).await?)
    }

    async fn get_candidate(
        &self,
        project_id: &str,
    ) -> Result<Option<PluginReadyCandidateRow>, PluginServiceError> {
        Ok(self.inner.get_ready_candidate(project_id).await?)
    }

    async fn get_artifact(
        &self,
        artifact_digest: &str,
    ) -> Result<Option<PluginArtifactRow>, PluginServiceError> {
        nomifun_db::sqlx::query_as(
            "SELECT * FROM plugin_artifacts WHERE artifact_digest = ?",
        )
        .bind(artifact_digest)
        .fetch_optional(&self.pool)
        .await
        .map_err(query_error)
    }

    async fn get_test_receipt(
        &self,
        candidate_id: &str,
    ) -> Result<Option<PluginCandidateTestReceiptRow>, PluginServiceError> {
        nomifun_db::sqlx::query_as(
            "SELECT * FROM plugin_candidate_test_receipts WHERE candidate_id = ?",
        )
        .bind(candidate_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(query_error)
    }

    async fn create_project(
        &self,
        params: &CreatePluginProjectParams,
    ) -> Result<PluginProjectRow, PluginServiceError> {
        Ok(self.inner.create_project(params).await?)
    }

    async fn delete_project_cas(
        &self,
        params: &DeletePluginProjectParams,
    ) -> Result<bool, PluginServiceError> {
        Ok(self.inner.delete_project_cas(params).await?)
    }

    async fn link_project(
        &self,
        params: &LinkPluginProjectParams,
    ) -> Result<PluginProjectRow, PluginServiceError> {
        let mut tx = self.pool.begin().await.map_err(query_error)?;
        let project = nomifun_db::sqlx::query_as::<_, PluginProjectRow>(
            "SELECT * FROM plugin_projects WHERE project_id = ?",
        )
        .bind(&params.project_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(query_error)?
        .ok_or_else(|| {
            PluginServiceError::not_found(format!("project {}", params.project_id))
        })?;
        if project.owner_user_id != params.owner_user_id {
            return Err(PluginServiceError::forbidden(
                "project belongs to another owner",
            ));
        }
        if project.updated_at as u64 != params.expected_project_revision {
            return Err(PluginServiceError::stale("project revision changed"));
        }
        let mount = nomifun_db::sqlx::query_as::<_, PluginMountRow>(
            "SELECT * FROM plugin_mounts WHERE mount_id = ?",
        )
        .bind(&params.mount_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(query_error)?
        .ok_or_else(|| PluginServiceError::not_found(format!("mount {}", params.mount_id)))?;
        if mount.revision as u64 != params.expected_mount_revision
            || mount.current_artifact_digest.as_deref()
                != Some(params.expected_target_digest.as_str())
        {
            return Err(PluginServiceError::stale(
                "Mount revision or target changed",
            ));
        }
        if mount.package_id != project.package_id {
            return Err(PluginServiceError::conflict(
                "project and Mount package identities differ",
            ));
        }
        let mount_owner = nomifun_db::sqlx::query_scalar::<_, String>(
            "SELECT COALESCE(
                 (
                     SELECT linked.owner_user_id
                     FROM plugin_projects linked
                     WHERE linked.linked_mount_id = ?
                 ),
                 (
                     SELECT identity.owner_user_id
                     FROM installation_identity identity
                     WHERE identity.singleton_key = 'installation'
                 )
             )",
        )
        .bind(&params.mount_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(query_error)?;
        if mount_owner != params.owner_user_id {
            return Err(PluginServiceError::forbidden(
                "Mount belongs to another owner",
            ));
        }
        if project.linked_mount_id.as_deref() == Some(params.mount_id.as_str()) {
            tx.commit().await.map_err(query_error)?;
            return Ok(project);
        }
        if project.linked_mount_id.is_some() {
            return Err(PluginServiceError::conflict(
                "project is already linked to another Mount",
            ));
        }
        let linked_elsewhere: bool = nomifun_db::sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1 FROM plugin_projects
                 WHERE linked_mount_id = ? AND project_id <> ?
             )",
        )
        .bind(&params.mount_id)
        .bind(&params.project_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(query_error)?;
        if linked_elsewhere {
            return Err(PluginServiceError::conflict(
                "Mount is already linked to another project",
            ));
        }
        let changed = nomifun_db::sqlx::query(
            "UPDATE plugin_projects
             SET linked_mount_id = ?, updated_at = ?
             WHERE project_id = ? AND owner_user_id = ?
               AND updated_at = ? AND linked_mount_id IS NULL",
        )
        .bind(&params.mount_id)
        .bind(params.updated_at)
        .bind(&params.project_id)
        .bind(&params.owner_user_id)
        .bind(project.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?
        .rows_affected();
        if changed != 1 {
            return Err(PluginServiceError::stale(
                "project-to-Mount link lost its exact CAS",
            ));
        }
        let linked =
            nomifun_db::sqlx::query_as("SELECT * FROM plugin_projects WHERE project_id = ?")
                .bind(&params.project_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(query_error)?;
        tx.commit().await.map_err(query_error)?;
        Ok(linked)
    }

    async fn put_artifact(
        &self,
        params: &CreatePluginArtifactParams,
    ) -> Result<PluginArtifactRow, PluginServiceError> {
        Ok(self.inner.put_artifact(params).await?)
    }

    async fn start_operation(
        &self,
        params: &StartProductOperationParams,
    ) -> Result<ProductOperationRow, PluginServiceError> {
        Ok(self.inner.start_operation(params).await?)
    }

    async fn finish_operation(
        &self,
        params: &FinishProductOperationParams,
    ) -> Result<ProductOperationRow, PluginServiceError> {
        Ok(self.inner.finish_operation(params).await?)
    }

    async fn record_candidate(
        &self,
        params: &RecordPluginReadyCandidateParams,
    ) -> Result<PluginReadyCandidateRow, PluginServiceError> {
        Ok(self.inner.record_ready_candidate(params).await?)
    }

    async fn record_test_receipt(
        &self,
        params: &RecordPluginCandidateTestReceiptParams,
    ) -> Result<PluginCandidateTestReceiptRow, PluginServiceError> {
        Ok(self.inner.record_candidate_test_receipt(params).await?)
    }

    async fn apply_candidate(
        &self,
        params: &ApplyPluginCandidateParams,
    ) -> Result<PluginMountRow, PluginServiceError> {
        Ok(self.inner.apply_candidate(params).await?)
    }

    async fn restore_previous(
        &self,
        params: &RestorePluginMountParams,
    ) -> Result<PluginMountRow, PluginServiceError> {
        Ok(self.inner.restore_previous(params).await?)
    }

    async fn uninstall_retain_data(
        &self,
        params: &UninstallPluginMountParams,
    ) -> Result<PluginMountRow, PluginServiceError> {
        Ok(self.inner.uninstall_retain_data(params).await?)
    }

    async fn mark_delete_pending(
        &self,
        mount_id: &str,
        expected_revision: i64,
        updated_at: i64,
    ) -> Result<PluginMountRow, PluginServiceError> {
        Ok(self
            .inner
            .mark_mount_delete_pending(mount_id, expected_revision, updated_at)
            .await?)
    }

    async fn complete_data_delete(&self, mount_id: &str) -> Result<bool, PluginServiceError> {
        Ok(self.inner.complete_mount_data_delete(mount_id).await?)
    }

    async fn update_config(
        &self,
        params: &UpdatePluginMountConfigParams,
    ) -> Result<PluginMountRow, PluginServiceError> {
        Ok(self.inner.update_mount_config_cas(params).await?)
    }

    async fn replace_credentials(
        &self,
        params: &ReplacePluginCredentialBindingsParams,
    ) -> Result<nomifun_db::PluginCredentialBindingSnapshot, PluginServiceError> {
        Ok(self.inner.replace_credential_bindings(params).await?)
    }

    async fn list_credentials(
        &self,
        params: &ListPluginCredentialBindingsParams,
    ) -> Result<nomifun_db::PluginCredentialBindingSnapshot, PluginServiceError> {
        Ok(self.inner.list_credential_bindings(params).await?)
    }

    async fn set_enabled(
        &self,
        mount_id: &str,
        expected_revision: i64,
        expected_digest: &str,
        enabled: bool,
        updated_at: i64,
    ) -> Result<PluginMountRow, PluginServiceError> {
        let current = self
            .inner
            .get_mount(mount_id)
            .await?
            .ok_or_else(|| PluginServiceError::not_found(format!("mount {mount_id}")))?;
        if current.revision != expected_revision
            || current.current_artifact_digest.as_deref() != Some(expected_digest)
            || current.retained
            || current.delete_pending
            || updated_at < current.updated_at
        {
            return Err(PluginServiceError::stale(
                "Mount enable state lost its exact CAS",
            ));
        }
        if current.enabled == enabled {
            return Ok(current);
        }
        let changed = nomifun_db::sqlx::query(
            "UPDATE plugin_mounts
             SET enabled = ?, revision = revision + 1, updated_at = ?
             WHERE mount_id = ? AND revision = ?
               AND current_artifact_digest = ?
               AND retained = 0 AND delete_pending = 0",
        )
        .bind(enabled)
        .bind(updated_at)
        .bind(mount_id)
        .bind(expected_revision)
        .bind(expected_digest)
        .execute(&self.pool)
        .await
        .map_err(query_error)?
        .rows_affected();
        if changed != 1 {
            return Err(PluginServiceError::stale(
                "Mount enable state lost its exact CAS",
            ));
        }
        self.inner
            .get_mount(mount_id)
            .await?
            .ok_or_else(|| PluginServiceError::not_found(format!("mount {mount_id}")))
    }

    async fn retry_mount(
        &self,
        mount_id: &str,
        expected_revision: i64,
        expected_digest: &str,
        updated_at: i64,
    ) -> Result<PluginMountRow, PluginServiceError> {
        let current = self
            .inner
            .get_mount(mount_id)
            .await?
            .ok_or_else(|| PluginServiceError::not_found(format!("mount {mount_id}")))?;
        if current.revision != expected_revision
            || current.current_artifact_digest.as_deref() != Some(expected_digest)
            || current.retained
            || current.delete_pending
            || updated_at < current.updated_at
        {
            return Err(PluginServiceError::stale(
                "Mount retry lost its exact CAS",
            ));
        }
        if current.last_error.is_none() {
            return Ok(current);
        }
        let changed = nomifun_db::sqlx::query(
            "UPDATE plugin_mounts
             SET last_error = NULL, revision = revision + 1, updated_at = ?
             WHERE mount_id = ? AND revision = ?
               AND current_artifact_digest = ?
               AND retained = 0 AND delete_pending = 0
               AND last_error IS NOT NULL",
        )
        .bind(updated_at)
        .bind(mount_id)
        .bind(expected_revision)
        .bind(expected_digest)
        .execute(&self.pool)
        .await
        .map_err(query_error)?
        .rows_affected();
        if changed != 1 {
            return Err(PluginServiceError::stale(
                "Mount retry lost its exact CAS",
            ));
        }
        self.inner
            .get_mount(mount_id)
            .await?
            .ok_or_else(|| PluginServiceError::not_found(format!("mount {mount_id}")))
    }

    async fn list_operations(
        &self,
        owner_user_id: &str,
    ) -> Result<Vec<ProductOperationRow>, PluginServiceError> {
        nomifun_db::sqlx::query_as(
            "SELECT operation.*
             FROM product_operations operation
             WHERE (
                 operation.owner_kind = 'plugin_project'
                 AND EXISTS (
                     SELECT 1 FROM plugin_projects project
                     WHERE project.project_id = operation.owner_id
                       AND project.owner_user_id = ?
                 )
             )
             OR (
                 operation.owner_kind = 'plugin_mount'
                 AND EXISTS (
                     SELECT 1 FROM plugin_mounts mount
                     WHERE mount.mount_id = operation.owner_id
                       AND (
                           EXISTS (
                               SELECT 1 FROM plugin_projects project
                               WHERE project.linked_mount_id = mount.mount_id
                                 AND project.owner_user_id = ?
                           )
                           OR (
                               NOT EXISTS (
                                   SELECT 1 FROM plugin_projects project
                                   WHERE project.linked_mount_id = mount.mount_id
                               )
                               AND EXISTS (
                                   SELECT 1 FROM installation_identity identity
                                   WHERE identity.singleton_key = 'installation'
                                     AND identity.owner_user_id = ?
                               )
                           )
                       )
                 )
             )
             ORDER BY operation.started_at_ms DESC, operation.operation_id DESC",
        )
        .bind(owner_user_id)
        .bind(owner_user_id)
        .bind(owner_user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(query_error)
    }

    async fn get_operation(
        &self,
        owner_user_id: &str,
        operation_id: &str,
    ) -> Result<Option<ProductOperationRow>, PluginServiceError> {
        nomifun_db::sqlx::query_as(
            "SELECT operation.*
             FROM product_operations operation
             WHERE operation.operation_id = ?
               AND (
                   (
                       operation.owner_kind = 'plugin_project'
                       AND EXISTS (
                           SELECT 1 FROM plugin_projects project
                           WHERE project.project_id = operation.owner_id
                             AND project.owner_user_id = ?
                       )
                   )
                   OR (
                       operation.owner_kind = 'plugin_mount'
                       AND EXISTS (
                           SELECT 1 FROM plugin_mounts mount
                           WHERE mount.mount_id = operation.owner_id
                             AND (
                                 EXISTS (
                                     SELECT 1 FROM plugin_projects project
                                     WHERE project.linked_mount_id = mount.mount_id
                                       AND project.owner_user_id = ?
                                 )
                                 OR (
                                     NOT EXISTS (
                                         SELECT 1 FROM plugin_projects project
                                         WHERE project.linked_mount_id = mount.mount_id
                                     )
                                     AND EXISTS (
                                         SELECT 1 FROM installation_identity identity
                                         WHERE identity.singleton_key = 'installation'
                                           AND identity.owner_user_id = ?
                                     )
                                 )
                             )
                       )
                   )
               )",
        )
        .bind(operation_id)
        .bind(owner_user_id)
        .bind(owner_user_id)
        .bind(owner_user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(query_error)
    }

    async fn cancel_operation(
        &self,
        owner_user_id: &str,
        operation_id: &str,
        expected_revision: u64,
        finished_at_ms: i64,
    ) -> Result<ProductOperationRow, PluginServiceError> {
        let operation = self
            .get_operation(owner_user_id, operation_id)
            .await?
            .ok_or_else(|| PluginServiceError::not_found(format!("operation {operation_id}")))?;
        if operation_revision(&operation) != expected_revision || operation.state != "running" {
            return Err(PluginServiceError::stale(
                "operation state or revision changed",
            ));
        }
        if operation.kind == "miniapp_permanent_delete" {
            return Err(PluginServiceError::conflict(
                "MiniApp permanent delete is not cancelable",
            ));
        }
        let progress_percent = operation
            .progress_percent
            .map(u8::try_from)
            .transpose()
            .map_err(|_| PluginServiceError::integration("stored operation progress is invalid"))?;
        let bounded_log_tail = serde_json::from_str(&operation.bounded_log_tail_json)
            .map_err(|error| {
                PluginServiceError::integration(format!(
                    "stored operation log tail is invalid: {error}"
                ))
            })?;
        Ok(self
            .inner
            .finish_operation(&FinishProductOperationParams {
                operation_id: operation.operation_id,
                state: ProductOperationState::Canceled,
                progress_percent,
                last_error_code: None,
                bounded_log_tail,
                finished_at_ms: finished_at_ms.max(operation.started_at_ms),
            })
            .await?)
    }
}

fn query_error(error: nomifun_db::sqlx::Error) -> PluginServiceError {
    PluginServiceError::from(nomifun_db::DbError::Query(error))
}

fn operation_revision(operation: &ProductOperationRow) -> u64 {
    u64::from(operation.finished_at_ms.is_some()) + 1
}

fn inventory_revision(
    artifacts: &[PluginArtifactRow],
    projects: &[PluginProjectRow],
    mounts: &[PluginMountRow],
    candidates: &[PluginReadyCandidateRow],
    receipts: &[PluginCandidateTestReceiptRow],
) -> Result<u64, PluginServiceError> {
    if artifacts.is_empty()
        && projects.is_empty()
        && mounts.is_empty()
        && candidates.is_empty()
        && receipts.is_empty()
    {
        return Ok(0);
    }
    let bytes = serde_json::to_vec(&(artifacts, projects, mounts, candidates, receipts))
        .map_err(|error| PluginServiceError::integration(error.to_string()))?;
    let digest = Sha256::digest(bytes);
    let mut prefix = [0_u8; 8];
    prefix.copy_from_slice(&digest[..8]);
    Ok(u64::from_be_bytes(prefix).max(1))
}
