//! Canonical Remote MCP adapter for the Fresh-v4 AgentSession chain.
//!
//! The MCP transport session managed by rmcp is only a connection lifecycle.
//! Product identity is always the explicit `agent_session_id` carried by the
//! four Remote operations below. No capability registry or GatewayDeps lookup
//! is used by this adapter.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;
use axum::middleware::from_fn_with_state;
use nomifun_agent_contracts::{
    AgentBindingValue, AgentSessionId, CorrelationId, EventProducerId, IdempotencyKey,
    OperationId, PrincipalRef, RemoteBindingId, RemoteBindingProvenance, StrictJsonValue,
};
use nomifun_agent_platform::{
    AgentPlatform, AgentPlatformError, AgentSessionCommandPort, AgentSessionQueryPort,
    OpenAgentSessionRequest, StartAgentTurnRequest,
};
use nomifun_api_types::{
    RemoteCancelRequestDto, RemoteMutationResponseDto, RemoteObserveRequestDto,
    RemoteObserveResponseDto, RemoteOpenRequestDto, RemoteOpenResponseDto, RemoteOpenStateViewDto,
    RemoteTurnRequestDto, SessionCursorDto,
};
use nomifun_auth::{InstanceTokenValidator, RemoteAuthAdmissionFence};
use nomifun_common::{UserId, validate_uuidv7};
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, JsonObject, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Value, json};
use tower_http::limit::RequestBodyLimitLayer;

use crate::result::build_tool_result;
use crate::router::{
    McpAuthState, RemoteInstanceOwner, initialize_preflight_middleware,
    mcp_instance_token_middleware,
};
use crate::session::{
    RemoteMcpSessionAdmissionAuthority, RemoteMcpSessionIdentity, RemoteSessionManager,
};

pub const CANONICAL_REMOTE_OPEN_TOOL: &str = "open";
pub const CANONICAL_REMOTE_TURN_TOOL: &str = "turn";
pub const CANONICAL_REMOTE_OBSERVE_TOOL: &str = "observe";
pub const CANONICAL_REMOTE_CANCEL_TOOL: &str = "cancel";

/// Host-owned Runtime admission for the post-commit Remote open step.
///
/// The public crate does not know how a host resolves or launches its pinned
/// sidecar. It receives one explicit admission port and never creates a
/// second Runtime or Session authority.
pub trait CanonicalRemoteRuntimeAdmission: Send + Sync {
    fn ensure_started<'a>(
        &'a self,
        session_id: AgentSessionId,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
}

#[derive(Clone)]
pub struct CanonicalRemoteMcpHandler {
    platform: Arc<AgentPlatform>,
    runtime: Arc<dyn CanonicalRemoteRuntimeAdmission>,
}

impl CanonicalRemoteMcpHandler {
    pub fn new(
        platform: Arc<AgentPlatform>,
        runtime: Arc<dyn CanonicalRemoteRuntimeAdmission>,
    ) -> Self {
        Self { platform, runtime }
    }
}

