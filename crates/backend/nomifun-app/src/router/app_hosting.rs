//! Ordinary application hosting. HTTP and Agent tools use the same owner.
use std::{path::PathBuf, sync::Arc};
use axum::{Router, Json, body::Bytes, extract::{State, Path, Query, DefaultBodyLimit}, routing::{get, post}};
use nomifun_api_types::ApiResponse;
use nomifun_common::AppError;
use nomifun_app_hosting::{HostingOwner, AppDetails, AppMetadata, AppRecord, AppSettingsPatch, ImportKind, AppOperation};
use serde::{Deserialize, Serialize};
use super::nomi_core_session::NomiCoreAgentApiState;

#[derive(Clone)]
pub(crate) struct AppHostingRouterState {
    owner: Arc<HostingOwner>,
    agent: NomiCoreAgentApiState,
    owner_id: Arc<str>,
    uploads: PathBuf,
    sources: Arc<super::app_mcp_sources::AppMcpSources>,
    publisher: Arc<dyn nomifun_mcp::service::McpCatalogPublisher>,
}

impl AppHostingRouterState {
    pub(crate) fn new(services: &crate::services::AppServices, agent: NomiCoreAgentApiState,
        publisher: Arc<dyn nomifun_mcp::service::McpCatalogPublisher>) -> Self {
        Self { owner: services.app_hosting.clone(), agent,
            owner_id: services.authoritative_user_id.clone(), uploads: services.data_dir.join("app-imports"),
            sources: services.app_mcp_sources.clone(), publisher }
    }
}

pub(crate) fn routes(state: AppHostingRouterState) -> Router {
    let sources = super::app_mcp_sources::routes(super::app_mcp_sources::AppMcpRouterState {
        sources: state.sources.clone(), publisher: state.publisher.clone(),
    });
    Router::new()
        .route("/api/apps", get(list).post(create))
        .route("/api/apps/import", post(import))
        .route("/api/apps/import-zip", post(import_zip))
        .route("/api/apps/environment", post(environment))
        .route("/api/apps/{id}", get(detail).put(update).delete(delete))
        .route("/api/apps/{id}/operate", post(operate))
        .route("/api/apps/{id}/development-session", post(development_session))
        .route("/api/apps/{id}/development-sessions", get(development_sessions))
        .route("/api/apps/{id}/import", post(import_into))
        .route("/api/apps/{id}/import-zip", post(import_zip_into))
        .route("/api/apps/{id}/overwrite", post(overwrite))
        .route("/api/apps/{id}/overwrite-zip", post(overwrite_zip))
        .layer(DefaultBodyLimit::max(256 * 1024 * 1024))
        .with_state(state)
        .merge(sources)
}

fn failure(error: impl std::fmt::Display) -> AppError {
    AppError::BadRequest(format!("{error}"))
}

