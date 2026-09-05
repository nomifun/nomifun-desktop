//! Canonical Remote REST ingress for the Fresh-v4 AgentSession chain.
//!
//! This adapter deliberately owns no Remote state. It authenticates the
//! installation owner, converts the four wire DTOs, and delegates all
//! persistence, ownership, idempotency, and runtime admission to the Remote
//! package's manifest-declared AgentSession command/query ports. The concrete
//! platform is retained only for RemoteBinding control-plane lookup.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use nomifun_agent_contracts::{
    AgentBindingValue, AgentSessionId, CorrelationId, EventProducerId, IdempotencyKey,
    OperationId, PrincipalRef, RemoteBindingId, RemoteBindingProvenance, SessionEventCursor,
    StrictJsonValue,
};
use nomifun_agent_control_plane::ControlPlaneError;
use nomifun_agent_platform::{
    AgentPlatform, AgentPlatformError, AgentSessionQueryPort,
    CanonicalAgentSessionCommandPort, OpenAgentSessionRequest, StartAgentTurnRequest,
};
use nomifun_api_types::{
    ErrorResponse, RemoteCancelRequestDto, RemoteMutationResponseDto, RemoteObserveResponseDto,
    RemoteOpenRequestDto, RemoteOpenResponseDto,
    RemoteOpenStateViewDto, RemoteTurnRequestDto, SessionCursorDto,
};
use nomifun_common::UserId;
use nomifun_public::{
    PublicMcpState, RemoteInstanceOwner, instance_token_middleware,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use super::remote_runtime::{
    RemoteDetachedMutationAdmissionError, RemoteDetachedMutationPermit,
    RemoteDetachedMutationRegistry, RemoteRuntimeCoordinator,
};

const REMOTE_OPERATION_TIMEOUT_CODE: &str = "REMOTE_OPERATION_TIMEOUT";
const REMOTE_REQUEST_TIMEOUT_CODE: &str = "REMOTE_REQUEST_TIMEOUT";
const REMOTE_OPERATION_BLOCKED_CODE: &str = "REMOTE_OPERATION_BLOCKED";
const REMOTE_PERSISTENCE_BLOCKED_CODE: &str = "REMOTE_SESSION_PERSISTENCE_BLOCKED";

// Each boundary has its own budget. The request timeout is only a final
// protection for body extraction/middleware and must not replace operation
// budgets below.
const REMOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
const REMOTE_OPEN_BINDING_TIMEOUT: Duration = Duration::from_secs(5);
const REMOTE_OPEN_SESSION_TIMEOUT: Duration = Duration::from_secs(30);
const REMOTE_OPEN_WORKFLOW_TIMEOUT: Duration = Duration::from_secs(60);
const REMOTE_OPEN_ADMISSION_TIMEOUT: Duration = Duration::from_secs(20);
const REMOTE_OPEN_HEAD_TIMEOUT: Duration = Duration::from_secs(5);
const REMOTE_TURN_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);
const REMOTE_TURN_ADMISSION_TIMEOUT: Duration = Duration::from_secs(20);
const REMOTE_TURN_DISPATCH_TIMEOUT: Duration = Duration::from_secs(150);
const REMOTE_TURN_HEAD_TIMEOUT: Duration = Duration::from_secs(5);
const REMOTE_OBSERVE_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);
const REMOTE_OBSERVE_ADMISSION_TIMEOUT: Duration = Duration::from_secs(20);
const REMOTE_OBSERVE_HEAD_TIMEOUT: Duration = Duration::from_secs(5);
const REMOTE_OBSERVE_PAGE_TIMEOUT: Duration = Duration::from_secs(15);
const REMOTE_CANCEL_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);
const REMOTE_CANCEL_HEAD_TIMEOUT: Duration = Duration::from_secs(5);
const REMOTE_CANCEL_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const REMOTE_CANCEL_FINAL_HEAD_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct RemoteRestState {
    platform: Arc<AgentPlatform>,
    session_command: Arc<dyn CanonicalAgentSessionCommandPort>,
    session_query: Arc<dyn AgentSessionQueryPort>,
    runtime: Arc<RemoteRuntimeCoordinator>,
    detached_mutations: RemoteDetachedMutationRegistry,
}

#[derive(Debug, Deserialize)]
struct RemoteObserveQuery {
    agent_session_id: String,
    #[serde(default)]
    after_seq: u64,
    #[serde(default = "default_observe_limit")]
    limit: u32,
}

#[derive(Debug)]
struct RemoteHttpError {
    status: StatusCode,
    code: String,
    message: String,
    details: Option<Value>,
}

#[derive(Debug)]
enum DetachedCallFailure<E> {
    Failed(E),
    TimedOut(RemoteHttpError),
    Panicked,
    Admission(RemoteDetachedMutationAdmissionError),
}

#[derive(Debug)]
enum RemoteOpenWorkflowFailure {
    Session(AgentPlatformError),
    SessionTimedOut,
    SessionPanicked,
}

impl RemoteHttpError {
    fn canonical(
        code: impl Into<String>,
        status: StatusCode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    fn canonical_with_details(
        code: impl Into<String>,
        status: StatusCode,
        message: impl Into<String>,
        details: Value,
    ) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            details: Some(details),
        }
    }
}

impl IntoResponse for RemoteHttpError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse::new_with_details(
                self.message,
                self.code,
                self.details,
            )),
        )
            .into_response()
    }
}

impl From<ControlPlaneError> for RemoteHttpError {
    fn from(error: ControlPlaneError) -> Self {
        Self {
            status: error.status(),
            code: error.code().as_ref().to_owned(),
            message: error.to_string(),
            details: error.details(),
        }
    }
}

impl From<serde_json::Error> for RemoteHttpError {
    fn from(error: serde_json::Error) -> Self {
        Self::canonical(
            "REMOTE_INVALID_REQUEST",
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Remote wire conversion failed: {error}"),
        )
    }
}

