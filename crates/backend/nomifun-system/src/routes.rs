use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Json, Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post};
use std::path::PathBuf;

use nomifun_api_types::{
    ApiResponse, ClientPreferencesResponse, CloneProviderRequest, CreateProviderRequest,
    FetchModelsAnonymousRequest, FetchModelsRequest, FetchModelsResponse, ManagedModel,
    ManagedModelHealthBatchResult, ModelTask,
    ManagedModelHealthResult, ManagedModelServiceStatus, SaveProviderConnectionRequest,
    ProbeProviderConnectionAnonymousRequest, ProbeProviderConnectionRequest,
    ProbeProviderConnectionResponse,
    ProviderConnectionResponse,
    ProviderModelKeyRequest, ProviderModelResponse, ProviderResponse, SaveProviderModelRequest,
    SetManagedModelEnabledRequest,
    SetManagedModelServiceEnabledRequest, SystemInfoResponse, SystemSettingsResponse, UpdateCheckRequest,
    UpdateCheckResult, UpdateClientPreferencesRequest, UpdateProviderRequest, UpdateSettingsRequest,
    UpdateWorkDirRequest,
};
use nomifun_common::AppError;

use crate::client_pref::ClientPrefService;
use crate::managed_model::ManagedModelService;
use crate::model_fetcher::ModelFetchService;
use crate::provider::ProviderService;
use crate::provider_connection::ProviderConnectionService;
use crate::provider_model::ProviderModelService;
use crate::settings::SettingsService;
use crate::version::VersionCheckService;

/// Shared state for system route handlers.
#[derive(Clone)]
pub struct SystemRouterState {
    pub settings_service: SettingsService,
    pub client_pref_service: ClientPrefService,
    pub provider_service: ProviderService,
    pub provider_connection_service: ProviderConnectionService,
    pub model_fetch_service: ModelFetchService,
    pub provider_model_service: ProviderModelService,
    pub managed_model_service: Option<std::sync::Arc<ManagedModelService>>,
    pub version_check_service: VersionCheckService,
    /// Data directory root — used to arm the v3 reset request consumed by the
    /// next boot. See `nomifun_common::factory_reset`.
    pub data_dir: PathBuf,
    /// Canonical work root used by the live dataset. Explicit reset requests
    /// are bound to this value so config/env changes cannot redirect them.
    pub work_dir: PathBuf,
    /// True when `--work-dir` has authoritative priority on every restart.
    pub work_dir_is_cli_override: bool,
}

