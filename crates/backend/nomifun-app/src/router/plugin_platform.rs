use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use nomifun_agent_contracts::{
    CANDIDATE_TEST_CONTRACT_VERSION, CandidateTestCredentialMode, CandidateTestOutcome,
    CandidateTestReceipt, CandidateTestReceiptId, CanonicalErrorCode, CapabilityConsumer,
    CanonicalSchemaRef, CredentialId, CredentialSlotBinding, DigestHex,
    JAVASCRIPT_HOST_PROTOCOL_VERSION, JAVASCRIPT_SDK_CONTRACT_VERSION,
    PLUGIN_PACKAGE_PROFILE_VERSION, PluginHostCommitFence, PluginMountId,
    PluginSourceLineage, ResolvedCapability,
    RuntimeSwitchParticipantKind, RuntimeSwitchParticipantOutcome,
    RuntimeSwitchParticipantResult, StrictJsonValue, ValidatedPluginConfig,
};
use nomifun_ai_agent::NomiPluginToolSchemaResolver;
use nomifun_agent_kernel::{KernelRegistry, PluginRegistration};
use nomifun_agent_platform::KernelCatalogProvider;
use nomifun_api_types::{
    ApiResponse, ApplyPluginCandidateRequest, BuildPluginProjectRequest,
    ConfigurePluginRequest, CreatePluginProjectRequest,
    DeletePluginDataRequest, DeletePluginProjectRequest, DurableOperationDetailDto,
    DiscardPluginCandidateRequest, DurableOperationSummaryDto, ErrorResponse,
    ImportPluginRequest,
    PluginDetailDto, PluginLibraryResponseDto, PluginProjectDetailDto,
    RestorePluginPreviousRequest, RetryPluginRequest,
    SetPluginEnabledRequest, TestPluginCandidateRequest,
    UninstallPluginRequest,
};
use nomifun_auth::CurrentUser;
use nomifun_db::{
    ListPluginCredentialBindingsParams, PluginMountRow, PluginProjectRow,
    PluginReadyCandidateRow, SqlitePool,
};
use nomifun_js_host::{
    ExtensionHostDemandPort, ExtensionHostSupervisor,
    JavaScriptHostConfig, materialize_bundled_extension_host,
};
use nomifun_js_authoring::{
    ContentAddressedNpmCache, FixedPluginPacker, NodeBuildHost, SourceStoreLimits,
};
use nomifun_js_kernel_adapter::{
    JsKernelPluginAdapter, PluginPackageInput,
};
use nomifun_js_runtime::{
    CommittedRuntimeProvider, JavaScriptRuntimeError, JavaScriptWorkKind,
    ResolvedNodeRuntime, RuntimeUseLease,
};
use nomifun_plugin_platform::{
    ArtifactStoreLimits, OwnerMutationCoordinator,
};
use nomifun_plugin_service::{
    DbPluginRepositoryAdapter, FsPluginArtifactStore, FsPluginMountDataStore,
    FsPluginSourceStore,
    CandidateTestOutput, FsPluginBuildExecutor, PluginApplicationService,
    PluginArtifactStorePort, PluginBuildExecutor, PluginCandidateTestExecutor,
    PluginHostCoordinator, PluginOperationCancellation, PluginRegistryPublisher,
    PluginRepository, PluginRouterState,
    PluginServiceDependencies, PluginServiceError, PluginServicePaths,
    ApplyPluginSourceEditInput, ApplyPluginSourceEditRequest,
    PluginSourceFileEdit,
};
use tokio::sync::{Mutex, RwLock};
use serde::Deserialize;

const AGENT_EXECUTOR_UNAVAILABLE: &str = "CAPABILITY_UNAVAILABLE";
const PLUGIN_PLATFORM_DIRECTORY: &str = "plugin-platform";
const PLUGIN_ARTIFACT_DIRECTORY: &str = "artifact-store";
const PLUGIN_AUTHORING_DIRECTORY: &str = "authoring";
const PLUGIN_NPM_CACHE_DIRECTORY: &str = "npm-cache";
const PLUGIN_MOUNT_DATA_DIRECTORY: &str = "plugin-mount-data";
const PLUGIN_CANDIDATE_TEST_DIRECTORY: &str = "candidate-tests";

pub(crate) struct NomiCorePluginComposition {
    pub router: PluginRouterState,
    pub schema_resolver: Arc<dyn NomiPluginToolSchemaResolver>,
    pub runtime_participant: Arc<NomiCorePluginRuntimeParticipant>,
}

pub(crate) struct NomiCorePluginRuntimeParticipant {
    shared_host:
        Arc<super::plugin_runtime_host::RuntimeBoundExtensionHost>,
    build: Arc<RuntimeBoundPluginBuildExecutor>,
    repository: Arc<DbPluginRepositoryAdapter>,
    artifacts: Arc<FsPluginArtifactStore>,
    data_root: PathBuf,
    host_module: PathBuf,
    kernel: Arc<KernelRegistry>,
    publisher: Arc<NomiCorePluginRegistryPublisher>,
}

impl NomiCorePluginRuntimeParticipant {
    pub(crate) async fn stop_for_runtime_switch(
        &self,
    ) -> Result<(), JavaScriptRuntimeError> {
        self.kernel
            .release_all_resources()
            .await
            .map_err(|error| {
                JavaScriptRuntimeError::SwitchNotCovered(format!(
                    "Runtime-bound Kernel resource cleanup failed: {error}"
                ))
            })?;
        self.shared_host
            .stop_for_runtime_switch()
            .await
            .map_err(|error| {
                JavaScriptRuntimeError::SwitchNotCovered(error.to_string())
            })?;
        self.build.stop_for_runtime_switch().await;
        Ok(())
    }

    pub(crate) async fn prepare_runtime(
        &self,
        runtime: Option<&ResolvedNodeRuntime>,
    ) -> Result<(), JavaScriptRuntimeError> {
        self.build.prepare_runtime(runtime).await
    }

    pub(crate) async fn finalize_runtime(
        &self,
    ) -> Result<(), JavaScriptRuntimeError> {
        self.publisher
            .refresh_agent_availability()
            .await
            .map_err(|error| {
                JavaScriptRuntimeError::SwitchNotCovered(format!(
                    "Runtime availability reconciliation failed: {error}"
                ))
            })
    }

    pub(crate) async fn restore_runtime(
        &self,
        runtime: Option<&ResolvedNodeRuntime>,
    ) -> Result<(), JavaScriptRuntimeError> {
        self.prepare_runtime(runtime).await?;
        self.finalize_runtime().await
    }

    pub(crate) async fn validate_candidate(
        &self,
        owner_user_id: &str,
        candidate: &ResolvedNodeRuntime,
    ) -> Result<Vec<RuntimeSwitchParticipantResult>, JavaScriptRuntimeError> {
        let inventory = self
            .repository
            .inventory(owner_user_id)
            .await
            .map_err(|error| {
                JavaScriptRuntimeError::SwitchNotCovered(format!(
                    "Plugin Runtime inventory is unavailable: {error}"
                ))
            })?;
        let enabled = inventory
            .mounts
            .into_iter()
            .filter(mount_is_materializable)
            .collect::<Vec<_>>();
        if enabled.is_empty() {
            return Ok(Vec::new());
        }
        let candidate_data = RuntimeCandidateDataGuard::allocate(
            &self.data_root,
        )
        .map_err(|error| {
            JavaScriptRuntimeError::FoundationValidationFailed(
                error.to_string(),
            )
        })?;

        let host = ExtensionHostSupervisor::new(
            JavaScriptHostConfig::for_host_module(
                candidate.executable_path.clone(),
                candidate.fingerprint.clone(),
                self.host_module.clone(),
            ),
        )
        .map_err(|error| {
            JavaScriptRuntimeError::FoundationValidationFailed(
                error.to_string(),
            )
        })?;
        let mut results = Vec::with_capacity(enabled.len());
        for mount in enabled {
            let outcome = match plugin_adapter_for_mount(
                &self.repository,
                &self.artifacts,
                &self.data_root,
                &mount,
                Some(&candidate_data.path),
                false,
            )
            .await
            {
                Ok(adapter) => match host.load_mount(adapter.mount_demand()).await
                {
                    Ok(_) => RuntimeSwitchParticipantResult {
                        kind: RuntimeSwitchParticipantKind::PluginMount,
                        owner_id: mount.mount_id.clone(),
                        outcome: RuntimeSwitchParticipantOutcome::Passed,
                        error_code: None,
                    },
                    Err(error) => RuntimeSwitchParticipantResult {
                        kind: RuntimeSwitchParticipantKind::PluginMount,
                        owner_id: mount.mount_id.clone(),
                        outcome: RuntimeSwitchParticipantOutcome::Failed,
                        error_code: Some(CanonicalErrorCode::from(
                            runtime_participant_error_code(&error.to_string()),
                        )),
                    },
                },
                Err(error) => RuntimeSwitchParticipantResult {
                    kind: RuntimeSwitchParticipantKind::PluginMount,
                    owner_id: mount.mount_id.clone(),
                    outcome: RuntimeSwitchParticipantOutcome::Failed,
                    error_code: Some(CanonicalErrorCode::from(
                        runtime_participant_error_code(error.code()),
                    )),
                },
            };
            results.push(outcome);
        }
        if let nomifun_js_host::JavaScriptHostState::Running {
            generation,
            ..
        } = host.state()
        {
            host.stop_generation(generation).await.map_err(|error| {
                JavaScriptRuntimeError::FoundationValidationFailed(format!(
                    "candidate Plugin Host cleanup failed: {error}"
                ))
            })?;
        }
        if host.process_count() != 0 {
            return Err(JavaScriptRuntimeError::FoundationValidationFailed(
                "candidate Plugin Host process tree is not empty".to_owned(),
            ));
        }
        Ok(results)
    }
}