impl From<AgentPlatformError> for RemoteHttpError {
    fn from(error: AgentPlatformError) -> Self {
        match &error {
            AgentPlatformError::ControlPlane(error) => Self {
                status: error.status(),
                code: error.code().as_ref().to_owned(),
                message: error.to_string(),
                details: error.details(),
            },
            AgentPlatformError::Session(error) => {
                let code = error.code().unwrap_or("REMOTE_SESSION_NOT_FOUND");
                let (status, code) = match code {
                    "SESSION_DELETED" => (StatusCode::GONE, "SESSION_DELETED"),
                    "IDEMPOTENCY_CONFLICT" => {
                        (StatusCode::CONFLICT, "REMOTE_IDEMPOTENCY_CONFLICT")
                    }
                    "SESSION_NOT_FOUND" => {
                        (StatusCode::NOT_FOUND, "REMOTE_SESSION_NOT_FOUND")
                    }
                    "INVALID_SESSION_EVENT" | "INVALID_PAYLOAD" | "INVALID_SESSION" => {
                        (StatusCode::UNPROCESSABLE_ENTITY, "REMOTE_OPEN_FAILED")
                    }
                    _ => (StatusCode::CONFLICT, "REMOTE_SESSION_BUSY"),
                };
                Self::canonical(code, status, error.to_string())
            }
            AgentPlatformError::Contract(message) => {
                let lower = message.to_ascii_lowercase();
                if lower.contains("opening") {
                    Self::canonical(
                        "REMOTE_SESSION_OPENING",
                        StatusCode::CONFLICT,
                        message.clone(),
                    )
                } else if lower.contains("busy")
                    || lower.contains("completed-turn boundary")
                    || lower.contains("active turn")
                {
                    Self::canonical(
                        "REMOTE_SESSION_BUSY",
                        StatusCode::CONFLICT,
                        message.clone(),
                    )
                } else {
                    Self::canonical(
                        "REMOTE_OPEN_FAILED",
                        StatusCode::UNPROCESSABLE_ENTITY,
                        message.clone(),
                    )
                }
            }
            AgentPlatformError::Runtime(_) | AgentPlatformError::Model(_) => Self::canonical(
                "SNAPSHOT_EXECUTOR_UNAVAILABLE",
                StatusCode::BAD_GATEWAY,
                error.to_string(),
            ),
            AgentPlatformError::Kernel(_) => Self::canonical(
                "REMOTE_OPEN_FAILED",
                StatusCode::UNPROCESSABLE_ENTITY,
                error.to_string(),
            ),
            AgentPlatformError::Sqlite(_)
            | AgentPlatformError::Json(_)
            | AgentPlatformError::Digest(_)
            | AgentPlatformError::PluginState(_) => Self::canonical(
                "REMOTE_OPEN_FAILED",
                StatusCode::INTERNAL_SERVER_ERROR,
                error.to_string(),
            ),
        }
    }
}

/// Build the four canonical Remote REST operations.
pub fn build(
    platform: Arc<AgentPlatform>,
    session_command: Arc<dyn CanonicalAgentSessionCommandPort>,
    session_query: Arc<dyn AgentSessionQueryPort>,
    validator: Arc<nomifun_auth::InstanceTokenValidator>,
    authoritative_user_id: UserId,
    runtime: Arc<RemoteRuntimeCoordinator>,
) -> Router {
    let detached_mutations = runtime.detached_mutation_registry();
    let state = RemoteRestState {
        platform,
        session_command,
        session_query,
        runtime,
        detached_mutations,
    };
    Router::new()
        .route("/api/remote/open", post(open))
        .route("/api/remote/turn", post(turn))
        .route("/api/remote/observe", get(observe))
        .route("/api/remote/cancel", post(cancel))
        .with_state(state)
        .layer(from_fn(remote_request_deadline))
        .layer(from_fn(reject_undeclared_query_parameters))
        .layer(from_fn_with_state(
            PublicMcpState {
                validator,
                authoritative_user_id,
            },
            instance_token_middleware,
        ))
}

/// A final request-level guard for body extraction and any future middleware.
/// Individual Remote operations still use their own deadlines so a timeout
/// can identify whether a mutation's outcome is unknown.
async fn remote_request_deadline(request: Request, next: Next) -> Response {
    match tokio::time::timeout(REMOTE_REQUEST_TIMEOUT, next.run(request)).await {
        Ok(response) => response,
        Err(_) => RemoteHttpError::canonical_with_details(
            REMOTE_REQUEST_TIMEOUT_CODE,
            StatusCode::GATEWAY_TIMEOUT,
            format!(
                "Remote request exceeded its {} ms deadline",
                REMOTE_REQUEST_TIMEOUT.as_millis()
            ),
            serde_json::json!({
                "operation": "remote.request",
                "timeout_ms": REMOTE_REQUEST_TIMEOUT.as_millis() as u64,
                "recovery": "retry_same_request"
            }),
        )
        .into_response(),
    }
}

async fn reject_undeclared_query_parameters(request: Request, next: Next) -> Response {
    let Some(query) = request.uri().query() else {
        return next.run(request).await;
    };
    let allowed_observe = request.uri().path() == "/api/remote/observe";
    let invalid = url::form_urlencoded::parse(query.as_bytes()).any(|(key, _)| {
        !allowed_observe
            || !matches!(key.as_ref(), "agent_session_id" | "after_seq" | "limit")
    });
    if invalid {
        return RemoteHttpError::canonical(
            "REMOTE_INVALID_REQUEST",
            StatusCode::BAD_REQUEST,
            "Remote endpoints do not accept undeclared query parameters",
        )
        .into_response();
    }
    next.run(request).await
}