/// Build the system router (settings + client prefs + providers + system).
///
/// All routes require authentication (applied by the caller).
///
/// Endpoints:
/// - `GET  /api/settings`                    — get all backend settings
/// - `PATCH /api/settings`                   — partial update backend settings
/// - `GET  /api/settings/client`             — get client preferences
/// - `PUT  /api/settings/client`             — batch update client preferences
/// - `GET  /api/providers`                   — list all providers
/// - `POST /api/providers`                   — create a provider
/// - `PUT  /api/providers/:provider_id`      — update a provider
/// - `DELETE /api/providers/:provider_id`    — delete a provider
/// - `POST /api/providers/:provider_id/clone` — clone a provider (models + connections)
/// - `GET  /api/providers/:provider_id/connections` — list connection profiles
/// - `PUT  /api/providers/:provider_id/connections` — upsert a connection profile
/// - `DELETE /api/providers/:provider_id/connections/:role` — delete a connection profile
/// - `POST /api/providers/:provider_id/models` — fetch models from remote API
/// - `POST /api/providers/fetch-models`      — fetch models anonymously (pre-create preview)
/// - `GET  /api/provider-models`             — list model catalog rows (`?provider_id=` filter)
/// - `PUT  /api/provider-models`             — full-save one model and all capabilities
/// - `DELETE /api/provider-models`           — delete one model and all capabilities
/// - `GET  /api/system/info`                 — system directory & platform info
/// - `POST /api/system/check-update`         — check GitHub for new versions
/// - `POST /api/system/factory-reset`        — arm a factory reset (wipes on next boot)
/// - `POST /api/system/work-dir`             — request a restart-time work-root change
pub fn system_routes(state: SystemRouterState) -> Router {
    Router::new()
        .route("/api/settings", get(get_settings).patch(update_settings))
        .route(
            "/api/settings/client",
            get(get_client_preferences).put(update_client_preferences),
        )
        .route("/api/providers", get(list_providers).post(create_provider))
        // Literal-segment routes must register BEFORE the provider routes so
        // axum matches the literal instead of treating "fetch-models" as a
        // provider id.
        .route("/api/providers/fetch-models", post(fetch_models_anonymous))
        .route("/api/providers/probe-connection", post(probe_connection_anonymous))
        .route("/api/model-protocols", get(list_model_protocols))
        .route("/api/model-services/free/status", get(get_free_model_status))
        .route("/api/model-services/free/models", get(get_free_models))
        .route("/api/model-services/free/refresh", post(refresh_free_models))
        .route(
            "/api/model-services/free/health",
            get(get_free_model_health).post(check_all_free_model_health),
        )
        .route("/api/model-services/free/activate", post(activate_free_models))
        .route(
            "/api/model-services/free/models/{model_id}/health",
            post(check_free_model_health),
        )
        .route(
            "/api/model-services/free/models/{model_id}",
            patch(set_free_model_enabled),
        )
        .route(
            "/api/providers/{provider_id}",
            delete(delete_provider).put(update_provider),
        )
        .route(
            "/api/providers/{provider_id}/clone",
            post(clone_provider),
        )
        .route(
            "/api/providers/{provider_id}/connections",
            get(list_provider_connections).put(upsert_provider_connection),
        )
        .route(
            "/api/providers/{provider_id}/connections/{role}",
            delete(delete_provider_connection),
        )
        .route(
            "/api/providers/{provider_id}/models",
            post(fetch_models),
        )
        .route(
            "/api/providers/{provider_id}/probe-connection",
            post(probe_connection),
        )
        // One configuration write surface: PUT atomically saves model metadata
        // and replaces its complete task-capability set.
        .route(
            "/api/provider-models",
            get(list_provider_models)
                .put(save_provider_model)
                .delete(delete_provider_model),
        )
        .route("/api/system/info", get(get_system_info))
        .route("/api/system/check-update", post(check_update))
        .route("/api/system/factory-reset", post(factory_reset))
        .route("/api/system/work-dir", post(set_work_dir))
        .with_state(state)
}

// ===========================================================================
// Settings handlers
// ===========================================================================

async fn get_settings(
    State(state): State<SystemRouterState>,
) -> Result<Json<ApiResponse<SystemSettingsResponse>>, AppError> {
    let settings = state.settings_service.get_settings().await?;
    Ok(Json(ApiResponse::ok(settings)))
}

async fn update_settings(
    State(state): State<SystemRouterState>,
    body: Result<Json<UpdateSettingsRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<SystemSettingsResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let settings = state.settings_service.update_settings(req).await?;
    Ok(Json(ApiResponse::ok(settings)))
}

// ===========================================================================
// Client preferences handlers
// ===========================================================================

#[derive(Debug, serde::Deserialize, Default)]
struct ClientPrefQuery {
    keys: Option<String>,
}

async fn get_client_preferences(
    State(state): State<SystemRouterState>,
    Query(query): Query<ClientPrefQuery>,
) -> Result<Json<ApiResponse<ClientPreferencesResponse>>, AppError> {
    let keys_filter: Option<Vec<String>> = query.keys.map(|k| {
        k.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });

    let key_refs: Option<Vec<&str>> = keys_filter.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect());

    let prefs = state.client_pref_service.get_preferences(key_refs.as_deref()).await?;
    Ok(Json(ApiResponse::ok(prefs)))
}

async fn update_client_preferences(
    State(state): State<SystemRouterState>,
    body: Result<Json<UpdateClientPreferencesRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.client_pref_service.update_preferences(req).await?;
    Ok(Json(ApiResponse::success()))
}

#[cfg(test)]
mod protocol_manifest_tests {
    use super::*;