async fn list(State(state): State<AppHostingRouterState>) -> Result<Json<ApiResponse<Vec<AppRecord>>>, AppError> {
    Ok(Json(ApiResponse::ok(state.owner.list().await.map_err(failure)?)))
}
async fn detail(State(state): State<AppHostingRouterState>, Path(id): Path<String>) -> Result<Json<ApiResponse<AppRecord>>, AppError> {
    Ok(Json(ApiResponse::ok(state.owner.get(&id).await.map_err(failure)?)))
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct CreateRequest { name: Option<String>, description: String, tags: Vec<String> }
async fn create(State(state): State<AppHostingRouterState>, Json(request): Json<CreateRequest>) -> Result<Json<ApiResponse<AppRecord>>, AppError> {
    retained(async move {
        Ok(Json(ApiResponse::ok(state.owner.create(request.name.unwrap_or_else(|| "新应用".into()),
            AppMetadata { description: request.description, tags: request.tags }).await.map_err(failure)?)))
    }).await
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct DevelopmentSessionRequest { model: Option<nomifun_api_types::AgentChatModelSelectionDto>,
    reasoning_effort: Option<nomifun_api_types::SessionReasoningEffortDto>, refresh_workflow: bool,
    session_id: Option<String>, new_session: bool, creation_key: Option<String> }
#[derive(Serialize)]
struct DevelopmentSessionResponse { app: AppRecord, conversation: nomifun_api_types::ConversationResponse }
async fn development_session(State(state): State<AppHostingRouterState>, Path(id): Path<String>,
    Json(request): Json<DevelopmentSessionRequest>) -> Result<Json<ApiResponse<DevelopmentSessionResponse>>, AppError> {
    retained(async move {
        if request.new_session && request.session_id.is_some() {
            return Err(AppError::BadRequest("Select an existing project Session or create a new one".into()));
        }
        if request.new_session != request.creation_key.is_some() {
            return Err(AppError::BadRequest("A new project Session requires its creation_key; other commands must omit it".into()));
        }
        if let Some(key) = &request.creation_key {
            let parsed = uuid::Uuid::parse_str(key).map_err(|_| AppError::BadRequest("creation_key must be a UUIDv7".into()))?;
            if parsed.get_version_num() != 7 || parsed.to_string() != *key {
                return Err(AppError::BadRequest("creation_key must be an exact UUIDv7".into()));
            }
        }
        let current = state.owner.get(&id).await.map_err(failure)?;
        // The shared project admission fence precedes the application gate,
        // so physical file commands cannot race a new Session's registration.
        let _project_fence = super::nomi_core_session::guard_app_project_directory_read(
            &state.agent, &state.owner_id, &current.directory).map_err(|error| AppError::Conflict(error.message))?;
        let session = Arc::new(std::sync::Mutex::new(None));
        let captured = session.clone();
        let agent = state.agent.clone();
        let owner_id = state.owner_id.clone();
        let app = state.owner.development_session_with(&id, move |app| async move {
            let conversation = super::nomi_core_session::resolve_app_development_session_selection(&agent, &owner_id, &app,
                request.session_id.as_deref(), request.creation_key.as_deref(), request.model, request.reasoning_effort).await
                .map_err(|error| nomifun_app_hosting::AppError::Invalid(error.message))?;
            let conversation = if request.refresh_workflow {
                super::nomi_core_session::refresh_app_development_workflow_session(&agent, &owner_id, &app, conversation).await
                    .map_err(|error| nomifun_app_hosting::AppError::Invalid(error.message))?
            } else { conversation };
            let session_id = conversation.conversation_id.clone();
            *captured.lock().unwrap_or_else(|error| error.into_inner()) = Some(conversation);
            Ok(session_id)
        }).await.map_err(failure)?;
        let conversation = session.lock().unwrap_or_else(|error| error.into_inner()).take()
            .ok_or_else(|| AppError::Conflict("Application development Session outcome is unknown".into()))?;
        Ok(Json(ApiResponse::ok(DevelopmentSessionResponse { app, conversation })))
    }).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DevelopmentSessionsQuery {
    cursor: Option<String>,
    #[serde(default = "default_project_session_limit")]
    limit: u32,
}
fn default_project_session_limit() -> u32 { 50 }

async fn development_sessions(State(state): State<AppHostingRouterState>, Path(id): Path<String>,
    Query(query): Query<DevelopmentSessionsQuery>) -> Result<Json<ApiResponse<super::nomi_core_session::AppProjectSessionHistory>>, AppError> {
    if query.limit == 0 || query.limit > 10_000 {
        return Err(AppError::BadRequest("Project Session limit must be between 1 and 10000".into()));
    }
    let app = state.owner.get(&id).await.map_err(failure)?;
    let history = super::nomi_core_session::app_project_session_history(&state.agent, &state.owner_id,
        &app, query.cursor.as_deref(), query.limit).await.map_err(|error| AppError::Conflict(error.message))?;
    Ok(Json(ApiResponse::ok(history)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportRequest { source: PathBuf, name: Option<String>, import_kind: ImportKind }
async fn import(State(state): State<AppHostingRouterState>, Json(request): Json<ImportRequest>) -> Result<Json<ApiResponse<AppRecord>>, AppError> {
    retained(async move {
        Ok(Json(ApiResponse::ok(state.owner.import(&request.source, request.name, request.import_kind).await.map_err(failure)?)))
    }).await
}
async fn import_into(State(state): State<AppHostingRouterState>, Path(id): Path<String>,
    Json(request): Json<ImportRequest>) -> Result<Json<ApiResponse<AppRecord>>, AppError> {
    retained(async move {
        let (previous, _development_fence) = guard_manual_file_change(&state, &id).await?;
        let reference = previous.development_session_id.clone();
        if previous.settings.mcp_url.is_some() {
            state.sources.remove_observation(&id)?;
            let _ = state.sources.publish().await;
        }
        let app = state.owner.import_into_with(&id, &request.source, request.name, request.import_kind,
            move |current| async move { verify_development_reference(&current, &reference) }).await.map_err(failure)?;
        refresh_sources(&state, &app, previous.settings.mcp_url.is_some()).await;
        Ok(Json(ApiResponse::ok(app)))
    }).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ZipQuery { name: Option<String>, import_kind: ImportKind }
async fn store_upload(state: &AppHostingRouterState, bytes: Bytes) -> Result<PathBuf, AppError> {
    tokio::fs::create_dir_all(&state.uploads).await.map_err(failure)?;
    let path = state.uploads.join(format!("{}.zip", uuid::Uuid::now_v7()));
    tokio::fs::write(&path, bytes).await.map_err(failure)?;
    Ok(path)
}
async fn import_zip(State(state): State<AppHostingRouterState>, Query(query): Query<ZipQuery>, bytes: Bytes) -> Result<Json<ApiResponse<AppRecord>>, AppError> {
    retained(async move {
        let path = store_upload(&state, bytes).await?;
        let result = state.owner.import(&path, query.name, query.import_kind).await.map_err(failure);
        let _ = tokio::fs::remove_file(path).await;
        Ok(Json(ApiResponse::ok(result?)))
    }).await
}
async fn import_zip_into(State(state): State<AppHostingRouterState>, Path(id): Path<String>,
    Query(query): Query<ZipQuery>, bytes: Bytes) -> Result<Json<ApiResponse<AppRecord>>, AppError> {
    retained(async move {
        let (previous, _development_fence) = guard_manual_file_change(&state, &id).await?;
        let reference = previous.development_session_id.clone();
        if previous.settings.mcp_url.is_some() {
            state.sources.remove_observation(&id)?;
            let _ = state.sources.publish().await;
        }
        let path = store_upload(&state, bytes).await?;
        let result = state.owner.import_into_with(&id, &path, query.name, query.import_kind,
            move |current| async move { verify_development_reference(&current, &reference) }).await.map_err(failure);
        let _ = tokio::fs::remove_file(path).await;
        let app = result?;
        refresh_sources(&state, &app, previous.settings.mcp_url.is_some()).await;
        Ok(Json(ApiResponse::ok(app)))
    }).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DevelopmentCompletion { session_id: String, turn_id: String }
/// One response observation derived from the exact completed canonical Turn.
/// It is never stored on the application or returned by detail/list reads.
#[derive(Serialize)]
struct DevelopmentCompletionObservation { session_id: String, turn_id: String, files_changed: bool }
#[derive(Serialize)]
struct UpdateResponse {
    #[serde(flatten)]
    app: AppRecord,
    #[serde(skip_serializing_if = "Option::is_none")]
    completion_observation: Option<DevelopmentCompletionObservation>,
}
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct UpdateRequest { name: Option<String>, #[serde(deserialize_with = "editable_settings_patch")] settings: Option<AppSettingsPatch>, description: Option<String>,
    tags: Option<Vec<String>>, import_kind: Option<ImportKind>, detect_settings: bool, completion: Option<DevelopmentCompletion> }

fn editable_settings_patch<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<AppSettingsPatch>, D::Error> {
    let mut value = serde_json::Value::deserialize(deserializer)?;
    // Full settings previously round-tripped this read-only owner reference.
    // It never authorizes a caller to replace the current Agent connection.
    if let Some(settings) = value.as_object_mut() { settings.remove("agent_binding_id"); }
    serde_json::from_value(value).map(Some).map_err(serde::de::Error::custom)
}
async fn update(State(state): State<AppHostingRouterState>, Path(id): Path<String>, Json(request): Json<UpdateRequest>) -> Result<Json<ApiResponse<UpdateResponse>>, AppError> {
    retained(async move { update_owned(state, id, request).await }).await
}
async fn update_owned(state: AppHostingRouterState, id: String, request: UpdateRequest) -> Result<Json<ApiResponse<UpdateResponse>>, AppError> {
    if request.completion.is_some() && !request.detect_settings {
        return Err(AppError::BadRequest("A development completion reference requires configuration detection".into()));
    }
    if request.completion.is_some() && (request.name.is_some() || request.description.is_some() || request.tags.is_some()
        || request.import_kind.is_some() || request.settings.is_some()) {
        return Err(AppError::BadRequest("A development completion request only detects current configuration; save explicit edits separately".into()));
    }
    let mut completion_observation = None;
    let (_development_fence, expected_reference) = if request.detect_settings {
        let current = state.owner.get(&id).await.map_err(failure)?;
        let (fence, generated_files_changed) = if let Some(completion) = &request.completion {
            let guarded = super::nomi_core_session::guard_app_development_completed_config(&state.agent, &state.owner_id, &current,
                &completion.session_id, &completion.turn_id).await.map_err(|error| AppError::Conflict(error.message))?;
            completion_observation = Some(DevelopmentCompletionObservation { session_id: completion.session_id.clone(),
                turn_id: completion.turn_id.clone(), files_changed: guarded.generated_files_changed });
            (guarded.fence, guarded.generated_files_changed)
        } else { (super::nomi_core_session::guard_app_development_workspace_change(&state.agent, &state.owner_id, &current).await
            .map_err(|error| AppError::Conflict(error.message))?, true) };
        if !generated_files_changed {
            let observed = state.owner.get(&id).await.map_err(failure)?;
            verify_development_reference(&observed, &current.development_session_id).map_err(failure)?;
            return Ok(Json(ApiResponse::ok(UpdateResponse { app: observed, completion_observation })));
        }
        (fence, Some(current.development_session_id))
    } else { (None, None) };
    let agent = state.agent.clone();
    let owner_id = state.owner_id.clone();
    let receipt = Arc::new(std::sync::Mutex::new(None::<super::nomi_core_session::AppAgentBindingChange>));
    let captured = receipt.clone();
    let rollback_agent = state.agent.clone();
    let rollback_owner = state.owner_id.clone();
    let previous_source = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let captured_source = previous_source.clone();
    let result = state.owner.update_guarded_patch_details_with_rollback(&id, request.name,
        AppDetails { description: request.description, tags: request.tags, import_kind: request.import_kind }, request.settings.unwrap_or_default(), request.detect_settings,
        move |current| { if let Some(expected) = expected_reference { verify_development_reference(current, &expected)?; } Ok(()) },
        move |previous, mut settings| async move {
        captured_source.store(previous.settings.mcp_url.is_some(), std::sync::atomic::Ordering::Relaxed);
        settings.agent_binding_id = previous.settings.agent_binding_id.clone();
        settings.agent_id = settings.agent_id.filter(|id| !id.trim().is_empty());
        if settings.agent_id != previous.settings.agent_id {
            if previous.state == nomifun_app_hosting::AppState::Running || previous.pid.is_some() {
                return Err(nomifun_app_hosting::AppError::Invalid("Stop the application before changing its Agent; the running process still uses its current connection".into()));
            }
            let change = match settings.agent_id.as_deref() {
                Some(id) => Some(super::nomi_core_session::prepare_app_agent_binding(
                    &agent, &owner_id, &previous.id, id, previous.settings.agent_binding_id.as_deref(),
                ).await.map_err(|error| nomifun_app_hosting::AppError::Invalid(error.message))?),
                None => match previous.settings.agent_binding_id.as_deref() {
                    Some(binding) => Some(super::nomi_core_session::clear_app_agent_binding(&agent, &owner_id, &previous.id, binding).await
                        .map_err(|error| nomifun_app_hosting::AppError::Invalid(error.message))?),
                    None => None,
                },
            };
            settings.agent_binding_id = if settings.agent_id.is_some() { change.as_ref().map(|change| change.binding_id.clone()) } else { None };
            *captured.lock().unwrap_or_else(|error| error.into_inner()) = change;
        }
        Ok(settings)
    }, move || async move {
        let change = receipt.lock().unwrap_or_else(|error| error.into_inner()).take();
        if let Some(change) = change {
            super::nomi_core_session::rollback_app_agent_binding(&rollback_agent, &rollback_owner, &change).await
                .map_err(|error| nomifun_app_hosting::AppError::Command(error.message))?;
        }
        Ok(())
    }).await;
    let updated = result.map_err(failure)?;
    refresh_sources(&state, &updated, previous_source.load(std::sync::atomic::Ordering::Relaxed)).await;
    Ok(Json(ApiResponse::ok(UpdateResponse { app: updated, completion_observation })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperateRequest { operation: AppOperation }
async fn operate(State(state): State<AppHostingRouterState>, Path(id): Path<String>, Json(request): Json<OperateRequest>) -> Result<Json<ApiResponse<AppRecord>>, AppError> {
    retained(async move {
        let previous = state.owner.get(&id).await.map_err(failure)?;
        if previous.settings.mcp_url.is_some() && (request.operation != AppOperation::Start || previous.state != nomifun_app_hosting::AppState::Running) {
            state.sources.remove_observation(&id)?;
            let _ = state.sources.publish().await;
        }
        let app = state.owner.operate(&id, request.operation).await.map_err(failure)?;
        refresh_sources(&state, &app, previous.settings.mcp_url.is_some()).await;
        Ok(Json(ApiResponse::ok(app)))
    }).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OverwriteRequest { source: PathBuf }
async fn overwrite(State(state): State<AppHostingRouterState>, Path(id): Path<String>, Json(request): Json<OverwriteRequest>) -> Result<Json<ApiResponse<AppRecord>>, AppError> {
    retained(async move {
        let (previous, _development_fence) = guard_manual_file_change(&state, &id).await?;
        let reference = previous.development_session_id.clone();
        let app = state.owner.overwrite_with(&id, &request.source,
            move |current| async move { verify_development_reference(&current, &reference) }).await.map_err(failure)?;
        refresh_sources(&state, &app, previous.settings.mcp_url.is_some()).await;
        Ok(Json(ApiResponse::ok(app)))
    }).await
}
async fn overwrite_zip(State(state): State<AppHostingRouterState>, Path(id): Path<String>, bytes: Bytes) -> Result<Json<ApiResponse<AppRecord>>, AppError> {
    retained(async move {
        let (previous, _development_fence) = guard_manual_file_change(&state, &id).await?;
        let reference = previous.development_session_id.clone();
        let path = store_upload(&state, bytes).await?;
        let result = state.owner.overwrite_with(&id, &path,
            move |current| async move { verify_development_reference(&current, &reference) }).await.map_err(failure);
        let _ = tokio::fs::remove_file(path).await;
        let app = result?;
        refresh_sources(&state, &app, previous.settings.mcp_url.is_some()).await;
        Ok(Json(ApiResponse::ok(app)))
    }).await
}

async fn guard_manual_file_change(state: &AppHostingRouterState, id: &str)
    -> Result<(AppRecord, Option<super::nomi_core_session::AppProjectWorkspaceFence>), AppError> {
    let app = state.owner.get(id).await.map_err(failure)?;
    let fence = super::nomi_core_session::guard_app_development_workspace_change(&state.agent, &state.owner_id, &app).await
        .map_err(|error| AppError::Conflict(error.message))?;
    Ok((app, fence))
}

fn verify_development_reference(app: &AppRecord, expected: &Option<String>) -> Result<(), nomifun_app_hosting::AppError> {
    if &app.development_session_id != expected {
        return Err(nomifun_app_hosting::AppError::Invalid("Application development Session changed; retry the file operation".into()));
    }
    Ok(())
}

async fn delete(State(state): State<AppHostingRouterState>, Path(id): Path<String>) -> Result<Json<ApiResponse<bool>>, AppError> {
    retained(async move { delete_owned(state, id).await }).await
}
async fn delete_owned(state: AppHostingRouterState, id: String) -> Result<Json<ApiResponse<bool>>, AppError> {
    let current = state.owner.get(&id).await.map_err(failure)?;
    let development_session_id = current.development_session_id.clone();
    let _development_fence = super::nomi_core_session::guard_app_development_workspace_change(&state.agent, &state.owner_id, &current).await
        .map_err(|error| AppError::Conflict(error.message))?;
    let agent = state.agent.clone();
    let owner_id = state.owner_id.clone();
    let previous_source = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let captured_source = previous_source.clone();
    let resource_cleanup_error = Arc::new(std::sync::Mutex::new(None));
    let captured_cleanup_error = resource_cleanup_error.clone();
    let result = state.owner.delete_with(&id, move |app| async move {
        if app.development_session_id != development_session_id {
            return Err(nomifun_app_hosting::AppError::Invalid("Application development Session changed; retry deletion".into()));
        }
        if let Err(error) = super::nomi_core_session::release_app_development_workspace_runtime(&agent, &app).await {
            *captured_cleanup_error.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error.message.clone());
            return Err(nomifun_app_hosting::AppError::Command(error.message));
        }
        captured_source.store(app.settings.mcp_url.is_some(), std::sync::atomic::Ordering::Relaxed);
        if let Some(binding) = app.settings.agent_binding_id.as_deref() {
            super::nomi_core_session::delete_app_agent_binding(&agent, &owner_id, binding).await
                .map_err(|error| nomifun_app_hosting::AppError::Command(error.message))?;
        }
        Ok(())
    }).await;
    if let Err(error) = result {
        if let Some(message) = resource_cleanup_error.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
            return Err(AppError::Conflict(message));
        }
        return Err(failure(error));
    }
    state.sources.remove_observation(&id)?;
    if previous_source.load(std::sync::atomic::Ordering::Relaxed) && let Err(error) = state.sources.publish().await {
        tracing::warn!(app_id = %id, %error, "Application removed; MCP catalog refresh failed");
    }
    Ok(Json(ApiResponse::ok(true)))
}

/// The owned effect continues when its HTTP waiter disconnects. It holds only
/// the application's existing owner gate, and Host shutdown still cancels it.
async fn retained<T: Send + 'static>(future: impl std::future::Future<Output=Result<T,AppError>> + Send + 'static) -> Result<T,AppError> {
    tokio::spawn(future).await.map_err(|error| AppError::Conflict(format!("Application operation outcome is unknown: {error}")))?
}

async fn refresh_sources(state: &AppHostingRouterState, app: &AppRecord, previous_source: bool) {
    // Ordinary applications cannot change the Agent tool registry. A removed
    // MCP address still needs to retire its previously published source.
    if app.settings.mcp_url.is_none() && !previous_source { return; }
    if app.state == nomifun_app_hosting::AppState::Running && app.settings.mcp_url.is_some() {
        // Discovery errors stay on this source and never stop the application.
        let _ = state.sources.refresh(&app.id).await;
    } else {
        let _ = state.sources.remove_observation(&app.id);
    }
    if let Err(error) = state.sources.publish().await {
        tracing::warn!(app_id = %app.id, %error, "Application MCP catalog refresh failed");
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentRequest { node_path: Option<String> }
async fn environment(Json(request): Json<EnvironmentRequest>) -> Result<Json<ApiResponse<nomifun_app_hosting::EnvironmentReport>>, AppError> {
    Ok(Json(ApiResponse::ok(nomifun_app_hosting::detect_environment(request.node_path.as_deref()).await)))
}
