//! Engine-neutral Remote MCP transport with host-injected product operations.
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
use nomifun_api_types::{
    RemoteCancelRequestDto, RemoteObserveRequestDto, RemoteOpenRequestDto, RemoteTurnRequestDto,
};
use nomifun_auth::InstanceTokenValidator;
use nomifun_common::UserId;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, JsonObject, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use serde::Serialize;
use serde::de::DeserializeOwned;
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

/// The boxed future returned by a [`CanonicalRemoteOperations`] implementation.
pub type CanonicalRemoteOperationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Value, CanonicalRemoteOperationError>> + Send + 'a>>;

/// Typed errors returned by an injected canonical Remote operation handler.
///
/// The error is kept as ordinary tool data instead of being collapsed into an
/// MCP transport error, so callers can reliably consume the stable `code`,
/// human-readable `message`, and optional structured `details` fields.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalRemoteOperationError {
    pub code: String,
    pub message: String,
    pub details: Option<Value>,
}

impl CanonicalRemoteOperationError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(
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

impl std::fmt::Display for CanonicalRemoteOperationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CanonicalRemoteOperationError {}

/// Host-provided implementation of the four canonical Remote operations.
///
/// The transport owns authentication, MCP session admission, and the fixed
/// tool schemas. Implementations only own product behavior and receive the
/// authenticated installation owner explicitly.
pub trait CanonicalRemoteOperations: Send + Sync {
    fn open<'a>(
        &'a self,
        owner: &'a UserId,
        request: RemoteOpenRequestDto,
    ) -> CanonicalRemoteOperationFuture<'a>;

    fn turn<'a>(
        &'a self,
        owner: &'a UserId,
        request: RemoteTurnRequestDto,
    ) -> CanonicalRemoteOperationFuture<'a>;

    fn observe<'a>(
        &'a self,
        owner: &'a UserId,
        request: RemoteObserveRequestDto,
    ) -> CanonicalRemoteOperationFuture<'a>;

    fn cancel<'a>(
        &'a self,
        owner: &'a UserId,
        request: RemoteCancelRequestDto,
    ) -> CanonicalRemoteOperationFuture<'a>;
}

#[derive(Clone)]
pub struct CanonicalRemoteMcpHandler {
    operations: Arc<dyn CanonicalRemoteOperations>,
}

impl CanonicalRemoteMcpHandler {
    /// Construct a transport handler around an injected operation provider.
    pub fn with_operations(operations: Arc<dyn CanonicalRemoteOperations>) -> Self {
        Self { operations }
    }
}