    #[tokio::test]
    async fn model_protocol_manifest_handler_returns_scoped_stepfun_protocols() {
        let Json(response) = list_model_protocols(Query(ModelProtocolManifestQuery {
            preset: Some("StepFun-Plan".to_owned()),
            platform: None,
            base_url: None,
            model: None,
            task: ModelTask::RealtimeConversation,
        }))
        .await
        .expect("manifest response");
        let data = response.data.expect("manifest data");
        assert_eq!(data.tasks.len(), 9);
        assert_eq!(data.platform, "stepfun-plan");
        assert_eq!(data.protocols.len(), 1);
        assert_eq!(data.protocols[0].protocol_id, "stepfun.realtime_s2s");
    }

    #[tokio::test]
    async fn model_protocol_manifest_handler_forwards_custom_model_hint() {
        let Json(response) = list_model_protocols(Query(ModelProtocolManifestQuery {
            preset: Some("custom".to_owned()),
            platform: None,
            base_url: Some("https://gateway.example/v1".to_owned()),
            model: Some("user-entered-model".to_owned()),
            task: ModelTask::ImageEdit,
        }))
        .await
        .expect("manifest response");
        let recommendation = response
            .data
            .expect("manifest data")
            .recommendation
            .expect("custom model recommendation");
        assert_eq!(recommendation.protocol_id, "openai.images");
        assert_eq!(recommendation.connection_role, None);
        assert_eq!(recommendation.default_base_url, None);
    }

    #[tokio::test]
    async fn model_protocol_manifest_handler_requires_preset_or_platform() {
        let error = list_model_protocols(Query(ModelProtocolManifestQuery {
            preset: None,
            platform: None,
            base_url: None,
            model: None,
            task: ModelTask::Chat,
        }))
        .await
        .expect_err("missing preset must fail");
        assert!(matches!(error, AppError::BadRequest(_)));
    }
}

#[derive(Debug, serde::Deserialize)]
struct ModelProtocolManifestQuery {
    /// Stable UI preset id (`MODEL_PLATFORMS[].value`).
    preset: Option<String>,
    /// Backward-compatible canonical platform lookup for an existing provider.
    platform: Option<String>,
    /// Stored provider base URL, used to disambiguate regional presets that
    /// share a canonical platform id (for example SiliconFlow CN/global).
    base_url: Option<String>,
    /// Optional configuration-time model id. The manifest never parses this
    /// value; a non-blank id only enables the safe generic recommendation for
    /// the `custom` preset.
    model: Option<String>,
    task: ModelTask,
}

async fn list_model_protocols(
    Query(query): Query<ModelProtocolManifestQuery>,
) -> Result<Json<ApiResponse<nomifun_model_invoke::ModelProtocolManifestResponse>>, AppError> {
    let preset = query
        .preset
        .or(query.platform)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::BadRequest("preset query parameter is required".into()))?;
    Ok(Json(ApiResponse::ok(
        nomifun_model_invoke::protocol_manifest_for_model_connection(
            &preset,
            query.base_url.as_deref(),
            query.model.as_deref(),
            query.task,
        ),
    )))
}

// ===========================================================================
// Provider handlers
// ===========================================================================

async fn list_providers(
    State(state): State<SystemRouterState>,
) -> Result<Json<ApiResponse<Vec<ProviderResponse>>>, AppError> {
    let providers = state.provider_service.list().await?;
    Ok(Json(ApiResponse::ok(providers)))
}

async fn create_provider(
    State(state): State<SystemRouterState>,
    body: Result<Json<CreateProviderRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApiResponse<ProviderResponse>>), AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let provider = state.provider_service.create(req).await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(provider))))
}

async fn update_provider(
    State(state): State<SystemRouterState>,
    Path(provider_id): Path<String>,
    body: Result<Json<UpdateProviderRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<ProviderResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let provider = state.provider_service.update(&provider_id, req).await?;
    Ok(Json(ApiResponse::ok(provider)))
}

async fn delete_provider(
    State(state): State<SystemRouterState>,
    Path(provider_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state.provider_service.delete(&provider_id).await?;
    Ok(Json(ApiResponse::success()))
}

/// Server-side provider clone: copies the provider row (api-key ciphertext
/// as-is), every model and task capability (without health observations), and
/// every named connection in one graph transaction.
///
/// The JSON body is optional: a missing body (or one without a usable
/// `name`) falls back to the default `"{source name} copy"` clone name; a
/// trimmed non-empty `name` wins.
async fn clone_provider(
    State(state): State<SystemRouterState>,
    Path(provider_id): Path<String>,
    body: Option<Json<CloneProviderRequest>>,
) -> Result<(StatusCode, Json<ApiResponse<ProviderResponse>>), AppError> {
    let req = body.map(|Json(req)| req).unwrap_or_default();
    let provider = state
        .provider_service
        .clone_provider(&provider_id, req.name.as_deref())
        .await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(provider))))
}