/// Build the canonical Fresh-v4 MCP front door.
///
/// `RemoteSessionManager` only bounds rmcp transport sessions and pins the
/// authenticated installation owner. All product work is delegated to the
/// injected `AgentPlatform` and Runtime admission port.
pub fn canonical_remote_mcp_router(
    platform: Arc<AgentPlatform>,
    validator: Arc<InstanceTokenValidator>,
    authoritative_user_id: UserId,
    runtime: Arc<dyn CanonicalRemoteRuntimeAdmission>,
    auth_fence: RemoteAuthAdmissionFence,
) -> Router {
    let transport_admission =
        RemoteMcpSessionAdmissionAuthority::for_owner(&authoritative_user_id);
    let sessions = Arc::new(RemoteSessionManager::with_owner_admission_authority(
        authoritative_user_id.clone(),
        transport_admission,
    ));
    let service = rmcp::transport::streamable_http_server::StreamableHttpService::new(
        {
            let platform = Arc::clone(&platform);
            let runtime = Arc::clone(&runtime);
            move || {
                Ok(CanonicalRemoteMcpHandler::new(
                    Arc::clone(&platform),
                    Arc::clone(&runtime),
                ))
            }
        },
        Arc::clone(&sessions),
        rmcp::transport::streamable_http_server::StreamableHttpServerConfig::default()
            .disable_allowed_hosts(),
    );

    Router::new()
        .fallback_service(service)
        .layer(RequestBodyLimitLayer::new(
            nomifun_common::constants::BODY_LIMIT,
        ))
        .layer(axum::middleware::from_fn(initialize_preflight_middleware))
        .layer(from_fn_with_state(
            McpAuthState {
                public: crate::router::PublicMcpState {
                    validator: validator.clone(),
                    authoritative_user_id: authoritative_user_id.clone(),
                },
                sessions,
            },
            mcp_instance_token_middleware,
        ))
        .layer(from_fn_with_state(
            crate::router::PublicMcpAdmissionState {
                public: crate::router::PublicMcpState {
                    validator,
                    authoritative_user_id,
                },
                admission: auth_fence,
            },
            crate::router::instance_token_middleware_with_admission,
        ))
}

impl ServerHandler for CanonicalRemoteMcpHandler {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "Canonical NomiFun Remote operations: open, turn, observe, cancel. \
             Every operation uses an explicit AgentSessionId; the MCP transport \
             session is not a product identity."
                .to_owned(),
        );
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        require_transport_identity(&context)?;
        Ok(ListToolsResult {
            tools: canonical_tools(),
            meta: None,
            next_cursor: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        require_transport_identity(&context)?;
        let owner = match owner_from_context(&context) {
            Ok(owner) => owner,
            Err(error) => return Ok(error_result("REMOTE_AUTH_REQUIRED", error)),
        };
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let name = request.name.as_ref();
        let result = match name {
            CANONICAL_REMOTE_OPEN_TOOL => {
                decode_and_run::<RemoteOpenRequestDto, _, _>(
                    arguments,
                    |request| self.open(&owner, request),
                )
                .await
            }
            CANONICAL_REMOTE_TURN_TOOL => {
                decode_and_run::<RemoteTurnRequestDto, _, _>(
                    arguments,
                    |request| self.turn(&owner, request),
                )
                .await
            }
            CANONICAL_REMOTE_OBSERVE_TOOL => {
                decode_and_run::<RemoteObserveRequestDto, _, _>(
                    arguments,
                    |request| self.observe(&owner, request),
                )
                .await
            }
            CANONICAL_REMOTE_CANCEL_TOOL => {
                decode_and_run::<RemoteCancelRequestDto, _, _>(
                    arguments,
                    |request| self.cancel(&owner, request),
                )
                .await
            }
            _ => Err(CanonicalRemoteError::new(
                "REMOTE_OPERATION_NOT_FOUND",
                format!("unknown canonical Remote operation {name}"),
            )),
        };
        Ok(match result {
            Ok(value) => success_result(value),
            Err(error) => error.into_tool_result(),
        })
    }
}

async fn decode_and_run<T, F, Fut>(
    value: Value,
    run: F,
) -> Result<Value, CanonicalRemoteError>
where
    T: DeserializeOwned,
    F: FnOnce(T) -> Fut,
    Fut: Future<Output = Result<Value, CanonicalRemoteError>>,
{
    let request = serde_json::from_value::<T>(value).map_err(|error| {
        CanonicalRemoteError::new(
            "REMOTE_INVALID_REQUEST",
            format!("canonical Remote operation arguments are invalid: {error}"),
        )
    })?;
    run(request).await
}