struct RuntimeCandidateDataGuard {
    path: PathBuf,
}

impl RuntimeCandidateDataGuard {
    fn allocate(root: &Path) -> Result<Self, std::io::Error> {
        let root = root.join("javascript-runtime").join("switch-candidates");
        std::fs::create_dir_all(&root)?;
        let root = std::fs::canonicalize(&root)?;
        let path = root.join(uuid::Uuid::now_v7().to_string());
        std::fs::create_dir(&path)?;
        let path = std::fs::canonicalize(&path)?;
        if path.parent() != Some(root.as_path()) {
            return Err(std::io::Error::other(
                "Runtime candidate data directory escaped its managed root",
            ));
        }
        Ok(Self { path })
    }
}

impl Drop for RuntimeCandidateDataGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn runtime_participant_error_code(input: &str) -> &'static str {
    if input.contains("ARTIFACT") {
        "PLUGIN_RUNTIME_ARTIFACT_INVALID"
    } else if input.contains("STALE") {
        "PLUGIN_RUNTIME_TARGET_STALE"
    } else {
        "PLUGIN_RUNTIME_VALIDATION_FAILED"
    }
}

async fn plugin_adapter_for_mount(
    repository: &DbPluginRepositoryAdapter,
    artifacts: &FsPluginArtifactStore,
    data_root: &Path,
    mount: &PluginMountRow,
    data_dir_override: Option<&Path>,
    include_credentials: bool,
) -> Result<JsKernelPluginAdapter, PluginServiceError> {
    let artifact_digest = mount
        .current_artifact_digest
        .as_deref()
        .ok_or_else(|| {
            PluginServiceError::stale(
                "materialized Mount has no current Artifact",
            )
        })?;
    let artifact_row = repository
        .get_artifact(artifact_digest)
        .await?
        .ok_or_else(|| {
            PluginServiceError::not_found("materialized Mount Artifact")
        })?;
    artifacts.verify(&artifact_row).await?;
    let stored = artifacts
        .store()
        .load(&DigestHex::from(artifact_digest.to_owned()))?;
    if stored.artifact.artifact_digest.as_ref() != artifact_digest
        || stored
            .artifact
            .manifest
            .payload
            .package
            .package_id
            .as_ref()
            != mount.package_id
    {
        return Err(PluginServiceError::stale(
            "Mount target differs from the verified Artifact",
        ));
    }

    let mount_id = PluginMountId::from(mount.mount_id.clone());
    let bindings = repository
        .list_credentials(&ListPluginCredentialBindingsParams {
            mount_id: mount.mount_id.clone(),
            expected_mount_revision: mount.revision,
            expected_current_artifact_digest: mount
                .current_artifact_digest
                .clone(),
        })
        .await?;
    let credential_bindings = if include_credentials {
        bindings
            .bindings
            .into_iter()
            .map(|binding| CredentialSlotBinding {
                mount_id: mount_id.clone(),
                slot_key: binding.slot.into(),
                credential_id: CredentialId::from(binding.credential_id),
            })
            .collect()
    } else {
        Vec::new()
    };
    let config_revision = u64::try_from(mount.config_revision).map_err(|_| {
        PluginServiceError::integration("Plugin config revision is negative")
    })?;
    let config_schema_digest =
        mount.config_schema_digest.clone().ok_or_else(|| {
            PluginServiceError::stale(
                "materialized Mount has no config schema digest",
            )
        })?;
    let config = serde_json::from_str(&mount.config_json).map_err(|error| {
        PluginServiceError::integration(format!(
            "persisted Plugin config is invalid JSON: {error}"
        ))
    })?;
    let data_dir = match data_dir_override {
        Some(root) => {
            let path = root.join(&mount.mount_id);
            tokio::fs::create_dir_all(&path).await.map_err(|error| {
                PluginServiceError::integration(format!(
                    "cannot create Runtime candidate data directory: {error}"
                ))
            })?;
            std::fs::canonicalize(&path).map_err(|error| {
                PluginServiceError::integration(format!(
                    "cannot canonicalize Runtime candidate data directory: {error}"
                ))
            })?
        }
        None => {
            resolve_mount_data_dir(data_root, &mount.data_dir_path, &mount.mount_id)
                .await?
        }
    };
    JsKernelPluginAdapter::new(PluginPackageInput {
        artifact: stored.artifact,
        mount_id,
        package_root: stored.package_root,
        config: ValidatedPluginConfig {
            schema_digest: DigestHex::from(config_schema_digest),
            config_revision,
            value: StrictJsonValue(config),
        },
        credential_bindings,
        data_dir,
    })
    .map_err(|error| {
        PluginServiceError::integration(format!(
            "JavaScript Plugin adapter rejected the Mount: {error}"
        ))
    })
}

pub(crate) async fn build_nomi_core_plugin_state(
    pool: SqlitePool,
    data_root: PathBuf,
    owner_user_id: &str,
    kernel: Arc<KernelRegistry>,
    catalog: Arc<KernelCatalogProvider>,
    base_registrations: Vec<PluginRegistration>,
    runtime: Arc<dyn CommittedRuntimeProvider>,
) -> anyhow::Result<NomiCorePluginComposition> {
    let data_root = std::fs::canonicalize(&data_root)?;
    let platform_root = data_root.join(PLUGIN_PLATFORM_DIRECTORY);
    tokio::fs::create_dir_all(&platform_root).await?;
    tokio::fs::create_dir_all(data_root.join(PLUGIN_MOUNT_DATA_DIRECTORY)).await?;

    let repository = Arc::new(DbPluginRepositoryAdapter::new(pool));
    let artifacts = Arc::new(FsPluginArtifactStore::new(
        platform_root.join(PLUGIN_ARTIFACT_DIRECTORY),
        ArtifactStoreLimits::default(),
    )?);
    let source_store = Arc::new(FsPluginSourceStore::new(
        platform_root.join(PLUGIN_AUTHORING_DIRECTORY),
        SourceStoreLimits::default(),
    )?);
    let host_module =
        materialize_bundled_extension_host(platform_root.join("host"))?;
    let shared_host = super::plugin_runtime_host::RuntimeBoundExtensionHost::new(
        Arc::clone(&runtime),
        host_module.clone(),
    )?;
    let host: Arc<dyn ExtensionHostDemandPort> = shared_host.clone();
    let participant_kernel = Arc::clone(&kernel);
    let publisher = Arc::new(NomiCorePluginRegistryPublisher {
        kernel,
        catalog,
        base_registrations,
        dynamic_registrations: RwLock::new(BTreeMap::new()),
        publish_lock: Mutex::new(()),
        repository: Arc::clone(&repository),
        artifacts: Arc::clone(&artifacts),
        host: Arc::clone(&host),
        runtime: Arc::clone(&runtime),
        data_root: data_root.clone(),
    });
    publisher.restore(owner_user_id).await?;

    let host_coordinator: Arc<dyn PluginHostCoordinator> =
        Arc::new(RuntimeBoundPluginHostCoordinator {
            host: Arc::clone(&shared_host),
        });
    let tester: Arc<dyn PluginCandidateTestExecutor> =
        Arc::new(NomiCorePluginCandidateTestExecutor {
            repository: Arc::clone(&repository),
            artifacts: Arc::clone(&artifacts),
            runtime: Arc::clone(&runtime),
            host_module: host_module.clone(),
            candidate_test_root: platform_root
                .join(PLUGIN_CANDIDATE_TEST_DIRECTORY),
        });
    let build = Arc::new(RuntimeBoundPluginBuildExecutor::new(
        Arc::clone(&runtime),
        Arc::clone(&source_store),
        Arc::clone(&artifacts),
        platform_root.join(PLUGIN_NPM_CACHE_DIRECTORY),
    )?);
    let builder = Arc::clone(&build) as Arc<dyn PluginBuildExecutor>;
    let operation_cancellation =
        Arc::clone(&build) as Arc<dyn PluginOperationCancellation>;
    let service = Arc::new(PluginApplicationService::new(
        PluginServiceDependencies {
            repository: Arc::clone(&repository) as Arc<dyn PluginRepository>,
            artifacts: Arc::clone(&artifacts) as Arc<dyn PluginArtifactStorePort>,
            host: host_coordinator,
            registry: Arc::clone(&publisher) as Arc<dyn PluginRegistryPublisher>,
            mutation_coordinator: Arc::new(OwnerMutationCoordinator::new()),
            builder,
            tester,
            operation_cancellation,
            source_store,
            data_store: Arc::new(FsPluginMountDataStore::new(&data_root)?),
            paths: PluginServicePaths {
                mount_data_relative_root:
                    PLUGIN_MOUNT_DATA_DIRECTORY.to_owned(),
            },
        },
    ));
    let runtime_participant = Arc::new(NomiCorePluginRuntimeParticipant {
        shared_host,
        build,
        repository: Arc::clone(&repository),
        artifacts: Arc::clone(&artifacts),
        data_root: data_root.clone(),
        host_module: host_module.clone(),
        kernel: participant_kernel,
        publisher,
    });
    Ok(NomiCorePluginComposition {
        router: PluginRouterState::new(service),
        schema_resolver: Arc::new(NomiCorePluginSchemaResolver { artifacts }),
        runtime_participant,
    })
}

