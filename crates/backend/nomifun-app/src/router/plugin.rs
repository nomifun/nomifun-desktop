use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Extension, Path as AxumPath, State};
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use base64::Engine as _;
use nomifun_agent_contracts::{
    DigestHex, PluginActionEffect, PluginArtifact, PluginBindingPoint, PluginDraftId,
    PluginId, PluginManifest, PLUGIN_MANIFEST_SCHEMA,
    plugin_action_id,
};
use nomifun_api_types::*;
use nomifun_auth::CurrentUser;
use nomifun_db::sqlx::Row as _;
use nomifun_plugin_platform::{
    DataGeneration, InstallArtifactRequest, InstallTarget, NeverCancel,
    PluginDataRootHandle, PluginDraftRecord,
    PluginDraftStatus, PluginActionRegistration, PluginArtifactStoreError, PluginDraftStore,
    PluginDraftStoreError, PluginInstallError, PluginInstallService, PluginInventory,
    PluginQueryResult, PluginRecord, PluginRepository, PluginRepositoryError, PluginSqlStatement,
    PluginRuntimeContext, PluginRuntimePort, PluginSqlValue, PluginStorage, PreviewDataRoot,
    SqlitePluginRepository, action_publications,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use uuid::Uuid;
pub(super) use super::plugin_authoring::{
    has_authoring_approval, request_authoring_approval, run_authoring_ui_test,
    save_authoring_draft,
};

pub(super) const PLUGIN_SDK: &str = include_str!(
    "../../../nomifun-plugin-platform/src/assets/plugin-sdk.js"
);
const MAX_PERMISSION_CONFIRMATIONS: usize = 128;
const MAX_SURFACE_CALLS: usize = 256;
const PLUGIN_STARTUP_ACTIVATION_FAILED: &str = "PLUGIN_STARTUP_ACTIVATION_FAILED";
const PLUGIN_BINDING_RECOVERY_FAILED: &str = "PLUGIN_BINDING_RECOVERY_FAILED";

#[derive(Clone)]
pub struct PluginRouterState {
    pub(super) repository: Arc<SqlitePluginRepository>,
    pub(super) install: Arc<PluginInstallService>,
    pub(super) runtime: Arc<nomifun_plugin_platform::PluginServiceRuntime>,
    pub(super) artifacts: Arc<nomifun_plugin_platform::PluginArtifactStore>,
    pub(super) data_roots: Arc<nomifun_plugin_platform::PluginDataRootManager>,
    pub(super) drafts: Arc<PluginDraftStore>,
    pub(super) transfers: Arc<nomifun_plugin_platform::PluginBackupFilesystem>,
    pub(super) confirmations: Arc<Mutex<HashMap<String, PermissionConfirmation>>>,
    pub(super) surfaces: Arc<Mutex<HashMap<String, SurfaceSession>>>,
    pub(crate) registry: nomifun_plugin_platform::InMemoryPluginBindingRegistry,
    pub(crate) agent: nomifun_plugin_platform::AgentPluginBindings,
    pub(crate) desktop: nomifun_plugin_platform::DesktopPluginBindings,
    host: Arc<dyn nomifun_plugin_platform::PluginServiceHostPort>,
    pub(super) events: Arc<dyn nomifun_realtime::UserEventSink>,
    pub(super) ui_tests: Arc<super::plugin_authoring::UiTests>,
}

impl std::fmt::Debug for PluginRouterState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PluginRouterState")
            .field("draft_root", &self.drafts.root())
            .finish_non_exhaustive()
    }
}

impl PluginRouterState {
    pub fn new(
        services: &crate::services::AppServices,
        install: Arc<PluginInstallService>,
        runtime: Arc<nomifun_plugin_platform::PluginServiceRuntime>,
        bindings: super::plugin_ports::PluginBindingConsumers,
    ) -> Self {
        Self {
            repository: services.plugin_repository.clone(),
            install,
            runtime,
            artifacts: services.plugin_artifacts.clone(),
            data_roots: services.plugin_data_roots.clone(),
            drafts: services.plugin_drafts.clone(),
            transfers: services.plugin_transfers.clone(),
            confirmations: Arc::new(Mutex::new(HashMap::new())),
            surfaces: Arc::new(Mutex::new(HashMap::new())),
            registry: bindings.registry,
            agent: bindings.agent,
            desktop: bindings.desktop,
            host: bindings.host,
            events: services.event_bus.clone(),
            ui_tests: Arc::new(super::plugin_authoring::UiTests::default()),
        }
    }