impl CanonicalRemoteMcpHandler {
    async fn open(
        &self,
        owner: &UserId,
        request: RemoteOpenRequestDto,
    ) -> Result<Value, CanonicalRemoteError> {
        let idempotency_key = nonempty(&request.idempotency_key, "idempotency_key")?;
        let initial_input = request
            .initial_input
            .map(|value| bounded_json(value, "initial_input"))
            .transpose()?;
        let binding_id = nonempty(&request.binding_id, "binding_id")?;
        let contract_owner = contract_user_id(owner);
        let binding = self
            .platform
            .control_plane()
            .get_remote_binding(&contract_owner, &binding_id)
            .await
            .map_err(|error| {
                CanonicalRemoteError::from(AgentPlatformError::from(error))
            })?
            .ok_or_else(|| {
                CanonicalRemoteError::new(
                    "REMOTE_BINDING_NOT_FOUND",
                    "RemoteBinding does not exist for the authenticated owner",
                )
            })?;
        let agent_binding: AgentBindingValue =
            decode_wire(&binding.agent_binding).map_err(CanonicalRemoteError::from)?;
        let mut open = OpenAgentSessionRequest::user(
            &contract_owner,
            agent_binding.clone(),
            IdempotencyKey::from(format!("remote-open:{idempotency_key}")),
        );
        open.remote_binding_provenance = Some(RemoteBindingProvenance {
            remote_binding_id: RemoteBindingId::from(binding.remote_binding_id),
            binding_version: agent_binding.binding_version,
        });
        open.operation_id = OperationId::from(format!("remote-open:{idempotency_key}"));
        open.producer_id = EventProducerId::from(format!("remote_mcp:{}", owner.as_ref()));
        open.correlation_id = CorrelationId::from(open.operation_id.as_ref().to_owned());
        open.scene = "remote".to_owned();
        open.surface = "remote".to_owned();
        open.audience = "owner".to_owned();
        open.initial_input = initial_input.map(StrictJsonValue);

        let created = self
            .platform
            .open_session(open)
            .await
            .map_err(CanonicalRemoteError::from)?;
        let session_id = created.session.agent_session_id.clone();
        let principal = user_principal(&contract_owner);
        let admission_error = self.runtime.ensure_started(session_id.clone()).await.err();
        let (status, last_seq) = if created.duplicate || admission_error.is_some() {
            let head = self
                .platform
                .session_head(&principal, &session_id)
                .await
                .map_err(CanonicalRemoteError::from)?;
            if let Some(error) = admission_error
                && head.status == "opening"
            {
                return Err(CanonicalRemoteError::with_details(
                    "REMOTE_SESSION_OPENING",
                    "Remote Runtime admission has not reached a durable terminal state",
                    json!({
                        "agent_session_id": session_id,
                        "cursor": cursor(&session_id, head.last_seq),
                        "recovery": "host_restart_reconcile",
                        "cause": error
                    }),
                ));
            }
            (head.status, head.last_seq)
        } else {
            ("opening".to_owned(), created.activation_ack.seq)
        };

        let response = RemoteOpenResponseDto {
            agent_session_id: session_id.as_ref().to_owned(),
            agent_binding: decode_wire(&created.session.agent_binding)
                .map_err(CanonicalRemoteError::from)?,
            open_state: open_state(&status)?,
            cursor: cursor(&session_id, last_seq),
        };
        serde_json::to_value(response).map_err(CanonicalRemoteError::from)
    }

