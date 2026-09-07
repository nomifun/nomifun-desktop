use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use nomifun_agent_contracts::{
    CanonicalErrorCode, CapabilityConsumer, CredentialId, CredentialSlotBinding, DigestHex,
    PluginHostCommitFence, PluginMountId, StrictJsonValue, ValidatedPluginConfig,
};
use nomifun_agent_kernel::{KernelRegistry, PluginRegistration};
use nomifun_agent_platform::KernelCatalogProvider;
use nomifun_api_types::{
    ApiResponse, ApplyPluginCandidateRequest, BuildPluginProjectRequest,
    ConfigurePluginRequest, CreatePluginProjectRequest,
    DeletePluginDataRequest, DeletePluginProjectRequest, DurableOperationDetailDto,
    DurableOperationSummaryDto, ErrorResponse, ImportPluginRequest,
    PluginDetailDto, PluginLibraryResponseDto, PluginProjectDetailDto,
    RestorePluginPreviousRequest, RetryPluginRequest,
    SetPluginEnabledRequest, TestPluginCandidateRequest,
    UninstallPluginRequest,
};
use nomifun_auth::CurrentUser;
use nomifun_db::{
    ListPluginCredentialBindingsParams, PluginMountRow, SqlitePool,
};
use nomifun_js_host::{
    ExtensionHostSupervisor, JavaScriptHostConfig,
    materialize_bundled_extension_host,
};
use nomifun_js_authoring::SourceStoreLimits;
use nomifun_js_kernel_adapter::{
    JsKernelPluginAdapter, PluginPackageInput,
};
use nomifun_js_runtime::{
    NodeDiscoveryRequest, NodeRuntimeResolver,
};
use nomifun_plugin_platform::{
    ArtifactStoreLimits, OwnerMutationCoordinator,
};
use nomifun_plugin_service::{
    DbPluginRepositoryAdapter, FsPluginArtifactStore, FsPluginMountDataStore,
    FsPluginSourceStore,
    PluginApplicationService, PluginArtifactStorePort, PluginHostCoordinator,
    PluginRegistryPublisher, PluginRepository, PluginRouterState,
    PluginServiceDependencies, PluginServiceError, PluginServicePaths,
    SharedJsHostCoordinator, UnconfiguredPluginBuildExecutor,
    UnconfiguredPluginCandidateTestExecutor,
    UnconfiguredPluginOperationCancellation,
};
use tokio::sync::{Mutex, RwLock};
use serde::Deserialize;

const AGENT_EXECUTOR_UNAVAILABLE: &str = "CAPABILITY_UNAVAILABLE";
const PLUGIN_PLATFORM_DIRECTORY: &str = "plugin-platform";
const PLUGIN_AUTHORING_DIRECTORY: &str = "authoring";
const PLUGIN_MOUNT_DATA_DIRECTORY: &str = "plugin-mount-data";

pub(crate) async fn build_nomi_core_plugin_state(
    pool: SqlitePool,
    data_root: PathBuf,
    owner_user_id: &str,
    kernel: Arc<KernelRegistry>,
    catalog: Arc<KernelCatalogProvider>,
    base_registrations: Vec<PluginRegistration>,
) -> anyhow::Result<PluginRouterState> {
    let data_root = std::fs::canonicalize(&data_root)?;
    let platform_root = data_root.join(PLUGIN_PLATFORM_DIRECTORY);
    tokio::fs::create_dir_all(&platform_root).await?;
    tokio::fs::create_dir_all(data_root.join(PLUGIN_MOUNT_DATA_DIRECTORY)).await?;

    let repository = Arc::new(DbPluginRepositoryAdapter::new(pool));
    let artifacts = Arc::new(FsPluginArtifactStore::new(
        &platform_root,
        ArtifactStoreLimits::default(),
    )?);
    let host = discover_shared_host(&platform_root).await;
    let publisher = Arc::new(NomiCorePluginRegistryPublisher {
        kernel,
        catalog,
        base_registrations,
        dynamic_registrations: RwLock::new(BTreeMap::new()),
        publish_lock: Mutex::new(()),
        repository: Arc::clone(&repository) as Arc<dyn PluginRepository>,
        artifacts: Arc::clone(&artifacts),
        host: host.clone(),
        data_root: data_root.clone(),
    });
    publisher.restore(owner_user_id).await?;

    let host_coordinator: Arc<dyn PluginHostCoordinator> = match host {
        Some(host) => Arc::new(SharedJsHostCoordinator::new(host)),
        None => Arc::new(UnavailablePluginHostCoordinator),
    };
    let service = Arc::new(PluginApplicationService::new(
        PluginServiceDependencies {
            repository: repository as Arc<dyn PluginRepository>,
            artifacts: artifacts as Arc<dyn PluginArtifactStorePort>,
            host: host_coordinator,
            registry: publisher as Arc<dyn PluginRegistryPublisher>,
            mutation_coordinator: Arc::new(OwnerMutationCoordinator::new()),
            builder: Arc::new(UnconfiguredPluginBuildExecutor),
            tester: Arc::new(UnconfiguredPluginCandidateTestExecutor),
            operation_cancellation: Arc::new(
                UnconfiguredPluginOperationCancellation,
            ),
            source_store: Arc::new(FsPluginSourceStore::new(
                platform_root.join(PLUGIN_AUTHORING_DIRECTORY),
                SourceStoreLimits::default(),
            )?),
            data_store: Arc::new(FsPluginMountDataStore::new(&data_root)?),
            paths: PluginServicePaths {
                mount_data_relative_root:
                    PLUGIN_MOUNT_DATA_DIRECTORY.to_owned(),
            },
        },
    ));
    Ok(PluginRouterState::new(service))
}