    pub(crate) async fn recover(&self) -> Result<(), String> {
        self.data_roots
            .cleanup_preview_roots()
            .map_err(|error| error.to_string())?;
        let owner = nomifun_db::installation_owner_id(self.repository.pool())
            .await
            .map_err(|error| error.to_string())?;
        for plugin in self
            .repository
            .list_plugins(&owner)
            .await
            .map_err(|error| error.to_string())?
        {
            let inventory = self
                .repository
                .inventory(&owner, &plugin.plugin_id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "Plugin inventory disappeared during recovery".to_owned())?;
            if inventory.plugin.trashed_at_ms.is_some() {
                self.registry
                    .remove_plugin(&inventory.plugin.plugin_id)
                    .map_err(|error| error.to_string())?;
                continue;
            }
            let failure = if inventory.plugin.enabled {
                activate_inventory_runtime(self, &inventory)
                    .await
                    .err()
                    .map(|_| PLUGIN_STARTUP_ACTIVATION_FAILED)
            } else {
                None
            };
            let failure = match failure {
                Some(failure) => Some(failure),
                None => replace_inventory_bindings(self, &inventory)
                    .err()
                    .map(|_| PLUGIN_BINDING_RECOVERY_FAILED),
            };
            if let Some(failure) = failure {
                let _ = self.runtime.remove(&inventory.plugin.plugin_id).await;
                self.registry
                    .remove_plugin(&inventory.plugin.plugin_id)
                    .map_err(|error| error.to_string())?;
                self.repository
                    .set_last_error(
                        &owner,
                        &inventory.plugin.plugin_id,
                        Some(failure),
                        now_ms(),
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                tracing::warn!(
                    plugin_id = %inventory.plugin.plugin_id.as_ref(),
                    code = failure,
                    "Plugin recovery failed in isolation"
                );
                continue;
            }
            if matches!(
                inventory.plugin.last_error.as_deref(),
                Some(PLUGIN_STARTUP_ACTIVATION_FAILED | PLUGIN_BINDING_RECOVERY_FAILED)
            ) {
                self.repository
                    .set_last_error(&owner, &inventory.plugin.plugin_id, None, now_ms())
                    .await
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct PermissionConfirmation {
    pub(super) owner_user_id: String,
    pub(super) artifact_digest: DigestHex,
    pub(super) permissions: BTreeSet<String>,
    pub(super) secret_slots: BTreeSet<String>,
    pub(super) trusted_local_service: bool,
    pub(super) expires_at_ms: i64,
}

enum SurfaceDataRoot {
    Installed(PluginDataRootHandle),
    Preview(PreviewDataRoot),
}

impl SurfaceDataRoot {
    fn storage(&self) -> PluginStorage {
        match self {
            Self::Installed(root) => root.storage(),
            Self::Preview(root) => root.storage(),
        }
    }

    fn cache(&self) -> nomifun_plugin_platform::ScopedPluginCache {
        match self {
            Self::Installed(root) => root.cache(),
            Self::Preview(root) => root.cache(),
        }
    }
}

pub(super) struct SurfaceSession {
    pub(super) owner_user_id: String,
    pub(super) plugin_id: Option<PluginId>,
    pub(super) runtime_plugin_id: PluginId,
    pub(super) draft_id: Option<PluginDraftId>,
    pub(super) artifact: PluginArtifact,
    pub(super) asset_root: PathBuf,
    data_root: SurfaceDataRoot,
    pub(super) generation: u64,
    pub(super) is_preview: bool,
    pub(super) config: Value,
    pub(super) granted_permissions: BTreeSet<String>,
    pub(super) completed_calls: HashMap<String, PluginBridgeResultDto>,
}

pub fn read_routes(state: PluginRouterState) -> Router {
    Router::new()
        .route("/api/plugins", get(list_plugins))
        .route("/api/plugins/desktop/commands", get(list_desktop_commands))
        .route("/api/plugins/library-state", get(get_library_state))
        .route("/api/plugins/credentials", get(list_credential_references))
        .route("/api/plugins/{plugin_id}", get(get_plugin))
        .route("/api/plugin-drafts", get(list_drafts))
        .route("/api/plugin-drafts/{draft_id}", get(get_draft))
        .route("/api/plugin-drafts/{draft_id}/authoring", get(super::plugin_authoring::details))
        .with_state(state)
}

/// Opaque-origin iframe navigation cannot attach the Desktop's local-trust
/// header. These two GET-only routes therefore use the unguessable live
/// Surface session plus exact generation and Artifact digest as their scoped
/// capability. No listing, Bridge, or mutation route is public.
pub fn asset_routes(state: PluginRouterState) -> Router {
    Router::new()
        .route(
            "/api/plugins/{plugin_id}/surface/assets/{surface_session_id}/{surface_generation}/{artifact_digest}/{*asset_path}",
            get(surface_asset),
        )
        .route(
            "/api/plugin-drafts/{draft_id}/surface/assets/{surface_session_id}/{surface_generation}/{artifact_digest}/{*asset_path}",
            get(surface_asset),
        )
        .with_state(state)
}

pub fn write_routes(state: PluginRouterState) -> Router {
    Router::new()
        .route("/api/plugins/import", post(install_import))
        .route("/api/plugins/import/inspect", post(inspect_import))
        .route("/api/plugins/library-state", put(update_library_state))
        .route(
            "/api/plugins/desktop/commands/invoke",
            post(invoke_desktop_command),
        )
        .route("/api/plugins/desktop/events", post(dispatch_desktop_event))
        .route("/api/plugins/{plugin_id}/enabled", put(set_enabled))
        .route("/api/plugins/{plugin_id}/config", put(configure))
        .route("/api/plugins/{plugin_id}/export", post(export_package))
        .route("/api/plugins/{plugin_id}/backup", post(export_backup))
        .route("/api/plugins/{plugin_id}/restore", post(restore))
        .route("/api/plugins/{plugin_id}/trash", post(trash))
        .route("/api/plugins/{plugin_id}", delete(permanent_delete))
        .route("/api/plugins/{plugin_id}/surface/open", post(open_surface))
        .route("/api/plugins/{plugin_id}/surface/bridge", post(dispatch_bridge))
        .route("/api/plugins/{plugin_id}/surface/close", post(close_surface))
        .route("/api/plugin-drafts", post(create_draft))
        .route("/api/plugin-drafts/{draft_id}/files", put(replace_draft_file).delete(delete_draft_file))
        .route("/api/plugin-drafts/{draft_id}/preview", post(preview_draft))
        .route("/api/plugin-drafts/{draft_id}/save", post(save_draft))
        .route("/api/plugin-drafts/{draft_id}/approve", post(super::plugin_authoring::approve))
        .route("/api/plugin-drafts/{draft_id}/ui-results", post(super::plugin_authoring::ui_results))
        .route("/api/plugin-drafts/{draft_id}", delete(delete_draft))
        .route("/api/plugin-drafts/{draft_id}/surface/bridge", post(dispatch_bridge))
        .route("/api/plugin-drafts/{draft_id}/surface/close", post(close_surface))
        .with_state(state)
}

async fn list_desktop_commands(
    State(state): State<PluginRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<PluginDesktopCommandDto>>>, PluginHttpError> {
    let commands = state
        .desktop
        .commands()?
        .into_iter()
        .filter(|command| {
            command.availability == nomifun_plugin_platform::PluginActionAvailability::Available
        })
        .map(|command| PluginDesktopCommandDto {
            action_id: command.stable_action_id,
            name: command.publication.action.name,
            description: command.publication.action.description,
            input_schema: command.publication.action.input.0,
            effect: action_effect_dto(command.publication.action.effect),
        })
        .collect();
    Ok(Json(ApiResponse::ok(commands)))
}

async fn invoke_desktop_command(
    State(state): State<PluginRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Json(request): Json<InvokePluginDesktopCommandRequest>,
) -> Result<Json<ApiResponse<Value>>, PluginHttpError> {
    let selection = state.desktop.select(&request.action_id)?;
    let result = state
        .desktop
        .trigger(
            &selection,
            nomifun_agent_contracts::StrictJsonValue(request.input),
            nomifun_plugin_platform::PluginDispatchOptions::default(),
        )
        .await?;
    Ok(Json(ApiResponse::ok(result.0)))
}

async fn dispatch_desktop_event(
    State(state): State<PluginRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Json(request): Json<DispatchPluginDesktopEventRequest>,
) -> Result<Json<ApiResponse<PluginDesktopEventReportDto>>, PluginHttpError> {
    let report = state
        .desktop
        .emit_event(
            nomifun_agent_contracts::StrictJsonValue(request.input),
            nomifun_plugin_platform::PluginDispatchOptions::default(),
        )
        .await?;
    Ok(Json(ApiResponse::ok(PluginDesktopEventReportDto {
        outputs: report
            .outputs
            .into_iter()
            .map(|output| PluginDesktopEventOutputDto {
                action_id: output.stable_action_id,
                output: output.output.0,
            })
            .collect(),
        failures: report
            .failures
            .into_iter()
            .map(|failure| PluginDesktopEventFailureDto {
                action_id: failure.stable_action_id,
                code: binding_error_code(&failure.error).into(),
            })
            .collect(),
    })))
}

async fn list_plugins(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<PluginLibraryResponseDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(list_plugins_owned(&state, &user.id.to_string()).await?)))
}

pub(super) async fn list_plugins_owned(state: &PluginRouterState, owner: &str) -> Result<PluginLibraryResponseDto, PluginHttpError> {
    let owner = owner.to_owned();
    let records = state.repository.list_plugins(&owner).await?;
    let mut plugins = Vec::with_capacity(records.len());
    for record in records {
        let inventory = state
            .repository
            .inventory(&owner, &record.plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)?;
        plugins.push(summary_dto(&state, &inventory).await?);
    }
    let revision = plugins.iter().map(|plugin| plugin.revision).max().unwrap_or(0);
    Ok(PluginLibraryResponseDto {
        revision,
        plugins,
    })
}

async fn get_library_state(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<PluginLibraryStateDto>>, PluginHttpError> {
    let owner = user.id.to_string();
    let states = state.repository.list_library_states(&owner).await?;
    Ok(Json(ApiResponse::ok(library_state_dto(states)?)))
}

async fn list_credential_references(
    State(state): State<PluginRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<PluginCredentialReferenceDto>>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(list_credential_references_owned(&state).await?)))
}

pub(super) async fn list_credential_references_owned(
    state: &PluginRouterState,
) -> Result<Vec<PluginCredentialReferenceDto>, PluginHttpError> {
    let mut references = Vec::new();
    for row in nomifun_db::sqlx::query(
        "SELECT provider_id, name, enabled FROM providers \
         WHERE trim(credentials_encrypted) <> '' ORDER BY sort_order, name, provider_id",
    )
    .fetch_all(state.repository.pool())
    .await
    .map_err(|error| PluginHttpError::internal(error.to_string()))?
    {
        let provider_id: String = row
            .try_get("provider_id")
            .map_err(|error| PluginHttpError::internal(error.to_string()))?;
        references.push(PluginCredentialReferenceDto {
            credential_id: format!("provider:{provider_id}"),
            kind: PluginCredentialReferenceKindDto::Provider,
            label: row
                .try_get("name")
                .map_err(|error| PluginHttpError::internal(error.to_string()))?,
            enabled: row
                .try_get("enabled")
                .map_err(|error| PluginHttpError::internal(error.to_string()))?,
        });
    }
    for row in nomifun_db::sqlx::query(
        "SELECT connection.connection_id, connection.role, connection.label, \
                provider.name AS provider_name, provider.enabled \
         FROM provider_connections connection \
         JOIN providers provider ON provider.provider_id = connection.provider_id \
         ORDER BY provider.sort_order, provider.name, connection.role, connection.connection_id",
    )
    .fetch_all(state.repository.pool())
    .await
    .map_err(|error| PluginHttpError::internal(error.to_string()))?
    {
        let connection_id: String = row
            .try_get("connection_id")
            .map_err(|error| PluginHttpError::internal(error.to_string()))?;
        let provider_name: String = row
            .try_get("provider_name")
            .map_err(|error| PluginHttpError::internal(error.to_string()))?;
        let role: String = row
            .try_get("role")
            .map_err(|error| PluginHttpError::internal(error.to_string()))?;
        let label = row
            .try_get::<Option<String>, _>("label")
            .map_err(|error| PluginHttpError::internal(error.to_string()))?
            .unwrap_or(role);
        references.push(PluginCredentialReferenceDto {
            credential_id: format!("connection:{connection_id}"),
            kind: PluginCredentialReferenceKindDto::Connection,
            label: format!("{provider_name} — {label}"),
            enabled: row
                .try_get("enabled")
                .map_err(|error| PluginHttpError::internal(error.to_string()))?,
        });
    }
    Ok(references)
}

async fn update_library_state(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<UpdatePluginLibraryStateRequest>,
) -> Result<Json<ApiResponse<PluginLibraryStateDto>>, PluginHttpError> {
    let owner = user.id.to_string();
    let collections = request
        .collections
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if collections.len() != request.collections.len()
        || collections.iter().any(|collection| {
            collection.trim().is_empty() || collection.len() > 160
        })
    {
        return Err(PluginHttpError::bad_request("Plugin collection is invalid"));
    }
    let states = request
        .items
        .into_iter()
        .map(|item| {
            if item
                .collection_id
                .as_ref()
                .is_some_and(|collection| !collections.contains(collection))
            {
                return Err(PluginHttpError::bad_request(
                    "Plugin item references an unknown collection",
                ));
            }
            Ok(nomifun_plugin_platform::PluginLibraryState {
                plugin_id: parse_plugin_id(&item.plugin_id)?,
                pinned: item.pinned,
                collection: item.collection_id,
                custom_name: item.custom_name,
                last_opened_at_ms: item
                    .last_opened_at_ms
                    .map(|value| i64::try_from(value).map_err(|_| {
                        PluginHttpError::bad_request("last_opened_at_ms exceeds SQLite range")
                    }))
                    .transpose()?,
                revision: request.expected_revision,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if states
        .iter()
        .map(|state| state.plugin_id.as_ref())
        .collect::<BTreeSet<_>>()
        .len()
        != states.len()
    {
        return Err(PluginHttpError::bad_request(
            "Plugin library state contains a duplicate Plugin",
        ));
    }
    state
        .repository
        .replace_library_states(&owner, request.expected_revision, &states)
        .await?;
    let states = state.repository.list_library_states(&owner).await?;
    Ok(Json(ApiResponse::ok(library_state_dto(states)?)))
}

fn library_state_dto(
    states: Vec<nomifun_plugin_platform::PluginLibraryState>,
) -> Result<PluginLibraryStateDto, PluginHttpError> {
    let revision = states.iter().map(|state| state.revision).max().unwrap_or(0);
    let collections = states
        .iter()
        .filter_map(|state| state.collection.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let items = states
        .into_iter()
        .map(|state| {
            Ok(PluginLibraryItemDto {
                plugin_id: state.plugin_id.as_ref().to_owned(),
                collection_id: state.collection,
                pinned: state.pinned,
                custom_name: state.custom_name,
                last_opened_at_ms: nonnegative(state.last_opened_at_ms)?,
            })
        })
        .collect::<Result<Vec<_>, PluginHttpError>>()?;
    Ok(PluginLibraryStateDto {
        revision,
        collections,
        items,
    })
}

async fn get_plugin(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(plugin_id): AxumPath<String>,
) -> Result<Json<ApiResponse<PluginDetailDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(get_plugin_owned(&state, &user.id.to_string(), &plugin_id).await?)))
}

pub(super) async fn get_plugin_owned(state: &PluginRouterState, owner: &str, plugin_id: &str) -> Result<PluginDetailDto, PluginHttpError> {
    let inventory = inventory_owned(state, owner, plugin_id).await?;
    Ok(detail_dto(&state, &inventory).await?)
}

async fn inspect_import(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<InspectPluginImportRequest>,
) -> Result<Json<ApiResponse<PluginImportInspectionDto>>, PluginHttpError> {
    if request.kind == PluginImportKindDto::Backup {
        let backup = import_backup_bundle(&state, &request.source_path)?;
        let owner = user.id.to_string();
        let mut inspection = inspection_dto(
            &state,
            &owner,
            PluginImportKindDto::Backup,
            backup.package.artifact.clone(),
            None,
        )
        .await?;
        inspection.backup = Some(PluginBackupDataSummaryDto {
            includes_data: true,
            data_version: backup.package.artifact.manifest.data_version,
            database_size_bytes: backup.data_sqlite.len() as u64,
            file_count: backup.files.len() as u64,
            includes_config: true,
            credential_slots_to_rebind: backup.credential_slots.into_iter().collect(),
        });
        return Ok(Json(ApiResponse::ok(inspection)));
    }
    let artifact = match request.kind {
        PluginImportKindDto::Directory => state
            .artifacts
            .inspect_directory(&request.source_path, &NeverCancel)?,
        PluginImportKindDto::Zip => state
            .artifacts
            .inspect_zip(&request.source_path, &NeverCancel)?,
        PluginImportKindDto::Backup => unreachable!(),
    };
    let owner = user.id.to_string();
    let existing = existing_for_package(&state, &owner, &artifact.manifest.id).await?;
    let inspection = inspection_dto(&state, &owner, request.kind, artifact, existing.as_ref()).await?;
    Ok(Json(ApiResponse::ok(inspection)))
}

async fn install_import(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<InstallPluginImportRequest>,
) -> Result<Json<ApiResponse<InstallPluginImportResponseDto>>, PluginHttpError> {
    if request.kind == PluginImportKindDto::Backup {
        if request.expected_plugin_revision.is_some() {
            return Err(PluginHttpError::bad_request(
                "a Plugin Backup is always restored as a new local Plugin",
            ));
        }
        let backup = import_backup_bundle(&state, &request.source_path)?;
        let owner = user.id.to_string();
        let artifact = backup.package.artifact.clone();
        if request.config.is_some() {
            return Err(PluginHttpError::bad_request(
                "Backup Config is authoritative and cannot be overridden",
            ));
        }
        let credential_bindings = request.credential_bindings.clone().unwrap_or_default();
        validate_requested_credential_bindings(
            &state,
            &artifact.manifest,
            &credential_bindings,
        )
        .await?;
        let confirmation = consume_confirmation(
            &state,
            &owner,
            &artifact,
            request.permission_confirmation_id.as_deref(),
            None,
        )
        .await?;
        if let Some(confirmation) = confirmation.required {
            return Ok(Json(ApiResponse::ok(InstallPluginImportResponseDto {
                result: PluginInstallOutcomeDto::ConfirmationRequired { confirmation },
            })));
        }
        let package_identity_in_use =
            package_identity_in_use(&state, &owner, &artifact.manifest.id).await?;
        let outcome = state
            .install
            .install_backup(
                InstallArtifactRequest {
                    owner_user_id: owner.clone(),
                    target: InstallTarget::new(),
                    local_package_id: package_identity_in_use
                        .then(|| copy_package_id(&artifact.manifest.id)),
                    config: json!({}),
                    credential_bindings,
                    confirmed_permissions: confirmation.permissions,
                    confirmed_secret_slots: confirmation.secret_slots,
                    trusted_local_service_confirmed: confirmation.trusted_local_service,
                },
                backup,
                None,
            )
            .await?;
        let detail = state
            .repository
            .inventory(&owner, &outcome.plugin.plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)?;
        return Ok(Json(ApiResponse::ok(InstallPluginImportResponseDto {
            result: PluginInstallOutcomeDto::Installed {
                plugin: Box::new(detail_dto(&state, &detail).await?),
            },
        })));
    }
    let artifact = match request.kind {
        PluginImportKindDto::Directory => state
            .artifacts
            .inspect_directory(&request.source_path, &NeverCancel)?,
        PluginImportKindDto::Zip => state
            .artifacts
            .inspect_zip(&request.source_path, &NeverCancel)?,
        PluginImportKindDto::Backup => unreachable!(),
    };
    let owner = user.id.to_string();
    let existing = existing_for_package(&state, &owner, &artifact.manifest.id).await?;
    let package_identity_in_use =
        package_identity_in_use(&state, &owner, &artifact.manifest.id).await?;
    let existing_inventory = if request.create_copy {
        None
    } else {
        match existing.as_ref() {
            Some(plugin) => state.repository.inventory(&owner, &plugin.plugin_id).await?,
            None => None,
        }
    };
    let install_config = request
        .config
        .clone()
        .or_else(|| {
            existing_inventory
                .as_ref()
                .map(|inventory| inventory.plugin.config.clone())
        })
        .unwrap_or_else(|| json!({}));
    let credential_bindings = request
        .credential_bindings
        .clone()
        .or_else(|| {
            existing_inventory
                .as_ref()
                .map(|inventory| inventory.credential_bindings.clone())
        })
        .unwrap_or_default();
    validate_config_value(&artifact.manifest, &install_config)?;
    validate_requested_credential_bindings(
        &state,
        &artifact.manifest,
        &credential_bindings,
    )
    .await?;
    let confirmation_existing = if request.create_copy {
        None
    } else {
        existing.as_ref()
    };
    let confirmation = consume_confirmation(
        &state,
        &owner,
        &artifact,
        request.permission_confirmation_id.as_deref(),
        confirmation_existing,
    )
    .await?;
    if let Some(confirmation) = confirmation.required {
        return Ok(Json(ApiResponse::ok(InstallPluginImportResponseDto {
            result: PluginInstallOutcomeDto::ConfirmationRequired { confirmation },
        })));
    }
    let (target, local_package_id) = if request.create_copy {
        (
            InstallTarget::new(),
            Some(copy_package_id(&artifact.manifest.id)),
        )
    } else if let Some(existing) = existing.as_ref() {
        let expected = request.expected_plugin_revision.ok_or_else(|| {
            PluginHttpError::conflict("expected_plugin_revision is required for an update")
        })?;
        if expected != existing.revision {
            return Err(PluginHttpError::conflict("Plugin revision changed"));
        }
        (
            InstallTarget::Existing {
                plugin_id: existing.plugin_id.clone(),
                expected_revision: expected,
            },
            None,
        )
    } else {
        (
            InstallTarget::new(),
            package_identity_in_use.then(|| copy_package_id(&artifact.manifest.id)),
        )
    };
    if let InstallTarget::Existing { plugin_id, .. } = &target {
        revoke_plugin_surfaces(&state, plugin_id).await?;
    }
    let install_request = InstallArtifactRequest {
        owner_user_id: owner.clone(),
        target,
        local_package_id,
        config: install_config,
        credential_bindings,
        confirmed_permissions: confirmation.permissions,
        confirmed_secret_slots: confirmation.secret_slots,
        trusted_local_service_confirmed: confirmation.trusted_local_service,
    };
    let outcome = match request.kind {
        PluginImportKindDto::Directory => state
            .install
            .install_directory(install_request, &request.source_path, None)
            .await?,
        PluginImportKindDto::Zip => state
            .install
            .install_zip(install_request, &request.source_path, None)
            .await?,
        PluginImportKindDto::Backup => unreachable!(),
    };
    let detail = state
        .repository
        .inventory(&owner, &outcome.plugin.plugin_id)
        .await?
        .ok_or(PluginRepositoryError::NotFound)?;
    Ok(Json(ApiResponse::ok(InstallPluginImportResponseDto {
        result: PluginInstallOutcomeDto::Installed {
            plugin: Box::new(detail_dto(&state, &detail).await?),
        },
    })))
}

async fn create_draft(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<CreatePluginDraftRequest>,
) -> Result<Json<ApiResponse<PluginDraftDetailDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(create_draft_owned(&state, &user.id.to_string(), request).await?)))
}

pub(super) async fn create_draft_owned(state: &PluginRouterState, owner: &str, request: CreatePluginDraftRequest) -> Result<PluginDraftDetailDto, PluginHttpError> {
    create_draft_with_source(state, owner, request, None).await
}

pub(super) struct PluginDraftSource {
    pub conversation_id: String,
    pub message_id: String,
    pub operation_key: String,
}

pub(super) async fn create_draft_with_source(
    state: &PluginRouterState, owner: &str, request: CreatePluginDraftRequest,
    source: Option<PluginDraftSource>,
) -> Result<PluginDraftDetailDto, PluginHttpError> {
    let request_digest = nomifun_agent_contracts::digest_payload(&request)
        .map_err(|error| PluginHttpError::internal(error.to_string()))?.as_ref().to_owned();
    if let Some(source) = &source {
        if let Some(existing) = find_source_draft(state, owner, source, &request_digest).await? {
            return draft_detail(state, &existing);
        }
    }
    let owner = owner.to_owned();
    if request
        .template
        .as_deref()
        .is_some_and(|template| template != "agent.before_tool")
    {
        return Err(PluginHttpError::bad_request("unknown Plugin Draft template"));
    }
    if request.plugin_id.is_some() && request.template.is_some() {
        return Err(PluginHttpError::bad_request(
            "an existing Plugin draft cannot also select a template",
        ));
    }
    let existing_inventory = match request.plugin_id.as_deref() {
        Some(plugin_id) => {
            let plugin_id = parse_plugin_id(plugin_id)?;
            let inventory = state
                .repository
                .inventory(&owner, &plugin_id)
                .await?
                .ok_or(PluginRepositoryError::NotFound)?;
            if request.expected_plugin_revision != Some(inventory.plugin.revision) {
                return Err(PluginHttpError::conflict("Plugin revision changed"));
            }
            Some(inventory)
        }
        None => None,
    };
    let draft_id = PluginDraftId::from(Uuid::now_v7().to_string());
    let workspace = state.drafts.create(&owner, &draft_id)?;
    let (plugin_id, base_revision, name) = if let Some(inventory) = existing_inventory {
        copy_artifact_into_draft(&state, &owner, &draft_id, &inventory).await?;
        (
            Some(inventory.plugin.plugin_id.clone()),
            Some(inventory.plugin.revision),
            inventory.plugin.name,
        )
    } else {
        let package_id = format!("local.plugin.{}", Uuid::now_v7().simple());
        let (manifest, files, name) = match request.template.as_deref() {
            None => (
                json!({
                    "schema": PLUGIN_MANIFEST_SCHEMA,
                    "id": package_id,
                    "version": "0.1.0",
                    "name": "New Plugin",
                    "description": "A new NomiFun Plugin",
                    "hostApi": ">=1 <2",
                    "entrypoints": {"ui": "ui/index.html"},
                    "actions": {}, "bindings": [], "dataVersion": 0, "migrations": [],
                    "configSchema": {"type": "object"}, "secrets": [], "permissions": []
                }),
                vec![(
                    "ui/index.html",
                    b"<!doctype html><html><head><meta charset=\"utf-8\"><title>New Plugin</title></head><body><main><h1>New Plugin</h1></main></body></html>".as_slice(),
                )],
                "New Plugin",
            ),
            Some("agent.before_tool") => (
                json!({
                    "schema": PLUGIN_MANIFEST_SCHEMA,
                    "id": package_id,
                    "version": "0.1.0",
                    "name": "Before Tool Guard",
                    "description": "Review each Agent tool call before execution.",
                    "hostApi": ">=1 <2",
                    "entrypoints": {"service": "service/main.mjs", "serviceMode": "onDemand"},
                    "actions": {
                        "before_tool": {
                            "name": "Before Tool Guard",
                            "description": "Return an allow or deny decision for one Agent tool call.",
                            "input": {"type":"object","additionalProperties":true},
                            "output": {
                                "oneOf": [
                                    {"type":"object","additionalProperties":false,"properties":{"decision":{"const":"allow"}},"required":["decision"]},
                                    {"type":"object","additionalProperties":false,"properties":{"decision":{"const":"deny"},"reason":{"type":"string","minLength":1,"maxLength":2048}},"required":["decision","reason"]}
                                ]
                            },
                            "effect": "read"
                        }
                    },
                    "bindings": [{"point":"agent.before_tool","action":"before_tool"}],
                    "dataVersion": 0,
                    "migrations": [],
                    "configSchema": {"type":"object","additionalProperties":false},
                    "secrets": [],
                    "permissions": []
                }),
                vec![(
                    "service/main.mjs",
                    b"export async function activate() { return { async invoke(action) { if (action !== 'before_tool') throw new Error('unsupported action'); return { decision: 'allow' }; } }; }\n".as_slice(),
                )],
                "Before Tool Guard",
            ),
            Some(_) => {
                return Err(PluginHttpError::bad_request("unknown Plugin Draft template"));
            }
        };
        state.drafts.write(
            &owner,
            &draft_id,
            "nomifun.plugin.json",
            serde_json::to_vec_pretty(&manifest)
                .map_err(|error| PluginHttpError::internal(error.to_string()))?
                .as_slice(),
        )?;
        for (path, bytes) in files {
            state.drafts.write(&owner, &draft_id, path, bytes)?;
        }
        (None, None, name.to_owned())
    };
    let now = now_ms();
    let draft = PluginDraftRecord {
        owner_user_id: owner.clone(),
        draft_id: draft_id.clone(),
        revision: 1,
        plugin_id,
        base_revision,
        name,
        workspace_path: workspace.to_string_lossy().into_owned(),
        source_conversation_id: source.as_ref().map(|value| value.conversation_id.clone()),
        source_message_id: source.as_ref().map(|value| value.message_id.clone()),
        source_operation_key: source.as_ref().map(|value| value.operation_key.clone()),
        source_request_digest: source.as_ref().map(|_| request_digest.clone()),
        verification: serde_json::json!({"edit_revision": 1}),
        imported_context: json!({}),
        status: PluginDraftStatus::Ready,
        last_error: None,
        created_at_ms: now,
        updated_at_ms: now,
    };
    if let Err(error) = state.repository.create_draft(&draft).await {
        let _ = state.drafts.discard(&owner, &draft_id);
        if let Some(source) = &source {
            if let Some(existing) = find_source_draft(state, &owner, source, &request_digest).await? {
                return draft_detail(state, &existing);
            }
        }
        return Err(error.into());
    }
    Ok(draft_detail(&state, &draft)?)
}

async fn find_source_draft(
    state: &PluginRouterState, owner: &str, source: &PluginDraftSource, request_digest: &str,
) -> Result<Option<PluginDraftRecord>, PluginHttpError> {
    let id: Option<String> = nomifun_db::sqlx::query_scalar(
        "SELECT draft_id FROM plugin_drafts WHERE owner_user_id = ? AND source_operation_key = ?"
    ).bind(owner).bind(&source.operation_key).fetch_optional(state.repository.pool()).await
        .map_err(|error| PluginHttpError::internal(error.to_string()))?;
    let Some(id) = id else { return Ok(None); };
    let existing = draft_owned(state, owner, &id).await?;
    if existing.source_conversation_id.as_deref() != Some(source.conversation_id.as_str())
        || existing.source_message_id.as_deref() != Some(source.message_id.as_str())
        || existing.source_request_digest.as_deref() != Some(request_digest)
    {
        return Err(PluginHttpError::conflict("Draft creation replay differs from the admitted request"));
    }
    Ok(Some(existing))
}

async fn list_drafts(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<PluginDraftListResponseDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(list_drafts_owned(&state, &user.id.to_string()).await?)))
}