/// Build the canonical Remote MCP front door around host-injected operations.
///
/// This keeps the same Streamable HTTP transport, installation-token
/// middleware, transport-session admission, and four fixed tool schemas,
/// without requiring a particular Engine, Session implementation, or legacy
/// platform dependency.
pub fn canonical_remote_mcp_router_with_operations(
    operations: Arc<dyn CanonicalRemoteOperations>,
    validator: Arc<InstanceTokenValidator>,
    authoritative_user_id: UserId,
) -> Router {
    let transport_admission = RemoteMcpSessionAdmissionAuthority::for_owner(&authoritative_user_id);
    let sessions = Arc::new(RemoteSessionManager::with_owner_admission_authority(
        authoritative_user_id.clone(),
        transport_admission,
    ));
    let service = rmcp::transport::streamable_http_server::StreamableHttpService::new(
        {
            let operations = Arc::clone(&operations);
            move || {
                Ok(CanonicalRemoteMcpHandler::with_operations(Arc::clone(
                    &operations,
                )))
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
                    validator,
                    authoritative_user_id,
                },
                sessions,
            },
            mcp_instance_token_middleware,
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
                decode_and_run::<RemoteOpenRequestDto, _, _>(arguments, |request| {
                    self.operations.open(&owner, request)
                })
                .await
            }
            CANONICAL_REMOTE_TURN_TOOL => {
                decode_and_run::<RemoteTurnRequestDto, _, _>(arguments, |request| {
                    self.operations.turn(&owner, request)
                })
                .await
            }
            CANONICAL_REMOTE_OBSERVE_TOOL => {
                decode_and_run::<RemoteObserveRequestDto, _, _>(arguments, |request| {
                    self.operations.observe(&owner, request)
                })
                .await
            }
            CANONICAL_REMOTE_CANCEL_TOOL => {
                decode_and_run::<RemoteCancelRequestDto, _, _>(arguments, |request| {
                    self.operations.cancel(&owner, request)
                })
                .await
            }
            _ => Err(CanonicalRemoteOperationError::new(
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

async fn decode_and_run<T, F, Fut>(value: Value, run: F) -> Result<Value, CanonicalRemoteError>
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

type CanonicalRemoteError = CanonicalRemoteOperationError;

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

fn require_transport_identity(context: &RequestContext<RoleServer>) -> Result<(), rmcp::ErrorData> {
    if context
        .extensions
        .get::<RemoteMcpSessionIdentity>()
        .is_none()
    {
        return Err(rmcp::ErrorData::invalid_request(
            "authenticated Remote MCP request has no server-pinned transport identity",
            None,
        ));
    }
    Ok(())
}

fn owner_from_context(context: &RequestContext<RoleServer>) -> Result<UserId, String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    #[derive(Default)]
    struct InjectedOperations {
        calls: Arc<AtomicUsize>,
    }

    impl CanonicalRemoteOperations for InjectedOperations {
        fn open<'a>(
            &'a self,
            owner: &'a UserId,
            request: RemoteOpenRequestDto,
        ) -> CanonicalRemoteOperationFuture<'a> {
            let calls = Arc::clone(&self.calls);
            let owner = owner.as_ref().to_owned();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::AcqRel);
                Ok(json!({
                    "operation": CANONICAL_REMOTE_OPEN_TOOL,
                    "owner": owner,
                    "binding_id": request.binding_id,
                }))
            })
        }

        fn turn<'a>(
            &'a self,
            owner: &'a UserId,
            request: RemoteTurnRequestDto,
        ) -> CanonicalRemoteOperationFuture<'a> {
            let calls = Arc::clone(&self.calls);
            let owner = owner.as_ref().to_owned();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::AcqRel);
                Ok(json!({
                    "operation": CANONICAL_REMOTE_TURN_TOOL,
                    "owner": owner,
                    "idempotency_key": request.idempotency_key,
                }))
            })
        }

        fn observe<'a>(
            &'a self,
            owner: &'a UserId,
            request: RemoteObserveRequestDto,
        ) -> CanonicalRemoteOperationFuture<'a> {
            let calls = Arc::clone(&self.calls);
            let owner = owner.as_ref().to_owned();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::AcqRel);
                Ok(json!({
                    "operation": CANONICAL_REMOTE_OBSERVE_TOOL,
                    "owner": owner,
                    "agent_session_id": request.agent_session_id,
                }))
            })
        }

        fn cancel<'a>(
            &'a self,
            owner: &'a UserId,
            request: RemoteCancelRequestDto,
        ) -> CanonicalRemoteOperationFuture<'a> {
            let calls = Arc::clone(&self.calls);
            let owner = owner.as_ref().to_owned();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::AcqRel);
                Err(CanonicalRemoteOperationError::with_details(
                    "TEST_CANCELLED",
                    format!("cancelled for {}", owner),
                    json!({ "idempotency_key": request.idempotency_key }),
                ))
            })
        }
    }

    async fn dispatch_mcp(
        router: &Router,
        session_id: Option<&str>,
        body: Value,
    ) -> (StatusCode, axum::http::HeaderMap, String) {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .header("authorization", "Bearer test-token")
            .header("host", "localhost")
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json");
        if let Some(session_id) = session_id {
            builder = builder.header("mcp-session-id", session_id);
            builder = builder.header("mcp-protocol-version", "2025-06-18");
        }
        let response = router
            .clone()
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .expect("dispatch MCP request");
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), nomifun_common::constants::BODY_LIMIT)
            .await
            .expect("read MCP response");
        (
            status,
            headers,
            String::from_utf8(body.to_vec()).expect("MCP response is UTF-8"),
        )
    }

    fn response_json(body: &str) -> Value {
        if let Ok(value) = serde_json::from_str(body) {
            return value;
        }
        let data = body
            .lines()
            .find_map(|line| line.strip_prefix("data: ").filter(|data| !data.is_empty()))
            .expect("SSE response contains a data event");
        serde_json::from_str(data)
            .unwrap_or_else(|error| panic!("SSE data is JSON: {error}; body={body:?}"))
    }

    #[test]
    fn canonical_tools_are_exactly_the_four_remote_operations() {
        let names = canonical_tools()
            .into_iter()
            .map(|tool| tool.name.into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, ["open", "turn", "observe", "cancel"]);
    }

    #[test]
    fn operation_error_constructors_preserve_typed_fields() {
        let simple = CanonicalRemoteOperationError::new("TEST_CODE", "test message");
        assert_eq!(simple.code, "TEST_CODE");
        assert_eq!(simple.message, "test message");
        assert_eq!(simple.details, None);

        let detailed = CanonicalRemoteOperationError::with_details(
            "TEST_DETAILED",
            "detailed message",
            json!({"retryable": true}),
        );
        assert_eq!(detailed.code, "TEST_DETAILED");
        assert_eq!(detailed.message, "detailed message");
        assert_eq!(detailed.details, Some(json!({"retryable": true})));
    }

    #[tokio::test]
    async fn injected_operations_are_used_by_list_tools_and_call_tool() {
        let operations = Arc::new(InjectedOperations::default());
        let owner = UserId::new();
        let router = canonical_remote_mcp_router_with_operations(
            operations.clone(),
            Arc::new(InstanceTokenValidator::new(Some(
                nomifun_auth::token_sha256_hex("test-token"),
            ))),
            owner.clone(),
        );

        let (status, headers, body) = dispatch_mcp(
            &router,
            None,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "nomifun-public-test", "version": "1"}
                }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "initialize response: {body}");
        let session_id = headers
            .get("mcp-session-id")
            .expect("initialize returns an MCP session id")
            .to_str()
            .expect("session id is valid UTF-8")
            .to_owned();
        let initialize = response_json(&body);
        assert!(initialize.get("result").is_some());

        let (status, _, _) = dispatch_mcp(
            &router,
            Some(&session_id),
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        let (status, _, body) = dispatch_mcp(
            &router,
            Some(&session_id),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let list = response_json(&body);
        let names = list["result"]["tools"]
            .as_array()
            .expect("tools/list returns tools")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name").to_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, ["open", "turn", "observe", "cancel"]);

        let (status, _, body) = dispatch_mcp(
            &router,
            Some(&session_id),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "turn",
                    "arguments": {
                        "agent_session_id": UserId::new().as_ref(),
                        "input": {"hello": "world"},
                        "idempotency_key": "injected-turn"
                    }
                }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let call = response_json(&body);
        assert_ne!(call["result"]["isError"], true);
        let operation_result: Value = serde_json::from_str(
            call["result"]["content"][0]["text"]
                .as_str()
                .expect("tool result has text"),
        )
        .expect("injected operation result is JSON");
        assert_eq!(operation_result["operation"], CANONICAL_REMOTE_TURN_TOOL);
        assert_eq!(operation_result["owner"], owner.as_ref());
        assert_eq!(operation_result["idempotency_key"], "injected-turn");
        assert_eq!(operations.calls.load(Ordering::Acquire), 1);
    }
}