async fn fetch_models(
    State(state): State<SystemRouterState>,
    Path(provider_id): Path<String>,
    body: Result<Json<FetchModelsRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<FetchModelsResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let result = state
        .model_fetch_service
        .fetch_models(&provider_id, &req)
        .await?;
    Ok(Json(ApiResponse::ok(result)))
}

async fn fetch_models_anonymous(
    State(state): State<SystemRouterState>,
    body: Result<Json<FetchModelsAnonymousRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<FetchModelsResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let result = state.model_fetch_service.fetch_models_anonymous(&req).await?;
    Ok(Json(ApiResponse::ok(result)))
}

/// Reachability test for one provider's configured connection root.
///
/// Unlike the per-model health check this needs no model or capability row, so a
/// freshly created custom provider can be validated before anything is built on
/// top of it.
async fn probe_connection(
    State(state): State<SystemRouterState>,
    Path(provider_id): Path<String>,
    body: Result<Json<ProbeProviderConnectionRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<ProbeProviderConnectionResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let result = state
        .model_fetch_service
        .probe_connection(&provider_id, &req)
        .await?;
    Ok(Json(ApiResponse::ok(result)))
}

/// Reachability test for a proposed connection, before the provider is saved.
async fn probe_connection_anonymous(
    State(state): State<SystemRouterState>,
    body: Result<Json<ProbeProviderConnectionAnonymousRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<ProbeProviderConnectionResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let result = state
        .model_fetch_service
        .probe_connection_anonymous(&req)
        .await?;
    Ok(Json(ApiResponse::ok(result)))
}

// ===========================================================================
// Provider connection handlers (non-default per-role connection profiles)
// ===========================================================================

async fn list_provider_connections(
    State(state): State<SystemRouterState>,
    Path(provider_id): Path<String>,
) -> Result<Json<ApiResponse<Vec<ProviderConnectionResponse>>>, AppError> {
    let connections = state.provider_connection_service.list(&provider_id).await?;
    Ok(Json(ApiResponse::ok(connections)))
}

async fn upsert_provider_connection(
    State(state): State<SystemRouterState>,
    Path(provider_id): Path<String>,
    body: Result<Json<SaveProviderConnectionRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<ProviderConnectionResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let connection = state
        .provider_connection_service
        .upsert(&provider_id, req)
        .await?;
    Ok(Json(ApiResponse::ok(connection)))
}

async fn delete_provider_connection(
    State(state): State<SystemRouterState>,
    Path((provider_id, role)): Path<(String, String)>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state
        .provider_connection_service
        .delete(&provider_id, &role)
        .await?;
    Ok(Json(ApiResponse::success()))
}

// ===========================================================================
// Managed model services
// ===========================================================================

fn managed_service(
    state: &SystemRouterState,
) -> Result<std::sync::Arc<ManagedModelService>, AppError> {
    state.managed_model_service.clone().ok_or_else(|| {
        AppError::ProviderUnavailable("managed model service is not available in this process".into())
    })
}

async fn get_free_model_status(
    State(state): State<SystemRouterState>,
) -> Result<Json<ApiResponse<ManagedModelServiceStatus>>, AppError> {
    Ok(Json(ApiResponse::ok(
        managed_service(&state)?.free_status().await,
    )))
}

async fn get_free_models(
    State(state): State<SystemRouterState>,
) -> Result<Json<ApiResponse<Vec<ManagedModel>>>, AppError> {
    Ok(Json(ApiResponse::ok(
        managed_service(&state)?.free_models().await,
    )))
}

async fn refresh_free_models(
    State(state): State<SystemRouterState>,
) -> Result<Json<ApiResponse<ManagedModelServiceStatus>>, AppError> {
    let status = managed_service(&state)?.refresh_free_models().await?;
    Ok(Json(ApiResponse::ok(status)))
}