struct BoundPluginBuildExecutor {
    runtime: ResolvedNodeRuntime,
    executor: Arc<FsPluginBuildExecutor>,
}

struct RuntimeBoundPluginBuildExecutor {
    runtime: Arc<dyn CommittedRuntimeProvider>,
    source_store: Arc<FsPluginSourceStore>,
    artifact_store: Arc<FsPluginArtifactStore>,
    npm_cache_root: PathBuf,
    current: RwLock<Option<BoundPluginBuildExecutor>>,
}

impl RuntimeBoundPluginBuildExecutor {
    fn new(
        runtime: Arc<dyn CommittedRuntimeProvider>,
        source_store: Arc<FsPluginSourceStore>,
        artifact_store: Arc<FsPluginArtifactStore>,
        npm_cache_root: PathBuf,
    ) -> Result<Self, PluginServiceError> {
        if !npm_cache_root.is_absolute() {
            return Err(PluginServiceError::integration(
                "Plugin npm cache root must be absolute",
            ));
        }
        Ok(Self {
            runtime,
            source_store,
            artifact_store,
            npm_cache_root,
            current: RwLock::new(None),
        })
    }

    fn executor(
        &self,
        runtime: &ResolvedNodeRuntime,
    ) -> Result<Arc<FsPluginBuildExecutor>, PluginServiceError> {
        Ok(Arc::new(FsPluginBuildExecutor::new(
            &self.source_store,
            &self.artifact_store,
            FixedPluginPacker::new(NodeBuildHost::new(
                &runtime.executable_path,
                Duration::from_secs(120),
            )
            .map_err(|error| {
                PluginServiceError::integration(format!(
                    "cannot bind Plugin Build Host to committed Runtime: {error}"
                ))
            })?)
            .with_npm_cache(ContentAddressedNpmCache::new(
                &self.npm_cache_root,
            )
            .map_err(|error| {
                PluginServiceError::integration(format!(
                    "cannot initialize Plugin npm cache: {error}"
                ))
            })?),
            runtime.fingerprint.runtime_target.clone(),
        )?))
    }

    async fn executor_for_use(
        &self,
    ) -> Result<(RuntimeUseLease, Arc<FsPluginBuildExecutor>), PluginServiceError>
    {
        let lease = self
            .runtime
            .acquire_use(JavaScriptWorkKind::BuildHost)
            .await
            .map_err(|error| PluginServiceError::Coded {
                code: nomifun_plugin_service::ERR_RUNTIME,
                message: error.to_string(),
            })?;
        let resolved = lease.runtime().clone();
        let mut current = self.current.write().await;
        if let Some(current) = current.as_ref() {
            if current.runtime == resolved {
                return Ok((lease, Arc::clone(&current.executor)));
            }
            return Err(PluginServiceError::Coded {
                code: nomifun_plugin_service::ERR_RUNTIME,
                message:
                    "committed Runtime changed before the Build executor was fenced"
                        .to_owned(),
            });
        }
        let executor = self.executor(&resolved)?;
        *current = Some(BoundPluginBuildExecutor {
            runtime: resolved,
            executor: Arc::clone(&executor),
        });
        Ok((lease, executor))
    }

    async fn stop_for_runtime_switch(&self) {
        *self.current.write().await = None;
    }

    async fn prepare_runtime(
        &self,
        runtime: Option<&ResolvedNodeRuntime>,
    ) -> Result<(), JavaScriptRuntimeError> {
        let next = match runtime {
            Some(runtime) => Some(BoundPluginBuildExecutor {
                runtime: runtime.clone(),
                executor: self.executor(runtime).map_err(|error| {
                    JavaScriptRuntimeError::FoundationValidationFailed(
                        error.to_string(),
                    )
                })?,
            }),
            None => None,
        };
        *self.current.write().await = next;
        Ok(())
    }
}

#[async_trait]
impl PluginBuildExecutor for RuntimeBoundPluginBuildExecutor {
    async fn build(
        &self,
        operation_id: &str,
        project: &PluginProjectRow,
        request: &BuildPluginProjectRequest,
    ) -> Result<nomifun_plugin_service::BuildOutput, PluginServiceError> {
        let (_lease, executor) = self.executor_for_use().await?;
        executor.build(operation_id, project, request).await
    }
}

#[async_trait]
impl PluginOperationCancellation for RuntimeBoundPluginBuildExecutor {
    async fn cancel(
        &self,
        operation: &nomifun_db::ProductOperationRow,
    ) -> Result<(), PluginServiceError> {
        let executor = self
            .current
            .read()
            .await
            .as_ref()
            .map(|current| Arc::clone(&current.executor))
            .ok_or_else(|| {
                PluginServiceError::conflict(
                    "Plugin Build has no active Runtime-bound executor",
                )
            })?;
        executor.cancel(operation).await
    }
}

struct NomiCorePluginSchemaResolver {
    artifacts: Arc<FsPluginArtifactStore>,
}

#[async_trait]
impl NomiPluginToolSchemaResolver for NomiCorePluginSchemaResolver {
    async fn resolve(
        &self,
        capability: &ResolvedCapability,
        reference: &CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String> {
        if capability.contribution_lock.source_kind
            != nomifun_agent_contracts::ContributionSourceKind::PluginMount
            || capability.contribution_lock.mount_id.as_ref()
                != Some(&capability.resolved_mount_id)
        {
            return Err(
                "ordinary Plugin Tool schema requires an exact Plugin Mount lock".into(),
            );
        }
        let stored = self
            .artifacts
            .store()
            .load(&capability.target_artifact_digest)
            .map_err(|error| error.to_string())?;
        let manifest = &stored.artifact.manifest.payload;
        if stored.artifact.artifact_digest != capability.target_artifact_digest
            || manifest.package_ref() != capability.source_package
        {
            return Err(
                "Plugin Tool schema Artifact differs from the frozen Snapshot target".into(),
            );
        }
        let materialized = manifest
            .package
            .contributions
            .capabilities
            .iter()
            .find(|candidate| {
                candidate.id == capability.capability.id
                    && candidate.version == capability.capability.version
                    && candidate.contribution_id == capability.contribution_id
            })
            .ok_or_else(|| {
                "Plugin Tool schema Artifact has no matching Capability contribution".to_owned()
            })?;
        let materialized_digest = nomifun_agent_contracts::digest_payload(materialized)
            .map_err(|error| error.to_string())?;
        if materialized_digest != capability.schema_digest
            || !materialized.contributions.actions.iter().any(|action| {
                &action.input_schema == reference || &action.output_schema == reference
            })
        {
            return Err(
                "Plugin Tool schema ref is not owned by the frozen Capability contract".into(),
            );
        }
        manifest
            .schemas
            .get(reference)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Plugin Tool Artifact is missing canonical schema {}",
                    reference.as_ref()
                )
            })
    }
}

pub(crate) fn plugin_routes(state: PluginRouterState) -> Router {
    Router::new()
        .route("/api/plugins", get(list_plugins))
        .route("/api/plugin-projects", post(create_project))
        .route(
            "/api/plugin-projects/{project_id}",
            get(get_project).delete(delete_project),
        )
        .route(
            "/api/plugin-projects/{project_id}/source/edit",
            post(apply_source_edit),
        )
        .route("/api/plugin-imports", post(import_prebuilt))
        .route(
            "/api/plugin-projects/{project_id}/build",
            post(build_project),
        )
        .route(
            "/api/plugin-projects/{project_id}/test",
            post(test_candidate),
        )
        .route(
            "/api/plugin-projects/{project_id}/apply",
            post(apply_candidate),
        )
        .route(
            "/api/plugin-projects/{project_id}/candidate/discard",
            post(discard_candidate),
        )
        .route(
            "/api/plugin-mounts/{mount_id}",
            get(get_mount),
        )
        .route(
            "/api/plugin-mounts/{mount_id}/config",
            put(configure_mount),
        )
        .route(
            "/api/plugin-mounts/{mount_id}/enabled",
            put(set_mount_enabled),
        )
        .route(
            "/api/plugin-mounts/{mount_id}/retry",
            post(retry_mount),
        )
        .route(
            "/api/plugin-mounts/{mount_id}/restore",
            post(restore_mount),
        )
        .route(
            "/api/plugin-mounts/{mount_id}/uninstall",
            post(uninstall_mount),
        )
        .route(
            "/api/plugin-mounts/{mount_id}/data",
            delete(delete_mount_data),
        )
        .route("/api/plugin-operations", get(list_operations))
        .route(
            "/api/plugin-operations/{operation_id}",
            get(get_operation),
        )
        .route(
            "/api/plugin-operations/{operation_id}/cancel",
            post(cancel_operation),
        )
        .with_state(state)
}

#[derive(Debug)]
struct PluginHttpError(PluginServiceError);

impl From<PluginServiceError> for PluginHttpError {
    fn from(value: PluginServiceError) -> Self {
        Self(value)
    }
}