pub(super) async fn list_drafts_owned(state: &PluginRouterState, owner: &str) -> Result<PluginDraftListResponseDto, PluginHttpError> {
    let owner = owner.to_owned();
    let drafts = state
        .repository
        .list_drafts(&owner)
        .await?
        .iter()
        .map(|draft| draft_summary(&state, draft))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PluginDraftListResponseDto { drafts })
}

async fn get_draft(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(draft_id): AxumPath<String>,
) -> Result<Json<ApiResponse<PluginDraftDetailDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(get_draft_owned(&state, &user.id.to_string(), &draft_id).await?)))
}

pub(super) async fn get_draft_owned(state: &PluginRouterState, owner: &str, draft_id: &str) -> Result<PluginDraftDetailDto, PluginHttpError> {
    let draft = draft_owned(state, owner, draft_id).await?;
    Ok(draft_detail(&state, &draft)?)
}

async fn replace_draft_file(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(draft_id): AxumPath<String>,
    Json(request): Json<ReplacePluginDraftFileRequest>,
) -> Result<Json<ApiResponse<PluginDraftDetailDto>>, PluginHttpError> {
    let mut draft = draft(&state, &user, &draft_id).await?;
    require_draft_revision(&draft, request.expected_revision)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(request.content_base64)
        .map_err(|_| PluginHttpError::bad_request("content_base64 is invalid"))?;
    let mut files = state.drafts.freeze(&draft.owner_user_id, &draft.draft_id)?;
    files.insert(request.path, bytes);
    let updated = publish_draft_edit(&state, &mut draft, &files, request.expected_revision).await?;
    Ok(Json(ApiResponse::ok(draft_detail(&state, &updated)?)))
}

async fn delete_draft_file(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(draft_id): AxumPath<String>,
    Json(request): Json<DeletePluginDraftFileRequest>,
) -> Result<Json<ApiResponse<PluginDraftDetailDto>>, PluginHttpError> {
    let mut draft = draft(&state, &user, &draft_id).await?;
    require_draft_revision(&draft, request.expected_revision)?;
    if request.path == "nomifun.plugin.json" {
        return Err(PluginHttpError::bad_request("the Plugin manifest cannot be deleted"));
    }
    let mut files = state.drafts.freeze(&draft.owner_user_id, &draft.draft_id)?;
    if files.remove(&request.path).is_none() {
        return Err(PluginHttpError::bad_request("Draft file does not exist"));
    }
    let updated = publish_draft_edit(&state, &mut draft, &files, request.expected_revision).await?;
    Ok(Json(ApiResponse::ok(draft_detail(&state, &updated)?)))
}

/// Keep the filesystem lock through the row CAS. A losing editor restores the
/// package it actually replaced, never leaving rejected bytes in the draft.
async fn publish_draft_edit(
    state: &PluginRouterState, draft: &mut PluginDraftRecord,
    files: &BTreeMap<String, Vec<u8>>, expected_revision: u64,
) -> Result<PluginDraftRecord, PluginHttpError> {
    let mut replacement = state.drafts.stage_exact_replacement(
        &draft.owner_user_id, &draft.draft_id, files, &NeverCancel,
    )?;
    mark_draft_edit(draft)?;
    draft.updated_at_ms = now_ms();
    revoke_draft_surfaces(state, &draft.draft_id).await?;
    replacement.publish()?;
    match state.repository.update_draft(draft, expected_revision).await {
        Ok(updated) => { replacement.commit()?; Ok(updated) }
        Err(error) => {
            replacement.rollback().map_err(|rollback| PluginHttpError::internal(
                format!("{error}; draft rollback failed: {rollback}")))?;
            Err(error.into())
        }
    }
}

async fn preview_draft(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(draft_id): AxumPath<String>,
    Json(request): Json<PreviewPluginDraftRequest>,
) -> Result<Json<ApiResponse<PluginDraftPreviewResponseDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(preview_draft_owned(&state, &user.id.to_string(), &draft_id, request).await?)))
}

pub(super) async fn preview_draft_owned(state: &PluginRouterState, owner: &str, draft_id: &str, request: PreviewPluginDraftRequest) -> Result<PluginDraftPreviewResponseDto, PluginHttpError> {
    let draft = draft_owned(state, owner, draft_id).await?;
    require_draft_revision(&draft, request.expected_revision)?;
    let files = state.drafts.freeze(&draft.owner_user_id, &draft.draft_id)?;
    let imported = state.artifacts.import_files(&files, &NeverCancel)?;
    let artifact = imported.stored.artifact.clone();
    validate_config_value(&artifact.manifest, &request.config)?;
    validate_requested_credential_bindings(
        &state,
        &artifact.manifest,
        &request.access.credential_bindings,
    )
    .await?;
    if !request.access.permissions.is_subset(&artifact.manifest.permissions) {
        return Err(PluginHttpError::bad_request("Preview requested undeclared permissions"));
    }
    let runtime_plugin_id = PluginId::from(draft.draft_id.as_ref().to_owned());
    let mut sessions = state.surfaces.lock().await;
    let session_id = sessions
        .iter()
        .find_map(|(session_id, session)| {
            (session.owner_user_id == draft.owner_user_id
                && session.draft_id.as_ref() == Some(&draft.draft_id)
                && session.is_preview)
                .then(|| session_id.clone())
        })
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    let existing_preview = sessions.remove(&session_id);
    let preview_config = request.config.clone();
    let (data_root, generation) = match existing_preview {
        Some(mut session) if session.draft_id.as_ref() == Some(&draft.draft_id) && session.is_preview => {
            let handle = match &session.data_root {
                SurfaceDataRoot::Preview(root) => root.handle().clone(),
                SurfaceDataRoot::Installed(_) => return Err(PluginHttpError::conflict("Preview session changed kind")),
            };
            session.generation = session.generation.saturating_add(1);
            let generation = session.generation;
            let root = match session.data_root {
                SurfaceDataRoot::Preview(root) => root,
                SurfaceDataRoot::Installed(_) => unreachable!(),
            };
            (SurfaceDataRoot::Preview(root), (handle, generation))
        }
        Some(_) | None => {
            let preview = if let Some(plugin_id) = &draft.plugin_id {
                let plugin = state
                    .repository
                    .get_plugin(&draft.owner_user_id, plugin_id)
                    .await?
                    .ok_or(PluginRepositoryError::NotFound)?;
                let source = state.data_roots.open_generation(
                    plugin_id.clone(),
                    DataGeneration::new(plugin.data_generation)?,
                )?;
                state.data_roots.clone_preview_for(
                    &source,
                    runtime_plugin_id.clone(),
                    &session_id,
                )?
            } else {
                state
                    .data_roots
                    .create_empty_preview(runtime_plugin_id.clone(), &session_id)?
            };
            let handle = preview.handle().clone();
            (SurfaceDataRoot::Preview(preview), (handle, 1))
        }
    };
    let (runtime_root, generation) = generation;
    let preview_plugin = PluginRecord {
        owner_user_id: draft.owner_user_id.clone(),
        plugin_id: runtime_plugin_id.clone(),
        package_id: artifact.manifest.id.clone(),
        name: artifact.manifest.name.clone(),
        description: artifact.manifest.description.clone(),
        enabled: true,
        trashed_at_ms: None,
        active_artifact_digest: artifact.artifact_digest.clone(),
        previous_artifact_digest: None,
        data_generation: runtime_root.generation().as_str().to_owned(),
        previous_data_generation: None,
        revision: draft.revision,
        config: preview_config.clone(),
        last_error: None,
        created_at_ms: now_ms(),
        updated_at_ms: now_ms(),
    };
    let entrypoint = artifact.manifest.entrypoints.ui.clone().unwrap_or_default();
    let session = SurfaceSession {
        owner_user_id: draft.owner_user_id.clone(),
        plugin_id: draft.plugin_id.clone(),
        runtime_plugin_id: runtime_plugin_id.clone(),
        draft_id: Some(draft.draft_id.clone()),
        artifact: artifact.clone(),
        asset_root: PathBuf::from(&draft.workspace_path),
        data_root,
        generation,
        is_preview: true,
        config: preview_config,
        granted_permissions: request.access.permissions.clone(),
        completed_calls: HashMap::new(),
    };
    sessions.insert(session_id.clone(), session);
    drop(sessions);
    if let Err(error) = state
        .runtime
        .activate_preview(PluginRuntimeContext {
            owner_user_id: draft.owner_user_id.clone(),
            plugin: preview_plugin.clone(),
            artifact: artifact.clone(),
            data_root: runtime_root,
            credential_bindings: request.access.credential_bindings,
            granted_permissions: request.access.permissions,
        })
        .await
    {
        let _ = state.runtime.remove(&preview_plugin.plugin_id).await;
        state.surfaces.lock().await.remove(&session_id);
        return Err(PluginHttpError::unavailable(&error));
    }
    let mut preview_actions = action_publications(&preview_plugin, &artifact);
    for action in &mut preview_actions {
        // Preview Actions are callable by their Surface but never published to
        // Agent/Desktop/Automation consumers.
        action.bindings.clear();
    }
    if let Err(error) = state.registry.replace_plugin(
        preview_plugin.plugin_id.clone(),
        artifact.artifact_digest.clone(),
        true,
        preview_actions
            .into_iter()
            .map(PluginActionRegistration::from)
            .collect(),
    ) {
        let _ = state.runtime.remove(&preview_plugin.plugin_id).await;
        state.surfaces.lock().await.remove(&session_id);
        return Err(error.into());
    }
    Ok(PluginDraftPreviewResponseDto {
        draft_revision: draft.revision,
        descriptor: PluginSurfaceDescriptorDto {
            plugin_id: draft.plugin_id.as_ref().map(|id| id.as_ref().to_owned()),
            draft_id: Some(draft.draft_id.as_ref().to_owned()),
            artifact_digest: artifact.artifact_digest.as_ref().to_owned(),
            surface_session_id: session_id,
            surface_generation: generation,
            entrypoint,
            is_preview: true,
        },
    })
}

async fn save_draft(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(draft_id): AxumPath<String>,
    Json(request): Json<SavePluginDraftRequest>,
) -> Result<Json<ApiResponse<SavePluginDraftResponseDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(save_draft_owned(&state, &user.id.to_string(), &draft_id, request).await?)))
}