async fn open(
    State(state): State<RemoteRestState>,
    Extension(RemoteInstanceOwner(owner)): Extension<RemoteInstanceOwner>,
    Json(request): Json<RemoteOpenRequestDto>,
) -> Result<Json<RemoteOpenResponseDto>, RemoteHttpError> {
    let idempotency_key = validated_idempotency_key(&request.idempotency_key)?;
    let binding_id = nonempty(&request.binding_id, "binding_id")?;
    let initial_input = request
        .initial_input
        .map(|value| bounded_json(value, "initial_input"))
        .transpose()?;
    let binding = with_remote_deadline(
        "open.binding_lookup",
        REMOTE_OPEN_BINDING_TIMEOUT,
        "retry_same_request",
        None,
        async {
            state
                .platform
                .control_plane()
                .get_remote_binding(&contract_user_id(&owner), &binding_id)
                .await
                .map_err(RemoteHttpError::from)
        },
    )
    .await?
        .ok_or_else(|| {
            RemoteHttpError::canonical(
                "REMOTE_BINDING_NOT_FOUND",
                StatusCode::NOT_FOUND,
                "RemoteBinding does not exist for the authenticated owner",
            )
        })?;
    let agent_binding: AgentBindingValue = decode(&binding.agent_binding)?;
    let remote_binding_id = RemoteBindingId::from(binding.remote_binding_id.clone());
    let internal_key = IdempotencyKey::from(format!("remote-open:{idempotency_key}"));
    let mut open = OpenAgentSessionRequest::user(
        &contract_user_id(&owner),
        agent_binding.clone(),
        internal_key.clone(),
    );
    open.remote_binding_provenance = Some(RemoteBindingProvenance {
        remote_binding_id,
        binding_version: agent_binding.binding_version,
    });
    open.operation_id = OperationId::from(format!("remote-open:{idempotency_key}"));
    open.producer_id = EventProducerId::from(format!("remote_rest:{}", owner.as_ref()));
    open.correlation_id = CorrelationId::from(open.operation_id.as_ref().to_owned());
    open.scene = "remote".to_owned();
    open.surface = "remote".to_owned();
    open.audience = "owner".to_owned();
    open.initial_input = initial_input.map(StrictJsonValue);

    let workflow_permit = state
        .detached_mutations
        .try_admit(remote_mutation_key(
            "open.workflow",
            &owner,
            None,
            &idempotency_key,
        ))
        .map_err(|error| remote_detached_admission_error("open.workflow", None, error))?;
    let session_permit = workflow_permit.clone();
    let admission_parent_permit = workflow_permit.clone();
    let command = Arc::clone(&state.session_command);
    let runtime = Arc::clone(&state.runtime);
    let detached_mutations = state.detached_mutations.clone();
    let workflow = run_detached_with_permit(
        workflow_permit,
        "open.workflow",
        REMOTE_OPEN_WORKFLOW_TIMEOUT,
        "retry_same_idempotency_key_and_observe",
        None,
        async move {
            open_session_and_admit(
                command,
                runtime,
                detached_mutations,
                session_permit,
                admission_parent_permit,
                open,
            )
            .await
        },
    )
    .await;
    let (created, admission) = match workflow {
        Ok(result) => result,
        Err(DetachedCallFailure::Failed(RemoteOpenWorkflowFailure::Session(error))) => {
            return Err(error.into());
        }
        Err(DetachedCallFailure::Failed(RemoteOpenWorkflowFailure::SessionTimedOut)) => {
            return Err(remote_operation_timeout(
                "open.session_command",
                REMOTE_OPEN_SESSION_TIMEOUT,
                "retry_same_idempotency_key_and_observe",
                None,
            ));
        }
        Err(DetachedCallFailure::Failed(RemoteOpenWorkflowFailure::SessionPanicked)) => {
            return Err(remote_operation_blocked(
                "open.session_command",
                "the open command panicked before its durable outcome was known",
                None,
            ));
        }
        Err(DetachedCallFailure::Panicked) => {
            return Err(remote_operation_blocked(
                "open.workflow",
                "the open workflow panicked before its durable outcome was known",
                None,
            ));
        }
        Err(DetachedCallFailure::TimedOut(error)) => return Err(error),
        Err(DetachedCallFailure::Admission(error)) => {
            return Err(remote_detached_admission_error(
                "open.workflow",
                None,
                error,
            ));
        }
    };
    let session_id = created.session.agent_session_id.clone();
    let principal = user_principal(&owner);
    let (status, last_seq) = if created.duplicate || admission.is_err() {
        let head = read_session_head(
            state.session_query.as_ref(),
            &principal,
            &session_id,
            "open.session_head",
            REMOTE_OPEN_HEAD_TIMEOUT,
        )
        .await?;
        if let Err(error) = admission {
            if head.status == "opening" {
                return Err(runtime_admission_error(
                    &state.runtime,
                    &session_id,
                    &head,
                    error,
                ));
            }
        }
        (head.status, head.last_seq)
    } else {
        // The first response represents the committed local transaction. The
        // post-commit sidecar attempt is intentionally observed through the
        // Session cursor and must not be presented as a cross-boundary
        // atomic operation.
        ("opening".to_owned(), created.activation_ack.seq)
    };
    Ok(Json(RemoteOpenResponseDto {
        agent_session_id: session_id.as_ref().to_owned(),
        agent_binding: decode(&created.session.agent_binding)?,
        open_state: open_state(&status)?,
        cursor: cursor(&session_id, last_seq),
    }))
}

async fn turn(
    State(state): State<RemoteRestState>,
    Extension(RemoteInstanceOwner(owner)): Extension<RemoteInstanceOwner>,
    Json(request): Json<RemoteTurnRequestDto>,
) -> Result<Json<RemoteMutationResponseDto>, RemoteHttpError> {
    let idempotency_key = validated_idempotency_key(&request.idempotency_key)?;
    let session_id = parse_session_id(&request.agent_session_id)?;
    let input = bounded_json(request.input, "input")?;
    ensure_remote_session(
        state.session_query.as_ref(),
        &owner,
        &session_id,
        "turn.session_lookup",
        REMOTE_TURN_LOOKUP_TIMEOUT,
    )
    .await?;
    let principal = user_principal(&owner);
    let mut current_head = read_session_head(
        state.session_query.as_ref(),
        &principal,
        &session_id,
        "turn.session_head",
        REMOTE_TURN_HEAD_TIMEOUT,
    )
    .await?;
    if current_head.status == "opening" {
        let admission = detached_runtime_admission(
            &state.runtime,
            state.detached_mutations.clone(),
            session_id.clone(),
            REMOTE_TURN_ADMISSION_TIMEOUT,
            None,
        )
        .await;
        if let Err(error) = admission {
            current_head = read_session_head(
                state.session_query.as_ref(),
                &principal,
                &session_id,
                "turn.opening_recheck",
                REMOTE_TURN_HEAD_TIMEOUT,
            )
            .await?;
            if current_head.status == "opening" {
                return Err(runtime_admission_error(
                    &state.runtime,
                    &session_id,
                    &current_head,
                    error,
                ));
            }
        }
        current_head = read_session_head(
            state.session_query.as_ref(),
            &principal,
            &session_id,
            "turn.ready_recheck",
            REMOTE_TURN_HEAD_TIMEOUT,
        )
        .await?;
    }
    ensure_turn_ready(&session_id, &current_head)?;
    let turn_request = StartAgentTurnRequest {
        agent_session_id: session_id.clone(),
        principal: principal.clone(),
        input: StrictJsonValue(input),
        idempotency_key: IdempotencyKey::from(format!("remote-turn:{idempotency_key}")),
    };
    let command = Arc::clone(&state.session_command);
    let mutation_key = remote_mutation_key(
        "turn.dispatch",
        &owner,
        Some(&session_id),
        &idempotency_key,
    );
    let dispatch = match run_detached_with_deadline(
        state.detached_mutations.clone(),
        mutation_key,
        "turn.dispatch",
        REMOTE_TURN_DISPATCH_TIMEOUT,
        "retry_same_idempotency_key_and_observe",
        Some(&session_id),
        async move { command.start_turn(turn_request).await },
    )
    .await
    {
        Ok(dispatch) => dispatch,
        Err(DetachedCallFailure::Failed(error)) => return Err(error.into()),
        Err(DetachedCallFailure::TimedOut(error)) => return Err(error),
        Err(DetachedCallFailure::Panicked) => {
            return Err(remote_operation_blocked(
                "turn.dispatch",
                "the turn command panicked before its durable outcome was known",
                Some(&session_id),
            ));
        }
        Err(DetachedCallFailure::Admission(error)) => {
            return Err(remote_detached_admission_error(
                "turn.dispatch",
                Some(&session_id),
                error,
            ));
        }
    };
    let head = read_session_head(
        state.session_query.as_ref(),
        &principal,
        &session_id,
        "turn.final_head",
        REMOTE_TURN_HEAD_TIMEOUT,
    )
    .await?;
    Ok(Json(RemoteMutationResponseDto {
        agent_session_id: dispatch.agent_session_id.as_ref().to_owned(),
        cursor: cursor(&session_id, head.last_seq),
        session_status: head.status,
    }))
}

