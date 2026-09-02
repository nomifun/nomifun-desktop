//! Canonical Agent control-plane and AgentSession HTTP composition.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{Next, from_fn};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use nomifun_agent_contracts::{
    AgentSessionId, AgentSessionMetadata, ArtifactId, DeleteAgentSessionCommand,
    EventProducerId, IdempotencyKey, OperationId, PrincipalRef, SessionPayloadBody,
    StrictJsonValue,
};
use nomifun_agent_control_plane::{
    AuthenticatedOwner, control_plane_router,
};
use nomifun_agent_platform::{
    AgentPlatform, AgentPlatformError, AgentSessionCommandPort, AgentSessionDeletePort,
    AgentSessionQueryPort, OpenAgentSessionRequest, StartAgentTurnRequest,
};
use nomifun_agent_session::ForkRequest;
use nomifun_api_types::{
    ApiResponse, CreateAgentSessionRequestDto, CreateAgentSessionResponseDto,
    CreateAgentSessionTurnRequestDto, CreateAgentSessionTurnResponseDto,
    ErrorResponse, ForkAgentSessionRequestDto, ForkAgentSessionResponseDto,
    SessionCursorDto,
};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

#[derive(Clone)]
struct AgentPlatformRouterState {
    platform: Arc<AgentPlatform>,
}

impl AgentPlatformRouterState {
    fn new(platform: Arc<AgentPlatform>) -> Self {
        Self { platform }
    }
}

#[derive(Debug, Deserialize)]
struct SessionEventsQuery {
    #[serde(default)]
    after_seq: u64,
    #[serde(default = "default_event_limit")]
    limit: u32,
}

#[derive(Serialize)]
struct AgentSessionCapabilityStateResponse {
    resolved_snapshot_ref: nomifun_agent_contracts::ResolvedSnapshotRef,
    generation: u64,
    initial_capabilities: Vec<String>,
    on_demand_capabilities: Vec<String>,
    active_capabilities: Vec<String>,
    compact_on_demand_index: Vec<nomifun_agent_contracts::CompactOnDemandCapabilityEntry>,
}

#[derive(Serialize)]
struct AgentSessionEventPageResponse {
    agent_session_id: String,
    events: Vec<nomifun_agent_contracts::SessionEventRecord>,
    next_cursor: SessionCursorDto,
}

#[derive(Serialize)]
struct AgentSessionMessagePageResponse {
    agent_session_id: String,
    messages: Vec<nomifun_agent_session::MessageProjection>,
    next_cursor: SessionCursorDto,
}

#[derive(Serialize)]
struct AgentSessionDeleteResponse {
    agent_session_id: String,
    state: &'static str,
    deleted_at: i64,
}

fn default_event_limit() -> u32 {
    100
}

struct AgentPlatformHttpError(AgentPlatformError);

impl From<AgentPlatformError> for AgentPlatformHttpError {
    fn from(error: AgentPlatformError) -> Self {
        Self(error)
    }
}

impl IntoResponse for AgentPlatformHttpError {
    fn into_response(self) -> Response {
        let (status, code) = match &self.0 {
            AgentPlatformError::ControlPlane(error) => {
                return error.status().to_owned().into_response_with_error(
                    error.to_string(),
                    error.code().as_ref(),
                    error.details(),
                );
            }
            AgentPlatformError::Session(error) => {
                let code = error.code().unwrap_or("AGENT_SESSION_STORE_FAILED");
                let status = match code {
                    "SESSION_NOT_FOUND" => StatusCode::NOT_FOUND,
                    "SESSION_DELETED" => StatusCode::GONE,
                    "IDEMPOTENCY_CONFLICT" => StatusCode::CONFLICT,
                    "INVALID_SESSION_EVENT" | "INVALID_PAYLOAD" | "INVALID_SESSION"
                    | "CONTRACT_ERROR" => StatusCode::UNPROCESSABLE_ENTITY,
                    _ => StatusCode::INTERNAL_SERVER_ERROR,
                };
                (status, code)
            }
            AgentPlatformError::Kernel(_) | AgentPlatformError::Contract(_) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "AGENT_PLATFORM_CONTRACT_ERROR",
            ),
            AgentPlatformError::Runtime(_) | AgentPlatformError::Model(_) => (
                StatusCode::BAD_GATEWAY,
                "AGENT_PLATFORM_RUNTIME_FAILED",
            ),
            AgentPlatformError::Sqlite(_)
            | AgentPlatformError::Json(_)
            | AgentPlatformError::Digest(_)
            | AgentPlatformError::PluginState(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "AGENT_PLATFORM_INTERNAL",
            ),
        };
        (
            status,
            Json(ErrorResponse::new(self.0.to_string(), code)),
        )
            .into_response()
    }
}