pub(super) async fn save_draft_owned(state: &PluginRouterState, owner: &str, draft_id: &str, request: SavePluginDraftRequest) -> Result<SavePluginDraftResponseDto, PluginHttpError> {
    let draft = draft_owned(state, owner, draft_id).await?;
    require_draft_revision(&draft, request.expected_revision)?;
    let files = state.drafts.freeze(&draft.owner_user_id, &draft.draft_id)?;
    let artifact = state.artifacts.inspect_files(&files, &NeverCancel)?;
    validate_config_value(&artifact.manifest, &request.config)?;
    validate_requested_credential_bindings(
        &state,
        &artifact.manifest,
        &request.credential_bindings,
    )
    .await?;
    let existing = match &draft.plugin_id {
        Some(plugin_id) => state.repository.get_plugin(&draft.owner_user_id, plugin_id).await?,
        None => None,
    };
    if request.expected_plugin_revision != draft.base_revision {
        return Err(PluginHttpError::conflict("Draft base Plugin revision changed"));
    }
    if let Some(plugin) = existing.as_ref().filter(|plugin|
        plugin.is_available() && plugin.last_error.is_none()
            && Some(plugin.revision) == draft.base_revision
            && plugin.active_artifact_digest == artifact.artifact_digest
            && plugin.config == request.config)
    {
        let inventory = state.repository.inventory(owner, &plugin.plugin_id).await?
            .ok_or(PluginRepositoryError::NotFound)?;
        if inventory.credential_bindings == request.credential_bindings {
            revoke_draft_surfaces(state, &draft.draft_id).await?;
            return Ok(SavePluginDraftResponseDto {
                draft: draft_summary(state, &draft)?,
                result: PluginInstallOutcomeDto::Installed {
                    plugin: Box::new(detail_dto(state, &inventory).await?),
                },
            });
        }
    }
    let confirmation = consume_confirmation(
        &state,
        &draft.owner_user_id,
        &artifact,
        request.permission_confirmation_id.as_deref(),
        existing.as_ref(),
    )
    .await?;
    if let Some(confirmation) = confirmation.required {
        return Ok(SavePluginDraftResponseDto {
            draft: draft_summary(&state, &draft)?,
            result: PluginInstallOutcomeDto::ConfirmationRequired { confirmation },
        });
    }
    let target = match (&draft.plugin_id, draft.base_revision) {
        (Some(plugin_id), Some(expected_revision)) => InstallTarget::Existing {
            plugin_id: plugin_id.clone(),
            expected_revision,
        },
        (None, None) => InstallTarget::new(),
        _ => return Err(PluginHttpError::conflict("Draft Plugin identity is inconsistent")),
    };
    revoke_draft_surfaces(&state, &draft.draft_id).await?;
    if let InstallTarget::Existing { plugin_id, .. } = &target {
        revoke_plugin_surfaces(&state, plugin_id).await?;
    }
    let outcome = state
        .install
        .install_draft_files(
            InstallArtifactRequest {
                owner_user_id: draft.owner_user_id.clone(),
                target,
                local_package_id: None,
                config: request.config,
                credential_bindings: request.credential_bindings,
                confirmed_permissions: confirmation.permissions,
                confirmed_secret_slots: confirmation.secret_slots,
                trusted_local_service_confirmed: confirmation.trusted_local_service,
            },
            &files,
            nomifun_plugin_platform::DraftInstallAssociation {
                draft_id: draft.draft_id.clone(), expected_revision: request.expected_revision,
            },
            None,
        )
        .await?;
    let updated = draft_owned(state, owner, draft_id).await?;
    let inventory = state
        .repository
        .inventory(&draft.owner_user_id, &outcome.plugin.plugin_id)
        .await?
        .ok_or(PluginRepositoryError::NotFound)?;
    Ok(SavePluginDraftResponseDto {
        draft: draft_summary(&state, &updated)?,
        result: PluginInstallOutcomeDto::Installed {
            plugin: Box::new(detail_dto(&state, &inventory).await?),
        },
    })
}

async fn delete_draft(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(draft_id): AxumPath<String>,
    Json(request): Json<DeletePluginDraftRequest>,
) -> Result<Json<ApiResponse<bool>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(delete_draft_owned(&state, &user.id.to_string(), &draft_id, request).await?)))
}

pub(super) async fn delete_draft_owned(state: &PluginRouterState, owner: &str, draft_id: &str, request: DeletePluginDraftRequest) -> Result<bool, PluginHttpError> {
    let draft = draft_owned(state, owner, draft_id).await?;
    require_draft_revision(&draft, request.expected_revision)?;
    revoke_draft_surfaces(&state, &draft.draft_id).await?;
    state.drafts.discard(&draft.owner_user_id, &draft.draft_id)?;
    state
        .repository
        .delete_draft(&draft.owner_user_id, &draft.draft_id)
        .await?;
    Ok(true)
}

async fn export_package(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(plugin_id): AxumPath<String>,
    Json(request): Json<ExportPluginPackageRequest>,
) -> Result<Json<ApiResponse<PluginExportResultDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(export_package_owned(&state, &user.id.to_string(), &plugin_id, request).await?)))
}

pub(super) async fn export_package_owned(state: &PluginRouterState, owner: &str, plugin_id: &str, request: ExportPluginPackageRequest) -> Result<PluginExportResultDto, PluginHttpError> {
    let inventory = inventory_owned(state, owner, plugin_id).await?;
    require_exportable(&inventory, request.expected_revision)?;
    let stored = state.artifacts.load(&inventory.plugin.active_artifact_digest)?;
    let destination = PathBuf::from(&request.destination_path);
    let artifact = if destination_is_zip(&destination) {
        state.transfers.export_package_zip(
            nomifun_plugin_platform::PluginPackageExport {
                artifact_digest: &inventory.plugin.active_artifact_digest,
                package_root: &stored.package_root,
                include_source: request.include_source,
            },
            &destination,
        )?
    } else {
        state.transfers.export_package_directory(
            nomifun_plugin_platform::PluginPackageExport {
                artifact_digest: &inventory.plugin.active_artifact_digest,
                package_root: &stored.package_root,
                include_source: request.include_source,
            },
            &destination,
        )?
    };
    Ok(PluginExportResultDto {
        destination_path: request.destination_path,
        digest: artifact.artifact_digest.as_ref().to_owned(),
        size_bytes: exported_size(&destination)?,
    })
}

async fn export_backup(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(plugin_id): AxumPath<String>,
    Json(request): Json<ExportPluginBackupRequest>,
) -> Result<Json<ApiResponse<PluginExportResultDto>>, PluginHttpError> {
    let inventory = inventory(&state, &user, &plugin_id).await?;
    require_exportable(&inventory, request.expected_revision)?;
    let stored = state.artifacts.load(&inventory.plugin.active_artifact_digest)?;
    let generation = state.data_roots.open_generation(
        inventory.plugin.plugin_id.clone(),
        DataGeneration::new(inventory.plugin.data_generation.clone())?,
    )?;
    let grants = inventory
        .artifact
        .artifact
        .manifest
        .permissions
        .iter()
        .map(|permission| nomifun_plugin_platform::PluginGrantMetadata {
            permission: permission.clone(),
            granted: inventory
                .grants
                .get(permission)
                .is_some_and(|grant| grant.granted),
        })
        .collect::<Vec<_>>();
    let credential_slots = inventory
        .artifact
        .artifact
        .manifest
        .secrets
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let export = nomifun_plugin_platform::PluginBackupExport {
        plugin_id: &inventory.plugin.plugin_id,
        artifact_digest: &inventory.plugin.active_artifact_digest,
        generation: &inventory.plugin.data_generation,
        package_root: &stored.package_root,
        generation_root: generation.path(),
        config: &inventory.plugin.config,
        grants: &grants,
        credential_slots: &credential_slots,
    };
    let destination = PathBuf::from(&request.destination_path);
    let descriptor = if destination_is_zip(&destination) {
        state.transfers.export_backup_zip(export, &destination)?
    } else {
        state.transfers.export_backup_directory(export, &destination)?
    };
    Ok(Json(ApiResponse::ok(PluginExportResultDto {
        destination_path: request.destination_path,
        digest: descriptor.bundle_digest.as_ref().to_owned(),
        size_bytes: exported_size(&destination)?,
    })))
}

fn require_exportable(
    inventory: &PluginInventory,
    expected_revision: u64,
) -> Result<(), PluginHttpError> {
    if inventory.plugin.revision != expected_revision || inventory.plugin.trashed_at_ms.is_some() {
        return Err(PluginHttpError::conflict("Plugin is unavailable or changed"));
    }
    Ok(())
}

fn destination_is_zip(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

fn exported_size(path: &Path) -> Result<u64, PluginHttpError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| PluginHttpError::internal(error.to_string()))?;
    if metadata.file_type().is_symlink() {
        return Err(PluginHttpError::internal("export destination became a symbolic link"));
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut total = 0u64;
    for entry in walkdir::WalkDir::new(path).follow_links(false) {
        let entry = entry.map_err(|error| PluginHttpError::internal(error.to_string()))?;
        let metadata = entry
            .metadata()
            .map_err(|error| PluginHttpError::internal(error.to_string()))?;
        if metadata.file_type().is_symlink() {
            return Err(PluginHttpError::internal("export contains a symbolic link"));
        }
        if metadata.is_file() {
            total = total
                .checked_add(metadata.len())
                .ok_or_else(|| PluginHttpError::internal("export size overflow"))?;
        }
    }
    Ok(total)
}

async fn set_enabled(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(plugin_id): AxumPath<String>,
    Json(request): Json<SetPluginEnabledRequest>,
) -> Result<Json<ApiResponse<PluginDetailDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(set_enabled_owned(&state, &user.id.to_string(), &plugin_id, request).await?)))
}

pub(super) async fn set_enabled_owned(state: &PluginRouterState, owner: &str, plugin_id: &str, request: SetPluginEnabledRequest) -> Result<PluginDetailDto, PluginHttpError> {
    let owner = owner.to_owned();
    let plugin_id = parse_plugin_id(&plugin_id)?;
    revoke_plugin_surfaces(&state, &plugin_id).await?;
    state
        .install
        .set_enabled(
            &owner,
            &plugin_id,
            request.expected_revision,
            request.enabled,
        )
        .await?;
    let inventory = state
        .repository
        .inventory(&owner, &plugin_id)
        .await?
        .ok_or(PluginRepositoryError::NotFound)?;
    Ok(detail_dto(&state, &inventory).await?)
}

async fn configure(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(plugin_id): AxumPath<String>,
    Json(request): Json<ConfigurePluginRequest>,
) -> Result<Json<ApiResponse<PluginDetailDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(configure_owned(&state, &user.id.to_string(), &plugin_id, request).await?)))
}

pub(super) async fn configure_owned(state: &PluginRouterState, owner: &str, plugin_id: &str, request: ConfigurePluginRequest) -> Result<PluginDetailDto, PluginHttpError> {
    let owner = owner.to_owned();
    let plugin_id = parse_plugin_id(&plugin_id)?;
    let inventory = state
        .repository
        .inventory(&owner, &plugin_id)
        .await?
        .ok_or(PluginRepositoryError::NotFound)?;
    validate_config_value(&inventory.artifact.artifact.manifest, &request.config)?;
    let declared_slots = inventory
        .artifact
        .artifact
        .manifest
        .secrets
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if request
        .credential_bindings
        .keys()
        .any(|slot| !declared_slots.contains(slot))
    {
        return Err(PluginHttpError::bad_request("Credential slot is not declared"));
    }
    for credential_id in request.credential_bindings.values().flatten() {
        super::plugin_ports::validate_plugin_credential_reference(
            state.repository.pool(),
            credential_id,
        )
        .await
        .map_err(|_| PluginHttpError::bad_request("Credential reference is invalid"))?;
    }
    if request
        .grants
        .keys()
        .any(|permission| !inventory.artifact.artifact.manifest.permissions.contains(permission))
    {
        return Err(PluginHttpError::bad_request("Grant is not declared"));
    }
    let bindings = request
        .credential_bindings
        .into_iter()
        .filter_map(|(slot, credential)| credential.map(|credential| (slot, credential)))
        .collect();
    revoke_plugin_surfaces(&state, &plugin_id).await?;
    state
        .install
        .configure(
            &owner,
            &plugin_id,
            request.expected_revision,
            request.config,
            bindings,
            request.grants,
        )
        .await?;
    let inventory = state
        .repository
        .inventory(&owner, &plugin_id)
        .await?
        .ok_or(PluginRepositoryError::NotFound)?;
    Ok(detail_dto(&state, &inventory).await?)
}

async fn restore(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(plugin_id): AxumPath<String>,
    Json(request): Json<RestorePluginRequest>,
) -> Result<Json<ApiResponse<PluginDetailDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(restore_owned(&state, &user.id.to_string(), &plugin_id, request).await?)))
}

pub(super) async fn restore_owned(state: &PluginRouterState, owner: &str, plugin_id: &str, request: RestorePluginRequest) -> Result<PluginDetailDto, PluginHttpError> {
    let owner = owner.to_owned();
    let plugin_id = parse_plugin_id(&plugin_id)?;
    let restore_data = request.mode == PluginRestoreModeDto::PreviousCodeAndData;
    if restore_data && !request.acknowledge_data_loss {
        return Err(PluginHttpError::bad_request(
            "full Previous restore requires data-loss acknowledgement",
        ));
    }
    revoke_plugin_surfaces(&state, &plugin_id).await?;
    if request.mode == PluginRestoreModeDto::FromTrash {
        state
            .install
            .restore_from_trash(&owner, &plugin_id, request.expected_revision)
            .await?;
    } else {
        state
            .install
            .restore_previous(
                &owner,
                &plugin_id,
                request.expected_revision,
                restore_data,
            )
            .await?;
    }
    let inventory = state
        .repository
        .inventory(&owner, &plugin_id)
        .await?
        .ok_or(PluginRepositoryError::NotFound)?;
    Ok(detail_dto(&state, &inventory).await?)
}

async fn trash(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(plugin_id): AxumPath<String>,
    Json(request): Json<TrashPluginRequest>,
) -> Result<Json<ApiResponse<PluginDetailDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(trash_owned(&state, &user.id.to_string(), &plugin_id, request).await?)))
}

pub(super) async fn trash_owned(state: &PluginRouterState, owner: &str, plugin_id: &str, request: TrashPluginRequest) -> Result<PluginDetailDto, PluginHttpError> {
    let owner = owner.to_owned();
    let plugin_id = parse_plugin_id(&plugin_id)?;
    revoke_plugin_surfaces(&state, &plugin_id).await?;
    state
        .install
        .trash(&owner, &plugin_id, request.expected_revision)
        .await?;
    let inventory = state
        .repository
        .inventory(&owner, &plugin_id)
        .await?
        .ok_or(PluginRepositoryError::NotFound)?;
    Ok(detail_dto(&state, &inventory).await?)
}

async fn permanent_delete(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(plugin_id): AxumPath<String>,
    Json(request): Json<DeletePluginRequest>,
) -> Result<Json<ApiResponse<PluginLibraryResponseDto>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(permanent_delete_owned(&state, &user.id.to_string(), &plugin_id, request).await?)))
}