async fn observe(
    State(state): State<RemoteRestState>,
    Extension(RemoteInstanceOwner(owner)): Extension<RemoteInstanceOwner>,
    Query(request): Query<RemoteObserveQuery>,
) -> Result<Json<RemoteObserveResponseDto>, RemoteHttpError> {
    validate_observe_limit(request.limit)?;
    let session_id = parse_session_id(&request.agent_session_id)?;
    ensure_remote_session(
        state.session_query.as_ref(),
        &owner,
        &session_id,
        "observe.session_lookup",
        REMOTE_OBSERVE_LOOKUP_TIMEOUT,
    )
    .await?;
    let principal = user_principal(&owner);
    let current_head = read_session_head(
        state.session_query.as_ref(),
        &principal,
        &session_id,
        "observe.session_head",
        REMOTE_OBSERVE_HEAD_TIMEOUT,
    )
    .await?;
    if current_head.status == "opening" {
        let admission = detached_runtime_admission(
            &state.runtime,
            state.detached_mutations.clone(),
            session_id.clone(),
            REMOTE_OBSERVE_ADMISSION_TIMEOUT,
            None,
        )
        .await;
        if let Err(error) = admission {
            let latest_head = read_session_head(
                state.session_query.as_ref(),
                &principal,
                &session_id,
                "observe.opening_recheck",
                REMOTE_OBSERVE_HEAD_TIMEOUT,
            )
            .await?;
            if latest_head.status == "opening" {
                return Err(runtime_admission_error(
                    &state.runtime,
                    &session_id,
                    &latest_head,
                    error,
                ));
            }
        }
    }
    let after = SessionEventCursor {
        agent_session_id: session_id.clone(),
        seq: request.after_seq,
    };
    let observation = with_remote_deadline(
        "observe.page",
        REMOTE_OBSERVE_PAGE_TIMEOUT,
        "retry_same_session_and_cursor",
        Some(&session_id),
        async {
            state
                .session_query
                .observe_session(&principal, &session_id, Some(&after), request.limit)
                .await
                .map_err(RemoteHttpError::from)
        },
    )
    .await?;
    let events = observation
        .events
        .into_iter()
        .map(|event| serde_json::to_value(event).map_err(RemoteHttpError::from))
        .collect::<Result<Vec<_>, _>>()?;
    let messages = observation
        .messages
        .into_iter()
        .map(|message| message.projection)
        .collect();
    Ok(Json(RemoteObserveResponseDto {
        agent_session_id: session_id.as_ref().to_owned(),
        events,
        messages,
        next_cursor: cursor(&observation.next_cursor.agent_session_id, observation.next_cursor.seq),
    }))
}

async fn cancel(
    State(state): State<RemoteRestState>,
    Extension(RemoteInstanceOwner(owner)): Extension<RemoteInstanceOwner>,
    Json(request): Json<RemoteCancelRequestDto>,
) -> Result<Json<RemoteMutationResponseDto>, RemoteHttpError> {
    let idempotency_key = validated_idempotency_key(&request.idempotency_key)?;
    let session_id = parse_session_id(&request.agent_session_id)?;
    ensure_remote_session(
        state.session_query.as_ref(),
        &owner,
        &session_id,
        "cancel.session_lookup",
        REMOTE_CANCEL_LOOKUP_TIMEOUT,
    )
    .await?;
    let principal = user_principal(&owner);
    let current_head = read_session_head(
        state.session_query.as_ref(),
        &principal,
        &session_id,
        "cancel.session_head",
        REMOTE_CANCEL_HEAD_TIMEOUT,
    )
    .await?;
    ensure_cancel_allowed(&session_id, &current_head)?;

    let command = Arc::clone(&state.session_command);
    let cancel_principal = principal.clone();
    let cancel_session_id = session_id.clone();
    let mutation_key = remote_mutation_key(
        "cancel.command",
        &owner,
        Some(&session_id),
        &idempotency_key,
    );
    let cancel_result = run_detached_with_deadline(
        state.detached_mutations.clone(),
        mutation_key,
        "cancel.command",
        REMOTE_CANCEL_COMMAND_TIMEOUT,
        "retry_same_idempotency_key_and_observe",
        Some(&session_id),
        async move {
            command
                .cancel_remote_turn(
                    &cancel_principal,
                    &cancel_session_id,
                    IdempotencyKey::from(format!("remote-cancel:{idempotency_key}")),
                )
                .await
        },
    )
    .await;
    match cancel_result {
        Ok(_) => {}
        Err(DetachedCallFailure::Failed(error)) => return Err(error.into()),
        Err(DetachedCallFailure::TimedOut(error)) => return Err(error),
        Err(DetachedCallFailure::Panicked) => {
            return Err(remote_operation_blocked(
                "cancel.command",
                "the cancel command panicked before its durable outcome was known",
                Some(&session_id),
            ));
        }
        Err(DetachedCallFailure::Admission(error)) => {
            return Err(remote_detached_admission_error(
                "cancel.command",
                Some(&session_id),
                error,
            ));
        }
    }
    let head = read_session_head(
        state.session_query.as_ref(),
        &principal,
        &session_id,
        "cancel.final_head",
        REMOTE_CANCEL_FINAL_HEAD_TIMEOUT,
    )
    .await?;
    Ok(Json(RemoteMutationResponseDto {
        agent_session_id: session_id.as_ref().to_owned(),
        cursor: cursor(&session_id, head.last_seq),
        session_status: head.status,
    }))
}

async fn ensure_remote_session(
    session_query: &dyn AgentSessionQueryPort,
    owner: &UserId,
    session_id: &AgentSessionId,
    operation: &'static str,
    timeout: Duration,
) -> Result<(), RemoteHttpError> {
    let expected = user_principal(owner);
    let observation = with_remote_deadline(
        operation,
        timeout,
        "retry_same_session",
        Some(session_id),
        async {
            session_query
                .observe_session(&expected, session_id, None, 1)
                .await
                .map_err(remote_session_lookup_error)
        },
    )
    .await?;
    if observation.session.remote_binding_provenance.is_none() {
        return Err(RemoteHttpError::canonical(
            "REMOTE_SESSION_NOT_FOUND",
            StatusCode::NOT_FOUND,
            "AgentSession is not a Remote session owned by the authenticated installation",
        ));
    }
    Ok(())
}