trait StatusErrorResponse {
    fn into_response_with_error(
        self,
        message: String,
        code: &str,
        details: Option<serde_json::Value>,
    ) -> Response;
}

impl StatusErrorResponse for StatusCode {
    fn into_response_with_error(
        self,
        message: String,
        code: &str,
        details: Option<serde_json::Value>,
    ) -> Response {
        (
            self,
            Json(ErrorResponse::new_with_details(message, code, details)),
        )
            .into_response()
    }
}

async fn project_authenticated_owner(
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let current = request
        .extensions()
        .get::<CurrentUser>()
        .ok_or_else(|| AppError::Forbidden("Authentication required".into()))?;
    let owner = AuthenticatedOwner(current.id.as_str().to_owned().into());
    request.extensions_mut().insert(owner);
    Ok(next.run(request).await)
}

pub fn create_agent_platform_router(platform: Arc<AgentPlatform>) -> Router {
    agent_platform_routes(AgentPlatformRouterState::new(platform))
}

fn agent_platform_routes(state: AgentPlatformRouterState) -> Router {
    let control_plane = control_plane_router(state.platform.control_plane().clone());
    Router::new()
        .route("/api/agent-sessions", post(create_agent_session))
        .route(
            "/api/agent-sessions/{agent_session_id}",
            get(get_agent_session).delete(delete_agent_session),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/capabilities",
            get(agent_session_capabilities),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/turns",
            post(create_agent_session_turn),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/events",
            get(agent_session_events),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/messages",
            get(agent_session_messages),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/forks",
            post(fork_agent_session),
        )
        .with_state(state)
        .merge(control_plane)
        .route_layer(from_fn(project_authenticated_owner))
}

