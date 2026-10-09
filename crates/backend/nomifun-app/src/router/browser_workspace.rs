//! Authenticated user access to one managed browser per canonical Session.
//! This surface never creates Agent grants or resource bindings. Agent tools
//! independently validate frozen authority in the Browser Role owner.

use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use nomifun_agent_contracts::{
    AgentSessionId, PrincipalRef,
};
use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_browser_platform::{
    run_guard::{BrowserInputState, BrowserRunSnapshot, RunAdmissionError},
    runtime::{BrowserTabCommand, WorkspaceError, BrowserProfileStore, BrowserProfilePersistence},
    workspace::{BrowserResourceService, BrowserWorkspace, BrowserUserSnapshot, managed_workspace_key},
};
use nomifun_common::AppError;
use nomifun_conversation::CanonicalAgentSessionOwner;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub(crate) struct BrowserResourceApiState {
    pub resources: Option<Arc<BrowserResourceService>>,
    pub attached_chrome: Option<Arc<crate::AttachedChromeProviderService>>,
    pub sessions: CanonicalAgentSessionOwner,
    pub data_dir: PathBuf,
    pub operation_locks: Arc<dashmap::DashMap<String, Arc<tokio::sync::RwLock<()>>>>,
    pub close_owner: Arc<dyn crate::browser_workspace_provider::BrowserUserClosePort>,
}

pub(crate) fn routes(state: BrowserResourceApiState) -> Router {
    Router::new()
        .route(
            "/api/agent-sessions/{agent_session_id}/browser",
            get(snapshot).post(ensure).delete(close),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/browser/commands",
            post(command),
        )
        .route(
            "/api/browser-providers/attached-chrome",
            get(attached_snapshot)
                .post(connect_attached)
                .delete(disconnect_attached),
        )
        .with_state(state)
}

struct BrowserApiError(WorkspaceError);

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseRequest {
    runtime_generation: u64,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DisconnectAttachedRequest {
    incarnation: String,
}

struct AttachedProviderApiError(
    crate::browser_workspace_provider::attached_provider::AttachedProviderError,
);

impl IntoResponse for AttachedProviderApiError {
    fn into_response(self) -> Response {
        use crate::browser_workspace_provider::attached_provider::AttachedProviderError;
        let status = match self.0 {
            AttachedProviderError::OwnerMismatch => StatusCode::FORBIDDEN,
            AttachedProviderError::NotConnected => StatusCode::NOT_FOUND,
            AttachedProviderError::Closed => StatusCode::SERVICE_UNAVAILABLE,
            AttachedProviderError::Connection(_) => StatusCode::BAD_GATEWAY,
            AttachedProviderError::Busy
            | AttachedProviderError::StaleIncarnation
            | AttachedProviderError::CleanupFailed => StatusCode::CONFLICT,
        };
        (
            status,
            Json(serde_json::json!({
                "success": false,
                "code": "BROWSER_PROVIDER_ERROR",
                "error": self.0.to_string(),
            })),
        )
            .into_response()
    }
}

impl From<WorkspaceError> for BrowserApiError {
    fn from(error: WorkspaceError) -> Self {
        Self(error)
    }
}

impl IntoResponse for BrowserApiError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            WorkspaceError::NativeUnavailable => StatusCode::NOT_IMPLEMENTED,
            WorkspaceError::WorkspaceClosed | WorkspaceError::TabNotFound => {
                StatusCode::NOT_FOUND
            }
            WorkspaceError::ActionDenied => StatusCode::FORBIDDEN,
            WorkspaceError::InvalidUrl | WorkspaceError::InvalidZoom => StatusCode::BAD_REQUEST,
            WorkspaceError::NativeInitializationFailed => StatusCode::SERVICE_UNAVAILABLE,
            WorkspaceError::NativeCommandFailed => StatusCode::BAD_GATEWAY,
            WorkspaceError::Admission(
                RunAdmissionError::InputGateFailed | RunAdmissionError::WorkerFailed,
            ) => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::CONFLICT,
        };
        (
            status,
            Json(serde_json::json!({
                "success": false,
                "code": self.0.code(),
                "error": self.0.to_string(),
            })),
        )
            .into_response()
    }
}

impl BrowserResourceApiState {
    fn require_service(&self) -> Result<&Arc<BrowserResourceService>, BrowserApiError> {
        self.resources.as_ref().ok_or(BrowserApiError(WorkspaceError::NativeUnavailable))
    }

    /// User ownership and an existing canonical Session authorize the side
    /// browser. Agent grants and bindings are deliberately not consulted here.
    async fn require_user_session(&self, user: &CurrentUser, agent_session_id: &str) -> Result<nomifun_agent_session::SessionObservation, Response> {
        let principal = PrincipalRef { principal_kind: "user".to_owned(), principal_id: user.id.to_string() };
        self.sessions.get(&principal, &AgentSessionId::from(agent_session_id.to_owned()))
            .await.map_err(IntoResponse::into_response)
    }

    async fn user_workspace(&self, user: &CurrentUser, agent_session_id: &str, allow_running: bool) -> Result<Arc<BrowserWorkspace>, Response> {
        let observation = self.require_user_session(user, agent_session_id).await?;
        if observation.head.status == "running" || observation.head.active_turn_id.is_some() {
            // During canonical preparation, a native input gate may still be
            // joining. Never create/reveal a user-ready child in that window.
            let existing = self.require_service().map_err(IntoResponse::into_response)?
                .get_for_agent_session(&user.id.to_string(), agent_session_id).await
                .map_err(|error| BrowserApiError(error).into_response())?;
            if !allow_running || match existing { Some(workspace) => !workspace.has_active_run().await, None => true } {
                return Err(BrowserApiError(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)).into_response());
            }
        }
        let key = managed_workspace_key(&user.id.to_string(), agent_session_id)
            .map_err(|error| BrowserApiError(error).into_response())?;
        let profile = BrowserProfileStore::new(self.data_dir.clone())
            .and_then(|store| store.profile_for(&key, BrowserProfilePersistence::Persistent))
            .map_err(|error| BrowserApiError(error).into_response())?;
        self.require_service().map_err(IntoResponse::into_response)?
            .ensure_user(&user.id.to_string(), agent_session_id, profile)
            .await.map_err(|error| BrowserApiError(error).into_response())
    }
}

