//! Canonical Remote REST ingress for the Fresh-v4 AgentSession chain.
//!
//! This adapter deliberately owns no Remote state. It authenticates the
//! installation owner, converts the four wire DTOs, and delegates all
//! persistence, ownership, idempotency, and runtime admission to the Remote
//! package's manifest-declared AgentSession command/query ports. The concrete
//! platform is retained only for RemoteBinding control-plane lookup.

use std::sync::Arc;

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

use super::remote_runtime::RemoteRuntimeCoordinator;

#[derive(Clone)]
struct RemoteRestState {
    platform: Arc<AgentPlatform>,
    session_command: Arc<dyn CanonicalAgentSessionCommandPort>,
    session_query: Arc<dyn AgentSessionQueryPort>,
    runtime: Arc<RemoteRuntimeCoordinator>,
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
    let state = RemoteRestState {
        platform,
        session_command,
        session_query,
        runtime,
    };
    Router::new()
        .route("/api/remote/open", post(open))
        .route("/api/remote/turn", post(turn))
        .route("/api/remote/observe", get(observe))
        .route("/api/remote/cancel", post(cancel))
        .with_state(state)
        .layer(from_fn(reject_undeclared_query_parameters))
        .layer(from_fn_with_state(
            PublicMcpState {
                validator,
                authoritative_user_id,
            },
            instance_token_middleware,
        ))
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
    let idempotency_key = nonempty(&request.idempotency_key, "idempotency_key")?;
    let binding_id = nonempty(&request.binding_id, "binding_id")?;
    let initial_input = request
        .initial_input
        .map(|value| bounded_json(value, "initial_input"))
        .transpose()?;
    let binding = state
        .platform
        .control_plane()
        .get_remote_binding(&contract_user_id(&owner), &binding_id)
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

    let created = state.session_command.open_session(open).await?;
    let session_id = created.session.agent_session_id.clone();
    let principal = user_principal(&owner);
    let admission_error = state.runtime.ensure_started(session_id.clone()).await.err();
    let (status, last_seq) = if created.duplicate || admission_error.is_some() {
        let head = state
            .session_query
            .session_head(&principal, &session_id)
            .await?;
        if let Some(error) = admission_error {
            if head.status == "opening" {
                return Err(RemoteHttpError::canonical_with_details(
                    "REMOTE_SESSION_OPENING",
                    StatusCode::CONFLICT,
                    "Remote Runtime admission did not settle; the Session remains opening",
                    serde_json::json!({
                        "agent_session_id": session_id,
                        "cursor": cursor(&session_id, head.last_seq),
                        "recovery": "host_restart_reconcile",
                        "cause": error.to_string()
                    }),
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
    let idempotency_key = nonempty(&request.idempotency_key, "idempotency_key")?;
    let session_id = parse_session_id(&request.agent_session_id)?;
    let input = bounded_json(request.input, "input")?;
    ensure_remote_session(state.session_query.as_ref(), &owner, &session_id).await?;
    let mut current_head = state
        .session_query
        .session_head(&user_principal(&owner), &session_id)
        .await?;
    if current_head.status == "opening" {
        if let Err(error) = state.runtime.ensure_started(session_id.clone()).await {
            current_head = state
                .session_query
                .session_head(&user_principal(&owner), &session_id)
                .await?;
            if current_head.status == "opening" {
                return Err(remote_opening_error(&session_id, &current_head, &error));
            }
        }
        current_head = state
            .session_query
            .session_head(&user_principal(&owner), &session_id)
            .await?;
    }
    if current_head.status == "opening" {
        return Err(RemoteHttpError::canonical(
            "REMOTE_SESSION_OPENING",
            StatusCode::CONFLICT,
            "AgentSession runtime opening has not completed",
        ));
    }
    if current_head.status == "open_failed" {
        return Err(RemoteHttpError::canonical(
            "REMOTE_OPEN_FAILED",
            StatusCode::UNPROCESSABLE_ENTITY,
            "AgentSession runtime opening failed",
        ));
    }
    let dispatch = state
        .session_command
        .start_turn(StartAgentTurnRequest {
            agent_session_id: session_id.clone(),
            principal: user_principal(&owner),
            input: StrictJsonValue(input),
            idempotency_key: IdempotencyKey::from(format!("remote-turn:{idempotency_key}")),
        })
        .await?;
    let head = state
        .session_query
        .session_head(&user_principal(&owner), &session_id)
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
    ensure_remote_session(state.session_query.as_ref(), &owner, &session_id).await?;
    let current_head = state
        .session_query
        .session_head(&user_principal(&owner), &session_id)
        .await?;
    if current_head.status == "opening" {
        if let Err(error) = state.runtime.ensure_started(session_id.clone()).await {
            let latest_head = state
                .session_query
                .session_head(&user_principal(&owner), &session_id)
                .await?;
            if latest_head.status == "opening" {
                return Err(remote_opening_error(&session_id, &latest_head, &error));
            }
        }
    }
    let after = SessionEventCursor {
        agent_session_id: session_id.clone(),
        seq: request.after_seq,
    };
    let observation = state
        .session_query
        .observe_session(
            &user_principal(&owner),
            &session_id,
            Some(&after),
            request.limit,
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
    let idempotency_key = nonempty(&request.idempotency_key, "idempotency_key")?;
    let session_id = parse_session_id(&request.agent_session_id)?;
    ensure_remote_session(state.session_query.as_ref(), &owner, &session_id).await?;
    state
        .session_command
        .cancel_remote_turn(
            &user_principal(&owner),
            &session_id,
            IdempotencyKey::from(format!("remote-cancel:{idempotency_key}")),
        )
        .await?;
    let head = state
        .session_query
        .session_head(&user_principal(&owner), &session_id)
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
) -> Result<(), RemoteHttpError> {
    let expected = user_principal(owner);
    let observation = session_query
        .observe_session(&expected, session_id, None, 1)
        .await
        .map_err(remote_session_lookup_error)?;
    if observation.session.remote_binding_provenance.is_none() {
        return Err(RemoteHttpError::canonical(
            "REMOTE_SESSION_NOT_FOUND",
            StatusCode::NOT_FOUND,
            "AgentSession is not a Remote session owned by the authenticated installation",
        ));
    }
    Ok(())
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
        let error = nonempty("  ", "idempotency_key").expect_err("blank key must fail");
        assert_eq!(error.code, "REMOTE_INVALID_REQUEST");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);

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
}
