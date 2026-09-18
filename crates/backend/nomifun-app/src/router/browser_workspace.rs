//! Canonical AgentSession Browser Resource API.
//!
//! A route can materialize a provider-backed resource only after reloading the
//! immutable AgentSession binding, exact Browser Action allowlist, exact Role
//! Provider, and typed Browser Resource binding. Resource existence is never
//! treated as authority, and delegated AgentSessions use this same path.

use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use nomifun_agent_contracts::{
    AgentSessionId, ExecutionRoleId, PrincipalRef, TypedResourceBinding,
};
use nomifun_agent_control_plane::AgentControlPlane;
use nomifun_api_types::{AgentBindingValueDto, ApiResponse};
use nomifun_auth::CurrentUser;
use nomifun_browser_platform::{
    bound_resource::BoundBrowserProviderResource,
    product::{
        BrowserProviderKind, BrowserSessionAuthority, BROWSER_MODULE_ID,
        BROWSER_RESOURCE_KIND,
    },
    run_guard::{BrowserInputState, BrowserRunSnapshot, RunAdmissionError},
    runtime::{BrowserTabCommand, WorkspaceError},
    workspace::{BrowserResourceService, BrowserResourceSnapshot},
};
use nomifun_common::AppError;
use nomifun_conversation::CanonicalAgentSessionOwner;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub(crate) struct BrowserResourceApiState {
    pub resources: Option<Arc<BrowserResourceService>>,
    pub attached_chrome: Option<Arc<crate::AttachedChromeProviderService>>,
    pub sessions: CanonicalAgentSessionOwner,
    pub control_plane: Arc<AgentControlPlane>,
    pub data_dir: PathBuf,
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
            WorkspaceError::InvalidUrl => StatusCode::BAD_REQUEST,
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
        self.resources
            .as_ref()
            .ok_or(BrowserApiError(WorkspaceError::NativeUnavailable))
    }

    async fn authority(
        &self,
        user: &CurrentUser,
        agent_session_id: &str,
    ) -> Result<(BrowserSessionAuthority, bool), Response> {
        let session_id = AgentSessionId::from(agent_session_id.to_owned());
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: user.id.to_string(),
        };
        let observation = self
            .sessions
            .get(&principal, &session_id)
            .await
            .map_err(IntoResponse::into_response)?;
        let active = self
            .sessions
            .active_capability_ids(&principal, &session_id)
            .await
            .map_err(IntoResponse::into_response)?;
        if !active.iter().any(|id| id == BROWSER_MODULE_ID) {
            return Err(AppError::Forbidden(
                "This AgentSession does not have an active Browser Module grant.".into(),
            )
            .into_response());
        }

        let binding_value = serde_json::to_value(&observation.session.agent_binding)
            .map_err(|error| {
                AppError::Internal(format!("serialize canonical Agent binding: {error}"))
                    .into_response()
            })?;
        let binding_dto: AgentBindingValueDto = serde_json::from_value(binding_value)
            .map_err(|error| {
                AppError::Internal(format!("project canonical Agent binding: {error}"))
                    .into_response()
            })?;
        let owner = nomifun_agent_contracts::UserId::from(user.id.to_string());
        let (_, _, snapshot) = self
            .control_plane
            .saved_binding_artifacts(&owner, &binding_dto)
            .await
            .map_err(IntoResponse::into_response)?;
        if snapshot.snapshot_ref != observation.session.agent_binding.resolved_snapshot_ref {
            return Err(AppError::Conflict(
                "Browser authority Snapshot differs from the canonical AgentSession binding."
                    .into(),
            )
            .into_response());
        }
        let capability = snapshot
            .content
            .enabled_capabilities
            .iter()
            .find(|capability| capability.capability.id.as_ref() == BROWSER_MODULE_ID)
            .ok_or_else(|| {
                AppError::Forbidden(
                    "The frozen AgentSession Snapshot has no Browser Module grant.".into(),
                )
                .into_response()
            })?;
        if capability.action_allowlist.is_empty() {
            return Err(AppError::Forbidden(
                "The frozen Browser Module grant has no Browser Actions.".into(),
            )
            .into_response());
        }
        let provider = snapshot
            .content
            .resolved_role_providers
            .get(&ExecutionRoleId::from(
                nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID,
            ))
            .map(|lock| lock.provider.clone())
            .ok_or_else(|| {
                AppError::UnprocessableEntity(
                    "The frozen Browser Module has no exact Provider lock.".into(),
                )
                .into_response()
            })?;
        let binding = exact_browser_binding(
            &observation.session.agent_binding.typed_resource_bindings,
        )?;
        let ephemeral = crate::browser_workspace_provider::browser_resource_ephemeral(
            &binding,
        )
        .map_err(IntoResponse::into_response)?;
        let provider = crate::browser_workspace_provider::provider_descriptor(
            &provider,
            &binding,
        )
        .map_err(IntoResponse::into_response)?;
        let resource = crate::browser_workspace_provider::browser_resource_binding(
            binding,
            provider,
        )
        .map_err(IntoResponse::into_response)?;
        let granted_actions = capability
            .action_allowlist
            .iter()
            .map(|action| {
                nomifun_browser_platform::product::BrowserCapabilityAction::parse(
                    action.as_ref(),
                )
                .ok_or_else(|| {
                    AppError::UnprocessableEntity(
                        "The frozen Browser Module contains a non-canonical Action ID."
                            .into(),
                    )
                    .into_response()
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let authority = BrowserSessionAuthority::new(
            user.id.to_string(),
            agent_session_id,
            granted_actions,
            resource,
        )
        .map_err(|error| {
            AppError::UnprocessableEntity(error.to_string()).into_response()
        })?;
        Ok((authority, ephemeral))
    }

    async fn bind_resource(
        &self,
        user: &CurrentUser,
        agent_session_id: &str,
    ) -> Result<BoundBrowserProviderResource, Response> {
        let (authority, ephemeral) = self.authority(user, agent_session_id).await?;
        crate::browser_workspace_provider::bind_authorized_resource(
            self.resources.clone(),
            self.attached_chrome.clone(),
            &self.data_dir,
            authority,
            ephemeral,
        )
            .await
            .map_err(IntoResponse::into_response)
    }
}

fn exact_browser_binding(
    bindings: &[TypedResourceBinding],
) -> Result<TypedResourceBinding, Response> {
    let mut matches = bindings
        .iter()
        .filter(|binding| binding.resource_kind.as_ref() == BROWSER_RESOURCE_KIND);
    let binding = matches.next().cloned().ok_or_else(|| {
        AppError::UnprocessableEntity(
            "The frozen AgentSession has no Browser Resource binding.".into(),
        )
        .into_response()
    })?;
    if matches.next().is_some() {
        return Err(AppError::UnprocessableEntity(
            "The frozen AgentSession has multiple Browser Resource bindings.".into(),
        )
        .into_response());
    }
    Ok(binding)
}

async fn close(
    State(state): State<BrowserResourceApiState>,
    Extension(user): Extension<CurrentUser>,
    Path(agent_session_id): Path<String>,
    Json(request): Json<CloseRequest>,
) -> Result<Json<ApiResponse<()>>, Response> {
    let (authority, _) = state.authority(&user, &agent_session_id).await?;
    match authority.resource().provider().kind() {
        BrowserProviderKind::Managed => {
            state
                .require_service()
                .map_err(IntoResponse::into_response)?
                .clone()
                .close_idle(authority.key(), request.runtime_generation)
                .await
                .map_err(|error| BrowserApiError(error).into_response())?;
        }
        BrowserProviderKind::AttachedChrome => {
            return Err(BrowserApiError(WorkspaceError::UnsupportedAction).into_response());
        }
    }
    Ok(Json(ApiResponse::ok(())))
}

async fn snapshot(
    State(state): State<BrowserResourceApiState>,
    Extension(user): Extension<CurrentUser>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<BrowserResourceSnapshot>>, Response> {
    let (authority, ephemeral) = state.authority(&user, &agent_session_id).await?;
    let snapshot = match authority.resource().provider().kind() {
        BrowserProviderKind::Managed => match state
            .require_service()
            .map_err(IntoResponse::into_response)?
            .get(&authority)
            .await
            .map_err(|error| BrowserApiError(error).into_response())?
        {
            Some(resource) => resource
                .snapshot()
                .await
                .map_err(|error| BrowserApiError(error).into_response())?,
            None => inactive_snapshot(&authority),
        },
        BrowserProviderKind::AttachedChrome => {
            crate::browser_workspace_provider::bind_authorized_resource(
                state.resources.clone(),
                state.attached_chrome.clone(),
                &state.data_dir,
                authority,
                ephemeral,
            )
            .await
            .map_err(IntoResponse::into_response)?
            .snapshot()
            .await
            .map_err(|error| BrowserApiError(error).into_response())?
        }
    };
    Ok(Json(ApiResponse::ok(snapshot)))
}

async fn ensure(
    State(state): State<BrowserResourceApiState>,
    Extension(user): Extension<CurrentUser>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<BrowserResourceSnapshot>>, Response> {
    let resource = state.bind_resource(&user, &agent_session_id).await?;
    Ok(Json(ApiResponse::ok(resource.snapshot().await.map_err(
        |error| BrowserApiError(error).into_response(),
    )?)))
}

async fn command(
    State(state): State<BrowserResourceApiState>,
    Extension(user): Extension<CurrentUser>,
    Path(agent_session_id): Path<String>,
    Json(command): Json<BrowserTabCommand>,
) -> Result<Json<ApiResponse<BrowserResourceSnapshot>>, Response> {
    let resource = state.bind_resource(&user, &agent_session_id).await?;
    let snapshot = resource
        .user_command(command)
        .await
        .map_err(|error| BrowserApiError(error).into_response())?;
    Ok(Json(ApiResponse::ok(snapshot)))
}

fn inactive_snapshot(authority: &BrowserSessionAuthority) -> BrowserResourceSnapshot {
    BrowserResourceSnapshot {
        agent_session_id: authority.agent_session_id().to_owned(),
        resource_binding_id: authority.resource().binding_id().to_owned(),
        provider_id: authority.resource().provider().provider_id().to_owned(),
        provider_kind: authority.resource().provider().kind(),
        allowed_actions: nomifun_browser_platform::product::BrowserCapabilityAction::all()
            .into_iter()
            .filter(|action| authority.authorize(*action).is_ok())
            .map(|action| action.action_id().to_owned())
            .collect(),
        run: BrowserRunSnapshot {
            revision: 0,
            input_state: BrowserInputState::UserReady,
            input_gate_failed: false,
        },
        runtime: None,
    }
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