async fn close(
    State(state): State<BrowserResourceApiState>,
    Extension(user): Extension<CurrentUser>,
    Path(agent_session_id): Path<String>,
    Json(request): Json<CloseRequest>,
) -> Result<Json<ApiResponse<()>>, Response> {
    state.require_user_session(&user, &agent_session_id).await?;
    let operation_lock = state.operation_locks.entry(agent_session_id.clone())
        .or_insert_with(|| Arc::new(tokio::sync::RwLock::new(()))).clone();
    let fence = operation_lock.write_owned().await;
    tokio::spawn(async move {
        let _fence = fence;
        let observation = state.require_user_session(&user, &agent_session_id).await?;
        if observation.head.status == "running" || observation.head.active_turn_id.is_some() {
            return Err(BrowserApiError(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)).into_response());
        }
        state.close_owner.close_user_workspace(&user.id.to_string(), &agent_session_id, request.runtime_generation)
            .await.map_err(|error| BrowserApiError(error).into_response())?;
        Ok(Json(ApiResponse::ok(())))
    }).await.map_err(|_| BrowserApiError(WorkspaceError::Admission(RunAdmissionError::WorkerFailed)).into_response())?
}

async fn snapshot(
    State(state): State<BrowserResourceApiState>,
    Extension(user): Extension<CurrentUser>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<BrowserUserSnapshot>>, Response> {
    let observation = state.require_user_session(&user, &agent_session_id).await?;
    let workspace = state.require_service().map_err(IntoResponse::into_response)?
        .get_for_agent_session(&user.id.to_string(), &agent_session_id).await
        .map_err(|error| BrowserApiError(error).into_response())?;
    let mut snapshot = match workspace {
        Some(workspace) => workspace.snapshot().await.map_err(|error| BrowserApiError(error).into_response())?,
        None => BrowserUserSnapshot {
            interaction_capabilities: None,
            agent_session_id, browser_id: "managed-browser".to_owned(),
            run: BrowserRunSnapshot { revision: 0, input_state: BrowserInputState::UserReady, input_gate_failed: false },
            runtime: None,
        },
    };
    if observation.head.status == "running" || observation.head.active_turn_id.is_some() {
        snapshot.run.input_state = BrowserInputState::AgentRunning;
    }
    Ok(Json(ApiResponse::ok(snapshot)))
}

async fn ensure(
    State(state): State<BrowserResourceApiState>,
    Extension(user): Extension<CurrentUser>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<BrowserUserSnapshot>>, Response> {
    let workspace = state.user_workspace(&user, &agent_session_id, true).await?;
    Ok(Json(ApiResponse::ok(workspace.snapshot().await.map_err(|error| BrowserApiError(error).into_response())?)))
}

async fn command(
    State(state): State<BrowserResourceApiState>,
    Extension(user): Extension<CurrentUser>,
    Path(agent_session_id): Path<String>,
    Json(command): Json<BrowserTabCommand>,
) -> Result<Json<ApiResponse<BrowserUserSnapshot>>, Response> {
    let workspace = state.user_workspace(&user, &agent_session_id, false).await?;
    workspace.user_command(command).await.map_err(|error| BrowserApiError(error).into_response())?;
    Ok(Json(ApiResponse::ok(workspace.snapshot().await.map_err(|error| BrowserApiError(error).into_response())?)))
}

async fn attached_snapshot(
    State(state): State<BrowserResourceApiState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<
    Json<ApiResponse<Option<crate::browser_workspace_provider::attached_provider::AttachedProviderSnapshot>>>,
    Response,
> {
    let snapshot = match state.attached_chrome.as_ref() {
        Some(service) => service
            .snapshot(&user.id.to_string())
            .map_err(|error| AttachedProviderApiError(error).into_response())?,
        None => None,
    };
    Ok(Json(ApiResponse::ok(snapshot)))
}

async fn connect_attached(
    State(state): State<BrowserResourceApiState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<
    Json<ApiResponse<crate::browser_workspace_provider::attached_provider::AttachedProviderSnapshot>>,
    Response,
> {
    let service = state.attached_chrome.as_ref().ok_or_else(|| {
        AppError::ProviderUnavailable("The attached Chrome Provider is unavailable on this host.".into())
            .into_response()
    })?;
    let snapshot = service
        .connect(&user.id.to_string())
        .await
        .map_err(|error| AttachedProviderApiError(error).into_response())?;
    Ok(Json(ApiResponse::ok(snapshot)))
}

async fn disconnect_attached(
    State(state): State<BrowserResourceApiState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<DisconnectAttachedRequest>,
) -> Result<Json<ApiResponse<()>>, Response> {
    let service = state.attached_chrome.as_ref().ok_or_else(|| {
        AppError::ProviderUnavailable("The attached Chrome Provider is unavailable on this host.".into())
            .into_response()
    })?;
    service
        .disconnect(&user.id.to_string(), &request.incarnation)
        .await
        .map_err(|error| AttachedProviderApiError(error).into_response())?;
    Ok(Json(ApiResponse::ok(())))
}