async fn get_free_model_health(
    State(state): State<SystemRouterState>,
) -> Result<Json<ApiResponse<Vec<ManagedModelHealthResult>>>, AppError> {
    Ok(Json(ApiResponse::ok(
        managed_service(&state)?.free_health_snapshot().await?,
    )))
}

async fn check_free_model_health(
    State(state): State<SystemRouterState>,
    Path(model_id): Path<String>,
) -> Result<Json<ApiResponse<ManagedModelHealthResult>>, AppError> {
    let service = managed_service(&state)?;
    let result = service.check_free_model_health(&model_id).await?;
    Ok(Json(ApiResponse::ok(result)))
}

async fn check_all_free_model_health(
    State(state): State<SystemRouterState>,
) -> Result<Json<ApiResponse<ManagedModelHealthBatchResult>>, AppError> {
    let service = managed_service(&state)?;
    Ok(Json(ApiResponse::ok(
        service.check_all_free_model_health().await,
    )))
}

async fn activate_free_models(
    State(state): State<SystemRouterState>,
    body: Result<Json<SetManagedModelServiceEnabledRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<ManagedModelServiceStatus>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let status = managed_service(&state)?
        .set_free_enabled(req.enabled)
        .await?;
    Ok(Json(ApiResponse::ok(status)))
}

async fn set_free_model_enabled(
    State(state): State<SystemRouterState>,
    Path(model_id): Path<String>,
    body: Result<Json<SetManagedModelEnabledRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<ManagedModelServiceStatus>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let status = managed_service(&state)?
        .set_free_model_enabled(&model_id, req.enabled)
        .await?;
    Ok(Json(ApiResponse::ok(status)))
}

// ===========================================================================
// Provider-model handlers (one full-save configuration surface)
// ===========================================================================

#[derive(Debug, serde::Deserialize, Default)]
struct ListProviderModelsQuery {
    provider_id: Option<String>,
}

async fn list_provider_models(
    State(state): State<SystemRouterState>,
    Query(query): Query<ListProviderModelsQuery>,
) -> Result<Json<ApiResponse<Vec<ProviderModelResponse>>>, AppError> {
    let models = state
        .provider_model_service
        .list(query.provider_id.as_deref())
        .await?;
    Ok(Json(ApiResponse::ok(models)))
}

async fn save_provider_model(
    State(state): State<SystemRouterState>,
    body: Result<Json<SaveProviderModelRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<ProviderModelResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let model = state.provider_model_service.save(req).await?;
    Ok(Json(ApiResponse::ok(model)))
}

async fn delete_provider_model(
    State(state): State<SystemRouterState>,
    Query(req): Query<ProviderModelKeyRequest>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let deleted = state
        .provider_model_service
        .delete(&req.provider_id, &req.model)
        .await?;
    if !deleted {
        return Err(AppError::NotFound(format!(
            "Provider model '{}' not found for provider '{}'",
            req.model, req.provider_id
        )));
    }
    Ok(Json(ApiResponse::success()))
}

// ===========================================================================
// System info & version check handlers
// ===========================================================================

async fn get_system_info() -> Json<ApiResponse<SystemInfoResponse>> {
    let info = crate::sysinfo::get_system_info();
    Json(ApiResponse::ok(info))
}

async fn check_update(
    State(state): State<SystemRouterState>,
    body: Result<Json<UpdateCheckRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<UpdateCheckResult>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let result = state.version_check_service.check_update(&req).await?;
    Ok(Json(ApiResponse::ok(result)))
}

// ===========================================================================
// Factory reset handler
// ===========================================================================

/// Arm a factory reset: write the marker that the next boot consumes. The
/// actual database/derived-data wipe happens early on the next startup (see
/// `nomifun_common::factory_reset`); the client should restart the app right
/// after this returns. Nothing is deleted synchronously here — that would race
/// with the live connection pool and the background write loops.
async fn factory_reset(State(state): State<SystemRouterState>) -> Result<Json<ApiResponse<()>>, AppError> {
    nomifun_common::factory_reset::require_safe_data_work_root_layout(
        &state.data_dir,
        &state.work_dir,
    )
    .map_err(|_| {
        AppError::Conflict(
            "factory reset is unsafe because the active work directory overlaps \
             a product-managed data root; first change to a separate working directory"
                .into(),
        )
    })?;
    nomifun_common::factory_reset::request_v3_dataset_reset(
        &state.data_dir,
        &state.work_dir,
    )?;
    tracing::warn!(target: "factory_reset", "factory reset armed — will wipe database and derived data on next restart");
    Ok(Json(ApiResponse::success()))
}

