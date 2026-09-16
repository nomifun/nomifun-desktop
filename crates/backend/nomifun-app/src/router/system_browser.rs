//! Local-user connection management, separate from native Browser Workspace.
//! No route is an Agent tool, accepts protocol IDs, or opens a browser by GET.

use crate::system_browser::{SystemBrowserError, SystemBrowserService, SystemBrowserSnapshot};
use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    routing::{get, post},
};
use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use nomifun_conversation::ConversationService;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct SystemBrowserApiState {
    pub service: Option<Arc<SystemBrowserService>>,
    pub conversations: ConversationService,
}

pub(crate) fn routes(state: SystemBrowserApiState) -> Router {
    Router::new()
        .route(
            "/api/conversations/{conversation_id}/system-browser",
            get(snapshot).post(connect).delete(disconnect),
        )
        .route(
            "/api/conversations/{conversation_id}/system-browser/choices",
            post(choices),
        )
        .route(
            "/api/conversations/{conversation_id}/system-browser/tabs",
            post(grant),
        )
        .with_state(state)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectRequest {
    expected_incarnation: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionRequest {
    incarnation: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GrantRequest {
    incarnation: String,
    choice_id: String,
}

struct CancelRequestOnDrop(tokio_util::sync::CancellationToken);
impl Drop for CancelRequestOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

async fn while_request_open<T>(
    cancel: tokio_util::sync::CancellationToken,
    work: impl std::future::Future<Output = Result<T, AppError>>,
) -> Result<T, AppError> {
    tokio::select! { biased;
        _ = cancel.cancelled() => Err(AppError::Conflict("System browser request was cancelled".into())),
        result = work => result,
    }
}

impl SystemBrowserApiState {
    async fn owned(&self, user: &CurrentUser, id: &str) -> Result<(), AppError> {
        let conversation = self.conversations.get(user.id.as_str(), id).await?;
        if conversation.execution_step_id.is_some() {
            return Err(AppError::Forbidden(
                "Delegated tasks cannot connect a personal browser".into(),
            ));
        }
        Ok(())
    }

    fn service(&self) -> Result<Arc<SystemBrowserService>, AppError> {
        self.service.clone().ok_or_else(|| {
            AppError::ProviderUnavailable(
                "System browser connections are unavailable on this host".into(),
            )
        })
    }
}

fn system_error(error: SystemBrowserError) -> AppError {
    use SystemBrowserError::*;
    match error {
        Closed | NotConnected | Connection(_) => AppError::ProviderUnavailable(error.to_string()),
        CleanupFailed => {
            AppError::Internal("System browser disconnect is not yet confirmed".into())
        }
        Capacity => AppError::BadRequest(error.to_string()),
        StaleIncarnation | Busy | Cancelled => AppError::Conflict(error.to_string()),
    }
}

fn validate_id(id: &str) -> Result<(), AppError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(AppError::BadRequest(
            "Invalid system browser reference".into(),
        ));
    }
    Ok(())
}

fn require_incarnation(
    service: &SystemBrowserService,
    user: &str,
    id: &str,
    expected: Option<&str>,
) -> Result<(), AppError> {
    match (service.snapshot(user, id), expected) {
        (None, None) => Ok(()),
        (Some(snapshot), Some(expected)) if snapshot.incarnation == expected => Ok(()),
        _ => Err(AppError::Conflict(
            "The system browser connection changed; refresh its state".into(),
        )),
    }
}

async fn snapshot(
    State(state): State<SystemBrowserApiState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Option<SystemBrowserSnapshot>>>, AppError> {
    state.owned(&user, &id).await?;
    Ok(Json(ApiResponse::ok(
        state.service()?.snapshot(user.id.as_str(), &id),
    )))
}

async fn connect(
    State(state): State<SystemBrowserApiState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(request): Json<ConnectRequest>,
) -> Result<Json<ApiResponse<SystemBrowserSnapshot>>, AppError> {
    state.owned(&user, &id).await?;
    if let Some(expected) = &request.expected_incarnation {
        validate_id(expected)?;
    }
    let service = state.service()?;
    let (check_service, check_user, check_id, expected) = (
        service.clone(),
        user.id.to_string(),
        id.clone(),
        request.expected_incarnation.clone(),
    );
    let (operation_user, operation_id) = (user.id.to_string(), id.clone());
    let cancel = tokio_util::sync::CancellationToken::new();
    let _request = CancelRequestOnDrop(cancel.clone());
    let check_cancel = cancel.clone();
    let result = state
        .conversations
        .with_idle_runtime_reconfiguration(
            user.id.as_str(),
            &id,
            move || async move {
                if check_cancel.is_cancelled() {
                    return Err(AppError::Conflict(
                        "System browser request was cancelled".into(),
                    ));
                }
                require_incarnation(&check_service, &check_user, &check_id, expected.as_deref())
            },
            move || async move {
                while_request_open(cancel, async {
                    service
                        .connect(
                            &operation_user,
                            &operation_id,
                            request.expected_incarnation.as_deref(),
                        )
                        .await
                        .map_err(system_error)
                })
                .await
            },
        )
        .await?;
    Ok(Json(ApiResponse::ok(result)))
}

async fn choices(
    State(state): State<SystemBrowserApiState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(request): Json<ConnectionRequest>,
) -> Result<Json<ApiResponse<nomi_browser_engine::attached_browser::UserTabInventory>>, AppError> {
    state.owned(&user, &id).await?;
    validate_id(&request.incarnation)?;
    let result = state
        .service()?
        .choices(user.id.as_str(), &id, &request.incarnation)
        .await
        .map_err(system_error)?;
    Ok(Json(ApiResponse::ok(result)))
}

async fn grant(
    State(state): State<SystemBrowserApiState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(request): Json<GrantRequest>,
) -> Result<Json<ApiResponse<SystemBrowserSnapshot>>, AppError> {
    state.owned(&user, &id).await?;
    validate_id(&request.incarnation)?;
    validate_id(&request.choice_id)?;
    let service = state.service()?;
    let (check_service, check_user, check_id, expected) = (
        service.clone(),
        user.id.to_string(),
        id.clone(),
        request.incarnation.clone(),
    );
    let (operation_user, operation_id) = (user.id.to_string(), id.clone());
    let cancel = tokio_util::sync::CancellationToken::new();
    let _request = CancelRequestOnDrop(cancel.clone());
    let check_cancel = cancel.clone();
    let result = state
        .conversations
        .with_idle_runtime_reconfiguration(
            user.id.as_str(),
            &id,
            move || async move {
                if check_cancel.is_cancelled() {
                    return Err(AppError::Conflict(
                        "System browser request was cancelled".into(),
                    ));
                }
                require_incarnation(&check_service, &check_user, &check_id, Some(&expected))
            },
            move || async move {
                while_request_open(cancel, async {
                    service
                        .grant(
                            &operation_user,
                            &operation_id,
                            &request.incarnation,
                            &request.choice_id,
                        )
                        .await
                        .map_err(system_error)
                })
                .await
            },
        )
        .await?;
    Ok(Json(ApiResponse::ok(result)))
}

async fn disconnect(
    State(state): State<SystemBrowserApiState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(request): Json<ConnectionRequest>,
) -> Result<Json<ApiResponse<SystemBrowserSnapshot>>, AppError> {
    state.owned(&user, &id).await?;
    validate_id(&request.incarnation)?;
    let service = state.service()?;
    service
        .cancel_pending_connect(user.id.as_str(), &id, &request.incarnation)
        .map_err(system_error)?;
    let (check_service, check_user, check_id, expected) = (
        service.clone(),
        user.id.to_string(),
        id.clone(),
        request.incarnation.clone(),
    );
    let (operation_user, operation_id) = (user.id.to_string(), id.clone());
    let result = state
        .conversations
        .with_idle_runtime_reconfiguration(
            user.id.as_str(),
            &id,
            move || async move {
                require_incarnation(&check_service, &check_user, &check_id, Some(&expected))
            },
            move || async move {
                service
                    .disconnect(&operation_user, &operation_id, &request.incarnation)
                    .await
                    .map_err(system_error)
            },
        )
        .await?;
    Ok(Json(ApiResponse::ok(result)))
}