async fn read_session_head(
    session_query: &dyn AgentSessionQueryPort,
    principal: &PrincipalRef,
    session_id: &AgentSessionId,
    operation: &'static str,
    timeout: Duration,
) -> Result<nomifun_agent_session::SessionHeadProjection, RemoteHttpError> {
    with_remote_deadline(
        operation,
        timeout,
        "retry_same_session",
        Some(session_id),
        async {
            session_query
                .session_head(principal, session_id)
                .await
                .map_err(RemoteHttpError::from)
        },
    )
    .await
}

async fn with_remote_deadline<T, F>(
    operation: &'static str,
    timeout: Duration,
    recovery: &'static str,
    session_id: Option<&AgentSessionId>,
    future: F,
) -> Result<T, RemoteHttpError>
where
    F: Future<Output = Result<T, RemoteHttpError>>,
{
    match tokio::time::timeout(timeout, future).await {
        Ok(result) => result,
        Err(_) => Err(remote_operation_timeout(
            operation,
            timeout,
            recovery,
            session_id,
        )),
    }
}

async fn run_detached_with_deadline<T, E, F>(
    registry: RemoteDetachedMutationRegistry,
    key: String,
    operation: &'static str,
    timeout: Duration,
    recovery: &'static str,
    session_id: Option<&AgentSessionId>,
    future: F,
) -> Result<T, DetachedCallFailure<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Future<Output = Result<T, E>> + Send + 'static,
{
    let permit = registry
        .try_admit(key)
        .map_err(DetachedCallFailure::Admission)?;
    run_detached_with_permit(
        permit,
        operation,
        timeout,
        recovery,
        session_id,
        future,
    )
    .await
}

async fn run_detached_with_permit<T, E, F>(
    permit: RemoteDetachedMutationPermit,
    operation: &'static str,
    timeout: Duration,
    recovery: &'static str,
    session_id: Option<&AgentSessionId>,
    future: F,
) -> Result<T, DetachedCallFailure<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Future<Output = Result<T, E>> + Send + 'static,
{
    run_detached_with_permits(
        vec![permit],
        operation,
        timeout,
        recovery,
        session_id,
        future,
    )
    .await
}

async fn run_detached_with_permits<T, E, F>(
    permits: Vec<RemoteDetachedMutationPermit>,
    operation: &'static str,
    timeout: Duration,
    recovery: &'static str,
    session_id: Option<&AgentSessionId>,
    future: F,
) -> Result<T, DetachedCallFailure<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Future<Output = Result<T, E>> + Send + 'static,
{
    // A dropped JoinHandle detaches the mutation. This is intentional: a
    // client deadline must not cancel a command after it may have committed
    // its input/turn/cancel fact but before its own durable finalizer runs.
    // The permit remains in the task until the future reaches a terminal
    // state, so detached work is still bounded and observable by shutdown.
    let abort_registration = permits.first().cloned();
    let task = tokio::spawn(async move {
        let _permits = permits;
        future.await
    });
    if let Some(permit) = abort_registration {
        permit.register_abort_handle(task.abort_handle());
    }
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(Ok(value))) => Ok(value),
        Ok(Ok(Err(error))) => Err(DetachedCallFailure::Failed(error)),
        Ok(Err(_)) => Err(DetachedCallFailure::Panicked),
        Err(_) => Err(DetachedCallFailure::TimedOut(remote_operation_timeout(
            operation,
            timeout,
            recovery,
            session_id,
        ))),
    }
}

async fn open_session_and_admit(
    command: Arc<dyn CanonicalAgentSessionCommandPort>,
    runtime: Arc<RemoteRuntimeCoordinator>,
    detached_mutations: RemoteDetachedMutationRegistry,
    session_permit: RemoteDetachedMutationPermit,
    admission_parent_permit: RemoteDetachedMutationPermit,
    request: OpenAgentSessionRequest,
) -> Result<
    (
        nomifun_agent_session::SessionCreateResult,
        Result<(), DetachedCallFailure<AgentPlatformError>>,
    ),
    RemoteOpenWorkflowFailure,
> {
    let created = match run_detached_with_permit(
        session_permit,
        "open.session_command",
        REMOTE_OPEN_SESSION_TIMEOUT,
        "retry_same_idempotency_key_and_observe",
        None,
        async move { command.open_session(request).await },
    )
    .await
    {
        Ok(created) => created,
        Err(DetachedCallFailure::Failed(error)) => {
            return Err(RemoteOpenWorkflowFailure::Session(error));
        }
        Err(DetachedCallFailure::TimedOut(_)) => {
            return Err(RemoteOpenWorkflowFailure::SessionTimedOut);
        }
        Err(DetachedCallFailure::Panicked) => {
            return Err(RemoteOpenWorkflowFailure::SessionPanicked);
        }
        Err(DetachedCallFailure::Admission(_)) => {
            unreachable!("a pre-admitted workflow permit cannot be rejected")
        }
    };
    let session_id = created.session.agent_session_id.clone();
    let admission = detached_runtime_admission(
        &runtime,
        detached_mutations,
        session_id,
        REMOTE_OPEN_ADMISSION_TIMEOUT,
        Some(admission_parent_permit),
    )
    .await;
    Ok((created, admission))
}

async fn detached_runtime_admission(
    runtime: &Arc<RemoteRuntimeCoordinator>,
    detached_mutations: RemoteDetachedMutationRegistry,
    session_id: AgentSessionId,
    timeout: Duration,
    parent_permit: Option<RemoteDetachedMutationPermit>,
) -> Result<(), DetachedCallFailure<AgentPlatformError>> {
    let runtime = Arc::clone(runtime);
    let session_id_for_task = session_id.clone();
    let runtime_key = format!("runtime.admission:{}", session_id.as_ref());
    let runtime_permit = detached_mutations
        .try_admit(runtime_key)
        .map_err(DetachedCallFailure::Admission)?;
    let permits = match parent_permit {
        Some(parent_permit) => vec![parent_permit, runtime_permit],
        None => vec![runtime_permit],
    };
    run_detached_with_permits(
        permits,
        "runtime.admission",
        timeout,
        "observe_same_session_or_restart_host",
        Some(&session_id),
        async move { runtime.ensure_started(session_id_for_task).await },
    )
    .await
}