pub(super) async fn permanent_delete_owned(state: &PluginRouterState, owner: &str, plugin_id: &str, request: DeletePluginRequest) -> Result<PluginLibraryResponseDto, PluginHttpError> {
    if !request.acknowledge_permanent_delete {
        return Err(PluginHttpError::bad_request("permanent delete must be acknowledged"));
    }
    let owner = owner.to_owned();
    let plugin_id = parse_plugin_id(&plugin_id)?;
    revoke_plugin_surfaces(&state, &plugin_id).await?;
    state
        .install
        .permanent_delete(&owner, &plugin_id, request.expected_revision)
        .await?;
    let records = state.repository.list_plugins(&owner).await?;
    let mut plugins = Vec::with_capacity(records.len());
    for record in records {
        let inventory = state
            .repository
            .inventory(&owner, &record.plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)?;
        plugins.push(summary_dto(&state, &inventory).await?);
    }
    let revision = plugins.iter().map(|plugin| plugin.revision).max().unwrap_or(0);
    Ok(PluginLibraryResponseDto {
        revision,
        plugins,
    })
}

async fn open_surface(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(plugin_id): AxumPath<String>,
    Json(request): Json<OpenPluginSurfaceRequest>,
) -> Result<Json<ApiResponse<PluginSurfaceDescriptorDto>>, PluginHttpError> {
    let owner = user.id.to_string();
    Ok(Json(ApiResponse::ok(open_surface_owned(&state, &owner, &plugin_id, request).await?)))
}

pub(super) async fn open_surface_owned(
    state: &PluginRouterState, owner: &str, plugin_id: &str, request: OpenPluginSurfaceRequest,
) -> Result<PluginSurfaceDescriptorDto, PluginHttpError> {
    let plugin_id = parse_plugin_id(&plugin_id)?;
    let inventory = state
        .repository
        .inventory(&owner, &plugin_id)
        .await?
        .ok_or(PluginRepositoryError::NotFound)?;
    if inventory.plugin.revision != request.expected_revision || !inventory.plugin.is_available() {
        return Err(PluginHttpError::conflict("Plugin is unavailable or changed"));
    }
    if !inventory.artifact.artifact.manifest.has_ui() {
        return Err(PluginHttpError::bad_request("this headless Plugin has no App Surface"));
    }
    let stored = state
        .artifacts
        .load(&inventory.plugin.active_artifact_digest)?;
    let session_id = Uuid::now_v7().to_string();
    let generation = 1;
    let root = state.data_roots.open_generation(
        plugin_id.clone(),
        DataGeneration::new(inventory.plugin.data_generation.clone())?,
    )?;
    let descriptor = PluginSurfaceDescriptorDto {
        plugin_id: Some(plugin_id.as_ref().to_owned()),
        draft_id: None,
        artifact_digest: inventory.plugin.active_artifact_digest.as_ref().to_owned(),
        surface_session_id: session_id.clone(),
        surface_generation: generation,
        entrypoint: "ui/index.html".into(),
        is_preview: false,
    };
    state.surfaces.lock().await.insert(
        session_id.clone(),
        SurfaceSession {
            owner_user_id: owner.to_owned(),
            plugin_id: Some(plugin_id.clone()),
            runtime_plugin_id: plugin_id,
            draft_id: None,
            artifact: inventory.artifact.artifact,
            asset_root: stored.package_root,
            data_root: SurfaceDataRoot::Installed(root),
            generation,
            is_preview: false,
            config: inventory.plugin.config,
            granted_permissions: inventory
                .grants
                .into_iter()
                .filter_map(|(permission, grant)| grant.granted.then_some(permission))
                .collect(),
            completed_calls: HashMap::new(),
        },
    );
    Ok(descriptor)
}

pub(super) async fn revoke_draft_surfaces(
    state: &PluginRouterState,
    draft_id: &PluginDraftId,
) -> Result<(), PluginHttpError> {
    let removed = {
        let mut sessions = state.surfaces.lock().await;
        let session_ids = sessions
            .iter()
            .filter_map(|(session_id, session)| {
                (session.draft_id.as_ref() == Some(draft_id) && session.is_preview)
                    .then(|| session_id.clone())
            })
            .collect::<Vec<_>>();
        session_ids
            .into_iter()
            .filter_map(|session_id| sessions.remove(&session_id))
            .collect::<Vec<_>>()
    };
    revoke_removed_preview_surfaces(state, removed).await
}

async fn revoke_plugin_surfaces(
    state: &PluginRouterState,
    plugin_id: &PluginId,
) -> Result<(), PluginHttpError> {
    let removed = {
        let mut sessions = state.surfaces.lock().await;
        let session_ids = sessions
            .iter()
            .filter_map(|(session_id, session)| {
                (session.plugin_id.as_ref() == Some(plugin_id)).then(|| session_id.clone())
            })
            .collect::<Vec<_>>();
        session_ids
            .into_iter()
            .filter_map(|session_id| sessions.remove(&session_id))
            .collect::<Vec<_>>()
    };
    revoke_removed_preview_surfaces(state, removed).await
}

async fn revoke_removed_preview_surfaces(
    state: &PluginRouterState,
    removed: Vec<SurfaceSession>,
) -> Result<(), PluginHttpError> {
    for surface in removed {
        if !surface.is_preview {
            continue;
        }
        state
            .runtime
            .remove(&surface.runtime_plugin_id)
            .await
            .map_err(|error| PluginHttpError::unavailable(&error))?;
        state.registry.remove_plugin(&surface.runtime_plugin_id)?;
    }
    Ok(())
}

async fn close_surface(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<ClosePluginSurfaceRequest>,
) -> Result<Json<ApiResponse<bool>>, PluginHttpError> {
    Ok(Json(ApiResponse::ok(close_surface_owned(&state, &user.id.to_string(), request).await?)))
}

pub(super) async fn close_surface_owned(state: &PluginRouterState, owner: &str, request: ClosePluginSurfaceRequest) -> Result<bool, PluginHttpError> {
    let owner = owner.to_owned();
    let mut sessions = state.surfaces.lock().await;
    let session = sessions
        .get(&request.surface_session_id)
        .ok_or(PluginRepositoryError::NotFound)?;
    if session.owner_user_id != owner || session.generation != request.surface_generation {
        return Err(PluginHttpError::not_found());
    }
    let session = sessions
        .remove(&request.surface_session_id)
        .expect("Surface was checked above");
    drop(sessions);
    if session.is_preview {
        state
            .runtime
            .remove(&session.runtime_plugin_id)
            .await
            .map_err(|error| PluginHttpError::unavailable(&error))?;
        state.registry.remove_plugin(&session.runtime_plugin_id)?;
    }
    Ok(true)
}

async fn surface_asset(
    State(state): State<PluginRouterState>,
    headers: HeaderMap,
    AxumPath((owner_id, surface_session_id, surface_generation, artifact_digest, asset_path)):
        AxumPath<(String, String, u64, String, String)>,
) -> Result<Response<Body>, PluginHttpError> {
    let sessions = state.surfaces.lock().await;
    let session = sessions
        .get(&surface_session_id)
        .ok_or(PluginRepositoryError::NotFound)?;
    let expected_owner = if session.is_preview {
        session.draft_id.as_ref().map(AsRef::as_ref)
    } else {
        session.plugin_id.as_ref().map(AsRef::as_ref)
    };
    let owner_matches = expected_owner == Some(owner_id.as_str());
    if !owner_matches
        || session.generation != surface_generation
        || session.artifact.artifact_digest.as_ref() != artifact_digest
    {
        return Err(PluginHttpError::not_found());
    }
    let asset_path = asset_path.trim_start_matches('/');
    if !asset_path.starts_with("ui/") {
        return Err(PluginHttpError::not_found());
    }
    let file = session
        .artifact
        .files
        .iter()
        .find(|file| file.normalized_relative_path == asset_path)
        .ok_or(PluginRepositoryError::NotFound)?;
    let path = safe_asset_path(&session.asset_root, asset_path)?;
    let bytes = std::fs::read(&path).map_err(|error| PluginHttpError::internal(error.to_string()))?;
    if bytes.len() as u64 != file.size_bytes
        || hex::encode(Sha256::digest(&bytes)) != file.digest.as_ref()
    {
        return Err(PluginHttpError::conflict("Surface Artifact changed"));
    }
    let (bytes, content_type) = if asset_path == "ui/index.html" {
        (inject_sdk(&bytes)?, "text/html; charset=utf-8")
    } else {
        (bytes, content_type(asset_path))
    };
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        surface_content_security_policy(
            &headers,
            session.granted_permissions.contains("network"),
        )?,
    );
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("null"),
    );
    response.headers_mut().insert(
        axum::http::HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("cross-origin"),
    );
    Ok(response)
}

fn surface_content_security_policy(
    headers: &HeaderMap,
    network_granted: bool,
) -> Result<HeaderValue, PluginHttpError> {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 255
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric()
                        || matches!(byte, b'.' | b'-' | b':' | b'[' | b']')
                })
        });
    let origins = host
        .map(|host| format!("'self' http://{host} https://{host}"))
        .unwrap_or_else(|| "'self'".into());
    let connect_sources = if network_granted {
        "http: https: ws: wss:"
    } else {
        "'none'"
    };
    HeaderValue::from_str(&format!(
        "default-src 'none'; script-src 'unsafe-inline' {origins}; style-src 'unsafe-inline' {origins}; img-src {origins} data: blob:; font-src {origins} data:; media-src {origins} data: blob:; connect-src {connect_sources}; object-src 'none'; worker-src 'none'; base-uri 'none'; form-action 'none'"
    ))
    .map_err(|_| PluginHttpError::internal("Surface CSP could not be encoded"))
}

async fn dispatch_bridge(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<DispatchPluginBridgeRequest>,
) -> Result<Json<ApiResponse<PluginBridgeResultDto>>, PluginHttpError> {
    let owner = user.id.to_string();
    let mut sessions = state.surfaces.lock().await;
    let session = sessions
        .get_mut(&request.surface_session_id)
        .ok_or(PluginRepositoryError::NotFound)?;
    if session.owner_user_id != owner
        || session.generation != request.surface_generation
        || session.is_preview != request.is_preview
        || session.artifact.artifact_digest.as_ref() != request.artifact_digest
        || session.plugin_id.as_ref().map(AsRef::as_ref) != request.plugin_id.as_deref()
        || session.draft_id.as_ref().map(AsRef::as_ref) != request.draft_id.as_deref()
    {
        return Err(PluginHttpError::not_found());
    }
    if let Some(result) = session.completed_calls.get(&request.request.call_id) {
        return Ok(Json(ApiResponse::ok(result.clone())));
    }
    let call_id = request.request.call_id.clone();
    let dispatched = match request.request.target {
        PluginBridgeTargetDto::Actions { action, input } => {
            let own_action_prefix = format!("plugin:{}/", session.runtime_plugin_id.as_ref());
            if action.starts_with("plugin:")
                && !action.starts_with(&own_action_prefix)
                && !session.granted_permissions.contains("actions.invoke")
            {
                Err(PluginHttpError::forbidden(
                    "cross-Plugin Action invocation is not granted",
                ))
            } else {
                let (stable_action, expected_artifact_digest) = if action.starts_with("plugin:") {
                    let expected = action
                        .starts_with(&own_action_prefix)
                        .then(|| session.artifact.artifact_digest.clone());
                    (action, expected)
                } else {
                    (
                        plugin_action_id(&session.runtime_plugin_id, &action),
                        Some(session.artifact.artifact_digest.clone()),
                    )
                };
                state
                    .registry
                    .dispatch_action(
                        &stable_action,
                        nomifun_agent_contracts::StrictJsonValue(input),
                        nomifun_plugin_platform::PluginDispatchOptions {
                            expected_artifact_digest,
                            ..Default::default()
                        },
                    )
                    .await
                    .map(|result| PluginBridgeSuccessDto::Actions { result: result.0 })
                    .map_err(PluginHttpError::from)
            }
        }
        PluginBridgeTargetDto::Host { capability, input } => {
            if !session.granted_permissions.contains(&capability) {
                Err(PluginHttpError::forbidden("Host capability is not granted"))
            } else {
                state
                    .host
                    .invoke(
                        &session.runtime_plugin_id,
                        &capability,
                        input,
                        session.is_preview,
                        nomifun_plugin_platform::PluginServiceCancellation::default(),
                    )
                    .await
                    .map(|result| PluginBridgeSuccessDto::Host { result })
                    .map_err(|_| PluginHttpError::unavailable("Host capability failed"))
            }
        }
        target => dispatch_storage(session, target),
    };
    let result = match dispatched {
        Ok(result) => PluginBridgeResultDto::Success { call_id, result },
        Err(error) => PluginBridgeResultDto::Failure {
            call_id,
            error: PluginBridgeErrorDto {
                code: "PLUGIN_BRIDGE_FAILED".into(),
                message: error.to_string(),
                outcome_unknown: false,
            },
        },
    };
    if session.completed_calls.len() >= MAX_SURFACE_CALLS {
        if let Some(key) = session.completed_calls.keys().next().cloned() {
            session.completed_calls.remove(&key);
        }
    }
    session
        .completed_calls
        .insert(request.request.call_id, result.clone());
    Ok(Json(ApiResponse::ok(result)))
}