impl IntoResponse for PluginHttpError {
    fn into_response(self) -> Response {
        let code = self.0.code();
        let status = match code {
            nomifun_plugin_service::ERR_INVALID_INPUT => StatusCode::BAD_REQUEST,
            nomifun_plugin_service::ERR_FORBIDDEN => StatusCode::FORBIDDEN,
            nomifun_plugin_service::ERR_NOT_FOUND => StatusCode::NOT_FOUND,
            nomifun_plugin_service::ERR_STALE
            | nomifun_plugin_service::ERR_CONFLICT
            | nomifun_plugin_service::ERR_OPERATION_CANCELED => StatusCode::CONFLICT,
            nomifun_plugin_service::ERR_ARTIFACT => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            nomifun_plugin_service::ERR_RUNTIME
            | nomifun_plugin_service::ERR_RECONCILE_REQUIRED
            | nomifun_plugin_service::ERR_INTEGRATION => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(ErrorResponse::new(self.0.to_string(), code)),
        )
            .into_response()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CancelPluginOperationRequest {
    expected_operation_revision: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyPluginSourceEditHttpRequest {
    project_id: String,
    expected_source_snapshot_digest: String,
    edit: ApplyPluginSourceEditHttp,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ApplyPluginSourceEditHttp {
    Replace { path: String, content: String },
    Delete { path: String },
}

impl ApplyPluginSourceEditHttpRequest {
    fn into_service(
        self,
    ) -> Result<ApplyPluginSourceEditRequest, PluginServiceError> {
        let edit = match self.edit {
            ApplyPluginSourceEditHttp::Replace { path, content } => {
                PluginSourceFileEdit::Replace {
                    path,
                    bytes: content.into_bytes(),
                }
            }
            ApplyPluginSourceEditHttp::Delete { path } => {
                PluginSourceFileEdit::Delete { path }
            }
        };
        Ok(ApplyPluginSourceEditRequest {
            project_id: self.project_id,
            expected_source_snapshot_digest: self
                .expected_source_snapshot_digest,
            edit,
        })
    }
}

async fn list_plugins(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<PluginLibraryResponseDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(
        state.service.list_library(user.id.as_str()).await?,
    )))
}

async fn get_project(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(project_id): AxumPath<String>,
) -> Result<Json<ApiResponse<PluginProjectDetailDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(
        state
            .service
            .get_project(user.id.as_str(), &project_id)
            .await?,
    )))
}

async fn apply_source_edit(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(project_id): AxumPath<String>,
    Json(request): Json<ApplyPluginSourceEditHttpRequest>,
) -> Result<Json<ApiResponse<PluginProjectDetailDto>>, PluginHttpError> {
    require_route_id("project_id", &project_id, &request.project_id)?;
    let request = request.into_service()?;
    Ok(Json(ApiResponse::ok(
        state
            .service
            .apply_source_edit(ApplyPluginSourceEditInput {
                owner_user_id: user.id.as_str().to_owned(),
                request,
            })
            .await?,
    )))
}

async fn delete_project(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(project_id): AxumPath<String>,
    Json(request): Json<DeletePluginProjectRequest>,
) -> Result<Json<ApiResponse<bool>>, PluginHttpError> {
    require_route_id("project_id", &project_id, &request.project_id)?;
    Ok(Json(ApiResponse::ok(
        state
            .service
            .delete_project(user.id.as_str(), request)
            .await?,
    )))
}

async fn create_project(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<CreatePluginProjectRequest>,
) -> Result<Json<ApiResponse<PluginProjectDetailDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(
        state
            .service
            .create_project(nomifun_plugin_service::CreateProjectInput {
                owner_user_id: user.id.as_str().to_owned(),
                request,
            })
            .await?,
    )))
}

async fn import_prebuilt(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<ImportPluginRequest>,
) -> Result<Json<ApiResponse<PluginProjectDetailDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(
        state
            .service
            .import_prebuilt(user.id.as_str(), request)
            .await?,
    )))
}

async fn build_project(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(project_id): AxumPath<String>,
    Json(request): Json<BuildPluginProjectRequest>,
) -> Result<Json<ApiResponse<PluginProjectDetailDto>>, PluginHttpError> {
    require_route_id("project_id", &project_id, &request.project_id)?;
    Ok(Json(ApiResponse::ok(
        state.service.build(user.id.as_str(), request).await?,
    )))
}

async fn test_candidate(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(project_id): AxumPath<String>,
    Json(request): Json<TestPluginCandidateRequest>,
) -> Result<Json<ApiResponse<PluginProjectDetailDto>>, PluginHttpError> {
    require_route_id("project_id", &project_id, &request.project_id)?;
    Ok(Json(ApiResponse::ok(
        state
            .service
            .test_candidate(user.id.as_str(), request)
            .await?,
    )))
}

async fn apply_candidate(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(project_id): AxumPath<String>,
    Json(request): Json<ApplyPluginCandidateRequest>,
) -> Result<Json<ApiResponse<PluginDetailDto>>, PluginHttpError> {
    require_route_id("project_id", &project_id, &request.project_id)?;
    Ok(Json(ApiResponse::ok(
        state
            .service
            .apply_candidate(user.id.as_str(), request)
            .await?,
    )))
}

async fn discard_candidate(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(project_id): AxumPath<String>,
    Json(request): Json<DiscardPluginCandidateRequest>,
) -> Result<Json<ApiResponse<PluginProjectDetailDto>>, PluginHttpError> {
    require_route_id("project_id", &project_id, &request.project_id)?;
    Ok(Json(ApiResponse::ok(
        state
            .service
            .discard_candidate(user.id.as_str(), request)
            .await?,
    )))
}

async fn get_mount(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(mount_id): AxumPath<String>,
) -> Result<Json<ApiResponse<PluginDetailDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(
        state
            .service
            .get_mount(user.id.as_str(), &mount_id)
            .await?,
    )))
}

async fn configure_mount(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(mount_id): AxumPath<String>,
    Json(request): Json<ConfigurePluginRequest>,
) -> Result<Json<ApiResponse<PluginDetailDto>>, PluginHttpError> {
    require_route_id("mount_id", &mount_id, &request.mount_id)?;
    Ok(Json(ApiResponse::ok(
        state
            .service
            .configure(nomifun_plugin_service::ConfigureInput {
                owner_user_id: user.id.as_str().to_owned(),
                request,
            })
            .await?,
    )))
}

async fn set_mount_enabled(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(mount_id): AxumPath<String>,
    Json(request): Json<SetPluginEnabledRequest>,
) -> Result<Json<ApiResponse<PluginDetailDto>>, PluginHttpError> {
    require_route_id("mount_id", &mount_id, &request.mount_id)?;
    Ok(Json(ApiResponse::ok(
        state
            .service
            .set_enabled(user.id.as_str(), request)
            .await?,
    )))
}

async fn retry_mount(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(mount_id): AxumPath<String>,
    Json(request): Json<RetryPluginRequest>,
) -> Result<Json<ApiResponse<PluginDetailDto>>, PluginHttpError> {
    require_route_id("mount_id", &mount_id, &request.mount_id)?;
    Ok(Json(ApiResponse::ok(
        state.service.retry(user.id.as_str(), request).await?,
    )))
}

async fn restore_mount(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(mount_id): AxumPath<String>,
    Json(request): Json<RestorePluginPreviousRequest>,
) -> Result<Json<ApiResponse<PluginDetailDto>>, PluginHttpError> {
    require_route_id("mount_id", &mount_id, &request.mount_id)?;
    Ok(Json(ApiResponse::ok(
        state.service.restore(user.id.as_str(), request).await?,
    )))
}

async fn uninstall_mount(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(mount_id): AxumPath<String>,
    Json(request): Json<UninstallPluginRequest>,
) -> Result<Json<ApiResponse<PluginDetailDto>>, PluginHttpError> {
    require_route_id("mount_id", &mount_id, &request.mount_id)?;
    Ok(Json(ApiResponse::ok(
        state
            .service
            .uninstall(user.id.as_str(), request)
            .await?,
    )))
}

async fn delete_mount_data(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(mount_id): AxumPath<String>,
    Json(request): Json<DeletePluginDataRequest>,
) -> Result<Json<ApiResponse<()>>, PluginHttpError> {
    require_route_id("mount_id", &mount_id, &request.mount_id)?;
    state
        .service
        .delete_data(user.id.as_str(), request)
        .await?;
    Ok(Json(ApiResponse::success()))
}

async fn list_operations(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<
    Json<ApiResponse<Vec<DurableOperationSummaryDto>>>,
    PluginHttpError,
> {
    Ok(Json(ApiResponse::ok(
        state.service.list_operations(user.id.as_str()).await?,
    )))
}

async fn get_operation(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(operation_id): AxumPath<String>,
) -> Result<Json<ApiResponse<DurableOperationDetailDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(
        state
            .service
            .get_operation(user.id.as_str(), &operation_id)
            .await?,
    )))
}

async fn cancel_operation(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(operation_id): AxumPath<String>,
    Json(request): Json<CancelPluginOperationRequest>,
) -> Result<Json<ApiResponse<DurableOperationSummaryDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(
        state
            .service
            .cancel_operation(
                user.id.as_str(),
                &operation_id,
                request.expected_operation_revision,
            )
            .await?,
    )))
}