    async fn turn(
        &self,
        owner: &UserId,
        request: RemoteTurnRequestDto,
    ) -> Result<Value, CanonicalRemoteError> {
        let idempotency_key = nonempty(&request.idempotency_key, "idempotency_key")?;
        let session_id = parse_session_id(&request.agent_session_id)?;
        let input = bounded_json(request.input, "input")?;
        ensure_remote_session(&self.platform, owner, &session_id).await?;
        let mut head = self
            .platform
            .session_head(&user_principal(&contract_user_id(owner)), &session_id)
            .await
            .map_err(CanonicalRemoteError::from)?;
        if head.status == "opening" {
            if let Err(error) = self.runtime.ensure_started(session_id.clone()).await {
                head = self
                    .platform
                    .session_head(&user_principal(&contract_user_id(owner)), &session_id)
                    .await
                    .map_err(CanonicalRemoteError::from)?;
                if head.status == "opening" {
                    return Err(CanonicalRemoteError::with_details(
                        "REMOTE_SESSION_OPENING",
                        "Remote Runtime admission has not reached a durable terminal state",
                        json!({
                            "agent_session_id": session_id,
                            "cursor": cursor(&session_id, head.last_seq),
                            "recovery": "host_restart_reconcile",
                            "cause": error
                        }),
                    ));
                }
            }
            head = self
                .platform
                .session_head(&user_principal(&contract_user_id(owner)), &session_id)
                .await
                .map_err(CanonicalRemoteError::from)?;
        }
        if head.status == "opening" {
            return Err(CanonicalRemoteError::new(
                "REMOTE_SESSION_OPENING",
                "AgentSession runtime opening has not completed",
            ));
        }
        if head.status == "open_failed" {
            return Err(CanonicalRemoteError::new(
                "REMOTE_OPEN_FAILED",
                "AgentSession runtime opening failed",
            ));
        }
        let dispatch = self
            .platform
            .start_turn(StartAgentTurnRequest {
                agent_session_id: session_id.clone(),
                principal: user_principal(&contract_user_id(owner)),
                input: StrictJsonValue(input),
                idempotency_key: IdempotencyKey::from(format!(
                    "remote-turn:{idempotency_key}"
                )),
            })
            .await
            .map_err(CanonicalRemoteError::from)?;
        let head = self
            .platform
            .session_head(&user_principal(&contract_user_id(owner)), &session_id)
            .await
            .map_err(CanonicalRemoteError::from)?;
        serde_json::to_value(RemoteMutationResponseDto {
            agent_session_id: dispatch.agent_session_id.as_ref().to_owned(),
            cursor: cursor(&session_id, head.last_seq),
            session_status: head.status,
        })
        .map_err(CanonicalRemoteError::from)
    }

    async fn observe(
        &self,
        owner: &UserId,
        request: RemoteObserveRequestDto,
    ) -> Result<Value, CanonicalRemoteError> {
        validate_observe_limit(request.limit)?;
        let session_id = parse_session_id(&request.agent_session_id)?;
        if request.after_cursor.agent_session_id != request.agent_session_id {
            return Err(CanonicalRemoteError::new(
                "REMOTE_SESSION_NOT_FOUND",
                "after_cursor must reference the same AgentSession",
            ));
        }
        ensure_remote_session(&self.platform, owner, &session_id).await?;
        let principal = user_principal(&contract_user_id(owner));
        let head = self
            .platform
            .session_head(&principal, &session_id)
            .await
            .map_err(CanonicalRemoteError::from)?;
        if head.status == "opening" {
            if let Err(error) = self.runtime.ensure_started(session_id.clone()).await {
                let latest = self
                    .platform
                    .session_head(&principal, &session_id)
                    .await
                    .map_err(CanonicalRemoteError::from)?;
                if latest.status == "opening" {
                    return Err(CanonicalRemoteError::with_details(
                        "REMOTE_SESSION_OPENING",
                        "Remote Runtime admission has not reached a durable terminal state",
                        json!({
                            "agent_session_id": session_id,
                            "cursor": cursor(&session_id, latest.last_seq),
                            "recovery": "host_restart_reconcile",
                            "cause": error
                        }),
                    ));
                }
            }
        }
        let observation = self
            .platform
            .observe_from_cursor(
                &contract_user_id(owner),
                session_id.as_ref(),
                Some(request.after_cursor),
                request.limit,
            )
            .await
            .map_err(CanonicalRemoteError::from)?;
        let events = observation
            .events
            .into_iter()
            .map(|event| serde_json::to_value(event).map_err(CanonicalRemoteError::from))
            .collect::<Result<Vec<_>, _>>()?;
        let messages = observation
            .messages
            .into_iter()
            .map(|message| message.projection)
            .collect();
        serde_json::to_value(RemoteObserveResponseDto {
            agent_session_id: session_id.as_ref().to_owned(),
            events,
            messages,
            next_cursor: cursor(
                &observation.next_cursor.agent_session_id,
                observation.next_cursor.seq,
            ),
        })
        .map_err(CanonicalRemoteError::from)
    }