fn dispatch_storage(
    session: &SurfaceSession,
    target: PluginBridgeTargetDto,
) -> Result<PluginBridgeSuccessDto, PluginHttpError> {
    let storage = session.data_root.storage();
    Ok(match target {
        PluginBridgeTargetDto::Kv { request } => {
            let result = match request {
                PluginKvRequestDto::Get { key } => {
                    let value = storage.kv_get(&key)?;
                    PluginKvResultDto::Value {
                        value: value.value,
                        revision: value.revision,
                    }
                }
                PluginKvRequestDto::Set { key, value } => PluginKvResultDto::Written {
                    revision: storage.kv_set(&key, &value)?,
                },
                PluginKvRequestDto::Delete { key } => PluginKvResultDto::Deleted {
                    existed: storage.kv_delete(&key)?,
                },
                PluginKvRequestDto::CompareAndSwap {
                    key,
                    expected_revision,
                    value,
                } => {
                    let result = storage.kv_compare_and_swap(
                        &key,
                        expected_revision,
                        value.as_ref(),
                    )?;
                    PluginKvResultDto::CompareAndSwap {
                        applied: result.applied,
                        current_revision: result.revision,
                    }
                }
            };
            PluginBridgeSuccessDto::Kv { result }
        }
        PluginBridgeTargetDto::Db { request } => {
            let result = match request {
                PluginDatabaseRequestDto::Query { sql, parameters } => {
                    query_result(storage.db_query(&sql_statement(sql, parameters)?)?)
                }
                PluginDatabaseRequestDto::Execute { sql, parameters } => {
                    let result = storage.db_execute(&sql_statement(sql, parameters)?)?;
                    PluginDatabaseResultDto::Executed {
                        rows_affected: result.affected_rows,
                        last_insert_rowid: None,
                    }
                }
                PluginDatabaseRequestDto::Batch { statements } => {
                    let statements = statements
                        .into_iter()
                        .map(|statement| sql_statement(statement.sql, statement.parameters))
                        .collect::<Result<Vec<_>, _>>()?;
                    PluginDatabaseResultDto::Batch {
                        results: storage
                            .db_batch(&statements)?
                            .into_iter()
                            .map(|result| PluginDatabaseResultDto::Executed {
                                rows_affected: result.affected_rows,
                                last_insert_rowid: None,
                            })
                            .collect(),
                    }
                }
            };
            PluginBridgeSuccessDto::Db { result }
        }
        PluginBridgeTargetDto::Files { request } => {
            let result = match request {
                PluginFilesRequestDto::Read { path } => PluginFilesResultDto::Data {
                    content_base64: base64::engine::general_purpose::STANDARD
                        .encode(storage.file_read(&path)?),
                },
                PluginFilesRequestDto::Write {
                    path,
                    content_base64,
                    overwrite,
                } => {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(content_base64)
                        .map_err(|_| PluginHttpError::bad_request("content_base64 is invalid"))?;
                    if !overwrite && storage.file_read(&path).is_ok() {
                        return Err(PluginHttpError::conflict("file already exists"));
                    }
                    storage.file_write(&path, &bytes)?;
                    PluginFilesResultDto::Written {
                        size_bytes: bytes.len() as u64,
                    }
                }
                PluginFilesRequestDto::List { path } => PluginFilesResultDto::Entries {
                    entries: storage
                        .file_list((!path.is_empty()).then_some(path.as_str()))?
                        .into_iter()
                        .map(|entry| PluginFileEntryDto {
                            path: entry.path,
                            is_directory: entry.is_directory,
                            size_bytes: entry.size_bytes,
                        })
                        .collect(),
                },
                PluginFilesRequestDto::Delete { path } => PluginFilesResultDto::Deleted {
                    existed: storage.file_delete(&path)?,
                },
            };
            PluginBridgeSuccessDto::Files { result }
        }
        PluginBridgeTargetDto::Cache { request } => {
            let cache = session.data_root.cache();
            let result = match request {
                PluginCacheRequestDto::Get { key } => PluginCacheResultDto::Value {
                    value: cache.get(&key)?,
                },
                PluginCacheRequestDto::Set { key, value, ttl_ms } => {
                    cache.set(&key, value, ttl_ms.map(Duration::from_millis))?;
                    PluginCacheResultDto::Stored
                }
                PluginCacheRequestDto::Delete { key } => PluginCacheResultDto::Deleted {
                    existed: cache.delete(&key)?,
                },
            };
            PluginBridgeSuccessDto::Cache { result }
        }
        PluginBridgeTargetDto::Config => PluginBridgeSuccessDto::Config {
            config: session.config.clone(),
        },
        PluginBridgeTargetDto::Actions { .. } | PluginBridgeTargetDto::Host { .. } => unreachable!(),
    })
}

// DTO and validation helpers -------------------------------------------------

async fn activate_inventory_runtime(
    state: &PluginRouterState,
    inventory: &PluginInventory,
) -> Result<(), PluginHttpError> {
    let root = state.data_roots.open_generation(
        inventory.plugin.plugin_id.clone(),
        DataGeneration::new(inventory.plugin.data_generation.clone())?,
    )?;
    state
        .runtime
        .activate(PluginRuntimeContext {
            owner_user_id: inventory.plugin.owner_user_id.clone(),
            plugin: inventory.plugin.clone(),
            artifact: inventory.artifact.artifact.clone(),
            data_root: root,
            credential_bindings: inventory.credential_bindings.clone(),
            granted_permissions: inventory
                .grants
                .iter()
                .filter_map(|(permission, grant)| grant.granted.then_some(permission.clone()))
                .collect(),
        })
        .await
        .map_err(|error| PluginHttpError::unavailable(&error))?;
    Ok(())
}

fn replace_inventory_bindings(
    state: &PluginRouterState,
    inventory: &PluginInventory,
) -> Result<(), PluginHttpError> {
    state.registry.replace_plugin(
        inventory.plugin.plugin_id.clone(),
        inventory.plugin.active_artifact_digest.clone(),
        inventory.plugin.is_available(),
        action_publications(&inventory.plugin, &inventory.artifact.artifact)
            .into_iter()
            .map(PluginActionRegistration::from)
            .collect(),
    )?;
    Ok(())
}

async fn inventory(
    state: &PluginRouterState,
    user: &CurrentUser,
    plugin_id: &str,
) -> Result<PluginInventory, PluginHttpError> {
    inventory_owned(state, &user.id.to_string(), plugin_id).await
}

pub(super) async fn inventory_owned(state: &PluginRouterState, owner: &str, plugin_id: &str) -> Result<PluginInventory, PluginHttpError> {
    let plugin_id = parse_plugin_id(plugin_id)?;
    state
        .repository
        .inventory(owner, &plugin_id)
        .await?
        .ok_or_else(PluginHttpError::not_found)
}

async fn draft(
    state: &PluginRouterState,
    user: &CurrentUser,
    draft_id: &str,
) -> Result<PluginDraftRecord, PluginHttpError> {
    draft_owned(state, &user.id.to_string(), draft_id).await
}

pub(super) async fn draft_owned(state: &PluginRouterState, owner: &str, draft_id: &str) -> Result<PluginDraftRecord, PluginHttpError> {
    let draft_id = parse_draft_id(draft_id)?;
    state
        .repository
        .get_draft(owner, &draft_id)
        .await?
        .ok_or_else(PluginHttpError::not_found)
}

async fn summary_dto(
    state: &PluginRouterState,
    inventory: &PluginInventory,
) -> Result<PluginSummaryDto, PluginHttpError> {
    let manifest = &inventory.artifact.artifact.manifest;
    let previous = inventory
        .previous_artifact
        .as_ref()
        .map(|artifact| PluginArtifactDataPointerDto {
            artifact_digest: artifact.artifact.artifact_digest.as_ref().to_owned(),
            package_version: artifact.artifact.manifest.version.clone(),
            data_generation: inventory
                .plugin
                .previous_data_generation
                .clone()
                .unwrap_or_else(|| inventory.plugin.data_generation.clone()),
            data_version: artifact.artifact.manifest.data_version,
        });
    Ok(PluginSummaryDto {
        plugin_id: inventory.plugin.plugin_id.as_ref().to_owned(),
        package_id: inventory.plugin.package_id.clone(),
        display_name: inventory.plugin.name.clone(),
        description: inventory.plugin.description.clone(),
        enabled: inventory.plugin.enabled,
        trashed_at_ms: nonnegative(inventory.plugin.trashed_at_ms)?,
        revision: inventory.plugin.revision,
        active: PluginArtifactDataPointerDto {
            artifact_digest: inventory.plugin.active_artifact_digest.as_ref().to_owned(),
            package_version: manifest.version.clone(),
            data_generation: inventory.plugin.data_generation.clone(),
            data_version: manifest.data_version,
        },
        previous,
        has_ui: manifest.has_ui(),
        has_service: manifest.has_service(),
        service_mode: manifest.entrypoints.service.as_ref().map(|_| match manifest.service_mode() {
            nomifun_agent_contracts::PluginServiceMode::OnDemand => PluginServiceModeDto::OnDemand,
            nomifun_agent_contracts::PluginServiceMode::Continuous => PluginServiceModeDto::Continuous,
        }),
        action_count: manifest.actions.len().try_into().unwrap_or(u32::MAX),
        binding_count: manifest.bindings.len().try_into().unwrap_or(u32::MAX),
        runtime: runtime_observation_dto(
            state.runtime.observation(&inventory.plugin.plugin_id).await,
            inventory.plugin.enabled && manifest.has_service(),
            inventory.plugin.last_error.as_deref(),
        ),
        last_error: inventory.plugin.last_error.clone(),
        updated_at_ms: u64::try_from(inventory.plugin.updated_at_ms)
            .map_err(|_| PluginHttpError::internal("negative Plugin timestamp"))?,
    })
}

async fn detail_dto(
    state: &PluginRouterState,
    inventory: &PluginInventory,
) -> Result<PluginDetailDto, PluginHttpError> {
    let manifest = &inventory.artifact.artifact.manifest;
    let config_errors = config_errors(manifest, &inventory.plugin.config);
    let mut credential_bindings = Vec::with_capacity(manifest.secrets.len());
    for slot in &manifest.secrets {
        let credential_id = inventory.credential_bindings.get(slot).cloned();
        let status = match credential_id.as_deref() {
            None => PluginCredentialBindingStatusDto::Unbound,
            Some(credential_id) => match super::plugin_ports::plugin_credential_reference_state(
                state.repository.pool(),
                credential_id,
            )
            .await
            {
                super::plugin_ports::PluginCredentialReferenceState::Available => {
                    PluginCredentialBindingStatusDto::Bound
                }
                super::plugin_ports::PluginCredentialReferenceState::Missing => {
                    PluginCredentialBindingStatusDto::Missing
                }
                super::plugin_ports::PluginCredentialReferenceState::Invalid => {
                    PluginCredentialBindingStatusDto::Invalid
                }
            },
        };
        credential_bindings.push(PluginCredentialBindingDto {
            slot: slot.clone(),
            required: true,
            status,
            credential_id,
        });
    }
    Ok(PluginDetailDto {
        summary: summary_dto(state, inventory).await?,
        manifest: manifest_dto(manifest, Some(&inventory.plugin.plugin_id)),
        config: PluginConfigDto {
            schema: manifest.config_schema.0.clone(),
            values: inventory.plugin.config.clone(),
            valid: config_errors.is_empty(),
            validation_errors: config_errors,
        },
        credential_bindings,
        grants: manifest
            .permissions
            .iter()
            .map(|permission| {
                let grant = inventory.grants.get(permission);
                PluginGrantDto {
                    permission: permission.clone(),
                    granted: grant.is_some_and(|grant| grant.granted),
                    confirmed_at_ms: grant.and_then(|grant| u64::try_from(grant.updated_at_ms).ok()),
                }
            })
            .collect(),
    })
}

fn runtime_observation_dto(
    observation: nomifun_plugin_platform::PluginServiceObservation,
    expected_running: bool,
    last_error: Option<&str>,
) -> PluginRuntimeObservationDto {
    match observation {
        nomifun_plugin_platform::PluginServiceObservation::Stopped
            if expected_running
                && matches!(
                    last_error,
                    Some(PLUGIN_STARTUP_ACTIVATION_FAILED | PLUGIN_BINDING_RECOVERY_FAILED)
                ) =>
        {
            PluginRuntimeObservationDto {
                state: PluginObservedStateDto::Failed,
                generation: None,
                process_id: None,
                started_at_ms: None,
                error_code: last_error.map(str::to_owned),
            }
        }
        nomifun_plugin_platform::PluginServiceObservation::Stopped => {
            PluginRuntimeObservationDto {
                state: PluginObservedStateDto::Stopped,
                generation: None,
                process_id: None,
                started_at_ms: None,
                error_code: None,
            }
        }
        nomifun_plugin_platform::PluginServiceObservation::Running {
            generation,
            process_id,
        } => PluginRuntimeObservationDto {
            state: PluginObservedStateDto::Running,
            generation: Some(generation),
            process_id: Some(process_id),
            started_at_ms: None,
            error_code: None,
        },
        nomifun_plugin_platform::PluginServiceObservation::Failed { generation, code } => {
            PluginRuntimeObservationDto {
                state: PluginObservedStateDto::Failed,
                generation: Some(generation),
                process_id: None,
                started_at_ms: None,
                error_code: Some(code),
            }
        }
    }
}

fn manifest_dto(manifest: &PluginManifest, plugin_id: Option<&PluginId>) -> PluginManifestSummaryDto {
    PluginManifestSummaryDto {
        schema: manifest.schema.clone(),
        package_id: manifest.id.clone(),
        version: manifest.version.clone(),
        name: manifest.name.clone(),
        description: manifest.description.clone(),
        host_api: manifest.host_api.clone(),
        entrypoints: PluginEntrypointsSummaryDto {
            ui: manifest.entrypoints.ui.clone(),
            service: manifest.entrypoints.service.clone(),
            service_mode: manifest.entrypoints.service.as_ref().map(|_| match manifest.service_mode() {
                nomifun_agent_contracts::PluginServiceMode::OnDemand => PluginServiceModeDto::OnDemand,
                nomifun_agent_contracts::PluginServiceMode::Continuous => PluginServiceModeDto::Continuous,
            }),
        },
        actions: manifest
            .actions
            .iter()
            .map(|(id, action)| PluginActionSummaryDto {
                action_id: id.clone(),
                stable_id: plugin_id.map(|plugin_id| plugin_action_id(plugin_id, id)),
                name: action.name.clone(),
                description: action.description.clone(),
                input_schema: action.input.0.clone(),
                output_schema: action.output.0.clone(),
                effect: match action.effect {
                    PluginActionEffect::Read => PluginActionEffectDto::Read,
                    PluginActionEffect::Write => PluginActionEffectDto::Write,
                    PluginActionEffect::External => PluginActionEffectDto::External,
                },
            })
            .collect(),
        bindings: manifest
            .bindings
            .iter()
            .map(|binding| PluginBindingSummaryDto {
                point: binding_point_dto(binding.point),
                action_id: binding.action.clone(),
                action_stable_id: plugin_id.map(|plugin_id| plugin_action_id(plugin_id, &binding.action)),
                optional: binding.optional,
                supported: true,
                unavailable_reason: None,
            })
            .collect(),
        data_version: manifest.data_version,
        config_schema: manifest.config_schema.0.clone(),
        secret_slots: manifest.secrets.clone(),
        permissions: manifest.permissions.clone(),
    }
}

fn binding_point_dto(point: PluginBindingPoint) -> PluginBindingPointDto {
    match point {
        PluginBindingPoint::AgentTool => PluginBindingPointDto::AgentTool,
        PluginBindingPoint::AgentContext => PluginBindingPointDto::AgentContext,
        PluginBindingPoint::AgentBeforeModel => PluginBindingPointDto::AgentBeforeModel,
        PluginBindingPoint::AgentBeforeTool => PluginBindingPointDto::AgentBeforeTool,
        PluginBindingPoint::DesktopCommand => PluginBindingPointDto::DesktopCommand,
        PluginBindingPoint::DesktopEvent => PluginBindingPointDto::DesktopEvent,
        PluginBindingPoint::AutomationAction => PluginBindingPointDto::AutomationAction,
    }
}

pub(super) fn draft_summary(
    state: &PluginRouterState,
    draft: &PluginDraftRecord,
) -> Result<PluginDraftSummaryDto, PluginHttpError> {
    let manifest = state
        .drafts
        .freeze(&draft.owner_user_id, &draft.draft_id)
        .ok()
        .and_then(|files| files.get("nomifun.plugin.json").cloned())
        .and_then(|bytes| PluginManifest::parse(&bytes).ok());
    Ok(PluginDraftSummaryDto {
        draft_id: draft.draft_id.as_ref().to_owned(),
        source_conversation_id: draft.source_conversation_id.clone(),
        source_message_id: draft.source_message_id.clone(),
        revision: draft.revision,
        plugin_id: draft.plugin_id.as_ref().map(|id| id.as_ref().to_owned()),
        base_plugin_revision: draft.base_revision,
        package_id: manifest.as_ref().map(|manifest| manifest.id.clone()),
        delivered_artifact_digest: draft
            .verification["delivery"]["artifact_digest"]
            .as_str()
            .map(str::to_owned),
        display_name: manifest
            .as_ref()
            .map(|manifest| manifest.name.clone())
            .unwrap_or_else(|| draft.name.clone()),
        description: manifest
            .as_ref()
            .map(|manifest| manifest.description.clone())
            .unwrap_or_default(),
        status: match draft.status {
            PluginDraftStatus::Ready => PluginDraftStatusDto::Ready,
            PluginDraftStatus::Failed => PluginDraftStatusDto::Failed,
        },
        error_code: draft.last_error.clone(),
        updated_at_ms: u64::try_from(draft.updated_at_ms)
            .map_err(|_| PluginHttpError::internal("negative Draft timestamp"))?,
    })
}

