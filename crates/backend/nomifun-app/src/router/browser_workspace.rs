//! Native browser application API. Identity and workspace come from ConversationService.
//! No endpoint can create a BrowserRunGuard or unlock Agent-owned input.

use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_browser_platform::{
    run_guard::{BrowserInputState, BrowserRunSnapshot, RunAdmissionError},
    runtime::{
        BrowserProfile, BrowserTabCommand, BrowserWorkspaceKey,
        WorkspaceError,
    },
    workspace::{BrowserWorkspace, BrowserWorkspaceService, BrowserWorkspaceSnapshot},
};
use nomifun_common::AppError;
use nomifun_conversation::ConversationService;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub(crate) struct BrowserWorkspaceApiState {
    pub workspaces: Option<Arc<BrowserWorkspaceService>>,
    pub conversations: ConversationService,
    pub data_dir: PathBuf,
}

pub(crate) fn routes(state: BrowserWorkspaceApiState) -> Router {
    Router::new()
        .route(
            "/api/conversations/{conversation_id}/browser",
            get(snapshot).post(ensure).delete(close),
        )
        .route(
            "/api/conversations/{conversation_id}/browser/commands",
            post(command),
        )
        .with_state(state)
}

struct BrowserApiError(WorkspaceError);

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseRequest { runtime_generation:u64 }

async fn close(
    State(state):State<BrowserWorkspaceApiState>,Extension(user):Extension<CurrentUser>,Path(id):Path<String>,
    Json(request):Json<CloseRequest>,
)->Result<Json<ApiResponse<()>>,Response> {
    state.owned_conversation(&user,&id).await.map_err(IntoResponse::into_response)?;
    let service=state.require_service().map_err(IntoResponse::into_response)?.clone();
    let key=BrowserWorkspaceKey {user_id:user.id.to_string(),conversation_id:id.clone()};
    let check_service=service.clone();
    let check_key=key.clone();
    state.conversations.with_idle_runtime_reconfiguration(user.id.as_str(),&id,
        move ||async move {
            // A lost HTTP acknowledgement can be retried after the old browser
            // is gone. Still retire an idle cached Agent holding that old Arc.
            let Some(workspace)=check_service.get(&check_key).await else {return Ok(());};
            if workspace.runtime_generation()!=request.runtime_generation {return Err(AppError::Conflict("The browser has changed; refresh its state before closing".into()));}
            if workspace.has_active_run().await {return Err(AppError::Conflict("Stop the Agent before rebuilding its browser".into()));}
            Ok(())
        },
        move ||async move {service.close_idle(key,request.runtime_generation).await.map_err(|error|AppError::Conflict(error.to_string()))},
    ).await.map_err(IntoResponse::into_response)?;
    Ok(Json(ApiResponse::ok(())))
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
            WorkspaceError::WorkspaceClosed | WorkspaceError::TabNotFound => StatusCode::NOT_FOUND,
            WorkspaceError::InvalidUrl => StatusCode::BAD_REQUEST,
            WorkspaceError::NativeCommandFailed => StatusCode::BAD_GATEWAY,
            WorkspaceError::Admission(
                RunAdmissionError::InputGateFailed | RunAdmissionError::WorkerFailed,
            ) => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::CONFLICT,
        };
        (status, Json(serde_json::json!({"success":false,"code":self.0.code(),"error":self.0.to_string()}))).into_response()
    }
}

impl BrowserWorkspaceApiState {
    fn require_service(&self) -> Result<&Arc<BrowserWorkspaceService>, BrowserApiError> {
        self.workspaces
            .as_ref()
            .ok_or(BrowserApiError(WorkspaceError::NativeUnavailable))
    }
    async fn owned_conversation(
        &self,
        user: &CurrentUser,
        id: &str,
    ) -> Result<nomifun_api_types::ConversationResponse, AppError> {
        let conversation = self.conversations.get(user.id.as_str(), id).await?;
        if conversation.execution_step_id.is_some() {
            return Err(AppError::Forbidden(
                "Delegated Agent tasks do not own an interactive browser.".into(),
            ));
        }
        Ok(conversation)
    }
    async fn ensure_workspace(
        &self,
        user: &CurrentUser,
        id: &str,
    ) -> Result<Arc<BrowserWorkspace>, Response> {
        let conversation = self
            .owned_conversation(user, id)
            .await
            .map_err(IntoResponse::into_response)?;
        let service = self
            .require_service()
            .map_err(IntoResponse::into_response)?;
        let workspace = conversation
            .extra
            .get("workspace")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let key = BrowserWorkspaceKey {
            user_id: user.id.to_string(),
            conversation_id: id.into(),
        };
        let temporary = conversation.extra.get("temp_workspace_id").is_some()
            || workspace.trim().is_empty();
        let profile = BrowserProfile::for_conversation(&self.data_dir, &key, temporary);
        service
            .ensure_user(key, profile)
            .await
            .map_err(|error| BrowserApiError(error).into_response())
    }
}

async fn snapshot(
    State(state): State<BrowserWorkspaceApiState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<BrowserWorkspaceSnapshot>>, Response> {
    state
        .owned_conversation(&user, &id)
        .await
        .map_err(IntoResponse::into_response)?;
    let service = state
        .require_service()
        .map_err(IntoResponse::into_response)?;
    let key = BrowserWorkspaceKey {
        user_id: user.id.to_string(),
        conversation_id: id.clone(),
    };
    let snapshot = match service.get(&key).await {
        Some(workspace) => workspace
            .snapshot()
            .await
            .map_err(|error| BrowserApiError(error).into_response())?,
        None => BrowserWorkspaceSnapshot {
            conversation_id: id,
            run: BrowserRunSnapshot {
                revision: 0,
                input_state: BrowserInputState::UserReady,
                input_gate_failed: false,
            },
            runtime: None,
        },
    };
    Ok(Json(ApiResponse::ok(snapshot)))
}

async fn ensure(
    State(state): State<BrowserWorkspaceApiState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<BrowserWorkspaceSnapshot>>, Response> {
    let workspace = state.ensure_workspace(&user, &id).await?;
    Ok(Json(ApiResponse::ok(workspace.snapshot().await.map_err(
        |error| BrowserApiError(error).into_response(),
    )?)))
}

async fn command(
    State(state): State<BrowserWorkspaceApiState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(command): Json<BrowserTabCommand>,
) -> Result<Json<ApiResponse<BrowserWorkspaceSnapshot>>, Response> {
    let workspace = state.ensure_workspace(&user, &id).await?;
    workspace
        .user_command(command)
        .await
        .map_err(|error| BrowserApiError(error).into_response())?;
    Ok(Json(ApiResponse::ok(workspace.snapshot().await.map_err(
        |error| BrowserApiError(error).into_response(),
    )?)))
}
