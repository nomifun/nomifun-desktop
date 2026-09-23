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
    DataGeneration, ImportCancellation, InstallArtifactRequest, InstallTarget, NeverCancel,
    PluginDataRootHandle, PluginDraftMessage, PluginDraftMessageRole, PluginDraftRecord,
    PluginDraftStatus, PluginActionRegistration, PluginArtifactStoreError, PluginDraftStore,
    PluginDraftStoreError, PluginInstallError, PluginInstallService, PluginInventory,
    PluginQueryResult, PluginRecord, PluginRepository, PluginRepositoryError, PluginSqlStatement,
    PluginRuntimeContext, PluginRuntimePort, PluginSqlValue, PluginStorage, PreviewDataRoot,
    SqlitePluginRepository, StagedPluginDraftReplacement, action_publications,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const PLUGIN_SDK: &str = include_str!(
    "../../../nomifun-plugin-platform/src/assets/plugin-sdk.js"
);
const MAX_PERMISSION_CONFIRMATIONS: usize = 128;
const MAX_SURFACE_CALLS: usize = 256;
const DRAFT_GENERATION_INTERRUPTED: &str = "PLUGIN_GENERATION_INTERRUPTED";
const PLUGIN_STARTUP_ACTIVATION_FAILED: &str = "PLUGIN_STARTUP_ACTIVATION_FAILED";
const PLUGIN_BINDING_RECOVERY_FAILED: &str = "PLUGIN_BINDING_RECOVERY_FAILED";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DraftGenerationKey {
    owner_user_id: String,
    draft_id: String,
}

impl DraftGenerationKey {
    fn new(owner_user_id: &str, draft_id: &PluginDraftId) -> Self {
        Self {
            owner_user_id: owner_user_id.to_owned(),
            draft_id: draft_id.as_ref().to_owned(),
        }
    }
}

#[derive(Clone)]
struct ActiveDraftGeneration {
    revision: u64,
    cancellation: CancellationToken,
    done: Arc<Notify>,
}

struct DraftGenerationCancellation(CancellationToken);