// ===========================================================================
// Work directory handler
// ===========================================================================

/// Request a user-chosen working directory. This only takes effect on the next
/// boot: the backend resolves `work_dir` before the HTTP server exists.
///
/// Changing the root of a finalized v3 dataset also arms one durable reset so
/// database/side-store IDs cannot be attached to an unrelated workspace. The
/// old workspace is not migrated or deleted. The request is consumed by the
/// immutable reset plan, so later boots do not reset the new generation again.
///
/// The new path is validated to be a non-empty, absolute, creatable directory so
/// the next boot does not fail on an unusable value.
async fn set_work_dir(
    State(state): State<SystemRouterState>,
    body: Result<Json<UpdateWorkDirRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    if state.work_dir_is_cli_override {
        return Err(AppError::Conflict(
            "the working directory is controlled by the --work-dir startup option; remove that option before changing it in Settings"
                .into(),
        ));
    }

    let trimmed = req.work_dir.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("work_dir must not be empty".into()));
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err(AppError::BadRequest(format!("work_dir must be an absolute path: {trimmed}")));
    }
    // Reject paths with a leading/trailing-whitespace segment up front, with the
    // same dedicated error the conversation layer raises (service.rs) — otherwise
    // such a work_dir is accepted here only to make every later workspace
    // creation fail, and create_dir_all's behavior on these names is OS-specific.
    if nomifun_common::workspace_path_has_edge_whitespace_segment(&path) {
        return Err(AppError::WorkspacePathEdgeWhitespace(path.display().to_string()));
    }
    let current_work_dir =
        nomifun_common::factory_reset::finalized_v3_work_dir(
            &state.data_dir,
        )?
        .ok_or_else(|| {
            AppError::Conflict(
                "the current dataset has no finalized v3 work-root binding; preserving it without changing directories"
                    .into(),
            )
        })?;
    // Create it now so we (a) confirm the location is writable and (b) reject a
    // path that collides with an existing file — both would otherwise surface as
    // a confusing failure on the next boot.
    std::fs::create_dir_all(&path)
        .map_err(|e| AppError::BadRequest(format!("cannot use work_dir {}: {e}", path.display())))?;
    if !path.is_dir() {
        return Err(AppError::BadRequest(format!(
            "work_dir is not a directory: {}",
            path.display()
        )));
    }

    let canonical = std::fs::canonicalize(&path).map_err(|error| {
        AppError::BadRequest(format!(
            "cannot canonicalize work_dir {}: {error}",
            path.display()
        ))
    })?;
    if let Some(pending_work_dir) =
        nomifun_common::factory_reset::requested_v3_reset_work_dir(
            &state.data_dir,
        )?
        && pending_work_dir != canonical
    {
        return Err(AppError::Conflict(format!(
            "a restart-time dataset operation is already bound to {}; restart NomiFun before requesting another work directory",
            pending_work_dir.display()
        )));
    }
    match current_work_dir {
        current if current == canonical => {
            // Repair or refresh only the host-local control pointer; the
            // dataset binding is already correct, so no reset is needed.
            nomifun_common::dir_config::set_work_dir(
                &state.data_dir,
                &canonical,
            )?;
            tracing::info!(
                target: "system",
                work_dir = %canonical.display(),
                "work dir override refreshed without changing dataset generation"
            );
        }
        _ => {
            nomifun_common::factory_reset::request_v3_dataset_reset_for_work_dir(
                &state.data_dir,
                &canonical,
            )?;
            tracing::warn!(
                target: "system",
                work_dir = %canonical.display(),
                "work dir change armed a one-shot fresh v3 dataset; historical data will not be migrated"
            );
        }
    }
    Ok(Json(ApiResponse::success()))
}