fn runtime_admission_error(
    runtime: &RemoteRuntimeCoordinator,
    session_id: &AgentSessionId,
    head: &nomifun_agent_session::SessionHeadProjection,
    failure: DetachedCallFailure<AgentPlatformError>,
) -> RemoteHttpError {
    if let Some(reason) = runtime.failure_persistence_blocker(session_id) {
        return remote_persistence_blocked(session_id, head, &reason);
    }
    match failure {
        DetachedCallFailure::Failed(error) => remote_opening_error(session_id, head, &error),
        DetachedCallFailure::TimedOut(error) => error,
        DetachedCallFailure::Panicked => remote_operation_blocked(
            "runtime.admission",
            "Runtime admission panicked before a durable Session state was known",
            Some(session_id),
        ),
        DetachedCallFailure::Admission(error) => {
            remote_detached_admission_error("runtime.admission", Some(session_id), error)
        }
    }
}

fn ensure_turn_ready(
    session_id: &AgentSessionId,
    head: &nomifun_agent_session::SessionHeadProjection,
) -> Result<(), RemoteHttpError> {
    match head.status.as_str() {
        "opening" => Err(remote_opening_state_error(session_id, head)),
        "open_failed" => Err(RemoteHttpError::canonical(
            "REMOTE_OPEN_FAILED",
            StatusCode::UNPROCESSABLE_ENTITY,
            "AgentSession runtime opening failed",
        )),
        "failed" => Err(RemoteHttpError::canonical(
            "REMOTE_OPEN_FAILED",
            StatusCode::UNPROCESSABLE_ENTITY,
            "AgentSession is terminally failed",
        )),
        _ => Ok(()),
    }
}

fn ensure_cancel_allowed(
    session_id: &AgentSessionId,
    head: &nomifun_agent_session::SessionHeadProjection,
) -> Result<(), RemoteHttpError> {
    match head.status.as_str() {
        "opening" => Err(remote_opening_state_error(session_id, head)),
        "open_failed" => Err(RemoteHttpError::canonical(
            "REMOTE_OPEN_FAILED",
            StatusCode::UNPROCESSABLE_ENTITY,
            "AgentSession runtime opening failed",
        )),
        "failed" => Err(RemoteHttpError::canonical(
            "REMOTE_OPEN_FAILED",
            StatusCode::UNPROCESSABLE_ENTITY,
            "AgentSession is terminally failed",
        )),
        _ => Ok(()),
    }
}

fn remote_session_lookup_error(error: AgentPlatformError) -> RemoteHttpError {
    match &error {
        AgentPlatformError::Session(session)
            if session.code() == Some("SESSION_NOT_FOUND") =>
        {
            RemoteHttpError::canonical(
                "REMOTE_SESSION_NOT_FOUND",
                StatusCode::NOT_FOUND,
                "AgentSession is not a Remote session owned by the authenticated installation",
            )
        }
        AgentPlatformError::Contract(message)
            if message
                .to_ascii_lowercase()
                .contains("ownership check") =>
        {
            RemoteHttpError::canonical(
                "REMOTE_SESSION_NOT_FOUND",
                StatusCode::NOT_FOUND,
                "AgentSession is not a Remote session owned by the authenticated installation",
            )
        }
        _ => error.into(),
    }
}

fn open_state(status: &str) -> Result<RemoteOpenStateViewDto, RemoteHttpError> {
    match status {
        "opening" => Ok(RemoteOpenStateViewDto::Opening),
        // A replayed open can observe an active turn. The Runtime admission
        // already completed in this state, so reporting `opening` would make
        // an idempotent client wait forever for a transition that happened.
        "ready" | "running" => Ok(RemoteOpenStateViewDto::Ready),
        "open_failed" => Ok(RemoteOpenStateViewDto::Failed {
            code: "REMOTE_OPEN_FAILED".to_owned(),
            recoverable: true,
        }),
        "failed" => Ok(RemoteOpenStateViewDto::Failed {
            code: "REMOTE_OPEN_FAILED".to_owned(),
            recoverable: false,
        }),
        other => Err(RemoteHttpError::canonical(
            "REMOTE_OPEN_FAILED",
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("AgentSession has unsupported open state {other:?}"),
        )),
    }
}

fn remote_opening_error(
    session_id: &AgentSessionId,
    head: &nomifun_agent_session::SessionHeadProjection,
    cause: &AgentPlatformError,
) -> RemoteHttpError {
    RemoteHttpError::canonical_with_details(
        "REMOTE_SESSION_OPENING",
        StatusCode::CONFLICT,
        "Remote Runtime admission has not reached a durable terminal state",
        serde_json::json!({
            "agent_session_id": session_id,
            "cursor": cursor(session_id, head.last_seq),
            "recovery": "host_restart_reconcile",
            "cause": cause.to_string()
        }),
    )
}

fn remote_opening_state_error(
    session_id: &AgentSessionId,
    head: &nomifun_agent_session::SessionHeadProjection,
) -> RemoteHttpError {
    RemoteHttpError::canonical_with_details(
        "REMOTE_SESSION_OPENING",
        StatusCode::CONFLICT,
        "AgentSession runtime opening has not completed",
        serde_json::json!({
            "agent_session_id": session_id,
            "cursor": cursor(session_id, head.last_seq),
            "recovery": "observe_same_session_or_host_restart_reconcile"
        }),
    )
}

fn remote_persistence_blocked(
    session_id: &AgentSessionId,
    head: &nomifun_agent_session::SessionHeadProjection,
    reason: &str,
) -> RemoteHttpError {
    RemoteHttpError::canonical_with_details(
        REMOTE_PERSISTENCE_BLOCKED_CODE,
        StatusCode::SERVICE_UNAVAILABLE,
        "Remote Runtime failure could not be durably recorded as session/open-failed",
        serde_json::json!({
            "agent_session_id": session_id,
            "cursor": cursor(session_id, head.last_seq),
            "recovery": "restore_storage_then_restart_host",
            "blocker": reason
        }),
    )
}

fn remote_operation_timeout(
    operation: &'static str,
    timeout: Duration,
    recovery: &'static str,
    session_id: Option<&AgentSessionId>,
) -> RemoteHttpError {
    let details = match session_id {
        Some(session_id) => serde_json::json!({
            "operation": operation,
            "timeout_ms": timeout.as_millis() as u64,
            "agent_session_id": session_id,
            "outcome": "unknown",
            "recovery": recovery
        }),
        None => serde_json::json!({
            "operation": operation,
            "timeout_ms": timeout.as_millis() as u64,
            "outcome": "unknown",
            "recovery": recovery
        }),
    };
    RemoteHttpError::canonical_with_details(
        REMOTE_OPERATION_TIMEOUT_CODE,
        StatusCode::GATEWAY_TIMEOUT,
        format!(
            "Remote {operation} exceeded its {} ms deadline",
            timeout.as_millis()
        ),
        details,
    )
}