fn require_route_id(
    field: &'static str,
    route: &str,
    body: &str,
) -> Result<(), PluginHttpError> {
    if route == body {
        Ok(())
    } else {
        Err(PluginServiceError::invalid(format!(
            "{field} must match the route identity"
        ))
        .into())
    }
}

struct RuntimeBoundPluginHostCoordinator {
    host: Arc<super::plugin_runtime_host::RuntimeBoundExtensionHost>,
}

#[async_trait]
impl PluginHostCoordinator for RuntimeBoundPluginHostCoordinator {
    async fn runtime_available(&self) -> Result<bool, PluginServiceError> {
        self.host.runtime_available().await
    }

    async fn commit_fence(
        &self,
        mount_id: &str,
    ) -> Result<PluginHostCommitFence, PluginServiceError> {
        self.host
            .commit_fence_for_mount(&PluginMountId::from(mount_id.to_owned()))
            .await
            .map_err(Into::into)
    }
}

struct CandidateTestDataGuard {
    path: PathBuf,
}

impl CandidateTestDataGuard {
    fn allocate(root: &Path, candidate_id: &str) -> Result<Self, PluginServiceError> {
        std::fs::create_dir_all(root).map_err(|error| {
            PluginServiceError::integration(format!(
                "cannot create Candidate Test data root {}: {error}",
                root.display()
            ))
        })?;
        let root = std::fs::canonicalize(root).map_err(|error| {
            PluginServiceError::integration(format!(
                "cannot canonicalize Candidate Test data root {}: {error}",
                root.display()
            ))
        })?;
        let path = root.join(format!("{candidate_id}-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir(&path).map_err(|error| {
            PluginServiceError::integration(format!(
                "cannot allocate Candidate Test data directory {}: {error}",
                path.display()
            ))
        })?;
        let path = std::fs::canonicalize(&path).map_err(|error| {
            PluginServiceError::integration(format!(
                "cannot canonicalize Candidate Test data directory {}: {error}",
                path.display()
            ))
        })?;
        if path.parent() != Some(root.as_path()) {
            return Err(PluginServiceError::conflict(
                "Candidate Test data directory escaped its managed root",
            ));
        }
        Ok(Self { path })
    }
}

impl Drop for CandidateTestDataGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

struct NomiCorePluginCandidateTestExecutor {
    repository: Arc<DbPluginRepositoryAdapter>,
    artifacts: Arc<FsPluginArtifactStore>,
    runtime: Arc<dyn CommittedRuntimeProvider>,
    host_module: PathBuf,
    candidate_test_root: PathBuf,
}

#[async_trait]
impl PluginCandidateTestExecutor for NomiCorePluginCandidateTestExecutor {
    async fn test(
        &self,
        project: &PluginProjectRow,
        candidate: &PluginReadyCandidateRow,
        request: &TestPluginCandidateRequest,
    ) -> Result<CandidateTestOutput, PluginServiceError> {
        let runtime_lease = self
            .runtime
            .acquire_use(JavaScriptWorkKind::CandidateTestHost)
            .await
            .map_err(|error| PluginServiceError::Coded {
                code: nomifun_plugin_service::ERR_RUNTIME,
                message: error.to_string(),
            })?;
        let host_config = JavaScriptHostConfig::for_host_module(
            runtime_lease.executable_path().to_path_buf(),
            runtime_lease.fingerprint().clone(),
            self.host_module.clone(),
        );
        let artifact_row = self
            .repository
            .get_artifact(&candidate.artifact_digest)
            .await?
            .ok_or_else(|| PluginServiceError::not_found("Candidate Test Artifact"))?;
        self.artifacts.verify(&artifact_row).await?;
        let stored = self
            .artifacts
            .store()
            .load(&DigestHex::from(candidate.artifact_digest.clone()))?;
        if stored.artifact.artifact_id.as_ref() != candidate.artifact_id
            || stored.artifact.artifact_digest.as_ref() != candidate.artifact_digest
            || stored.artifact.manifest.payload.package.package_id.as_ref()
                != project.package_id
        {
            return Err(PluginServiceError::stale(
                "Candidate Test Artifact differs from the exact Project Candidate",
            ));
        }
        let manifest = &stored.artifact.manifest.payload;
        let (mount_id, config_revision, config_value) =
            match project.linked_mount_id.as_deref() {
                Some(mount_id) => {
                    let mount = self
                        .repository
                        .get_mount(mount_id)
                        .await?
                        .ok_or_else(|| {
                            PluginServiceError::not_found(
                                "Candidate Test linked Mount",
                            )
                        })?;
                    if u64::try_from(mount.config_revision).ok()
                        != Some(request.expected_config_revision)
                    {
                        return Err(PluginServiceError::stale(
                            "Candidate Test config revision changed",
                        ));
                    }
                    let value = serde_json::from_str(&mount.config_json).map_err(|error| {
                        PluginServiceError::integration(format!(
                            "persisted Candidate Test config is invalid JSON: {error}"
                        ))
                    })?;
                    (
                        PluginMountId::from(mount.mount_id),
                        request.expected_config_revision,
                        value,
                    )
                }
                None => (
                    PluginMountId::from(uuid::Uuid::now_v7().to_string()),
                    0,
                    serde_json::json!({}),
                ),
            };
        let data = CandidateTestDataGuard::allocate(
            &self.candidate_test_root,
            &candidate.candidate_id,
        )?;
        let config_schema_digest = nomifun_agent_contracts::digest_payload(
            &manifest.package.config_schema,
        )
        .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
        let adapter = JsKernelPluginAdapter::new(PluginPackageInput {
            artifact: stored.artifact,
            mount_id,
            package_root: stored.package_root,
            config: ValidatedPluginConfig {
                schema_digest: config_schema_digest,
                config_revision,
                value: StrictJsonValue(config_value),
            },
            credential_bindings: Vec::new(),
            data_dir: data.path.clone(),
        })
        .map_err(|error| {
            PluginServiceError::integration(format!(
                "Candidate Test adapter rejected the Artifact: {error}"
            ))
        })?;
        let host = ExtensionHostSupervisor::candidate_test(host_config.clone())
            .map_err(|error| {
                PluginServiceError::integration(format!(
                    "Candidate Test Host configuration failed: {error}"
                ))
            })?;
        let generation = host.load_mount(adapter.mount_demand()).await?;
        let requires_managed_input = {
            let contributions = &adapter.manifest().package.contributions;
            !contributions.capabilities.is_empty()
                || !contributions.skills.is_empty()
                || !contributions.mcp_tools.is_empty()
        };
        host.stop_generation(generation).await?;
        let source_lineage = match (
            candidate.source_snapshot_digest.as_deref(),
            candidate.dependency_lock_digest.as_deref(),
        ) {
            (Some(source), Some(lock)) => PluginSourceLineage::Managed {
                source_snapshot_digest: source.to_owned().into(),
                dependency_lock_digest: lock.to_owned().into(),
                build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
            },
            (None, None) => PluginSourceLineage::RuntimeOnly,
            _ => {
                return Err(PluginServiceError::integration(
                    "Candidate Test source lineage is incomplete",
                ));
            }
        };
        Ok(CandidateTestOutput {
            receipt: CandidateTestReceipt {
                receipt_id: CandidateTestReceiptId::from(
                    uuid::Uuid::now_v7().to_string(),
                ),
                candidate_id: candidate.candidate_id.clone().into(),
                candidate_digest: candidate.candidate_digest.clone().into(),
                outcome: if requires_managed_input {
                    CandidateTestOutcome::NeedsTestInput
                } else {
                    CandidateTestOutcome::Passed
                },
                runtime: host_config.runtime.clone(),
                host_target: host_config.runtime.runtime_target.clone(),
                host_contract_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                javascript_sdk_contract_version: JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
                test_contract_version: CANDIDATE_TEST_CONTRACT_VERSION.into(),
                source_lineage,
                credential_mode: CandidateTestCredentialMode::None,
                resolved_test_input_digest: request
                    .resolved_test_input_digest
                    .clone()
                    .into(),
                host_generation: generation,
                issued_at_ms: nomifun_common::now_ms(),
            },
        })
    }
}

struct NomiCorePluginRegistryPublisher {
    kernel: Arc<KernelRegistry>,
    catalog: Arc<KernelCatalogProvider>,
    base_registrations: Vec<PluginRegistration>,
    dynamic_registrations:
        RwLock<BTreeMap<PluginMountId, PluginRegistration>>,
    publish_lock: Mutex<()>,
    repository: Arc<DbPluginRepositoryAdapter>,
    artifacts: Arc<FsPluginArtifactStore>,
    host: Arc<dyn ExtensionHostDemandPort>,
    runtime: Arc<dyn CommittedRuntimeProvider>,
    data_root: PathBuf,
}

impl NomiCorePluginRegistryPublisher {
    async fn restore(
        &self,
        owner_user_id: &str,
    ) -> Result<(), PluginServiceError> {
        let inventory = self.repository.inventory(owner_user_id).await?;
        let mut candidates = BTreeMap::new();
        for mount in inventory.mounts {
            if !mount_is_materializable(&mount) {
                continue;
            }
            match self.registration_for(&mount).await {
                Ok(registration) => {
                    candidates.insert(
                        PluginMountId::from(mount.mount_id.clone()),
                        registration,
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        mount_id = mount.mount_id,
                        code = error.code(),
                        %error,
                        "Plugin Mount restore remains unavailable"
                    );
                }
            }
        }
        let _guard = self.publish_lock.lock().await;
        self.publish(BTreeMap::new()).await?;
        for (mount_id, registration) in candidates {
            let mut next = self.dynamic_registrations.read().await.clone();
            next.insert(mount_id.clone(), registration);
            if let Err(error) = self.publish(next).await {
                tracing::warn!(
                    mount_id = mount_id.as_ref(),
                    code = error.code(),
                    %error,
                    "Plugin Mount registration conflicts with the active Kernel generation"
                );
            }
        }
        Ok(())
    }