pub(super) fn draft_detail(
    state: &PluginRouterState,
    draft: &PluginDraftRecord,
) -> Result<PluginDraftDetailDto, PluginHttpError> {
    let files = state.drafts.freeze(&draft.owner_user_id, &draft.draft_id)?;
    let files = files
        .into_iter()
        .map(|(path, bytes)| PluginDraftFileDto {
            media_type: content_type(&path).to_owned(),
            digest: hex::encode(Sha256::digest(&bytes)),
            size_bytes: bytes.len() as u64,
            text: String::from_utf8(bytes).ok(),
            path,
        })
        .collect();
    Ok(PluginDraftDetailDto {
        summary: draft_summary(state, draft)?,
        imported_context: draft.imported_context.clone(),
        files,
    })
}

async fn existing_for_package(
    state: &PluginRouterState,
    owner: &str,
    package_id: &str,
) -> Result<Option<PluginRecord>, PluginHttpError> {
    Ok(state
        .repository
        .list_plugins(owner)
        .await?
        .into_iter()
        .find(|plugin| plugin.package_id == package_id && plugin.trashed_at_ms.is_none()))
}

async fn package_identity_in_use(
    state: &PluginRouterState,
    owner: &str,
    package_id: &str,
) -> Result<bool, PluginHttpError> {
    Ok(state
        .repository
        .list_plugins(owner)
        .await?
        .into_iter()
        .any(|plugin| plugin.package_id == package_id))
}

async fn inspection_dto(
    state: &PluginRouterState,
    owner: &str,
    kind: PluginImportKindDto,
    artifact: PluginArtifact,
    existing: Option<&PluginRecord>,
) -> Result<PluginImportInspectionDto, PluginHttpError> {
    let existing_inventory = match existing {
        Some(plugin) => state.repository.inventory(owner, &plugin.plugin_id).await?,
        None => None,
    };
    let existing_permissions = existing_inventory
        .as_ref()
        .map(|inventory| inventory.grants.keys().cloned().collect::<BTreeSet<_>>())
        .unwrap_or_default();
    let existing_secret_slots = existing_inventory
        .as_ref()
        .map(|inventory| {
            inventory
                .artifact
                .artifact
                .manifest
                .secrets
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let added_permissions = artifact
        .manifest
        .permissions
        .difference(&existing_permissions)
        .cloned()
        .collect::<BTreeSet<_>>();
    let secret_slots = artifact
        .manifest
        .secrets
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let added_secret_slots = secret_slots
        .difference(&existing_secret_slots)
        .cloned()
        .collect::<BTreeSet<_>>();
    let trusted_local_service = artifact.manifest.has_service()
        && existing_inventory
            .as_ref()
            .is_none_or(|inventory| !inventory.artifact.artifact.manifest.has_service());
    let needs_confirmation = trusted_local_service
        || !added_permissions.is_empty()
        || !added_secret_slots.is_empty();
    let permission_expansion = if needs_confirmation {
        let confirmation_id = Uuid::now_v7().to_string();
        let confirmation = PluginPermissionExpansionDto {
            confirmation_id: confirmation_id.clone(),
            added_permissions: added_permissions.clone(),
            added_secret_slots,
            trusted_local_service,
        };
        let mut confirmations = state.confirmations.lock().await;
        if confirmations.len() >= MAX_PERMISSION_CONFIRMATIONS {
            let now = now_ms();
            confirmations.retain(|_, value| value.expires_at_ms > now);
        }
        confirmations.insert(
            confirmation_id,
            PermissionConfirmation {
                owner_user_id: owner.to_owned(),
                artifact_digest: artifact.artifact_digest.clone(),
                permissions: artifact.manifest.permissions.clone(),
                secret_slots,
                trusted_local_service,
                expires_at_ms: now_ms() + 10 * 60 * 1000,
            },
        );
        Some(confirmation)
    } else {
        None
    };
    Ok(PluginImportInspectionDto {
        kind,
        artifact_digest: artifact.artifact_digest.as_ref().to_owned(),
        manifest: manifest_dto(&artifact.manifest, existing.map(|plugin| &plugin.plugin_id)),
        trusted_local_service: artifact.manifest.has_service(),
        target_plugin_id: existing.map(|plugin| plugin.plugin_id.as_ref().to_owned()),
        target_plugin_revision: existing.map(|plugin| plugin.revision),
        backup: None,
        permission_expansion,
    })
}

pub(super) struct ConfirmationDecision {
    pub(super) required: Option<PluginPermissionExpansionDto>,
    pub(super) permissions: BTreeSet<String>,
    pub(super) secret_slots: BTreeSet<String>,
    pub(super) trusted_local_service: bool,
}

pub(super) async fn consume_confirmation(
    state: &PluginRouterState,
    owner: &str,
    artifact: &PluginArtifact,
    confirmation_id: Option<&str>,
    existing: Option<&PluginRecord>,
) -> Result<ConfirmationDecision, PluginHttpError> {
    let existing_inventory = match existing {
        Some(plugin) => state.repository.inventory(owner, &plugin.plugin_id).await?,
        None => None,
    };
    let existing_permissions = existing_inventory
        .as_ref()
        .map(|inventory| inventory.grants.keys().cloned().collect::<BTreeSet<_>>())
        .unwrap_or_default();
    let existing_secret_slots = existing_inventory
        .as_ref()
        .map(|inventory| {
            inventory
                .artifact
                .artifact
                .manifest
                .secrets
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let added = artifact
        .manifest
        .permissions
        .difference(&existing_permissions)
        .cloned()
        .collect::<BTreeSet<_>>();
    let secret_slots = artifact
        .manifest
        .secrets
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let added_secret_slots = secret_slots
        .difference(&existing_secret_slots)
        .cloned()
        .collect::<BTreeSet<_>>();
    let trusted_local_service = artifact.manifest.has_service()
        && existing_inventory
            .as_ref()
            .is_none_or(|inventory| !inventory.artifact.artifact.manifest.has_service());
    let needs = trusted_local_service || !added.is_empty() || !added_secret_slots.is_empty();
    if !needs {
        return Ok(ConfirmationDecision {
            required: None,
            permissions: artifact.manifest.permissions.clone(),
            secret_slots: BTreeSet::new(),
            trusted_local_service: false,
        });
    }
    if let Some(id) = confirmation_id {
        let confirmation = state.confirmations.lock().await.remove(id);
        if let Some(confirmation) = confirmation
            && confirmation.owner_user_id == owner
            && confirmation.artifact_digest == artifact.artifact_digest
            && confirmation.permissions == artifact.manifest.permissions
            && confirmation.secret_slots == secret_slots
            && confirmation.trusted_local_service == trusted_local_service
            && confirmation.expires_at_ms >= now_ms()
        {
            return Ok(ConfirmationDecision {
                required: None,
                permissions: confirmation.permissions,
                secret_slots: confirmation.secret_slots,
                trusted_local_service: confirmation.trusted_local_service,
            });
        }
        return Err(PluginHttpError::conflict("permission confirmation is stale"));
    }
    let inspection = inspection_dto(
        state,
        owner,
        PluginImportKindDto::Directory,
        artifact.clone(),
        existing,
    )
    .await?;
    Ok(ConfirmationDecision {
        required: inspection.permission_expansion,
        permissions: BTreeSet::new(),
        secret_slots: BTreeSet::new(),
        trusted_local_service: false,
    })
}

async fn copy_artifact_into_draft(
    state: &PluginRouterState,
    owner: &str,
    draft_id: &PluginDraftId,
    inventory: &PluginInventory,
) -> Result<(), PluginHttpError> {
    let stored = state
        .artifacts
        .load(&inventory.artifact.artifact.artifact_digest)?;
    for file in &stored.artifact.files {
        let path = safe_asset_path(&stored.package_root, &file.normalized_relative_path)?;
        let bytes = std::fs::read(path).map_err(|error| PluginHttpError::internal(error.to_string()))?;
        state
            .drafts
            .write(owner, draft_id, &file.normalized_relative_path, &bytes)?;
    }
    Ok(())
}

fn sql_statement(sql: String, parameters: Vec<Value>) -> Result<PluginSqlStatement, PluginHttpError> {
    Ok(PluginSqlStatement::new(
        sql,
        parameters
            .into_iter()
            .map(json_sql_value)
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

fn json_sql_value(value: Value) -> Result<PluginSqlValue, PluginHttpError> {
    Ok(match value {
        Value::Null => PluginSqlValue::Null,
        Value::Bool(value) => PluginSqlValue::Integer(i64::from(value)),
        Value::Number(value) if value.is_i64() => PluginSqlValue::Integer(value.as_i64().unwrap()),
        Value::Number(value) if value.is_f64() => PluginSqlValue::Real(value.as_f64().unwrap()),
        Value::String(value) => PluginSqlValue::Text(value),
        _ => return Err(PluginHttpError::bad_request("SQL parameters must be scalar JSON values")),
    })
}

fn query_result(result: PluginQueryResult) -> PluginDatabaseResultDto {
    PluginDatabaseResultDto::Rows {
        rows: result
            .rows
            .into_iter()
            .map(|row| {
                result
                    .columns
                    .iter()
                    .cloned()
                    .zip(row.into_iter().map(sql_json_value))
                    .collect()
            })
            .collect(),
    }
}

fn sql_json_value(value: PluginSqlValue) -> Value {
    match value {
        PluginSqlValue::Null => Value::Null,
        PluginSqlValue::Integer(value) => value.into(),
        PluginSqlValue::Real(value) => json!(value),
        PluginSqlValue::Text(value) => value.into(),
        PluginSqlValue::Blob(value) => base64::engine::general_purpose::STANDARD.encode(value).into(),
    }
}

async fn validate_requested_credential_bindings(
    state: &PluginRouterState,
    manifest: &PluginManifest,
    bindings: &BTreeMap<String, String>,
) -> Result<(), PluginHttpError> {
    if bindings
        .keys()
        .any(|slot| !manifest.secrets.contains(slot))
    {
        return Err(PluginHttpError::bad_request(
            "Credential slot is not declared",
        ));
    }
    for credential_id in bindings.values() {
        super::plugin_ports::validate_plugin_credential_reference(
            state.repository.pool(),
            credential_id,
        )
        .await
        .map_err(|_| PluginHttpError::bad_request("Credential reference is invalid"))?;
    }
    Ok(())
}

fn validate_config_value(manifest: &PluginManifest, value: &Value) -> Result<(), PluginHttpError> {
    let errors = config_errors(manifest, value);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(PluginHttpError::bad_request(&errors.join("; ")))
    }
}

fn config_errors(manifest: &PluginManifest, value: &Value) -> Vec<String> {
    match jsonschema::validator_for(&manifest.config_schema.0) {
        Ok(validator) => validator
            .iter_errors(value)
            .take(16)
            .map(|error| error.to_string())
            .collect(),
        Err(error) => vec![error.to_string()],
    }
}


pub(super) fn require_draft_revision(draft: &PluginDraftRecord, expected: u64) -> Result<(), PluginHttpError> {
    if draft.revision != expected {
        return Err(PluginHttpError::conflict("Draft revision changed"));
    }
    Ok(())
}

/// Evidence writes advance the row CAS without changing the editable source or
/// plan. An authoring caller may reuse a revision in that exact interval; file
/// and plan edits raise the floor, including a change subsequently reverted.
/// HTTP editors still use `require_draft_revision` and the exact row CAS.
/// Drafts without a valid host watermark fail closed at the current revision.
pub(super) fn require_authoring_revision(draft: &PluginDraftRecord, expected: u64) -> Result<(), PluginHttpError> {
    let edit_revision = draft.verification["edit_revision"].as_u64()
        .filter(|revision| *revision > 0 && *revision <= draft.revision)
        .unwrap_or(draft.revision);
    if expected < edit_revision || expected > draft.revision {
        return Err(PluginHttpError::conflict(&format!(
            "Draft revision changed: expected_revision {expected} predates the current editable state or is unknown. Call read to load its current files and revision, then continue from that state; do not resend edits based on the older revision."
        )));
    }
    Ok(())
}

fn mark_draft_edit(draft: &mut PluginDraftRecord) -> Result<(), PluginHttpError> {
    let next = draft.revision.checked_add(1)
        .ok_or_else(|| PluginHttpError::conflict("Draft revision exhausted"))?;
    // GUI source edits invalidate the same derived facts as Agent apply.
    // Preserve the accepted requirement, but never carry old runtime,
    // authorization or delivery evidence onto newly edited bytes.
    draft.verification = json!({
        "edit_revision": next,
        "task_message_id": draft.verification["task_message_id"],
        "plan": draft.verification["plan"],
    });
    Ok(())
}

pub(super) fn parse_plugin_id(value: &str) -> Result<PluginId, PluginHttpError> {
    let id = Uuid::parse_str(value).map_err(|_| PluginHttpError::not_found())?;
    if id.get_version_num() != 7 || id.to_string() != value {
        return Err(PluginHttpError::not_found());
    }
    Ok(PluginId::from(value.to_owned()))
}

pub(super) fn parse_draft_id(value: &str) -> Result<PluginDraftId, PluginHttpError> {
    let id = Uuid::parse_str(value).map_err(|_| PluginHttpError::not_found())?;
    if id.get_version_num() != 7 || id.to_string() != value {
        return Err(PluginHttpError::not_found());
    }
    Ok(PluginDraftId::from(value.to_owned()))
}

fn copy_package_id(package_id: &str) -> String {
    let suffix = Uuid::now_v7().simple().to_string();
    let maximum_base = 160usize.saturating_sub(".copy.".len() + 32);
    format!("{}.copy.{suffix}", &package_id[..package_id.len().min(maximum_base)])
}

fn import_backup_bundle(
    state: &PluginRouterState,
    source: &str,
) -> Result<nomifun_plugin_platform::ImportedPluginBackup, PluginHttpError> {
    let path = Path::new(source);
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| PluginHttpError::bad_request(&format!("Backup is unavailable: {error}")))?;
    if metadata.file_type().is_symlink() {
        return Err(PluginHttpError::bad_request(
            "Backup source cannot be a symbolic link",
        ));
    }
    if metadata.is_dir() {
        state
            .transfers
            .import_backup_directory(path)
            .map_err(PluginHttpError::from)
    } else if metadata.is_file() {
        state
            .transfers
            .import_backup_zip(path)
            .map_err(PluginHttpError::from)
    } else {
        Err(PluginHttpError::bad_request(
            "Backup source must be a directory or ZIP",
        ))
    }
}

fn safe_asset_path(root: &Path, relative: &str) -> Result<PathBuf, PluginHttpError> {
    if relative.is_empty()
        || relative.starts_with('/')
        || relative.contains(['\\', ':'])
        || relative.split('/').any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(PluginHttpError::not_found());
    }
    let root = std::fs::canonicalize(root).map_err(|_| PluginHttpError::not_found())?;
    let path = relative.split('/').fold(root.clone(), |mut path, part| {
        path.push(part);
        path
    });
    if std::fs::symlink_metadata(&path)
        .map_err(|_| PluginHttpError::not_found())?
        .file_type()
        .is_symlink()
    {
        return Err(PluginHttpError::not_found());
    }
    let path = std::fs::canonicalize(path).map_err(|_| PluginHttpError::not_found())?;
    if !path.starts_with(root) {
        return Err(PluginHttpError::not_found());
    }
    Ok(path)
}

fn inject_sdk(bytes: &[u8]) -> Result<Vec<u8>, PluginHttpError> {
    let html = std::str::from_utf8(bytes)
        .map_err(|_| PluginHttpError::conflict("UI entrypoint is not UTF-8"))?;
    let script = format!("<script>{PLUGIN_SDK}</script>");
    Ok(match html.find("</head>") {
        Some(index) => format!("{}{}{}", &html[..index], script, &html[index..]).into_bytes(),
        None => format!("{script}{html}").into_bytes(),
    })
}

fn content_type(path: &str) -> &'static str {
    match Path::new(path).extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

fn nonnegative(value: Option<i64>) -> Result<Option<u64>, PluginHttpError> {
    value
        .map(|value| u64::try_from(value).map_err(|_| PluginHttpError::internal("negative timestamp")))
        .transpose()
}

fn now_ms() -> i64 {
    nomifun_common::now_ms().max(1)
}

fn action_effect_dto(effect: PluginActionEffect) -> PluginActionEffectDto {
    match effect {
        PluginActionEffect::Read => PluginActionEffectDto::Read,
        PluginActionEffect::Write => PluginActionEffectDto::Write,
        PluginActionEffect::External => PluginActionEffectDto::External,
    }
}

fn binding_error_code(error: &nomifun_plugin_platform::PluginBindingError) -> &'static str {
    use nomifun_plugin_platform::PluginBindingError;
    match error {
        PluginBindingError::ActionNotFound(_) => "ACTION_NOT_FOUND",
        PluginBindingError::ActionUnavailable { .. } => "ACTION_UNAVAILABLE",
        PluginBindingError::ActionNotBound { .. } => "ACTION_NOT_BOUND",
        PluginBindingError::Canceled => "ACTION_CANCELED",
        PluginBindingError::Timeout => "ACTION_TIMEOUT",
        PluginBindingError::InputSchema(_) | PluginBindingError::OutputSchema(_) => {
            "ACTION_SCHEMA_INVALID"
        }
        PluginBindingError::ArtifactFence(_) => "ACTION_ARTIFACT_STALE",
        PluginBindingError::RecursiveCall(_) | PluginBindingError::CallDepthExceeded => {
            "ACTION_CALL_CHAIN_REJECTED"
        }
        PluginBindingError::Runtime { .. } | PluginBindingError::RuntimeTask(_) => {
            "ACTION_RUNTIME_FAILED"
        }
        PluginBindingError::InvalidContract(_)
        | PluginBindingError::UnsupportedBinding(_)
        | PluginBindingError::DuplicateBindingOwner(_)
        | PluginBindingError::MultiplicityConflict(_)
        | PluginBindingError::Poisoned => "ACTION_BINDING_INVALID",
    }
}

#[derive(Debug)]
pub(super) struct PluginHttpError {
    pub(super) status: StatusCode,
    pub(super) code: &'static str,
    pub(super) message: String,
}

impl std::fmt::Display for PluginHttpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl PluginHttpError {
    pub(super) fn bad_request(message: &str) -> Self {
        Self { status: StatusCode::BAD_REQUEST, code: "PLUGIN_INVALID_INPUT", message: message.into() }
    }
    pub(super) fn conflict(message: &str) -> Self {
        Self { status: StatusCode::CONFLICT, code: "PLUGIN_CONFLICT", message: message.into() }
    }
    pub(super) fn forbidden(message: &str) -> Self {
        Self { status: StatusCode::FORBIDDEN, code: "PLUGIN_PERMISSION_DENIED", message: message.into() }
    }
    pub(super) fn not_found() -> Self {
        Self { status: StatusCode::NOT_FOUND, code: "PLUGIN_NOT_FOUND", message: "Plugin resource was not found".into() }
    }
    pub(super) fn unavailable(message: &str) -> Self {
        Self { status: StatusCode::SERVICE_UNAVAILABLE, code: "PLUGIN_UNAVAILABLE", message: message.into() }
    }

    pub(super) fn internal(message: impl Into<String>) -> Self {
        Self { status: StatusCode::INTERNAL_SERVER_ERROR, code: "PLUGIN_INTERNAL", message: message.into() }
    }
}

impl IntoResponse for PluginHttpError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(ErrorResponse::new(self.message, self.code))).into_response()
    }
}