fn remote_operation_blocked(
    operation: &'static str,
    message: &'static str,
    session_id: Option<&AgentSessionId>,
) -> RemoteHttpError {
    let details = match session_id {
        Some(session_id) => serde_json::json!({
            "operation": operation,
            "agent_session_id": session_id,
            "outcome": "unknown",
            "recovery": "observe_same_session_and_reuse_same_idempotency_key"
        }),
        None => serde_json::json!({
            "operation": operation,
            "outcome": "unknown",
            "recovery": "observe_same_session_and_reuse_same_idempotency_key"
        }),
    };
    RemoteHttpError::canonical_with_details(
        REMOTE_OPERATION_BLOCKED_CODE,
        StatusCode::SERVICE_UNAVAILABLE,
        message,
        details,
    )
}

fn remote_detached_admission_error(
    operation: &'static str,
    session_id: Option<&AgentSessionId>,
    admission: RemoteDetachedMutationAdmissionError,
) -> RemoteHttpError {
    let (status, message, outcome, recovery, reason) = match admission {
        RemoteDetachedMutationAdmissionError::AlreadyRunning => (
            StatusCode::CONFLICT,
            "Remote operation with the same idempotency key is already in flight",
            "unknown",
            "observe_same_session_and_reuse_same_idempotency_key",
            "already_in_flight",
        ),
        RemoteDetachedMutationAdmissionError::CapacityExceeded => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Remote detached mutation capacity is temporarily exhausted",
            "not_started",
            "retry_same_idempotency_key_after_capacity_recovers",
            "capacity_exhausted",
        ),
        RemoteDetachedMutationAdmissionError::Closed => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Remote detached mutation admission is closed during host shutdown",
            "not_started",
            "restart_host_then_retry_same_idempotency_key",
            "coordinator_closed",
        ),
    };
    let details = match session_id {
        Some(session_id) => serde_json::json!({
            "operation": operation,
            "agent_session_id": session_id,
            "outcome": outcome,
            "recovery": recovery,
            "reason": reason
        }),
        None => serde_json::json!({
            "operation": operation,
            "outcome": outcome,
            "recovery": recovery,
            "reason": reason
        }),
    };
    RemoteHttpError::canonical_with_details(
        REMOTE_OPERATION_BLOCKED_CODE,
        status,
        message,
        details,
    )
}

fn remote_mutation_key(
    operation: &'static str,
    owner: &UserId,
    session_id: Option<&AgentSessionId>,
    idempotency_key: &str,
) -> String {
    let session = session_id.map(|id| id.as_ref()).unwrap_or("none");
    let scope = format!(
        "nomifun-remote-detached-mutation-v1\0{operation}\0{}\0{session}\0{idempotency_key}",
        owner.as_ref()
    );
    format!(
        "remote:{operation}:{}",
        nomifun_auth::token_sha256_hex(&scope)
    )
}

fn user_principal(owner: &UserId) -> PrincipalRef {
    PrincipalRef {
        principal_kind: "user".to_owned(),
        principal_id: owner.as_ref().to_owned(),
    }
}

fn contract_user_id(owner: &UserId) -> nomifun_agent_contracts::UserId {
    nomifun_agent_contracts::UserId::from(owner.as_ref().to_owned())
}

fn cursor(session_id: &AgentSessionId, seq: u64) -> SessionCursorDto {
    SessionCursorDto {
        agent_session_id: session_id.as_ref().to_owned(),
        seq,
    }
}

fn nonempty(value: &str, field: &str) -> Result<String, RemoteHttpError> {
    if value.trim().is_empty() || value.trim() != value {
        return Err(RemoteHttpError::canonical(
            "REMOTE_INVALID_REQUEST",
            StatusCode::BAD_REQUEST,
            format!("{field} must be canonical and non-empty"),
        ));
    }
    Ok(value.to_owned())
}

fn validated_idempotency_key(value: &str) -> Result<String, RemoteHttpError> {
    if !nomifun_common::is_visible_ascii_key(
        value,
        nomifun_common::MAX_IDEMPOTENCY_KEY_LEN,
    ) {
        return Err(RemoteHttpError::canonical(
            "REMOTE_INVALID_REQUEST",
            StatusCode::BAD_REQUEST,
            format!(
                "idempotency_key must contain 1..={} visible ASCII bytes",
                nomifun_common::MAX_IDEMPOTENCY_KEY_LEN
            ),
        ));
    }
    Ok(value.to_owned())
}

fn validate_observe_limit(limit: u32) -> Result<(), RemoteHttpError> {
    if limit == 0 {
        return Err(RemoteHttpError::canonical(
            "REMOTE_INVALID_REQUEST",
            StatusCode::BAD_REQUEST,
            "limit must be greater than zero",
        ));
    }
    Ok(())
}

fn parse_session_id(value: &str) -> Result<AgentSessionId, RemoteHttpError> {
    let parsed = Uuid::parse_str(value).map_err(|_| {
        RemoteHttpError::canonical(
            "REMOTE_SESSION_NOT_FOUND",
            StatusCode::NOT_FOUND,
            "agent_session_id must be a canonical UUIDv7",
        )
    })?;
    if parsed.get_version_num() != 7 || parsed.hyphenated().to_string() != value {
        return Err(RemoteHttpError::canonical(
            "REMOTE_SESSION_NOT_FOUND",
            StatusCode::NOT_FOUND,
            "agent_session_id must be a canonical UUIDv7",
        ));
    }
    Ok(AgentSessionId::from(value.to_owned()))
}

fn bounded_json(value: Value, field: &str) -> Result<Value, RemoteHttpError> {
    let bytes = nomifun_agent_contracts::canonical_json_bytes(&value).map_err(|error| {
        RemoteHttpError::canonical(
            "REMOTE_INVALID_REQUEST",
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{field} is not canonical JSON: {error}"),
        )
    })?;
    if bytes.len() > nomifun_agent_session::MAX_INLINE_JSON_BYTES {
        return Err(RemoteHttpError::canonical(
            "REMOTE_INVALID_REQUEST",
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "{field} exceeds the {}-byte Remote input limit",
                nomifun_agent_session::MAX_INLINE_JSON_BYTES
            ),
        ));
    }
    Ok(value)
}

fn default_observe_limit() -> u32 {
    100
}