    async fn cancel(
        &self,
        owner: &UserId,
        request: RemoteCancelRequestDto,
    ) -> Result<Value, CanonicalRemoteError> {
        let idempotency_key = nonempty(&request.idempotency_key, "idempotency_key")?;
        let session_id = parse_session_id(&request.agent_session_id)?;
        ensure_remote_session(&self.platform, owner, &session_id).await?;
        let principal = user_principal(&contract_user_id(owner));
        let head = self
            .platform
            .session_head(&principal, &session_id)
            .await
            .map_err(CanonicalRemoteError::from)?;
        if head.status != "running" || head.active_turn_id.is_none() {
            return Err(CanonicalRemoteError::new(
                "REMOTE_SESSION_BUSY",
                "AgentSession has no cancellable active turn",
            ));
        }
        self.platform
            .cancel_remote_turn(
                &principal,
                &session_id,
                IdempotencyKey::from(format!("remote-cancel:{idempotency_key}")),
            )
            .await
            .map_err(CanonicalRemoteError::from)?;
        let head = self
            .platform
            .session_head(&principal, &session_id)
            .await
            .map_err(CanonicalRemoteError::from)?;
        serde_json::to_value(RemoteMutationResponseDto {
            agent_session_id: session_id.as_ref().to_owned(),
            cursor: cursor(&session_id, head.last_seq),
            session_status: head.status,
        })
        .map_err(CanonicalRemoteError::from)
    }
}

#[derive(Debug)]
struct CanonicalRemoteError {
    code: String,
    message: String,
    details: Option<Value>,
}

impl CanonicalRemoteError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    fn with_details(
        code: impl Into<String>,
        message: impl Into<String>,
        details: Value,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: Some(details),
        }
    }

    fn into_tool_result(self) -> CallToolResult {
        let mut error = json!({
            "code": self.code,
            "message": self.message,
        });
        if let Some(details) = self.details
            && let Value::Object(map) = &mut error
        {
            map.insert("details".to_owned(), details);
        }
        build_tool_result(json!({ "error": error }))
    }
}

impl From<AgentPlatformError> for CanonicalRemoteError {
    fn from(error: AgentPlatformError) -> Self {
        let code = match &error {
            AgentPlatformError::ControlPlane(error) => error.code().as_ref().to_owned(),
            AgentPlatformError::Session(error) => error
                .code()
                .unwrap_or("REMOTE_SESSION_NOT_FOUND")
                .to_owned(),
            AgentPlatformError::Contract(message) => {
                let lower = message.to_ascii_lowercase();
                if lower.contains("opening") {
                    "REMOTE_SESSION_OPENING".to_owned()
                } else if lower.contains("busy")
                    || lower.contains("completed-turn boundary")
                    || lower.contains("active turn")
                {
                    "REMOTE_SESSION_BUSY".to_owned()
                } else {
                    "REMOTE_OPEN_FAILED".to_owned()
                }
            }
            AgentPlatformError::Runtime(_) | AgentPlatformError::Model(_) => {
                "SNAPSHOT_EXECUTOR_UNAVAILABLE".to_owned()
            }
            _ => "REMOTE_OPEN_FAILED".to_owned(),
        };
        Self::new(code, error.to_string())
    }
}

impl From<serde_json::Error> for CanonicalRemoteError {
    fn from(error: serde_json::Error) -> Self {
        Self::new("REMOTE_INVALID_REQUEST", error.to_string())
    }
}