async fn get_agent_session(
    State(state): State<AgentPlatformRouterState>,
    axum::Extension(owner): axum::Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<nomifun_agent_session::SessionObservation>>, AgentPlatformHttpError>
{
    let agent_session_id = AgentSessionId(agent_session_id);
    let observation = state
        .platform
        .observe_session(
            &user_principal(&owner),
            &agent_session_id,
            None,
            nomifun_agent_session::MAX_EVENT_PAGE_SIZE,
        )
        .await?;
    Ok(Json(ApiResponse::ok(observation)))
}

async fn agent_session_capabilities(
    State(state): State<AgentPlatformRouterState>,
    axum::Extension(owner): axum::Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<AgentSessionCapabilityStateResponse>>, AgentPlatformHttpError> {
    let agent_session_id = AgentSessionId(agent_session_id);
    let catalog = state
        .platform
        .session_capability_catalog(&user_principal(&owner), &agent_session_id)
        .await?;
    Ok(Json(ApiResponse::ok(
        AgentSessionCapabilityStateResponse {
            resolved_snapshot_ref: catalog.resolved_snapshot_ref,
            generation: catalog.generation,
            initial_capabilities: catalog
                .initial_capabilities
                .into_iter()
                .map(|capability| capability.as_ref().to_owned())
                .collect(),
            on_demand_capabilities: catalog
                .on_demand_capabilities
                .into_iter()
                .map(|capability| capability.as_ref().to_owned())
                .collect(),
            active_capabilities: catalog
                .active_capabilities
                .into_iter()
                .map(|capability| capability.as_ref().to_owned())
                .collect(),
            compact_on_demand_index: catalog.compact_on_demand_index,
        },
    )))
}

async fn create_agent_session(
    State(state): State<AgentPlatformRouterState>,
    axum::Extension(owner): axum::Extension<AuthenticatedOwner>,
    Json(request): Json<CreateAgentSessionRequestDto>,
) -> Result<Json<ApiResponse<CreateAgentSessionResponseDto>>, AgentPlatformHttpError> {
    let binding = serde_json::from_value(
        serde_json::to_value(&request.agent_binding)
            .map_err(AgentPlatformError::from)?,
    )
    .map_err(AgentPlatformError::from)?;
    let idempotency_key = IdempotencyKey::from(format!(
        "agent-session-create:{}",
        Uuid::now_v7()
    ));
    let mut open = OpenAgentSessionRequest::user(&owner, binding, idempotency_key);
    open.metadata.title = request.title;
    let created = state.platform.open_session(open).await?;
    let principal = user_principal(&owner);
    let head = state
        .platform
        .session_head(&principal, &created.session.agent_session_id)
        .await?;
    let agent_binding = serde_json::from_value(
        serde_json::to_value(&created.session.agent_binding)
            .map_err(AgentPlatformError::from)?,
    )
    .map_err(AgentPlatformError::from)?;
    Ok(Json(ApiResponse::ok(CreateAgentSessionResponseDto {
        agent_session_id: created.session.agent_session_id.as_ref().to_owned(),
        agent_binding,
        state: head.status,
        cursor: session_cursor(&created.session.agent_session_id, head.last_seq),
    })))
}

async fn create_agent_session_turn(
    State(state): State<AgentPlatformRouterState>,
    axum::Extension(owner): axum::Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(request): Json<CreateAgentSessionTurnRequestDto>,
) -> Result<Json<ApiResponse<CreateAgentSessionTurnResponseDto>>, AgentPlatformHttpError> {
    let agent_session_id = AgentSessionId(agent_session_id);
    let dispatch = state
        .platform
        .start_turn(StartAgentTurnRequest {
            agent_session_id: agent_session_id.clone(),
            principal: user_principal(&owner),
            input: nomifun_agent_contracts::StrictJsonValue(request.input),
            idempotency_key: IdempotencyKey::from(request.idempotency_key),
        })
        .await?;
    let head = state
        .platform
        .session_head(&user_principal(&owner), &agent_session_id)
        .await?;
    Ok(Json(ApiResponse::ok(
        CreateAgentSessionTurnResponseDto {
            agent_session_id: dispatch.agent_session_id.as_ref().to_owned(),
            operation_id: dispatch.operation_id.as_ref().to_owned(),
            cursor: session_cursor(&dispatch.agent_session_id, head.last_seq),
            status: head.status,
        },
    )))
}

async fn agent_session_events(
    State(state): State<AgentPlatformRouterState>,
    axum::Extension(owner): axum::Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Query(query): Query<SessionEventsQuery>,
) -> Result<Json<ApiResponse<AgentSessionEventPageResponse>>, AgentPlatformHttpError> {
    let agent_session_id = AgentSessionId(agent_session_id);
    let after = (query.after_seq > 0).then(|| nomifun_agent_contracts::SessionEventCursor {
        agent_session_id: agent_session_id.clone(),
        seq: query.after_seq,
    });
    let page = state
        .platform
        .session_events(
            &user_principal(&owner),
            &agent_session_id,
            after.as_ref(),
            query.limit,
        )
        .await?;
    Ok(Json(ApiResponse::ok(AgentSessionEventPageResponse {
        agent_session_id: page.agent_session_id.as_ref().to_owned(),
        events: page.events,
        next_cursor: session_cursor(&page.next_cursor.agent_session_id, page.next_cursor.seq),
    })))
}

async fn agent_session_messages(
    State(state): State<AgentPlatformRouterState>,
    axum::Extension(owner): axum::Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Query(query): Query<SessionEventsQuery>,
) -> Result<Json<ApiResponse<AgentSessionMessagePageResponse>>, AgentPlatformHttpError> {
    let agent_session_id = AgentSessionId(agent_session_id);
    let after = (query.after_seq > 0).then(|| nomifun_agent_contracts::SessionEventCursor {
        agent_session_id: agent_session_id.clone(),
        seq: query.after_seq,
    });
    let observation = state
        .platform
        .observe_session(
            &user_principal(&owner),
            &agent_session_id,
            after.as_ref(),
            query.limit,
        )
        .await?;
    let mut messages = observation.messages;
    messages.truncate(query.limit.max(1) as usize);
    let next_seq = messages
        .last()
        .map_or(query.after_seq, |message| message.last_seq);
    Ok(Json(ApiResponse::ok(
        AgentSessionMessagePageResponse {
            agent_session_id: agent_session_id.as_ref().to_owned(),
            messages,
            next_cursor: session_cursor(&agent_session_id, next_seq),
        },
    )))
}

async fn fork_agent_session(
    State(state): State<AgentPlatformRouterState>,
    axum::Extension(owner): axum::Extension<AuthenticatedOwner>,
    Path(parent_session_id): Path<String>,
    Json(request): Json<ForkAgentSessionRequestDto>,
) -> Result<Json<ApiResponse<ForkAgentSessionResponseDto>>, AgentPlatformHttpError> {
    let parent_session_id = AgentSessionId(parent_session_id);
    let principal = user_principal(&owner);
    let parent = state
        .platform
        .observe_session(
            &principal,
            &parent_session_id,
            None,
            nomifun_agent_session::MAX_EVENT_PAGE_SIZE,
        )
        .await?;
    if request.parent_through_seq > parent.head.last_seq {
        return Err(AgentPlatformError::Contract(
            "fork parent_through_seq exceeds the committed Session cursor".to_owned(),
        )
        .into());
    }
    let child_binding = serde_json::from_value(
        serde_json::to_value(&request.target_agent_binding)
            .map_err(AgentPlatformError::from)?,
    )
    .map_err(AgentPlatformError::from)?;
    let child_compiled = state
        .platform
        .compile_saved_binding(
            &principal,
            &child_binding,
            "fork",
            "desktop",
            "owner",
        )
        .await?;
    let child_session_id = AgentSessionId::from(Uuid::now_v7().to_string());
    let fork = state
        .platform
        .fork_session(
            &parent_session_id,
            ForkRequest {
                child_session_id: child_session_id.clone(),
                child_owner_ref: principal,
                child_metadata: AgentSessionMetadata {
                    title: request.title,
                    archived: false,
                    pinned: false,
                },
                child_agent_binding: child_binding,
                parent_through_seq: request.parent_through_seq,
                created_at: now_ms(),
                producer_id: EventProducerId::from("session_api"),
                operation_id: OperationId::from(format!(
                    "agent-session-fork:{}",
                    Uuid::now_v7()
                )),
                idempotency_key: IdempotencyKey::from(format!(
                    "agent-session-fork:{}",
                    Uuid::now_v7()
                )),
                correlation_id: nomifun_agent_contracts::CorrelationId::from(format!(
                    "agent-session-fork:{}",
                    Uuid::now_v7()
                )),
                event_id: None,
                base_payload_id: ArtifactId::from(Uuid::now_v7().to_string()),
                base_body: SessionPayloadBody::Json(StrictJsonValue(json!({
                    "parent_agent_session_id": parent_session_id,
                    "parent_through_seq": request.parent_through_seq,
                    "messages": parent.messages
                        .into_iter()
                        .filter(|message| message.last_seq <= request.parent_through_seq)
                        .collect::<Vec<_>>()
                }))),
                base_media_type: "application/json".to_owned(),
                child_initial_active_capability_ids: child_compiled
                    .content()
                    .initial_capabilities
                    .iter()
                    .map(|capability| capability.capability.id.as_ref().to_owned())
                    .collect(),
            },
        )
        .await?;
    let child_binding = serde_json::from_value(
        serde_json::to_value(&fork.child_session.agent_binding)
            .map_err(AgentPlatformError::from)?,
    )
    .map_err(AgentPlatformError::from)?;
    Ok(Json(ApiResponse::ok(ForkAgentSessionResponseDto {
        parent_agent_session_id: parent_session_id.as_ref().to_owned(),
        child_agent_session_id: child_session_id.as_ref().to_owned(),
        child_agent_binding: child_binding,
        parent_through_seq: request.parent_through_seq,
        child_base_is_self_contained: fork.contract.child_base_is_self_contained,
        copies_full_transcript: fork.contract.copies_full_transcript,
        migrates_runtime_private_handles: fork.contract.migrates_runtime_private_handles,
        replays_tool_or_effect: fork.contract.replays_tool_or_effect,
    })))
}

async fn delete_agent_session(
    State(state): State<AgentPlatformRouterState>,
    axum::Extension(owner): axum::Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<AgentSessionDeleteResponse>>, AgentPlatformHttpError> {
    let agent_session_id = AgentSessionId(agent_session_id);
    let requested_at = now_ms();
    let deleted = state
        .platform
        .delete_session(
            DeleteAgentSessionCommand {
                operation_id: nomifun_agent_contracts::OperationId::from(format!(
                    "agent-session-delete:{}",
                    Uuid::now_v7()
                )),
                agent_session_id,
                owner_ref: user_principal(&owner),
                requested_at,
            },
            requested_at,
        )
        .await?;
    Ok(Json(ApiResponse::ok(AgentSessionDeleteResponse {
        agent_session_id: deleted.tombstone.agent_session_id.as_ref().to_owned(),
        state: "deleted",
        deleted_at: deleted.tombstone.deleted_at,
    })))
}

fn user_principal(owner: &AuthenticatedOwner) -> PrincipalRef {
    PrincipalRef {
        principal_kind: "user".to_owned(),
        principal_id: owner.as_ref().to_owned(),
    }
}

fn session_cursor(agent_session_id: &AgentSessionId, seq: u64) -> SessionCursorDto {
    SessionCursorDto {
        agent_session_id: agent_session_id.as_ref().to_owned(),
        seq,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