    async fn registration_for(
        &self,
        mount: &PluginMountRow,
    ) -> Result<PluginRegistration, PluginServiceError> {
        let host = Arc::clone(&self.host);
        plugin_adapter_for_mount(
            &self.repository,
            &self.artifacts,
            &self.data_root,
            mount,
            None,
            true,
        )
        .await?
        .registration(host)
        .map_err(|error| {
            PluginServiceError::integration(format!(
                "JavaScript Plugin registration failed: {error}"
            ))
        })
    }

    async fn publish(
        &self,
        dynamic: BTreeMap<PluginMountId, PluginRegistration>,
    ) -> Result<(), PluginServiceError> {
        let mut registrations = self.base_registrations.clone();
        registrations.extend(dynamic.values().cloned());
        self.kernel.replace_all(registrations).map_err(|error| {
            PluginServiceError::integration(format!(
                "Kernel Plugin publication failed: {error}"
            ))
        })?;
        *self.dynamic_registrations.write().await = dynamic;
        self.refresh_agent_availability().await
    }

    async fn refresh_agent_availability(
        &self,
    ) -> Result<(), PluginServiceError> {
        let runtime_available = self
            .runtime
            .committed_runtime()
            .await
            .map_err(|error| PluginServiceError::Coded {
                code: nomifun_plugin_service::ERR_RUNTIME,
                message: error.to_string(),
            })?
            .is_some();
        let registry = self.kernel.snapshot().map_err(|error| {
            PluginServiceError::integration(format!(
                "Kernel Plugin snapshot failed: {error}"
            ))
        })?;
        let unavailable = registry
            .capabilities
            .values()
            .filter(|capability| {
                capability
                    .manifest
                    .supports_consumer(CapabilityConsumer::Agent)
            })
            .filter_map(|capability| {
                let native =
                    super::nomi_core_agent_projection::nomi_capability_projection(
                        capability.manifest.id.as_ref(),
                    )
                    .is_ok();
                let dynamic = capability.source.source_kind
                    == nomifun_agent_contracts::PluginSourceKind::ManagedLocal
                    && capability.contribution_lock.source_kind
                        == nomifun_agent_contracts::ContributionSourceKind::PluginMount
                    && capability.manifest.kind
                        == nomifun_agent_contracts::CapabilityKind::Tool
                    && capability
                        .manifest
                        .contributions
                        .actions
                        .iter()
                        .any(|action| {
                            action.presentation
                                == nomifun_agent_contracts::ToolPresentationKind::FunctionTool
                        });
                (!native && (!dynamic || !runtime_available)).then(|| {
                    (
                        capability.manifest.id.clone(),
                        CanonicalErrorCode::from(AGENT_EXECUTOR_UNAVAILABLE),
                    )
                })
            });
        self.catalog
            .replace_unavailable_capabilities(unavailable)
            .map_err(|error| {
                PluginServiceError::integration(format!(
                    "Agent Catalog availability update failed: {error}"
                ))
            })
    }

    async fn remove_stale_registration(
        &self,
        mount_id: &PluginMountId,
    ) -> Result<(), PluginServiceError> {
        let mut fallback = self.dynamic_registrations.read().await.clone();
        fallback.remove(mount_id);
        self.publish(fallback).await
    }
}

#[async_trait]
impl PluginRegistryPublisher for NomiCorePluginRegistryPublisher {
    async fn reconcile_mount(
        &self,
        _owner_user_id: &str,
        mount: &PluginMountRow,
    ) -> Result<(), PluginServiceError> {
        let mount_id = PluginMountId::from(mount.mount_id.clone());
        let registration = if mount_is_materializable(mount) {
            Some(self.registration_for(mount).await)
        } else {
            None
        };
        let _guard = self.publish_lock.lock().await;
        self.kernel
            .release_resources_for_mount(&mount_id)
            .await
            .map_err(|error| {
                PluginServiceError::integration(format!(
                    "Plugin resource cleanup failed: {error}"
                ))
            })?;

        let mut next = self.dynamic_registrations.read().await.clone();
        next.remove(&mount_id);
        match registration {
            Some(Ok(registration)) => {
                next.insert(mount_id.clone(), registration);
            }
            Some(Err(error)) => {
                self.publish(next).await?;
                return Err(error);
            }
            None => {}
        }
        if let Err(error) = self.publish(next).await {
            self.remove_stale_registration(&mount_id).await?;
            return Err(error);
        }
        Ok(())
    }
}

fn mount_is_materializable(mount: &PluginMountRow) -> bool {
    mount.enabled
        && !mount.retained
        && !mount.delete_pending
        && mount.current_artifact_digest.is_some()
        && mount.last_error.is_none()
}