fn canonical_tools() -> Vec<Tool> {
    vec![
        Tool::new(
            CANONICAL_REMOTE_OPEN_TOOL,
            "Open an owner-scoped Remote AgentSession from a RemoteBinding.",
            schema(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "binding_id": {"type": "string"},
                    "idempotency_key": {"type": "string"},
                    "initial_input": {}
                },
                "required": ["binding_id", "idempotency_key"]
            })),
        ),
        Tool::new(
            CANONICAL_REMOTE_TURN_TOOL,
            "Start one turn on an explicitly identified AgentSession.",
            schema(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "agent_session_id": {"type": "string"},
                    "input": {},
                    "idempotency_key": {"type": "string"}
                },
                "required": ["agent_session_id", "input", "idempotency_key"]
            })),
        ),
        Tool::new(
            CANONICAL_REMOTE_OBSERVE_TOOL,
            "Read canonical AgentSession events and message projections after a cursor.",
            schema(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "agent_session_id": {"type": "string"},
                    "after_cursor": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "agent_session_id": {"type": "string"},
                            "seq": {"type": "integer", "minimum": 0}
                        },
                        "required": ["agent_session_id", "seq"]
                    },
                    "limit": {"type": "integer", "minimum": 1}
                },
                "required": ["agent_session_id", "after_cursor", "limit"]
            })),
        ),
        Tool::new(
            CANONICAL_REMOTE_CANCEL_TOOL,
            "Cancel the active turn on an explicitly identified AgentSession.",
            schema(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "agent_session_id": {"type": "string"},
                    "idempotency_key": {"type": "string"}
                },
                "required": ["agent_session_id", "idempotency_key"]
            })),
        ),
    ]
}

fn schema(value: Value) -> Arc<JsonObject> {
    Arc::new(
        value
            .as_object()
            .cloned()
            .expect("canonical MCP schemas are JSON objects"),
    )
}

fn require_transport_identity(
    context: &RequestContext<RoleServer>,
) -> Result<(), rmcp::ErrorData> {
    if context.extensions.get::<RemoteMcpSessionIdentity>().is_none() {
        return Err(rmcp::ErrorData::invalid_request(
            "authenticated Remote MCP request has no server-pinned transport identity",
            None,
        ));
    }
    Ok(())
}

fn owner_from_context(
    context: &RequestContext<RoleServer>,
) -> Result<UserId, String> {
    let parts = context
        .extensions
        .get::<axum::http::request::Parts>()
        .ok_or_else(|| "authenticated Remote MCP request has no HTTP request parts".to_owned())?;
    parts
        .extensions
        .get::<RemoteInstanceOwner>()
        .map(|owner| owner.0.clone())
        .ok_or_else(|| "authenticated Remote MCP request has no owner identity".to_owned())
}

fn success_result<T: Serialize>(value: T) -> CallToolResult {
    match serde_json::to_value(value) {
        Ok(value) => build_tool_result(json!({ "result": value })),
        Err(error) => error_result("REMOTE_OPEN_FAILED", error.to_string()),
    }
}

fn error_result(code: &str, message: impl Into<String>) -> CallToolResult {
    build_tool_result(json!({
        "error": {
            "code": code,
            "message": message.into()
        }
    }))
}

fn ensure_remote_session<'a>(
    platform: &'a AgentPlatform,
    owner: &'a UserId,
    session_id: &'a AgentSessionId,
) -> impl Future<Output = Result<(), CanonicalRemoteError>> + 'a {
    async move {
        let session = platform
            .session_store()
            .get_live_session(session_id)
            .await
            .map_err(AgentPlatformError::from)
            .map_err(CanonicalRemoteError::from)?;
        if session.owner_ref != user_principal(&contract_user_id(owner))
            || session.remote_binding_provenance.is_none()
        {
            return Err(CanonicalRemoteError::new(
                "REMOTE_SESSION_NOT_FOUND",
                "AgentSession is not a Remote session owned by the authenticated installation",
            ));
        }
        Ok(())
    }
}

fn parse_session_id(value: &str) -> Result<AgentSessionId, CanonicalRemoteError> {
    validate_uuidv7(value).map_err(|_| {
        CanonicalRemoteError::new(
            "REMOTE_SESSION_NOT_FOUND",
            "agent_session_id must be a canonical UUIDv7",
        )
    })?;
    Ok(AgentSessionId::from(value.to_owned()))
}