fn decode<T: DeserializeOwned, U: Serialize>(value: &U) -> Result<T, RemoteHttpError> {
    Ok(serde_json::from_value(serde_json::to_value(value)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replayed_open_reports_ready_after_a_turn_has_started() {
        assert_eq!(
            open_state("running").expect("running is an admitted Runtime state"),
            RemoteOpenStateViewDto::Ready
        );
    }

    #[test]
    fn open_state_keeps_open_failure_terminal_and_does_not_hide_unknown_states() {
        assert_eq!(
            open_state("open_failed").expect("open_failed is a canonical terminal state"),
            RemoteOpenStateViewDto::Failed {
                code: "REMOTE_OPEN_FAILED".to_owned(),
                recoverable: true,
            }
        );
        assert_eq!(
            open_state("failed").expect("failed is a terminal Session state"),
            RemoteOpenStateViewDto::Failed {
                code: "REMOTE_OPEN_FAILED".to_owned(),
                recoverable: false,
            }
        );

        let error = open_state("unexpected").expect_err("unknown state must fail closed");
        assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.code, "REMOTE_OPEN_FAILED");
    }

    #[test]
    fn unresolved_opening_error_keeps_cursor_and_restart_recovery_hint() {
        let session_id = AgentSessionId::from(
            "0190f5fe-7c00-7a00-8000-000000000001".to_owned(),
        );
        let head = nomifun_agent_session::SessionHeadProjection {
            session_id: session_id.clone(),
            status: "opening".to_owned(),
            active_turn_id: None,
            active_set_generation: 0,
            runtime_checkpoint_locator: None,
            runtime_checkpoint_digest: None,
            runtime_bound_event_id: None,
            runtime_protocol_version: None,
            snapshot_digest: None,
            checkpoint_through_seq: None,
            last_seq: 2,
            unread_count: 0,
        };
        let cause = AgentPlatformError::Contract(
            "durable session/open-failed append failed".to_owned(),
        );
        let error = remote_opening_error(&session_id, &head, &cause);

        assert_eq!(error.status, StatusCode::CONFLICT);
        assert_eq!(error.code, "REMOTE_SESSION_OPENING");
        let details = error.details.expect("opening error details");
        assert_eq!(details["agent_session_id"], session_id.as_ref());
        assert_eq!(details["cursor"]["seq"], 2);
        assert_eq!(details["recovery"], "host_restart_reconcile");
    }

    #[test]
    fn request_validation_errors_use_the_shared_invalid_request_code() {
        let error = validated_idempotency_key("  ").expect_err("blank key must fail");
        assert_eq!(error.code, "REMOTE_INVALID_REQUEST");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        for invalid in [
            "contains space".to_owned(),
            "non-ascii-键".to_owned(),
            "x".repeat(nomifun_common::MAX_IDEMPOTENCY_KEY_LEN + 1),
        ] {
            let error = validated_idempotency_key(&invalid)
                .expect_err("non-canonical idempotency key must fail");
            assert_eq!(error.code, "REMOTE_INVALID_REQUEST");
            assert_eq!(error.status, StatusCode::BAD_REQUEST);
        }
        assert_eq!(
            validated_idempotency_key("remote-turn-abc_123").unwrap(),
            "remote-turn-abc_123"
        );

        let error = nonempty("", "binding_id").expect_err("blank binding id must fail");
        assert_eq!(error.code, "REMOTE_INVALID_REQUEST");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);

        let error = bounded_json(
            serde_json::json!({
                "text": "x".repeat(nomifun_agent_session::MAX_INLINE_JSON_BYTES)
            }),
            "input",
        )
        .expect_err("oversized input must fail");
        assert_eq!(error.code, "REMOTE_INVALID_REQUEST");
        assert_eq!(error.status, StatusCode::PAYLOAD_TOO_LARGE);

        let error = validate_observe_limit(0).expect_err("zero observe limit must fail");
        assert_eq!(error.code, "REMOTE_INVALID_REQUEST");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn detached_admission_preserves_unknown_result_for_same_key_retries() {
        let session_id = AgentSessionId::from(
            "0190f5fe-7c00-7a00-8000-000000000001".to_owned(),
        );
        let error = remote_detached_admission_error(
            "turn.dispatch",
            Some(&session_id),
            RemoteDetachedMutationAdmissionError::AlreadyRunning,
        );

        assert_eq!(error.status, StatusCode::CONFLICT);
        assert_eq!(error.code, REMOTE_OPERATION_BLOCKED_CODE);
        let details = error.details.expect("admission details");
        assert_eq!(details["outcome"], "unknown");
        assert_eq!(
            details["recovery"],
            "observe_same_session_and_reuse_same_idempotency_key"
        );
        assert_eq!(details["reason"], "already_in_flight");
    }

    #[tokio::test]
    async fn legacy_selector_queries_are_rejected_without_calling_a_handler() {
        use axum::body::Body;
        use axum::routing::post;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tower::ServiceExt;

        let calls = Arc::new(AtomicUsize::new(0));
        let probe = Arc::clone(&calls);
        let app = Router::new()
            .route(
                "/api/remote/open",
                post(move || {
                    let probe = Arc::clone(&probe);
                    async move {
                        probe.fetch_add(1, Ordering::AcqRel);
                        "unexpected"
                    }
                }),
            )
            .layer(from_fn(reject_undeclared_query_parameters));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/remote/open?domains=agent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(calls.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn timed_out_remote_mutation_stays_detached_for_idempotent_recovery() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        let finished = Arc::new(AtomicBool::new(false));
        let marker = Arc::clone(&finished);
        let calls = Arc::new(AtomicUsize::new(0));
        let first_calls = Arc::clone(&calls);
        let registry = RemoteDetachedMutationRegistry::new();
        let retry_registry = registry.clone();
        let result = run_detached_with_deadline(
            registry,
            "test.mutation:key".to_owned(),
            "test.mutation",
            Duration::from_millis(20),
            "retry_same_idempotency_key_and_observe",
            None,
            async move {
                first_calls.fetch_add(1, Ordering::AcqRel);
                tokio::time::sleep(Duration::from_millis(80)).await;
                marker.store(true, Ordering::Release);
                Ok::<_, ()>(())
            },
        )
        .await;

        let error = match result {
            Err(DetachedCallFailure::TimedOut(error)) => error,
            other => panic!("expected a bounded timeout, got {other:?}"),
        };
        assert_eq!(error.code, REMOTE_OPERATION_TIMEOUT_CODE);
        assert_eq!(error.status, StatusCode::GATEWAY_TIMEOUT);

        let retry_calls = Arc::clone(&calls);
        let retry = run_detached_with_deadline(
            retry_registry,
            "test.mutation:key".to_owned(),
            "test.mutation",
            Duration::from_millis(20),
            "retry_same_idempotency_key_and_observe",
            None,
            async move {
                retry_calls.fetch_add(1, Ordering::AcqRel);
                Ok::<_, ()>(())
            },
        )
        .await;
        assert!(
            matches!(
                retry,
                Err(DetachedCallFailure::Admission(
                    RemoteDetachedMutationAdmissionError::AlreadyRunning
                ))
            ),
            "same-key retry must not start a second detached command"
        );

        tokio::time::timeout(Duration::from_secs(1), async {
            while !finished.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("timed-out mutation should remain alive for durable convergence");
        assert_eq!(
            calls.load(Ordering::Acquire),
            1,
            "the timed-out command must be invoked exactly once"
        );
    }
}