async fn resolve_mount_data_dir(
    root: &Path,
    managed_relative_path: &str,
    mount_id: &str,
) -> Result<PathBuf, PluginServiceError> {
    let relative = Path::new(managed_relative_path);
    if relative.is_absolute()
        || managed_relative_path.is_empty()
        || managed_relative_path.contains('\\')
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || relative.file_name().and_then(|value| value.to_str())
            != Some(mount_id)
    {
        return Err(PluginServiceError::invalid(
            "Plugin data directory is not a managed Mount path",
        ));
    }
    let requested = root.join(relative);
    tokio::fs::create_dir_all(&requested).await.map_err(|error| {
        PluginServiceError::integration(format!(
            "cannot create Plugin data directory {}: {error}",
            requested.display()
        ))
    })?;
    let resolved = std::fs::canonicalize(&requested).map_err(|error| {
        PluginServiceError::integration(format!(
            "cannot canonicalize Plugin data directory {}: {error}",
            requested.display()
        ))
    })?;
    if resolved == root || !resolved.starts_with(root) {
        return Err(PluginServiceError::conflict(
            "Plugin data directory escaped the application data root",
        ));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    use async_trait::async_trait;
    use nomifun_agent_contracts::{
        ActionId, ArtifactEnvelope, ArtifactFileDigest, ArtifactId,
        CapabilityActionDescriptor, CapabilityContributions, CapabilityId,
        CapabilityKind, CapabilityManifest, CapabilityRef, CanonicalSchemaRef,
        CorrelationId, EffectClass, IdempotencyKey, OperationId, PrincipalRef,
        ScopeKey,
        ExactVersionRef, JAVASCRIPT_HOST_PROTOCOL_VERSION,
        JAVASCRIPT_SDK_CONTRACT_VERSION, JavaScriptBuildProfile,
        JavaScriptEntrypointMetadata, LocalizedMetadata, MINIMUM_NODE_MAJOR,
        PLUGIN_N1_SCHEMA_VERSION, PLUGIN_PACKAGE_PROFILE_VERSION,
        PackageContributions, PackageId, PackageManifest,
        PlatformConstraint, PluginPackageArtifactV1,
        PluginPackageV1Manifest, PluginSourceKind, RuntimeTarget,
        ToolPresentationKind, VersionString, canonical_json_bytes,
        capability_surface_declarations,
    };
    use nomifun_agent_control_plane::{CatalogProvider, CatalogSnapshot};
    use nomifun_agent_kernel::{
        CapabilityOperationRequest, InMemoryPluginStatePersistence,
        MaterializationPolicy, PluginStatePersistence,
    };
    use nomifun_api_types::{
        ApplyPluginCandidateRequest, ApplyPluginTargetDto,
        CreatePluginProjectRequest, ImportPluginRequest,
        PluginImportKindDto, PluginProjectLanguageDto,
        PluginProjectSourceStateDto, UninstallPluginRequest,
    };
    use nomifun_plugin_service::CreateProjectInput;
    use nomifun_js_runtime::{
        NodeDiscoveryRequest, NodeRuntimeManager, NodeRuntimeResolver,
        RuntimeAuthority, RuntimeSelectionStore, RuntimeSelectionStoreError,
        SystemNodeRuntimeProbePort, VersionedRuntimeSelection,
    };
    use sha2::{Digest, Sha256};
    use tokio::sync::Mutex;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    struct TestRuntimeStore {
        value: Mutex<VersionedRuntimeSelection>,
    }

    impl Default for TestRuntimeStore {
        fn default() -> Self {
            Self {
                value: Mutex::new(VersionedRuntimeSelection::empty()),
            }
        }
    }

    #[async_trait]
    impl RuntimeSelectionStore for TestRuntimeStore {
        async fn load(
            &self,
        ) -> Result<VersionedRuntimeSelection, RuntimeSelectionStoreError> {
            Ok(self.value.lock().await.clone())
        }

        async fn save_cas(
            &self,
            expected_revision: u64,
            selection: &nomifun_agent_contracts::RuntimeSelectionRecord,
            selected_executable_path: Option<&Path>,
            pending_candidate_executable_path: Option<&Path>,
            updated_at_ms: i64,
        ) -> Result<VersionedRuntimeSelection, RuntimeSelectionStoreError> {
            let mut value = self.value.lock().await;
            if value.revision != expected_revision {
                return Err(RuntimeSelectionStoreError::Conflict(
                    "test Runtime selection revision changed".to_owned(),
                ));
            }
            *value = VersionedRuntimeSelection {
                selection: selection.clone(),
                selected_executable_path:
                    selected_executable_path.map(Path::to_path_buf),
                pending_candidate_executable_path:
                    pending_candidate_executable_path.map(Path::to_path_buf),
                revision: expected_revision + 1,
                updated_at_ms,
            };
            Ok(value.clone())
        }
    }

    async fn test_runtime_authority() -> Arc<RuntimeAuthority> {
        let resolution = NodeRuntimeResolver::default()
            .resolve(NodeDiscoveryRequest::default())
            .await
            .unwrap();
        let fingerprint = resolution
            .selected
            .expect("Plugin E2E requires a compatible PATH Node");
        let executable_path = resolution
            .probes
            .iter()
            .find(|probe| probe.fingerprint.as_ref() == Some(&fingerprint))
            .map(|probe| PathBuf::from(&probe.executable_path))
            .expect("selected PATH Node must retain executable evidence");
        let manager = Arc::new(NodeRuntimeManager::new(Arc::new(
            TestRuntimeStore::default(),
        )));
        manager
            .select_initial(0, fingerprint, executable_path)
            .await
            .unwrap();
        RuntimeAuthority::new(
            manager,
            Arc::new(SystemNodeRuntimeProbePort::default()),
        )
    }

    fn sha256(bytes: &[u8]) -> DigestHex {
        DigestHex::from(format!("{:x}", Sha256::digest(bytes)))
    }

    fn display(name: &str, description: &str) -> LocalizedMetadata {
        LocalizedMetadata {
            name: name.to_owned(),
            description: description.to_owned(),
            localized_names: BTreeMap::new(),
            localized_descriptions: BTreeMap::new(),
        }
    }

    fn package_artifact(main: &[u8]) -> PluginPackageArtifactV1 {
        let package = ExactVersionRef {
            id: PackageId::from("test.nomicore.plugin"),
            version: VersionString::from("1.0.0"),
        };
        let input_schema = StrictJsonValue(serde_json::json!({
            "additionalProperties": false,
            "properties": {"message": {"type": "string"}},
            "required": ["message"],
            "type": "object"
        }));
        let output_schema = StrictJsonValue(serde_json::json!({
            "additionalProperties": true,
            "type": "object"
        }));
        let input_ref = CanonicalSchemaRef::from(format!(
            "schema://test.nomicore.plugin/echo-input@1#{}",
            nomifun_agent_contracts::digest_payload(&input_schema.0)
                .unwrap()
                .as_ref()
        ));
        let output_ref = CanonicalSchemaRef::from(format!(
            "schema://test.nomicore.plugin/echo-output@1#{}",
            nomifun_agent_contracts::digest_payload(&output_schema.0)
                .unwrap()
                .as_ref()
        ));
        let capability = CapabilityManifest {
            id: CapabilityId::from("test.nomicore.plugin.echo"),
            contribution_id: "test.nomicore.plugin.echo.contribution".into(),
            version: VersionString::from("1.0.0"),
            kind: CapabilityKind::Tool,
            package: package.clone(),
            display: display("Echo", "NomiCore Plugin publisher fixture."),
            requires: Vec::new(),
            conflicts: Vec::new(),
            supported_surfaces: capability_surface_declarations(
                ["desktop"],
                [CapabilityConsumer::Agent, CapabilityConsumer::Gateway],
            ),
            requires_runtime_features: Vec::new(),
            supported_platforms: vec![PlatformConstraint::Any],
            config_schema: StrictJsonValue(serde_json::json!({
                "type": "object",
                "additionalProperties": false
            })),
            contributions: CapabilityContributions {
                actions: vec![CapabilityActionDescriptor {
                    action_id: ActionId::from(
                        "test.nomicore.plugin.echo.invoke",
                    ),
                    input_schema: input_ref.clone(),
                    output_schema: output_ref.clone(),
                    effect_class: EffectClass::Pure,
                    presentation: ToolPresentationKind::FunctionTool,
                }],
                ..Default::default()
            },
        };
        PluginPackageArtifactV1::new(
            ArtifactId::from(Uuid::now_v7().to_string()),
            PluginPackageV1Manifest {
                schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
                build_profile: JavaScriptBuildProfile::PluginPackageV1,
                build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
                package: PackageManifest {
                    schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
                    host_contract_version:
                        JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                    package_id: package.id,
                    package_version: package.version,
                    display: display(
                        "NomiCore Plugin",
                        "NomiCore Plugin publisher fixture.",
                    ),
                    package_dependencies: Vec::new(),
                    requires_runtime_features: Vec::new(),
                    config_schema: StrictJsonValue(serde_json::json!({
                        "type": "object",
                        "additionalProperties": false
                    })),
                    provides_services: Vec::new(),
                    requires_services: Vec::new(),
                    entrypoint: JavaScriptEntrypointMetadata {
                        normalized_relative_path: "main.mjs".into(),
                        module_digest: sha256(main),
                        host_protocol_version:
                            JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                        sdk_contract_version:
                            JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
                    }
                    .into(),
                    contributions: PackageContributions {
                        capabilities: vec![capability],
                        ..Default::default()
                    },
                },
                schemas: BTreeMap::from([
                    (input_ref, input_schema),
                    (output_ref, output_schema),
                ]),
                supported_targets: BTreeSet::from([RuntimeTarget::from(
                    "x86_64-pc-windows-msvc",
                )]),
                minimum_node_major: MINIMUM_NODE_MAJOR,
                dependency_lock_digest: DigestHex::from("d".repeat(64)),
                credential_slots: Vec::new(),
            },
            vec![ArtifactFileDigest {
                normalized_relative_path: "main.mjs".into(),
                digest: sha256(main),
                size_bytes: u64::try_from(main.len()).unwrap(),
            }],
        )
        .unwrap()
    }

    fn write_package(
        root: &Path,
        artifact: &PluginPackageArtifactV1,
        main: &[u8],
    ) {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(
            root.join("manifest.json"),
            canonical_json_bytes(&ArtifactEnvelope::new(
                artifact.manifest.payload.clone(),
            )
            .unwrap())
            .unwrap(),
        )
        .unwrap();
        std::fs::write(root.join("main.mjs"), main).unwrap();
    }

    #[tokio::test]
    async fn nomi_core_plugin_flow_materializes_and_withdraws_exact_mount() {
        let data_root = tempfile::tempdir().unwrap();
        let database = nomifun_db::init_database_memory().await.unwrap();
        let owner_user_id =
            nomifun_db::installation_owner_id(database.pool())
                .await
                .unwrap();
        let mut policy = MaterializationPolicy::stable("1.0.0");
        policy.allowed_sources.insert(PluginSourceKind::ManagedLocal);
        let kernel = Arc::new(
            KernelRegistry::new(
                policy,
                Arc::new(InMemoryPluginStatePersistence::new())
                    as Arc<dyn PluginStatePersistence>,
            )
            .unwrap(),
        );
        let catalog = Arc::new(KernelCatalogProvider::new(Arc::clone(
            &kernel,
        )));
        let composition = build_nomi_core_plugin_state(
            database.pool().clone(),
            data_root.path().to_path_buf(),
            &owner_user_id,
            Arc::clone(&kernel),
            Arc::clone(&catalog),
            Vec::new(),
            test_runtime_authority().await,
        )
        .await
        .unwrap();
        let schema_resolver = Arc::clone(&composition.schema_resolver);
        let state = composition.router;
        assert_eq!(state.service.list_library(&owner_user_id).await.unwrap().library_revision, 0);
        let response = plugin_routes(state.clone())
            .layer(Extension(CurrentUser {
                id: nomifun_common::UserId::parse(
                    owner_user_id.clone(),
                )
                .unwrap(),
                username: "owner".to_owned(),
            }))
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/plugins")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let scaffolded = state
            .service
            .create_project(CreateProjectInput {
                owner_user_id: owner_user_id.clone(),
                request: CreatePluginProjectRequest {
                    expected_library_revision: 0,
                    package_id: "test.scaffold.plugin".into(),
                    package_version: "0.1.0".into(),
                    display_name: "Scaffold Plugin".into(),
                    description: "TypeScript authoring fixture.".into(),
                    language: PluginProjectLanguageDto::TypeScript,
                    linked_mount_id: None,
                    expected_linked_mount_revision: None,
                    expected_linked_target_digest: None,
                },
            })
            .await
            .unwrap();
        assert_eq!(
            scaffolded.summary.source_state,
            PluginProjectSourceStateDto::Editable
        );
        assert!(scaffolded.source_snapshot_digest.is_some());
        assert!(scaffolded.dependency_lock_digest.is_some());
        let managed_source_path: String = sqlx::query_scalar(
            "SELECT managed_source_path FROM plugin_projects WHERE project_id = ?",
        )
        .bind(&scaffolded.summary.project_id)
        .fetch_one(database.pool())
        .await
        .unwrap();
        let source_root = data_root
            .path()
            .join(PLUGIN_PLATFORM_DIRECTORY)
            .join(PLUGIN_AUTHORING_DIRECTORY)
            .join(managed_source_path);
        assert!(source_root.join("src/main.ts").is_file());
        assert!(
            source_root
                .parent()
                .unwrap()
                .join("dependency-lock.json")
                .is_file()
        );

        let main = br#"
            export async function activate() {
              return {
                capabilities: {
                  "test.nomicore.plugin.echo.contribution": {
                    async invoke({ input }) { return input; }
                  }
                }
              };
            }
        "#;
        let artifact = package_artifact(main);
        let source = data_root.path().join("incoming-plugin");
        write_package(&source, &artifact, main);
        let mut project = state
            .service
            .import_prebuilt(
                &owner_user_id,
                ImportPluginRequest {
                    expected_library_revision: state
                        .service
                        .list_library(&owner_user_id)
                        .await
                        .unwrap()
                        .library_revision,
                    import_kind:
                        PluginImportKindDto::PrebuiltArtifact,
                    source_path: source.display().to_string(),
                    expected_bundle_or_artifact_digest: artifact
                        .artifact_digest
                        .as_ref()
                        .to_owned(),
                    target_project_id: None,
                    expected_project_revision: None,
                },
            )
            .await
            .unwrap();
        let candidate = project.ready.clone().unwrap();
        let discard_response = plugin_routes(state.clone())
            .layer(Extension(CurrentUser {
                id: nomifun_common::UserId::parse(owner_user_id.clone()).unwrap(),
                username: "owner".to_owned(),
            }))
            .oneshot(
                axum::http::Request::builder()
                    .method(axum::http::Method::POST)
                    .uri(format!(
                        "/api/plugin-projects/{}/candidate/discard",
                        project.summary.project_id
                    ))
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(
                        serde_json::to_vec(&DiscardPluginCandidateRequest {
                            project_id: project.summary.project_id.clone(),
                            expected_project_revision: project.summary.project_revision,
                            expected_build_generation: project.summary.build_generation,
                            candidate_id: candidate.candidate.candidate_id,
                            expected_candidate_digest: candidate.candidate.candidate_digest,
                        })
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(discard_response.status(), StatusCode::OK);
        let discarded = state
            .service
            .get_project(&owner_user_id, &project.summary.project_id)
            .await
            .unwrap();
        assert!(discarded.ready.is_none());
        assert_eq!(
            discarded.summary.build_generation,
            project.summary.build_generation
        );
        project = state
            .service
            .import_prebuilt(
                &owner_user_id,
                ImportPluginRequest {
                    expected_library_revision: state
                        .service
                        .list_library(&owner_user_id)
                        .await
                        .unwrap()
                        .library_revision,
                    import_kind: PluginImportKindDto::PrebuiltArtifact,
                    source_path: source.display().to_string(),
                    expected_bundle_or_artifact_digest: artifact
                        .artifact_digest
                        .as_ref()
                        .to_owned(),
                    target_project_id: Some(project.summary.project_id.clone()),
                    expected_project_revision: Some(discarded.summary.project_revision),
                },
            )
            .await
            .unwrap();
        let candidate = project.ready.clone().unwrap();
        let tested = state
            .service
            .test_candidate(
                &owner_user_id,
                TestPluginCandidateRequest {
                    project_id: project.summary.project_id.clone(),
                    expected_project_revision: project.summary.project_revision,
                    expected_build_generation: project.summary.build_generation,
                    candidate_id: candidate.candidate.candidate_id.clone(),
                    expected_candidate_digest: candidate
                        .candidate
                        .candidate_digest
                        .clone(),
                    expected_config_revision: 0,
                    expected_credential_bindings_revision: 0,
                    resolved_test_input_digest: "9".repeat(64),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            tested.ready.as_ref().unwrap().test.status,
            nomifun_api_types::PluginCandidateTestStatusDto::NeedsTestInput
        );
        assert!(
            tested
                .ready
                .as_ref()
                .unwrap()
                .test
                .runtime
                .as_ref()
                .is_some_and(|runtime| {
                    !runtime.node_version.is_empty()
                        && !runtime.runtime_target.is_empty()
                })
        );
        let library = state
            .service
            .list_library(&owner_user_id)
            .await
            .unwrap();
        let installed = state
            .service
            .apply_candidate(
                &owner_user_id,
                ApplyPluginCandidateRequest {
                    project_id: project.summary.project_id,
                    expected_project_revision:
                        project.summary.project_revision,
                    expected_build_generation:
                        project.summary.build_generation,
                    candidate_id:
                        candidate.candidate.candidate_id,
                    expected_candidate_digest:
                        candidate.candidate.candidate_digest,
                    target: ApplyPluginTargetDto::InitialInstall {
                        expected_library_revision:
                            library.library_revision,
                    },
                    allow_breaking: false,
                    acknowledge_test_warning: true,
                },
            )
            .await
            .unwrap();
        let capability_id =
            CapabilityId::from("test.nomicore.plugin.echo");
        assert!(
            kernel
                .snapshot()
                .unwrap()
                .capability(&capability_id)
                .is_some()
        );
        let materialized_registry = kernel.snapshot().unwrap();
        let materialized = materialized_registry.capability(&capability_id).unwrap();
        let resolved = ResolvedCapability {
            capability: CapabilityRef {
                id: materialized.manifest.id.clone(),
                version: materialized.manifest.version.clone(),
            },
            source_package: materialized.manifest.package.clone(),
            contribution_id: materialized.contribution_id.clone(),
            contribution_lock: materialized.contribution_lock.clone(),
            resolved_mount_id: materialized.mount_id.clone(),
            resolved_source: materialized.source.clone(),
            target_artifact_digest: materialized.target_artifact_digest.clone(),
            schema_digest: materialized.schema_digest.clone(),
            dependency_path: vec![capability_id.clone()],
            required_runtime_features: BTreeSet::new(),
        };
        let action_schema = &materialized.manifest.contributions.actions[0].input_schema;
        let resolved_schema = schema_resolver
            .resolve(&resolved, action_schema)
            .await
            .unwrap();
        assert_eq!(resolved_schema.0["type"], "object");
        let CatalogSnapshot {
            unavailable_capabilities,
            ..
        } = catalog.snapshot().unwrap().as_ref().clone();
        assert!(!unavailable_capabilities.contains_key(&capability_id));
        let catalog_snapshot = catalog.snapshot().unwrap();
        let capability_ref = CapabilityRef {
            id: capability_id.clone(),
            version: VersionString::from("1.0.0"),
        };
        let operation_lock = catalog_snapshot
            .capability_catalog_entry(&capability_ref)
            .unwrap()
            .unwrap()
            .operation_lock(CapabilityConsumer::Gateway)
            .unwrap();
        let invoked = kernel
            .invoke_operation(CapabilityOperationRequest {
                principal: PrincipalRef {
                    principal_kind: "installation".into(),
                    principal_id: owner_user_id.clone(),
                },
                operation_id: OperationId::from("plugin-gateway-operation"),
                idempotency_key: IdempotencyKey::from("plugin-gateway-key"),
                correlation_id: CorrelationId::from("plugin-gateway-correlation"),
                operation_lock,
                action_id: ActionId::from(
                    "test.nomicore.plugin.echo.invoke",
                ),
                resource_bindings: Vec::new(),
                state_scope_key: ScopeKey::from("gateway:plugin-e2e"),
                input: StrictJsonValue(serde_json::json!({
                    "surface": "gateway"
                })),
            })
            .await
            .unwrap();
        assert_eq!(invoked.0["surface"], "gateway");
        let api_catalog = catalog_snapshot.as_api().unwrap();
        assert!(api_catalog.capabilities.iter().any(|capability| {
            capability.capability.id == capability_id.as_ref()
                && capability.unavailable_code.is_none()
        }));

        drop(state);
        let mut restarted_policy = MaterializationPolicy::stable("1.0.0");
        restarted_policy
            .allowed_sources
            .insert(PluginSourceKind::ManagedLocal);
        let restarted_kernel = Arc::new(
            KernelRegistry::new(
                restarted_policy,
                Arc::new(InMemoryPluginStatePersistence::new())
                    as Arc<dyn PluginStatePersistence>,
            )
            .unwrap(),
        );
        let restarted_catalog = Arc::new(KernelCatalogProvider::new(
            Arc::clone(&restarted_kernel),
        ));
        let restarted_state = build_nomi_core_plugin_state(
            database.pool().clone(),
            data_root.path().to_path_buf(),
            &owner_user_id,
            Arc::clone(&restarted_kernel),
            restarted_catalog,
            Vec::new(),
            test_runtime_authority().await,
        )
        .await
        .unwrap()
        .router;
        assert!(
            restarted_kernel
                .snapshot()
                .unwrap()
                .capability(&capability_id)
                .is_some(),
            "enabled Plugin Mount must restore into the Kernel after restart"
        );

        restarted_state
            .service
            .uninstall(
                &owner_user_id,
                UninstallPluginRequest {
                    mount_id: installed.summary.mount_id,
                    expected_mount_revision:
                        installed.summary.mount_revision,
                    expected_current_target_digest: installed
                        .summary
                        .current
                        .unwrap()
                        .artifact_digest,
                },
            )
            .await
            .unwrap();
        assert!(
            restarted_kernel
                .snapshot()
                .unwrap()
                .capability(&capability_id)
                .is_none()
        );
    }
}