fn nonempty(value: &str, field: &str) -> Result<String, CanonicalRemoteError> {
    if value.trim().is_empty() || value.trim() != value {
        return Err(CanonicalRemoteError::new(
            "REMOTE_INVALID_REQUEST",
            format!("{field} must be canonical and non-empty"),
        ));
    }
    Ok(value.to_owned())
}

fn validate_observe_limit(limit: u32) -> Result<(), CanonicalRemoteError> {
    if limit == 0 {
        return Err(CanonicalRemoteError::new(
            "REMOTE_INVALID_REQUEST",
            "limit must be greater than zero",
        ));
    }
    Ok(())
}

fn bounded_json(value: Value, field: &str) -> Result<Value, CanonicalRemoteError> {
    let bytes = nomifun_agent_contracts::canonical_json_bytes(&value).map_err(|error| {
        CanonicalRemoteError::new(
            "REMOTE_INVALID_REQUEST",
            format!("{field} is not canonical JSON: {error}"),
        )
    })?;
    if bytes.len() > nomifun_agent_session::MAX_INLINE_JSON_BYTES {
        return Err(CanonicalRemoteError::new(
            "REMOTE_INVALID_REQUEST",
            format!(
                "{field} exceeds the {}-byte Remote input limit",
                nomifun_agent_session::MAX_INLINE_JSON_BYTES
            ),
        ));
    }
    Ok(value)
}

fn decode_wire<T, U>(value: &U) -> Result<T, serde_json::Error>
where
    T: DeserializeOwned,
    U: Serialize,
{
    serde_json::from_value(serde_json::to_value(value)?)
}

fn contract_user_id(owner: &UserId) -> nomifun_agent_contracts::UserId {
    nomifun_agent_contracts::UserId::from(owner.as_ref().to_owned())
}

fn user_principal(owner: &nomifun_agent_contracts::UserId) -> PrincipalRef {
    PrincipalRef {
        principal_kind: "user".to_owned(),
        principal_id: owner.as_ref().to_owned(),
    }
}

fn cursor(session_id: &AgentSessionId, seq: u64) -> SessionCursorDto {
    SessionCursorDto {
        agent_session_id: session_id.as_ref().to_owned(),
        seq,
    }
}

fn open_state(status: &str) -> Result<RemoteOpenStateViewDto, CanonicalRemoteError> {
    match status {
        "opening" => Ok(RemoteOpenStateViewDto::Opening),
        "ready" | "running" => Ok(RemoteOpenStateViewDto::Ready),
        "open_failed" => Ok(RemoteOpenStateViewDto::Failed {
            code: "REMOTE_OPEN_FAILED".to_owned(),
            recoverable: true,
        }),
        "failed" => Ok(RemoteOpenStateViewDto::Failed {
            code: "REMOTE_OPEN_FAILED".to_owned(),
            recoverable: false,
        }),
        other => Err(CanonicalRemoteError::new(
            "REMOTE_OPEN_FAILED",
            format!("AgentSession has unsupported open state {other:?}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_tools_are_exactly_the_four_remote_operations() {
        let names = canonical_tools()
            .into_iter()
            .map(|tool| tool.name.into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, ["open", "turn", "observe", "cancel"]);
    }

    #[test]
    fn canonical_open_state_maps_running_to_ready() {
        assert_eq!(
            open_state("running").unwrap(),
            RemoteOpenStateViewDto::Ready
        );
    }

    #[test]
    fn invalid_session_ids_fail_closed() {
        assert!(parse_session_id("not-a-session").is_err());
    }

    #[test]
    fn observe_limit_zero_is_rejected_before_platform_access() {
        let error = validate_observe_limit(0).expect_err("zero observe limit must fail");
        assert_eq!(error.code, "REMOTE_INVALID_REQUEST");
    }
}