impl From<PluginRepositoryError> for PluginHttpError {
    fn from(error: PluginRepositoryError) -> Self {
        match error {
            PluginRepositoryError::NotFound => Self::not_found(),
            PluginRepositoryError::Conflict | PluginRepositoryError::PackageConflict => {
                Self::conflict(&error.to_string())
            }
            PluginRepositoryError::InvalidData(_) => Self::bad_request(&error.to_string()),
            other => Self::internal(other.to_string()),
        }
    }
}

impl From<PluginArtifactStoreError> for PluginHttpError {
    fn from(error: PluginArtifactStoreError) -> Self {
        match error {
            PluginArtifactStoreError::InvalidSource { .. }
            | PluginArtifactStoreError::UnsafePackagePath { .. }
            | PluginArtifactStoreError::UnsupportedEntry { .. }
            | PluginArtifactStoreError::DuplicateEntry { .. }
            | PluginArtifactStoreError::TooManyFiles { .. }
            | PluginArtifactStoreError::FileTooLarge { .. }
            | PluginArtifactStoreError::TotalSizeExceeded { .. }
            | PluginArtifactStoreError::ZipTooLarge { .. }
            | PluginArtifactStoreError::InvalidZip(_)
            | PluginArtifactStoreError::InvalidManifest(_)
            | PluginArtifactStoreError::Contract(_) => Self::bad_request(&error.to_string()),
            PluginArtifactStoreError::Canceled => Self::conflict(&error.to_string()),
            other => Self::internal(other.to_string()),
        }
    }
}

impl From<nomifun_plugin_platform::PluginTransferError> for PluginHttpError {
    fn from(error: nomifun_plugin_platform::PluginTransferError) -> Self {
        use nomifun_plugin_platform::PluginTransferError;
        match error {
            PluginTransferError::DestinationExists(_) => Self::conflict(&error.to_string()),
            PluginTransferError::InvalidInput(_)
            | PluginTransferError::UnsafePath { .. }
            | PluginTransferError::UnsupportedEntry(_)
            | PluginTransferError::DuplicateEntry(_)
            | PluginTransferError::LimitExceeded(_)
            | PluginTransferError::Tampered(_)
            | PluginTransferError::InvalidZip(_)
            | PluginTransferError::Json(_) => Self::bad_request(&error.to_string()),
            PluginTransferError::Io { .. } => Self::internal(error.to_string()),
        }
    }
}

impl From<PluginDraftStoreError> for PluginHttpError {
    fn from(error: PluginDraftStoreError) -> Self {
        match error {
            PluginDraftStoreError::NotFound => Self::not_found(),
            PluginDraftStoreError::Canceled => Self::conflict(&error.to_string()),
            PluginDraftStoreError::UnsafePath { .. } | PluginDraftStoreError::Limit(_) => {
                Self::bad_request(&error.to_string())
            }
            PluginDraftStoreError::Io { .. } => Self::internal(error.to_string()),
        }
    }
}

impl From<nomifun_plugin_platform::PluginDataRootError> for PluginHttpError {
    fn from(error: nomifun_plugin_platform::PluginDataRootError) -> Self {
        Self::internal(error.to_string())
    }
}

impl From<PluginInstallError> for PluginHttpError {
    fn from(error: PluginInstallError) -> Self {
        match error {
            PluginInstallError::PermissionConfirmationRequired(_)
            | PluginInstallError::SecretConfirmationRequired(_)
            | PluginInstallError::LocalServiceConfirmationRequired => {
                Self::conflict(&error.to_string())
            }
            PluginInstallError::InvalidConfig(_) => Self::bad_request(&error.to_string()),
            PluginInstallError::InvalidUpdate(_) => Self::conflict(&error.to_string()),
            PluginInstallError::Repository(error) => error.into(),
            PluginInstallError::Artifact(error) => error.into(),
            PluginInstallError::DataRoot(error) => error.into(),
            PluginInstallError::Validation(_) | PluginInstallError::Activation(_) => {
                Self::unavailable(&error.to_string())
            }
            PluginInstallError::Recovery { .. } => Self::internal(error.to_string()),
        }
    }
}

impl From<nomifun_plugin_platform::PluginBindingError> for PluginHttpError {
    fn from(error: nomifun_plugin_platform::PluginBindingError) -> Self {
        use nomifun_plugin_platform::PluginBindingError;
        match error {
            PluginBindingError::InputSchema(_)
            | PluginBindingError::OutputSchema(_)
            | PluginBindingError::RecursiveCall(_)
            | PluginBindingError::CallDepthExceeded => Self::bad_request(&error.to_string()),
            PluginBindingError::ActionNotFound(_) => Self::not_found(),
            PluginBindingError::ActionUnavailable { .. }
            | PluginBindingError::ActionNotBound { .. }
            | PluginBindingError::ArtifactFence(_)
            | PluginBindingError::Canceled => Self::conflict(&error.to_string()),
            PluginBindingError::Timeout
            | PluginBindingError::Runtime { .. }
            | PluginBindingError::RuntimeTask(_) => Self::unavailable(&error.to_string()),
            PluginBindingError::InvalidContract(_)
            | PluginBindingError::UnsupportedBinding(_)
            | PluginBindingError::DuplicateBindingOwner(_)
            | PluginBindingError::MultiplicityConflict(_)
            | PluginBindingError::Poisoned => Self::internal(error.to_string()),
        }
    }
}

#[cfg(test)]
mod surface_policy_tests {
    use super::*;

    #[test]
    fn ui_network_sources_require_the_declared_and_granted_network_permission() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:5197"));

        let denied = surface_content_security_policy(&headers, false)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert!(denied.contains("connect-src 'none'"));
        assert!(!denied.contains("connect-src http:"));

        let granted = surface_content_security_policy(&headers, true)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert!(granted.contains("connect-src http: https: ws: wss:"));
        assert!(granted.contains("script-src 'unsafe-inline' 'self' http://127.0.0.1:5197"));
    }

    #[test]
    fn isolated_startup_failure_is_visible_without_claiming_a_live_process() {
        let failed = runtime_observation_dto(
            nomifun_plugin_platform::PluginServiceObservation::Stopped,
            true,
            Some(PLUGIN_STARTUP_ACTIVATION_FAILED),
        );
        assert_eq!(failed.state, PluginObservedStateDto::Failed);
        assert_eq!(
            failed.error_code.as_deref(),
            Some(PLUGIN_STARTUP_ACTIVATION_FAILED)
        );
        assert_eq!(failed.process_id, None);

        let intentionally_stopped = runtime_observation_dto(
            nomifun_plugin_platform::PluginServiceObservation::Stopped,
            false,
            Some(PLUGIN_STARTUP_ACTIVATION_FAILED),
        );
        assert_eq!(intentionally_stopped.state, PluginObservedStateDto::Stopped);
        assert_eq!(intentionally_stopped.error_code, None);
    }
}

#[cfg(test)]
mod authoring_revision_tests {
    use super::*;

    fn draft(revision: u64, verification: Value) -> PluginDraftRecord {
        PluginDraftRecord {
            owner_user_id: "owner".into(), draft_id: "draft".into(), revision,
            plugin_id: None, base_revision: None, name: "App".into(),
            workspace_path: "workspace".into(), source_conversation_id: None,
            source_message_id: None, source_operation_key: None, source_request_digest: None,
            verification, imported_context: json!({}), status: PluginDraftStatus::Ready,
            last_error: None, created_at_ms: 1, updated_at_ms: 1,
        }
    }

    #[test]
    fn observations_allow_reusing_source_revision_but_edits_invalidate_it() {
        let mut draft = draft(9, json!({"edit_revision": 4}));
        assert!(require_authoring_revision(&draft, 4).is_ok());
        assert!(require_authoring_revision(&draft, 8).is_ok());
        assert!(require_authoring_revision(&draft, 9).is_ok());
        assert!(require_authoring_revision(&draft, 3).is_err());
        assert!(require_authoring_revision(&draft, 10).is_err());
        assert!(require_draft_revision(&draft, 8).is_err(), "GUI keeps exact CAS");
        mark_draft_edit(&mut draft).unwrap();
        draft.revision = 10;
        assert!(require_authoring_revision(&draft, 9).is_err());
        assert!(require_authoring_revision(&draft, 10).is_ok());
    }

    #[test]
    fn gui_source_edit_preserves_requirements_but_invalidates_prior_evidence() {
        let plan = json!({"output_key":"todo","outputs":[{"key":"todo","kind":"ui"}],
            "cases":{"persist":{"kind":"ui","steps":[{"operation":"reopen"}]}}});
        let mut draft = draft(9, json!({
            "edit_revision":4,"task_message_id":"request","plan":plan,
            "structure_passed":true,"artifact_digest":"old-artifact",
            "runtime_ready":true,"has_ui":true,"has_service":false,
            "cases":{"persist":{"passed":true,"persistence_checked":true}},
            "execution":{"config":{}},"context":{"verifier":"old"},
            "approval":{"approved":true},"surface":{"surface_session_id":"old"},
            "installed_observation":{"artifact_digest":"old-artifact","ui_ready":true},
            "delivery":{"artifact_digest":"old-artifact","plugin_revision":1},
            "future_derived_evidence":true,
        }));
        draft.source_conversation_id = Some("conversation".into());
        draft.source_message_id = Some("source-message".into());
        draft.source_operation_key = Some("source-operation".into());
        draft.source_request_digest = Some("source-digest".into());

        mark_draft_edit(&mut draft).unwrap();
        // Both successful GUI edit routes call this helper before their CAS.
        // Exact equality also prevents any new derived evidence surviving.
        assert_eq!(draft.verification, json!({
            "edit_revision":10,"task_message_id":"request","plan":plan,
        }));
        assert_eq!(draft.source_conversation_id.as_deref(), Some("conversation"));
        assert_eq!(draft.source_message_id.as_deref(), Some("source-message"));
        assert_eq!(draft.source_operation_key.as_deref(), Some("source-operation"));
        assert_eq!(draft.source_request_digest.as_deref(), Some("source-digest"));
        draft.revision = 10;
        assert!(require_authoring_revision(&draft, 9).is_err());
        assert!(require_authoring_revision(&draft, 10).is_ok());
    }

    #[test]
    fn missing_or_invalid_edit_watermark_requires_the_exact_current_revision() {
        for verification in [json!({}), json!({"edit_revision": 0}),
            json!({"edit_revision": 10}), json!({"edit_revision": "4"})] {
            let draft = draft(9, verification);
            assert!(require_authoring_revision(&draft, 8).is_err());
            assert!(require_authoring_revision(&draft, 9).is_ok());
        }
    }
}