pub(crate) fn plugin_routes(state: PluginRouterState) -> Router {
    Router::new()
        .route("/api/plugins", get(list_plugins))
        .route("/api/plugin-projects", post(create_project))
        .route(
            "/api/plugin-projects/{project_id}",
            get(get_project).delete(delete_project),
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
            | nomifun_plugin_service::ERR_CONFLICT => StatusCode::CONFLICT,
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

async fn discover_shared_host(
    platform_root: &Path,
) -> Option<Arc<ExtensionHostSupervisor>> {
    let resolution = match NodeRuntimeResolver::default()
        .resolve(NodeDiscoveryRequest::default())
        .await
    {
        Ok(resolution) => resolution,
        Err(error) => {
            tracing::warn!(%error, "Plugin Node Runtime discovery failed");
            return None;
        }
    };
    let Some(runtime) = resolution.selected else {
        tracing::info!(
            "Plugin Node Runtime is not selected; static Plugin management remains available"
        );
        return None;
    };
    let Some(executable_path) = resolution.probes.iter().find_map(|probe| {
        (probe.fingerprint.as_ref() == Some(&runtime))
            .then(|| PathBuf::from(&probe.executable_path))
    }) else {
        tracing::warn!(
            runtime_id = runtime.runtime_installation_id.as_ref(),
            "selected Plugin Node Runtime has no executable-path evidence"
        );
        return None;
    };
    let host_module = match materialize_bundled_extension_host(
        platform_root.join("host"),
    ) {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(%error, "Plugin JavaScript Host asset is unavailable");
            return None;
        }
    };
    match ExtensionHostSupervisor::new(JavaScriptHostConfig::for_host_module(
        executable_path,
        runtime,
        host_module,
    )) {
        Ok(host) => Some(Arc::new(host)),
        Err(error) => {
            tracing::warn!(%error, "Plugin shared JavaScript Host is unavailable");
            None
        }
    }
}

struct UnavailablePluginHostCoordinator;

#[async_trait]
impl PluginHostCoordinator for UnavailablePluginHostCoordinator {
    async fn commit_fence(
        &self,
        _mount_id: &str,
    ) -> Result<PluginHostCommitFence, PluginServiceError> {
        Ok(PluginHostCommitFence::NotResident)
    }
}

struct NomiCorePluginRegistryPublisher {
    kernel: Arc<KernelRegistry>,
    catalog: Arc<KernelCatalogProvider>,
    base_registrations: Vec<PluginRegistration>,
    dynamic_registrations:
        RwLock<BTreeMap<PluginMountId, PluginRegistration>>,
    publish_lock: Mutex<()>,
    repository: Arc<dyn PluginRepository>,
    artifacts: Arc<FsPluginArtifactStore>,
    host: Option<Arc<ExtensionHostSupervisor>>,
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
        let host = self.host.clone().ok_or_else(|| {
            PluginServiceError::Coded {
                code: nomifun_plugin_service::ERR_RUNTIME,
                message: "no compatible Node Runtime is selected".to_owned(),
            }
        })?;
        let artifact_digest = mount
            .current_artifact_digest
            .as_deref()
            .ok_or_else(|| {
                PluginServiceError::stale(
                    "materialized Mount has no current Artifact",
                )
            })?;
        let artifact_row = self
            .repository
            .get_artifact(artifact_digest)
            .await?
            .ok_or_else(|| {
                PluginServiceError::not_found(
                    "materialized Mount Artifact",
                )
            })?;
        self.artifacts.verify(&artifact_row).await?;
        let stored = self
            .artifacts
            .store()
            .load(&DigestHex::from(artifact_digest.to_owned()))?;
        if stored.artifact.artifact_digest.as_ref() != artifact_digest
            || stored.artifact.manifest.payload.package.package_id.as_ref()
                != mount.package_id
        {
            return Err(PluginServiceError::stale(
                "Mount target differs from the verified Artifact",
            ));
        }

        let mount_id = PluginMountId::from(mount.mount_id.clone());
        let bindings = self
            .repository
            .list_credentials(&ListPluginCredentialBindingsParams {
                mount_id: mount.mount_id.clone(),
                expected_mount_revision: mount.revision,
                expected_current_artifact_digest:
                    mount.current_artifact_digest.clone(),
            })
            .await?;
        let credential_bindings = bindings
            .bindings
            .into_iter()
            .map(|binding| CredentialSlotBinding {
                mount_id: mount_id.clone(),
                slot_key: binding.slot.into(),
                credential_id: CredentialId::from(binding.credential_id),
            })
            .collect();
        let config_revision = u64::try_from(mount.config_revision).map_err(|_| {
            PluginServiceError::integration(
                "Plugin config revision is negative",
            )
        })?;
        let config_schema_digest = mount
            .config_schema_digest
            .clone()
            .ok_or_else(|| {
                PluginServiceError::stale(
                    "materialized Mount has no config schema digest",
                )
            })?;
        let config = serde_json::from_str(&mount.config_json).map_err(|error| {
            PluginServiceError::integration(format!(
                "persisted Plugin config is invalid JSON: {error}"
            ))
        })?;
        let data_dir = resolve_mount_data_dir(
            &self.data_root,
            &mount.data_dir_path,
            &mount.mount_id,
        )
        .await?;
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
        })?
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
        self.refresh_agent_availability()
    }

    fn refresh_agent_availability(&self) -> Result<(), PluginServiceError> {
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
                super::nomi_core_agent_projection::nomi_capability_projection(
                    capability.manifest.id.as_ref(),
                )
                .err()
                .map(|_| {
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

    use nomifun_agent_contracts::{
        ActionId, ArtifactEnvelope, ArtifactFileDigest, ArtifactId,
        CapabilityActionDescriptor, CapabilityContributions, CapabilityId,
        CapabilityKind, CapabilityManifest, CanonicalSchemaRef, EffectClass,
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
        InMemoryPluginStatePersistence, MaterializationPolicy,
        PluginStatePersistence,
    };
    use nomifun_api_types::{
        ApplyPluginCandidateRequest, ApplyPluginTargetDto,
        CreatePluginProjectRequest, ImportPluginRequest,
        PluginImportKindDto, PluginProjectLanguageDto,
        PluginProjectSourceStateDto, UninstallPluginRequest,
    };
    use nomifun_plugin_service::CreateProjectInput;
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

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
                [CapabilityConsumer::Agent],
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
                    input_schema: CanonicalSchemaRef::from(
                        "schema://test.nomicore.plugin/echo-input@1",
                    ),
                    output_schema: CanonicalSchemaRef::from(
                        "schema://test.nomicore.plugin/echo-output@1",
                    ),
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
        let state = build_nomi_core_plugin_state(
            database.pool().clone(),
            data_root.path().to_path_buf(),
            &owner_user_id,
            Arc::clone(&kernel),
            Arc::clone(&catalog),
            Vec::new(),
        )
        .await
        .unwrap();
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
        let project = state
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
        let CatalogSnapshot {
            unavailable_capabilities,
            ..
        } = catalog.snapshot().unwrap().as_ref().clone();
        assert_eq!(
            unavailable_capabilities
                .get(&capability_id)
                .map(AsRef::as_ref),
            Some(AGENT_EXECUTOR_UNAVAILABLE)
        );

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
        )
        .await
        .unwrap();
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