impl ImportCancellation for DraftGenerationCancellation {
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

fn register_active_generation(
    generations: &mut HashMap<DraftGenerationKey, ActiveDraftGeneration>,
    key: DraftGenerationKey,
    active: ActiveDraftGeneration,
) -> bool {
    if generations.contains_key(&key) {
        false
    } else {
        generations.insert(key, active);
        true
    }
}

#[derive(Clone)]
pub struct PluginRouterState {
    repository: Arc<SqlitePluginRepository>,
    install: Arc<PluginInstallService>,
    runtime: Arc<nomifun_plugin_platform::PluginServiceRuntime>,
    artifacts: Arc<nomifun_plugin_platform::PluginArtifactStore>,
    data_roots: Arc<nomifun_plugin_platform::PluginDataRootManager>,
    drafts: Arc<PluginDraftStore>,
    transfers: Arc<nomifun_plugin_platform::PluginBackupFilesystem>,
    model: Arc<nomifun_model_invoke::ModelInvokeService>,
    workspace: PathBuf,
    confirmations: Arc<Mutex<HashMap<String, PermissionConfirmation>>>,
    surfaces: Arc<Mutex<HashMap<String, SurfaceSession>>>,
    generations: Arc<Mutex<HashMap<DraftGenerationKey, ActiveDraftGeneration>>>,
    pub(crate) registry: nomifun_plugin_platform::InMemoryPluginBindingRegistry,
    pub(crate) agent: nomifun_plugin_platform::AgentPluginBindings,
    pub(crate) desktop: nomifun_plugin_platform::DesktopPluginBindings,
    host: Arc<dyn nomifun_plugin_platform::PluginServiceHostPort>,
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
            model: services.model_invoke_service.clone(),
            workspace: services.work_dir.clone(),
            confirmations: Arc::new(Mutex::new(HashMap::new())),
            surfaces: Arc::new(Mutex::new(HashMap::new())),
            generations: Arc::new(Mutex::new(HashMap::new())),
            registry: bindings.registry,
            agent: bindings.agent,
            desktop: bindings.desktop,
            host: bindings.host,
        }
    }

    pub(crate) async fn recover(&self) -> Result<(), String> {
        self.data_roots
            .cleanup_preview_roots()
            .map_err(|error| error.to_string())?;
        let owner = nomifun_db::installation_owner_id(self.repository.pool())
            .await
            .map_err(|error| error.to_string())?;
        recover_interrupted_draft_generations(self.repository.as_ref(), &owner)
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

struct PermissionConfirmation {
    owner_user_id: String,
    artifact_digest: DigestHex,
    permissions: BTreeSet<String>,
    secret_slots: BTreeSet<String>,
    trusted_local_service: bool,
    expires_at_ms: i64,
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

struct SurfaceSession {
    owner_user_id: String,
    plugin_id: Option<PluginId>,
    runtime_plugin_id: PluginId,
    draft_id: Option<PluginDraftId>,
    artifact: PluginArtifact,
    asset_root: PathBuf,
    data_root: SurfaceDataRoot,
    generation: u64,
    is_preview: bool,
    config: Value,
    granted_permissions: BTreeSet<String>,
    completed_calls: HashMap<String, PluginBridgeResultDto>,
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
        .route("/api/plugin-drafts/{draft_id}/generate", post(generate_draft))
        .route("/api/plugin-drafts/{draft_id}/cancel", post(cancel_draft_generation))
        .route("/api/plugin-drafts/{draft_id}/files", put(replace_draft_file).delete(delete_draft_file))
        .route("/api/plugin-drafts/{draft_id}/preview", post(preview_draft))
        .route("/api/plugin-drafts/{draft_id}/save", post(save_draft))
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
    let owner = user.id.to_string();
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
    Ok(Json(ApiResponse::ok(PluginLibraryResponseDto {
        revision,
        plugins,
    })))
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
    Ok(Json(ApiResponse::ok(references)))
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
    let inventory = inventory(&state, &user, &plugin_id).await?;
    Ok(Json(ApiResponse::ok(detail_dto(&state, &inventory).await?)))
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
    let owner = user.id.to_string();
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
        messages: Vec::new(),
        status: PluginDraftStatus::Ready,
        last_error: None,
        created_at_ms: now,
        updated_at_ms: now,
    };
    if let Err(error) = state.repository.create_draft(&draft).await {
        let _ = state.drafts.discard(&owner, &draft_id);
        return Err(error.into());
    }
    Ok(Json(ApiResponse::ok(draft_detail(&state, &draft)?)))
}

async fn list_drafts(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<PluginDraftListResponseDto>>, PluginHttpError> {
    let owner = user.id.to_string();
    let drafts = state
        .repository
        .list_drafts(&owner)
        .await?
        .iter()
        .map(|draft| draft_summary(&state, draft))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(ApiResponse::ok(PluginDraftListResponseDto { drafts })))
}

async fn get_draft(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(draft_id): AxumPath<String>,
) -> Result<Json<ApiResponse<PluginDraftDetailDto>>, PluginHttpError> {
    let draft = draft(&state, &user, &draft_id).await?;
    Ok(Json(ApiResponse::ok(draft_detail(&state, &draft)?)))
}

async fn replace_draft_file(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(draft_id): AxumPath<String>,
    Json(request): Json<ReplacePluginDraftFileRequest>,
) -> Result<Json<ApiResponse<PluginDraftDetailDto>>, PluginHttpError> {
    let mut draft = draft(&state, &user, &draft_id).await?;
    require_draft_revision(&draft, request.expected_revision)?;
    require_draft_not_generating(&draft)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(request.content_base64)
        .map_err(|_| PluginHttpError::bad_request("content_base64 is invalid"))?;
    state
        .drafts
        .write(&draft.owner_user_id, &draft.draft_id, &request.path, &bytes)?;
    draft.updated_at_ms = now_ms();
    let updated = state
        .repository
        .update_draft(&draft, request.expected_revision)
        .await?;
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
    require_draft_not_generating(&draft)?;
    if request.path == "nomifun.plugin.json" {
        return Err(PluginHttpError::bad_request("the Plugin manifest cannot be deleted"));
    }
    state
        .drafts
        .delete_file(&draft.owner_user_id, &draft.draft_id, &request.path)?;
    draft.updated_at_ms = now_ms();
    let updated = state
        .repository
        .update_draft(&draft, request.expected_revision)
        .await?;
    Ok(Json(ApiResponse::ok(draft_detail(&state, &updated)?)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneratedPackage {
    assistant_message: String,
    files: BTreeMap<String, String>,
}

async fn generate_draft(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(draft_id): AxumPath<String>,
    Json(request): Json<GeneratePluginDraftRequest>,
) -> Result<Json<ApiResponse<PluginDraftDetailDto>>, PluginHttpError> {
    let mut draft = draft(&state, &user, &draft_id).await?;
    require_draft_revision(&draft, request.expected_revision)?;
    require_draft_not_generating(&draft)?;
    if request.requirement.trim().is_empty() || request.requirement.len() > 32_000 {
        return Err(PluginHttpError::bad_request("a bounded requirement is required"));
    }

    let key = DraftGenerationKey::new(&draft.owner_user_id, &draft.draft_id);
    let cancellation = CancellationToken::new();
    let done = Arc::new(Notify::new());
    let active = {
        let mut generations = state.generations.lock().await;
        if generations.contains_key(&key) {
            return Err(PluginHttpError::conflict("Draft generation is already running"));
        }
        draft.status = PluginDraftStatus::Generating;
        draft.last_error = None;
        draft.updated_at_ms = now_ms();
        let generating = state
            .repository
            .update_draft(&draft, request.expected_revision)
            .await?;
        let active = ActiveDraftGeneration {
            revision: generating.revision,
            cancellation: cancellation.clone(),
            done: done.clone(),
        };
        let registered = register_active_generation(
            &mut generations,
            key.clone(),
            active.clone(),
        );
        debug_assert!(registered, "generation key was checked while holding the same lock");
        draft = generating;
        active
    };

    let worker_state = state.clone();
    let worker_key = key.clone();
    let worker = tokio::spawn(async move {
        let result = complete_draft_generation(
            &worker_state,
            draft,
            request,
            active.cancellation.clone(),
        )
        .await;
        let mut generations = worker_state.generations.lock().await;
        if generations
            .get(&worker_key)
            .is_some_and(|current| current.revision == active.revision)
        {
            generations.remove(&worker_key);
        }
        drop(generations);
        active.done.notify_one();
        result
    });
    let updated = worker
        .await
        .map_err(|error| PluginHttpError::internal(format!("Draft generation task failed: {error}")))??;
    Ok(Json(ApiResponse::ok(draft_detail(&state, &updated)?)))
}

async fn cancel_draft_generation(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(draft_id): AxumPath<String>,
    Json(request): Json<CancelPluginDraftGenerationRequest>,
) -> Result<Json<ApiResponse<PluginDraftDetailDto>>, PluginHttpError> {
    let draft = draft(&state, &user, &draft_id).await?;
    require_draft_revision(&draft, request.expected_revision)?;
    if draft.status != PluginDraftStatus::Generating {
        return Err(PluginHttpError::conflict("Draft generation is not running"));
    }
    let key = DraftGenerationKey::new(&draft.owner_user_id, &draft.draft_id);
    let active = state
        .generations
        .lock()
        .await
        .get(&key)
        .filter(|active| active.revision == draft.revision)
        .cloned()
        .ok_or_else(|| PluginHttpError::conflict("Draft generation is not active"))?;
    active.cancellation.cancel();
    let settled = settle_cancelled_generation(&state, &draft).await?;
    let _ = tokio::time::timeout(Duration::from_secs(5), active.done.notified()).await;
    let current = state
        .repository
        .get_draft(&settled.owner_user_id, &settled.draft_id)
        .await?
        .unwrap_or(settled);
    Ok(Json(ApiResponse::ok(draft_detail(&state, &current)?)))
}

enum DraftGenerationFailure {
    Cancelled,
    Failed(PluginHttpError),
}

struct PreparedDraftGeneration {
    draft: PluginDraftRecord,
    replacement: StagedPluginDraftReplacement,
}

async fn complete_draft_generation(
    state: &PluginRouterState,
    generating: PluginDraftRecord,
    request: GeneratePluginDraftRequest,
    cancellation: CancellationToken,
) -> Result<PluginDraftRecord, PluginHttpError> {
    let result = run_draft_generation(state, generating.clone(), request, &cancellation).await;
    match result {
        Ok(prepared) => match persist_generated_draft(
            state,
            prepared,
            generating.revision,
            &cancellation,
        )
        .await
        {
            Ok(updated) => Ok(updated),
            Err(error) => {
                persist_failed_generation(state, generating, error, &cancellation).await
            }
        },
        Err(DraftGenerationFailure::Cancelled) => {
            settle_cancelled_generation(state, &generating).await
        }
        Err(DraftGenerationFailure::Failed(error)) => {
            persist_failed_generation(state, generating, error, &cancellation).await
        }
    }
}

async fn persist_failed_generation(
    state: &PluginRouterState,
    generating: PluginDraftRecord,
    error: PluginHttpError,
    cancellation: &CancellationToken,
) -> Result<PluginDraftRecord, PluginHttpError> {
    if cancellation.is_cancelled() {
        return settle_cancelled_generation(state, &generating).await;
    }
    let mut failed = generating;
    failed.status = PluginDraftStatus::Failed;
    failed.last_error = Some(error.code.to_owned());
    failed.updated_at_ms = now_ms();
    match state
        .repository
        .update_draft(&failed, failed.revision)
        .await
    {
        Ok(_) => Err(error),
        Err(PluginRepositoryError::Conflict) if cancellation.is_cancelled() => {
            settle_cancelled_generation(state, &failed).await
        }
        Err(settle_error) => Err(PluginHttpError::internal(format!(
            "Draft generation failed and its state could not be persisted: {settle_error}"
        ))),
    }
}

async fn run_draft_generation(
    state: &PluginRouterState,
    mut draft: PluginDraftRecord,
    request: GeneratePluginDraftRequest,
    cancellation: &CancellationToken,
) -> Result<PreparedDraftGeneration, DraftGenerationFailure> {
    require_generation_active(cancellation)?;
    let current_files = state
        .drafts
        .freeze(&draft.owner_user_id, &draft.draft_id)
        .map_err(PluginHttpError::from)
        .map_err(DraftGenerationFailure::Failed)?;
    let textual = current_files
        .iter()
        .filter_map(|(path, bytes)| String::from_utf8(bytes.clone()).ok().map(|text| (path, text)))
        .collect::<BTreeMap<_, _>>();
    require_generation_active(cancellation)?;
    let config = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(DraftGenerationFailure::Cancelled),
        result = nomifun_ai_agent::factory::provider_config::resolve_provider_config(
            state.model.as_ref(),
            &request.provider_id,
            &request.model,
            &state.workspace,
        ) => result.map_err(|_| DraftGenerationFailure::Failed(
            PluginHttpError::bad_gateway("the selected model is unavailable")
        ))?,
    };
    let system = "Create or edit one complete NomiFun Unified Plugin package. Return only JSON {assistant_message,files}; files is a map from normalized package path to complete UTF-8 content. It must include nomifun.plugin.json using schema nomifun.plugin/v1 and at least ui/index.html or service/main.mjs. Service modules export async activate(ctx) returning invoke(action,input) and optional deactivate(). Use only Action + Binding and inline JSON Schema. UI uses window.nomi.storage.kv/db/files, cache, actions.invoke, host.invoke and config.get. Do not emit old Project/Mount/Candidate/Release/Publish contracts, npm projects, external scripts, mobile layouts, secrets, or fake APIs.";
    let prompt = serde_json::to_string(&json!({
        "requirement": &request.requirement,
        "messages": &draft.messages,
        "current_files": textual,
        "instruction": "Return the complete package; preserve storage formats unless explicitly changed."
    }))
    .map_err(|error| DraftGenerationFailure::Failed(PluginHttpError::internal(error.to_string())))?;
    let raw = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(DraftGenerationFailure::Cancelled),
        result = nomifun_ai_agent::factory::provider_config::one_shot_completion_bounded(
            &config,
            system,
            vec![nomifun_ai_agent::factory::provider_config::user_message(prompt)],
            16_000,
            4 * 1024 * 1024,
        ) => result.map_err(|_| DraftGenerationFailure::Failed(
            PluginHttpError::bad_gateway("Plugin generation failed")
        ))?,
    };
    require_generation_active(cancellation)?;
    let generated: GeneratedPackage = parse_model_json(&raw).map_err(DraftGenerationFailure::Failed)?;
    if generated.files.is_empty() || generated.assistant_message.len() > 8_000 {
        return Err(DraftGenerationFailure::Failed(PluginHttpError::bad_gateway(
            "the model returned an invalid package",
        )));
    }
    require_generation_active(cancellation)?;
    let GeneratedPackage {
        assistant_message,
        files,
    } = generated;
    let replacement_files = files
        .into_iter()
        .map(|(path, content)| (path, content.into_bytes()))
        .collect::<BTreeMap<_, _>>();
    let artifact_cancellation = DraftGenerationCancellation(cancellation.clone());
    let replacement = state
        .drafts
        .stage_exact_replacement(
            &draft.owner_user_id,
            &draft.draft_id,
            &replacement_files,
            &artifact_cancellation,
        )
        .map_err(|error| match error {
            PluginDraftStoreError::Canceled => DraftGenerationFailure::Cancelled,
            other => DraftGenerationFailure::Failed(other.into()),
        })?;
    require_generation_active(cancellation)?;
    let artifact = state
        .artifacts
        .inspect_files(&replacement_files, &artifact_cancellation)
        .map_err(PluginHttpError::from)
        .map_err(DraftGenerationFailure::Failed)?;
    require_generation_active(cancellation)?;
    draft.name = artifact.manifest.name.clone();
    draft.messages.push(PluginDraftMessage {
        role: PluginDraftMessageRole::User,
        content: request.requirement,
        created_at_ms: now_ms(),
    });
    draft.messages.push(PluginDraftMessage {
        role: PluginDraftMessageRole::Assistant,
        content: assistant_message,
        created_at_ms: now_ms(),
    });
    draft.status = PluginDraftStatus::Ready;
    draft.last_error = None;
    draft.updated_at_ms = now_ms();
    Ok(PreparedDraftGeneration { draft, replacement })
}

fn require_generation_active(
    cancellation: &CancellationToken,
) -> Result<(), DraftGenerationFailure> {
    if cancellation.is_cancelled() {
        Err(DraftGenerationFailure::Cancelled)
    } else {
        Ok(())
    }
}

async fn persist_generated_draft(
    state: &PluginRouterState,
    prepared: PreparedDraftGeneration,
    expected_revision: u64,
    cancellation: &CancellationToken,
) -> Result<PluginDraftRecord, PluginHttpError> {
    let PreparedDraftGeneration {
        draft: ready,
        mut replacement,
    } = prepared;
    if cancellation.is_cancelled() {
        drop(replacement);
        return settle_cancelled_generation(state, &ready).await;
    }
    replacement.publish()?;
    if cancellation.is_cancelled() {
        replacement.rollback()?;
        return settle_cancelled_generation(state, &ready).await;
    }
    match state.repository.update_draft(&ready, expected_revision).await {
        Ok(updated) => {
            replacement.commit()?;
            Ok(updated)
        }
        Err(error) => {
            replacement.rollback().map_err(|rollback_error| {
                PluginHttpError::internal(format!(
                    "Draft metadata update failed ({error}) and file rollback failed ({rollback_error})"
                ))
            })?;
            if cancellation.is_cancelled() {
                settle_cancelled_generation(state, &ready).await
            } else {
                Err(error.into())
            }
        }
    }
}

async fn settle_cancelled_generation(
    state: &PluginRouterState,
    generating: &PluginDraftRecord,
) -> Result<PluginDraftRecord, PluginHttpError> {
    let mut ready = generating.clone();
    ready.status = PluginDraftStatus::Ready;
    ready.last_error = None;
    ready.updated_at_ms = now_ms();
    match state.repository.update_draft(&ready, generating.revision).await {
        Ok(updated) => Ok(updated),
        Err(PluginRepositoryError::Conflict) => {
            let current = state
                .repository
                .get_draft(&generating.owner_user_id, &generating.draft_id)
                .await?
                .ok_or_else(PluginHttpError::not_found)?;
            if current.status == PluginDraftStatus::Generating {
                Err(PluginHttpError::conflict("Draft generation state changed"))
            } else {
                Ok(current)
            }
        }
        Err(error) => Err(error.into()),
    }
}

async fn preview_draft(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(draft_id): AxumPath<String>,
    Json(request): Json<PreviewPluginDraftRequest>,
) -> Result<Json<ApiResponse<PluginDraftPreviewResponseDto>>, PluginHttpError> {
    let draft = draft(&state, &user, &draft_id).await?;
    require_draft_revision(&draft, request.expected_revision)?;
    require_draft_not_generating(&draft)?;
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
    if !artifact.manifest.has_ui() {
        return Err(PluginHttpError::bad_request("this headless Plugin has no App Surface"));
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
    let entrypoint = "ui/index.html".to_owned();
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
    Ok(Json(ApiResponse::ok(PluginDraftPreviewResponseDto {
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
    })))
}

async fn save_draft(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(draft_id): AxumPath<String>,
    Json(request): Json<SavePluginDraftRequest>,
) -> Result<Json<ApiResponse<SavePluginDraftResponseDto>>, PluginHttpError> {
    let mut draft = draft(&state, &user, &draft_id).await?;
    require_draft_revision(&draft, request.expected_revision)?;
    require_draft_not_generating(&draft)?;
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
    let confirmation = consume_confirmation(
        &state,
        &draft.owner_user_id,
        &artifact,
        request.permission_confirmation_id.as_deref(),
        existing.as_ref(),
    )
    .await?;
    if let Some(confirmation) = confirmation.required {
        return Ok(Json(ApiResponse::ok(SavePluginDraftResponseDto {
            draft: draft_summary(&state, &draft)?,
            result: PluginInstallOutcomeDto::ConfirmationRequired { confirmation },
        })));
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
        .install_files(
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
            None,
        )
        .await?;
    draft.plugin_id = Some(outcome.plugin.plugin_id.clone());
    draft.base_revision = Some(outcome.plugin.revision);
    draft.name = outcome.plugin.name.clone();
    draft.updated_at_ms = now_ms();
    let updated = state
        .repository
        .update_draft(&draft, request.expected_revision)
        .await?;
    let inventory = state
        .repository
        .inventory(&draft.owner_user_id, &outcome.plugin.plugin_id)
        .await?
        .ok_or(PluginRepositoryError::NotFound)?;
    Ok(Json(ApiResponse::ok(SavePluginDraftResponseDto {
        draft: draft_summary(&state, &updated)?,
        result: PluginInstallOutcomeDto::Installed {
            plugin: Box::new(detail_dto(&state, &inventory).await?),
        },
    })))
}

async fn delete_draft(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(draft_id): AxumPath<String>,
    Json(request): Json<DeletePluginDraftRequest>,
) -> Result<Json<ApiResponse<bool>>, PluginHttpError> {
    let draft = draft(&state, &user, &draft_id).await?;
    require_draft_revision(&draft, request.expected_revision)?;
    require_draft_not_generating(&draft)?;
    revoke_draft_surfaces(&state, &draft.draft_id).await?;
    state.drafts.discard(&draft.owner_user_id, &draft.draft_id)?;
    state
        .repository
        .delete_draft(&draft.owner_user_id, &draft.draft_id)
        .await?;
    Ok(Json(ApiResponse::ok(true)))
}

async fn export_package(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(plugin_id): AxumPath<String>,
    Json(request): Json<ExportPluginPackageRequest>,
) -> Result<Json<ApiResponse<PluginExportResultDto>>, PluginHttpError> {
    let inventory = inventory(&state, &user, &plugin_id).await?;
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
    Ok(Json(ApiResponse::ok(PluginExportResultDto {
        destination_path: request.destination_path,
        digest: artifact.artifact_digest.as_ref().to_owned(),
        size_bytes: exported_size(&destination)?,
    })))
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
    let owner = user.id.to_string();
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
    Ok(Json(ApiResponse::ok(detail_dto(&state, &inventory).await?)))
}

async fn configure(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(plugin_id): AxumPath<String>,
    Json(request): Json<ConfigurePluginRequest>,
) -> Result<Json<ApiResponse<PluginDetailDto>>, PluginHttpError> {
    let owner = user.id.to_string();
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
    Ok(Json(ApiResponse::ok(detail_dto(&state, &inventory).await?)))
}

async fn restore(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(plugin_id): AxumPath<String>,
    Json(request): Json<RestorePluginRequest>,
) -> Result<Json<ApiResponse<PluginDetailDto>>, PluginHttpError> {
    let owner = user.id.to_string();
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
    Ok(Json(ApiResponse::ok(detail_dto(&state, &inventory).await?)))
}

async fn trash(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(plugin_id): AxumPath<String>,
    Json(request): Json<TrashPluginRequest>,
) -> Result<Json<ApiResponse<PluginDetailDto>>, PluginHttpError> {
    let owner = user.id.to_string();
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
    Ok(Json(ApiResponse::ok(detail_dto(&state, &inventory).await?)))
}

async fn permanent_delete(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(plugin_id): AxumPath<String>,
    Json(request): Json<DeletePluginRequest>,
) -> Result<Json<ApiResponse<PluginLibraryResponseDto>>, PluginHttpError> {
    if !request.acknowledge_permanent_delete {
        return Err(PluginHttpError::bad_request("permanent delete must be acknowledged"));
    }
    let owner = user.id.to_string();
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
    Ok(Json(ApiResponse::ok(PluginLibraryResponseDto {
        revision,
        plugins,
    })))
}

async fn open_surface(
    State(state): State<PluginRouterState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(plugin_id): AxumPath<String>,
    Json(request): Json<OpenPluginSurfaceRequest>,
) -> Result<Json<ApiResponse<PluginSurfaceDescriptorDto>>, PluginHttpError> {
    let owner = user.id.to_string();
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
            owner_user_id: owner,
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
    Ok(Json(ApiResponse::ok(descriptor)))
}

async fn revoke_draft_surfaces(
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
    let owner = user.id.to_string();
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
    Ok(Json(ApiResponse::ok(true)))
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
    let plugin_id = parse_plugin_id(plugin_id)?;
    state
        .repository
        .inventory(&user.id.to_string(), &plugin_id)
        .await?
        .ok_or_else(PluginHttpError::not_found)
}

async fn draft(
    state: &PluginRouterState,
    user: &CurrentUser,
    draft_id: &str,
) -> Result<PluginDraftRecord, PluginHttpError> {
    let draft_id = parse_draft_id(draft_id)?;
    state
        .repository
        .get_draft(&user.id.to_string(), &draft_id)
        .await?
        .ok_or_else(PluginHttpError::not_found)
}

async fn recover_interrupted_draft_generations(
    repository: &SqlitePluginRepository,
    owner_user_id: &str,
) -> Result<usize, PluginRepositoryError> {
    let mut recovered = 0usize;
    for mut draft in repository.list_drafts(owner_user_id).await? {
        if draft.status != PluginDraftStatus::Generating {
            continue;
        }
        let expected_revision = draft.revision;
        draft.status = PluginDraftStatus::Failed;
        draft.last_error = Some(DRAFT_GENERATION_INTERRUPTED.to_owned());
        draft.updated_at_ms = now_ms();
        repository.update_draft(&draft, expected_revision).await?;
        recovered = recovered.saturating_add(1);
    }
    Ok(recovered)
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

fn draft_summary(
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
        revision: draft.revision,
        plugin_id: draft.plugin_id.as_ref().map(|id| id.as_ref().to_owned()),
        base_plugin_revision: draft.base_revision,
        package_id: manifest.as_ref().map(|manifest| manifest.id.clone()),
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
            PluginDraftStatus::Generating => PluginDraftStatusDto::Generating,
            PluginDraftStatus::Failed => PluginDraftStatusDto::Failed,
        },
        error_code: draft.last_error.clone(),
        updated_at_ms: u64::try_from(draft.updated_at_ms)
            .map_err(|_| PluginHttpError::internal("negative Draft timestamp"))?,
    })
}

fn draft_detail(
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
        messages: draft
            .messages
            .iter()
            .map(|message| PluginDraftMessageDto {
                role: match message.role {
                    PluginDraftMessageRole::User => PluginDraftMessageRoleDto::User,
                    PluginDraftMessageRole::Assistant => PluginDraftMessageRoleDto::Assistant,
                },
                content: message.content.clone(),
            })
            .collect(),
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

struct ConfirmationDecision {
    required: Option<PluginPermissionExpansionDto>,
    permissions: BTreeSet<String>,
    secret_slots: BTreeSet<String>,
    trusted_local_service: bool,
}

async fn consume_confirmation(
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

fn parse_model_json<T: for<'de> Deserialize<'de>>(raw: &str) -> Result<T, PluginHttpError> {
    let trimmed = raw.trim();
    let json = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|value| value.trim().strip_suffix("```").unwrap_or(value.trim()).trim())
        .unwrap_or(trimmed);
    serde_json::from_str(json)
        .map_err(|_| PluginHttpError::bad_gateway("the model returned invalid Plugin JSON"))
}

fn require_draft_revision(draft: &PluginDraftRecord, expected: u64) -> Result<(), PluginHttpError> {
    if draft.revision != expected {
        return Err(PluginHttpError::conflict("Draft revision changed"));
    }
    Ok(())
}

fn require_draft_not_generating(draft: &PluginDraftRecord) -> Result<(), PluginHttpError> {
    if draft.status == PluginDraftStatus::Generating {
        return Err(PluginHttpError::conflict("Draft generation is running"));
    }
    Ok(())
}

fn parse_plugin_id(value: &str) -> Result<PluginId, PluginHttpError> {
    let id = Uuid::parse_str(value).map_err(|_| PluginHttpError::not_found())?;
    if id.get_version_num() != 7 || id.to_string() != value {
        return Err(PluginHttpError::not_found());
    }
    Ok(PluginId::from(value.to_owned()))
}

fn parse_draft_id(value: &str) -> Result<PluginDraftId, PluginHttpError> {
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
struct PluginHttpError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl std::fmt::Display for PluginHttpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl PluginHttpError {
    fn bad_request(message: &str) -> Self {
        Self { status: StatusCode::BAD_REQUEST, code: "PLUGIN_INVALID_INPUT", message: message.into() }
    }
    fn conflict(message: &str) -> Self {
        Self { status: StatusCode::CONFLICT, code: "PLUGIN_CONFLICT", message: message.into() }
    }
    fn forbidden(message: &str) -> Self {
        Self { status: StatusCode::FORBIDDEN, code: "PLUGIN_PERMISSION_DENIED", message: message.into() }
    }
    fn not_found() -> Self {
        Self { status: StatusCode::NOT_FOUND, code: "PLUGIN_NOT_FOUND", message: "Plugin resource was not found".into() }
    }
    fn unavailable(message: &str) -> Self {
        Self { status: StatusCode::SERVICE_UNAVAILABLE, code: "PLUGIN_UNAVAILABLE", message: message.into() }
    }
    fn bad_gateway(message: &str) -> Self {
        Self { status: StatusCode::BAD_GATEWAY, code: "PLUGIN_GENERATION_FAILED", message: message.into() }
    }
    fn internal(message: impl Into<String>) -> Self {
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
mod draft_generation_tests {
    use super::*;

    #[tokio::test]
    async fn startup_recovery_closes_generating_drafts_once() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let owner = nomifun_db::installation_owner_id(database.pool())
            .await
            .unwrap();
        let repository = SqlitePluginRepository::new(database.pool().clone());
        let draft_id = PluginDraftId::from(Uuid::now_v7().to_string());
        repository
            .create_draft(&PluginDraftRecord {
                owner_user_id: owner.clone(),
                draft_id: draft_id.clone(),
                revision: 7,
                plugin_id: None,
                base_revision: None,
                name: "Interrupted generation".into(),
                workspace_path: "C:/test/draft".into(),
                messages: Vec::new(),
                status: PluginDraftStatus::Generating,
                last_error: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .await
            .unwrap();

        assert_eq!(
            recover_interrupted_draft_generations(&repository, &owner)
                .await
                .unwrap(),
            1
        );
        let recovered = repository
            .get_draft(&owner, &draft_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered.status, PluginDraftStatus::Failed);
        assert_eq!(recovered.revision, 8);
        assert_eq!(
            recovered.last_error.as_deref(),
            Some(DRAFT_GENERATION_INTERRUPTED)
        );
        assert_eq!(
            recover_interrupted_draft_generations(&repository, &owner)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            repository
                .get_draft(&owner, &draft_id)
                .await
                .unwrap()
                .unwrap()
                .revision,
            8
        );
    }

    #[tokio::test]
    async fn per_draft_generation_registry_is_single_flight_and_cancellable() {
        let generations = Arc::new(Mutex::new(HashMap::new()));
        let draft_id = PluginDraftId::from(Uuid::now_v7().to_string());
        let key = DraftGenerationKey::new("owner-a", &draft_id);
        let active = ActiveDraftGeneration {
            revision: 2,
            cancellation: CancellationToken::new(),
            done: Arc::new(Notify::new()),
        };
        {
            let mut locked = generations.lock().await;
            assert!(register_active_generation(
                &mut locked,
                key.clone(),
                active.clone(),
            ));
            assert!(!register_active_generation(
                &mut locked,
                key.clone(),
                ActiveDraftGeneration {
                    revision: 3,
                    cancellation: CancellationToken::new(),
                    done: Arc::new(Notify::new()),
                },
            ));
            assert_eq!(locked.len(), 1);
            assert_eq!(locked[&key].revision, 2);
        }

        let observed = active.cancellation.clone();
        let waiter = tokio::spawn(async move {
            observed.cancelled().await;
        });
        active.cancellation.cancel();
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("generation cancellation must wake the active request")
            .unwrap();

        assert!(generations.lock().await.remove(&key).is_some());
        assert!(generations.lock().await.is_empty());
    }
}
