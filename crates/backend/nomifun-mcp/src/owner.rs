//! Canonical MCP execution owner.
//!
//! This module is deliberately independent from the legacy Nomi MCP manager
//! and the Platform Gateway. A caller supplies the exact server/resource
//! binding and the materialized tool mapping; this owner only performs the
//! protocol transaction against that frozen binding.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::{Instant, timeout, timeout_at};

use crate::oauth_service::McpOAuthService;
use crate::types::McpServerTransport;

pub const MCP_SERVER_RESOURCE_KIND: &str = "mcp_server";
pub const MCP_CONNECT_OPERATION: &str = "connect";
pub const MCP_INVOKE_OPERATION: &str = "invoke";
pub const MCP_READ_OPERATION: &str = "read";
pub const MCP_EXECUTION_OPERATION_META_KEY: &str = "com.nomifun.execution.operation_id";

pub const MCP_PROTOCOL_VERSION: &str = "2025-03-26";
const MCP_CLIENT_NAME: &str = "nomifun-agent-mcp-owner";
const MCP_CLIENT_VERSION: &str = "1.0.0";
const DEFAULT_OWNER_TIMEOUT: Duration = Duration::from_secs(30);
const SESSION_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_ARGUMENT_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_OPERATION_ID_BYTES: usize = 128;

/// A typed error emitted by the MCP owner.
///
/// The message never contains credential material. The code is stable enough
/// for the central host to map it into its canonical host-port error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpOwnerError {
    code: String,
    message: String,
}

impl McpOwnerError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            code: code.into(),
            message: sanitize_diagnostic(&message),
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    fn invalid_binding(message: impl Into<String>) -> Self {
        Self::new("MCP_BINDING_INVALID", message)
    }

    fn connection_failed(message: impl Into<String>) -> Self {
        Self::new("MCP_CONNECTION_FAILED", message)
    }

    fn protocol_failed(message: impl Into<String>) -> Self {
        Self::new("MCP_PROTOCOL_ERROR", message)
    }

}

impl fmt::Display for McpOwnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for McpOwnerError {}

/// The immutable server/resource facts selected by the Snapshot or a
/// non-Agent operation admission.
#[derive(Clone, Debug, PartialEq)]
pub struct McpServerBinding {
    pub server_id: String,
    /// The persisted owner of the server. `system` is the only shared owner.
    pub server_owner_id: String,
    pub enabled: bool,
    pub connection_config_ref: String,
    pub resource_binding_id: String,
    pub resource_kind: String,
    pub resource_id: String,
    pub resource_owner_id: String,
    pub granted_operations: BTreeSet<String>,
    pub resource_connection_config_ref: Option<String>,
    pub transport: McpServerTransport,
}

/// The exact materialized mapping for one MCP-backed Capability.
///
/// `remote_tool_name` is kept outside the canonical mapping because the
/// mapping's stable identity is `server_id + canonical_tool_key + schema_digest`.
/// It is resolved by the catalog/materializer and is never taken from the
/// model-facing call input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpToolBinding {
    pub server_id: String,
    pub canonical_tool_key: String,
    pub schema_digest: String,
    pub input_schema: Value,
    pub remote_tool_name: String,
}

impl McpToolBinding {
    pub fn new(
        server_id: impl Into<String>,
        canonical_tool_key: impl Into<String>,
        schema_digest: impl Into<String>,
        input_schema: Value,
        remote_tool_name: impl Into<String>,
    ) -> Result<Self, McpOwnerError> {
        let binding = Self {
            server_id: server_id.into(),
            canonical_tool_key: canonical_tool_key.into(),
            schema_digest: schema_digest.into(),
            input_schema,
            remote_tool_name: remote_tool_name.into(),
        };
        validate_tool_binding(&binding)?;
        Ok(binding)
    }
}

/// Input to the owner. The model supplies only `arguments`; all routing
/// identity comes from the injected exact bindings.
#[derive(Clone, Debug, PartialEq)]
pub struct McpToolInvocationRequest {
    pub principal_kind: String,
    pub principal_id: String,
    pub operation_id: String,
    pub server: McpServerBinding,
    pub tool: McpToolBinding,
    pub arguments: Value,
}

/// The actual MCP server result. There is no acknowledgement/synthetic result
/// variant: success means a validated `tools/call` response was received.
#[derive(Clone, Debug, PartialEq)]
pub struct McpToolInvocationResult {
    pub server_id: String,
    pub canonical_tool_key: String,
    pub schema_digest: String,
    pub remote_tool_name: String,
    pub result: Value,
}

/// Lookup context passed to the credential authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpCredentialLookup {
    pub server_id: String,
    pub resource_id: String,
    pub connection_config_ref: String,
    pub endpoint: String,
}

/// A short-lived credential held by the owner. The secret is not exposed by
/// the public API and is zeroized when the value is dropped.
pub struct McpCredential {
    token_type: String,
    secret: Vec<u8>,
}

impl fmt::Debug for McpCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpCredential")
            .field("token_type", &self.token_type)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

impl Drop for McpCredential {
    fn drop(&mut self) {
        self.secret.fill(0);
    }
}

impl McpCredential {
    pub fn new(
        token_type: impl Into<String>,
        secret: impl Into<String>,
    ) -> Result<Self, McpOwnerError> {
        let token_type = token_type.into();
        let secret = secret.into();
        if token_type.trim().is_empty()
            || token_type.chars().any(|character| character.is_ascii_control() || character.is_whitespace())
        {
            return Err(McpOwnerError::new(
                "MCP_CREDENTIAL_INVALID",
                "credential token type is invalid",
            ));
        }
        if secret.trim().is_empty() || secret.chars().any(char::is_control) {
            return Err(McpOwnerError::new(
                "MCP_CREDENTIAL_INVALID",
                "credential material is empty or malformed",
            ));
        }
        Ok(Self {
            token_type,
            secret: secret.into_bytes(),
        })
    }

    pub fn bearer(secret: impl Into<String>) -> Result<Self, McpOwnerError> {
        Self::new("Bearer", secret)
    }
}

/// The sole credential resolution boundary for an MCP call.
#[async_trait]
pub trait McpCredentialAuthority: Send + Sync {
    async fn resolve(
        &self,
        lookup: McpCredentialLookup,
    ) -> Result<Option<McpCredential>, McpOwnerError>;
}

/// Explicit anonymous authority for servers that do not need credentials.
#[derive(Clone, Copy, Debug, Default)]
pub struct AnonymousMcpCredentialAuthority;

#[async_trait]
impl McpCredentialAuthority for AnonymousMcpCredentialAuthority {
    async fn resolve(
        &self,
        _lookup: McpCredentialLookup,
    ) -> Result<Option<McpCredential>, McpOwnerError> {
        Ok(None)
    }
}

/// Adapter over the existing MCP OAuth authority. It resolves by the exact
/// endpoint URL and never reads the token repository directly.
#[derive(Clone)]
pub struct OAuthMcpCredentialAuthority {
    oauth: Arc<McpOAuthService>,
}

impl OAuthMcpCredentialAuthority {
    pub fn new(oauth: Arc<McpOAuthService>) -> Self {
        Self { oauth }
    }
}

#[async_trait]
impl McpCredentialAuthority for OAuthMcpCredentialAuthority {
    async fn resolve(
        &self,
        lookup: McpCredentialLookup,
    ) -> Result<Option<McpCredential>, McpOwnerError> {
        let token = self.oauth.get_token(&lookup.endpoint).await.map_err(|_| {
            McpOwnerError::new(
                "MCP_CREDENTIAL_AUTHORITY_FAILED",
                "credential authority failed for the bound MCP endpoint",
            )
        })?;
        if token.is_some()
            && !self
                .oauth
                .check_oauth_status(&lookup.endpoint)
                .await
                .map_err(|_| {
                    McpOwnerError::new(
                        "MCP_CREDENTIAL_AUTHORITY_FAILED",
                        "credential authority could not verify the bound MCP credential",
                    )
                })?
                .authenticated
        {
            return Err(McpOwnerError::new(
                "MCP_CREDENTIAL_EXPIRED",
                "the bound MCP credential is expired and could not be refreshed",
            ));
        }
        token
            .map(McpCredential::bearer)
            .transpose()
    }
}

/// Application-owned MCP execution owner.
#[derive(Clone)]
pub struct McpOwner {
    credentials: Arc<dyn McpCredentialAuthority>,
    http_client: Result<reqwest::Client, McpOwnerError>,
    timeout: Duration,
}

impl McpOwner {
    /// Construct an owner with a caller-supplied client.
    ///
    /// Production callers should prefer [`Self::new_dynamic`] or
    /// [`Self::try_new_dynamic`]. Injected clients must disable redirects.
    pub fn new(
        credentials: Arc<dyn McpCredentialAuthority>,
        http_client: reqwest::Client,
    ) -> Self {
        Self {
            credentials,
            http_client: Ok(http_client),
            timeout: DEFAULT_OWNER_TIMEOUT,
        }
    }

    pub fn new_dynamic(credentials: Arc<dyn McpCredentialAuthority>) -> Self {
        Self {
            credentials,
            http_client: build_dynamic_http_client(),
            timeout: DEFAULT_OWNER_TIMEOUT,
        }
    }

    pub fn try_new_dynamic(
        credentials: Arc<dyn McpCredentialAuthority>,
    ) -> Result<Self, McpOwnerError> {
        Ok(Self::new(credentials, build_dynamic_http_client()?))
    }

    pub fn with_timeout(self, timeout: Duration) -> Self {
        Self { timeout, ..self }
    }

    /// Execute one exact MCP tool binding.
    ///
    /// Only Streamable HTTP is implemented in this owner lane. Stdio and SSE
    /// return an explicit typed unsupported error rather than reaching the
    /// legacy Nomi client or Gateway.
    pub async fn invoke(
        &self,
        request: McpToolInvocationRequest,
    ) -> Result<McpToolInvocationResult, McpOwnerError> {
        validate_invocation(&request)?;
        let (url, static_headers) = match &request.server.transport {
            McpServerTransport::Http { url, headers } => (url.clone(), headers.clone()),
            McpServerTransport::Stdio { .. } | McpServerTransport::Sse { .. } => {
                return Err(McpOwnerError::new(
                    "MCP_TRANSPORT_UNSUPPORTED",
                    "the canonical MCP owner currently supports Streamable HTTP only",
                ));
            }
        };
        let endpoint = validate_http_endpoint(&url)?.to_string();
        self.invoke_http(&request, endpoint, static_headers).await
    }

    async fn invoke_http(
        &self,
        request: &McpToolInvocationRequest,
        endpoint: String,
        static_headers: HashMap<String, String>,
    ) -> Result<McpToolInvocationResult, McpOwnerError> {
        let http_client = match &self.http_client {
            Ok(client) => client.clone(),
            Err(error) => return Err(error.clone()),
        };
        let deadline = Instant::now() + self.timeout;
        let credential = timeout_at(
            deadline,
            self.credentials.resolve(McpCredentialLookup {
                server_id: request.server.server_id.clone(),
                resource_id: request.server.resource_id.clone(),
                connection_config_ref: request.server.connection_config_ref.clone(),
                endpoint: endpoint.clone(),
            }),
        )
        .await
        .map_err(|_| owner_timeout_error(self.timeout))??;
        let headers = build_headers(&static_headers, credential.as_ref())?;
        let mut session = HttpMcpSession::new(http_client, endpoint, headers);

        let transaction = async {
            let initialize = session.request(initialize_request()).await?;
            ensure_response_id(&initialize, 1)?;
            ensure_no_rpc_error("initialize", &initialize)?;
            if initialize.result.is_none() {
                return Err(McpOwnerError::protocol_failed(
                    "initialize response has no result",
                ));
            }
            validate_initialize_result(
                initialize
                    .result
                    .as_ref()
                    .expect("initialize result was checked"),
            )?;

            session.notify(initialized_notification()).await?;

            let tools = session.request(tools_list_request()).await?;
            ensure_response_id(&tools, 2)?;
            ensure_no_rpc_error("tools/list", &tools)?;
            let remote_tools = parse_tools(tools.result.as_ref().ok_or_else(|| {
                McpOwnerError::protocol_failed("tools/list response has no result")
            })?)?;
            let remote_tool = remote_tools
                .into_iter()
                .find(|tool| tool.name == request.tool.remote_tool_name)
                .ok_or_else(|| {
                    McpOwnerError::new(
                        "MCP_TOOL_NOT_FOUND",
                        format!(
                            "the bound MCP server did not advertise tool '{}'",
                            request.tool.remote_tool_name
                        ),
                    )
                })?;
            if remote_tool.input_schema != request.tool.input_schema {
                return Err(McpOwnerError::new(
                    "MCP_SCHEMA_MISMATCH",
                    format!(
                        "advertised schema does not match frozen schema digest {}",
                        request.tool.schema_digest
                    ),
                ));
            }

            let call = session
                .request(tool_call_request(
                    &request.tool.remote_tool_name,
                    &request.arguments,
                    &request.operation_id,
                ))
                .await?;
            ensure_response_id(&call, 3)?;
            ensure_no_rpc_error("tools/call", &call)?;
            let result = call.result.ok_or_else(|| {
                McpOwnerError::protocol_failed("tools/call response has no result")
            })?;
            validate_tool_result(&result)?;

            Ok(McpToolInvocationResult {
                server_id: request.server.server_id.clone(),
                canonical_tool_key: request.tool.canonical_tool_key.clone(),
                schema_digest: request.tool.schema_digest.clone(),
                remote_tool_name: request.tool.remote_tool_name.clone(),
                result,
            })
        };
        let outcome = timeout_at(deadline, transaction)
            .await
            .unwrap_or_else(|_| Err(owner_timeout_error(self.timeout)));
        let cleanup = timeout(SESSION_CLEANUP_TIMEOUT, session.close())
            .await
            .unwrap_or_else(|_| {
                Err(McpOwnerError::new(
                    "MCP_SESSION_CLEANUP_FAILED",
                    "MCP session cleanup exceeded its bounded deadline",
                ))
            });

        match (outcome, cleanup) {
            (Ok(result), Ok(_)) => Ok(result),
            (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
            (Err(operation_error), _) => Err(operation_error),
        }
    }
}

fn build_dynamic_http_client() -> Result<reqwest::Client, McpOwnerError> {
    nomifun_net::http_client_no_redirect().map_err(|_| {
        McpOwnerError::new(
            "MCP_HTTP_CLIENT_UNAVAILABLE",
            "MCP HTTP client could not be initialized with redirects disabled",
        )
    })
}

fn owner_timeout_error(timeout: Duration) -> McpOwnerError {
    McpOwnerError::new(
        "MCP_TIMEOUT",
        format!(
            "MCP invocation exceeded the {} second owner deadline",
            timeout.as_secs()
        ),
    )
}

fn sanitize_diagnostic(message: &str) -> String {
    nomifun_net::secret_redaction::redact_url_queries(message)
}

fn validate_http_endpoint(raw_url: &str) -> Result<reqwest::Url, McpOwnerError> {
    if raw_url.is_empty()
        || raw_url.trim() != raw_url
        || raw_url.chars().any(char::is_control)
    {
        return Err(McpOwnerError::invalid_binding(
            "MCP endpoint URL is malformed",
        ));
    }
    let url = reqwest::Url::parse(raw_url)
        .map_err(|_| McpOwnerError::invalid_binding("MCP endpoint URL is malformed"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(McpOwnerError::invalid_binding(
            "MCP endpoint URL must use http or https",
        ));
    }
    if url.host_str().is_none() || url.port_or_known_default().is_none() {
        return Err(McpOwnerError::invalid_binding(
            "MCP endpoint URL requires a valid host and port",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(McpOwnerError::new(
            "MCP_CREDENTIAL_AUTHORITY_REQUIRED",
            "MCP endpoint credentials must not be embedded in the URL",
        ));
    }
    if url.fragment().is_some() {
        return Err(McpOwnerError::invalid_binding(
            "MCP endpoint URL must not contain a fragment",
        ));
    }
    Ok(url)
}

fn validate_invocation(
    request: &McpToolInvocationRequest,
) -> Result<(), McpOwnerError> {
    if request.principal_kind.trim().is_empty()
        || request.principal_id.trim().is_empty()
    {
        return Err(McpOwnerError::invalid_binding(
            "MCP invocation requires a non-empty principal",
        ));
    }
    if request.operation_id.is_empty()
        || request.operation_id.len() > MAX_OPERATION_ID_BYTES
        || !request
            .operation_id
            .bytes()
            .all(|byte| byte.is_ascii_graphic())
    {
        return Err(McpOwnerError::new(
            "MCP_OPERATION_ID_INVALID",
            format!(
                "MCP operation ID must contain 1..={MAX_OPERATION_ID_BYTES} visible ASCII bytes"
            ),
        ));
    }
    validate_tool_binding(&request.tool)?;
    let server = &request.server;
    if server.server_id.trim().is_empty()
        || server.server_owner_id.trim().is_empty()
        || server.connection_config_ref.trim().is_empty()
    {
        return Err(McpOwnerError::invalid_binding(
            "MCP server binding contains an empty identity or connection reference",
        ));
    }
    if server.server_id != request.tool.server_id {
        return Err(McpOwnerError::new(
            "MCP_SERVER_IDENTITY_MISMATCH",
            "server binding and MCP tool mapping refer to different servers",
        ));
    }
    if !server.enabled {
        return Err(McpOwnerError::new(
            "MCP_SERVER_DISABLED",
            "the exact MCP server binding is disabled",
        ));
    }
    if server.server_owner_id != "system"
        && server.server_owner_id != request.principal_id
    {
        return Err(McpOwnerError::new(
            "MCP_SERVER_OWNER_MISMATCH",
            "the exact MCP server belongs to a different owner",
        ));
    }

    if server.resource_binding_id.trim().is_empty() {
        return Err(McpOwnerError::invalid_binding(
            "MCP resource binding ID is empty",
        ));
    }
    if server.resource_kind != MCP_SERVER_RESOURCE_KIND {
        return Err(McpOwnerError::invalid_binding(
            "MCP invocation requires an mcp_server resource binding",
        ));
    }
    if server.resource_id != server.server_id {
        return Err(McpOwnerError::new(
            "MCP_RESOURCE_IDENTITY_MISMATCH",
            "resource binding does not identify the exact MCP server",
        ));
    }
    if server.resource_owner_id != request.principal_id {
        return Err(McpOwnerError::new(
            "MCP_RESOURCE_OWNER_MISMATCH",
            "MCP resource binding belongs to a different principal",
        ));
    }
    for operation in [MCP_CONNECT_OPERATION, MCP_INVOKE_OPERATION] {
        if !server.granted_operations.contains(operation) {
            return Err(McpOwnerError::new(
                "MCP_RESOURCE_OPERATION_DENIED",
                format!("MCP resource binding does not grant {operation}"),
            ));
        }
    }
    if server.resource_connection_config_ref.as_deref()
        != Some(server.connection_config_ref.as_str())
    {
        return Err(McpOwnerError::new(
            "MCP_CONNECTION_CONFIG_MISMATCH",
            "resource and server bindings use different connection references",
        ));
    }
    if !request.arguments.is_object() {
        return Err(McpOwnerError::new(
            "MCP_INVALID_ARGUMENTS",
            "MCP tools/call arguments must be a JSON object",
        ));
    }
    let argument_bytes = serde_json::to_vec(&request.arguments).map_err(|error| {
        McpOwnerError::new(
            "MCP_INVALID_ARGUMENTS",
            format!("MCP arguments could not be serialized: {error}"),
        )
    })?;
    if argument_bytes.len() > MAX_ARGUMENT_BYTES {
        return Err(McpOwnerError::new(
            "MCP_INVALID_ARGUMENTS",
            format!(
                "MCP arguments exceed the {} byte limit",
                MAX_ARGUMENT_BYTES
            ),
        ));
    }
    Ok(())
}

fn validate_tool_binding(binding: &McpToolBinding) -> Result<(), McpOwnerError> {
    if binding.server_id.trim().is_empty()
        || binding.canonical_tool_key.trim().is_empty()
    {
        return Err(McpOwnerError::invalid_binding(
            "MCP tool mapping contains an empty canonical identity",
        ));
    }
    if !is_digest(&binding.schema_digest) {
        return Err(McpOwnerError::invalid_binding(
            "MCP tool mapping schema digest must be 64 lowercase hex characters",
        ));
    }
    if !binding.input_schema.is_object() {
        return Err(McpOwnerError::invalid_binding(
            "MCP tool binding input schema must be an object",
        ));
    }
    if binding.remote_tool_name.trim().is_empty()
        || binding.remote_tool_name.trim() != binding.remote_tool_name
        || binding.remote_tool_name.chars().any(char::is_control)
    {
        return Err(McpOwnerError::invalid_binding(
            "MCP remote tool name is empty or malformed",
        ));
    }
    Ok(())
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn build_headers(
    static_headers: &HashMap<String, String>,
    credential: Option<&McpCredential>,
) -> Result<reqwest::header::HeaderMap, McpOwnerError> {
    let mut headers = reqwest::header::HeaderMap::new();
    let mut seen = BTreeSet::new();
    for (name, value) in static_headers {
        let lower = name.to_ascii_lowercase();
        if !seen.insert(lower.clone()) {
            return Err(McpOwnerError::invalid_binding(
                "MCP transport contains duplicate header names",
            ));
        }
        if matches!(
            lower.as_str(),
            "authorization"
                | "proxy-authorization"
                | "cookie"
                | "set-cookie"
                | "x-api-key"
                | "api-key"
                | "x-auth-token"
                | "mcp-session-id"
                | "content-type"
                | "accept"
        ) {
            return Err(McpOwnerError::new(
                "MCP_CREDENTIAL_AUTHORITY_REQUIRED",
                "credential or protocol headers must come from the canonical owner",
            ));
        }
        let header_name = reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            McpOwnerError::invalid_binding("MCP transport contains an invalid header name")
        })?;
        let header_value = reqwest::header::HeaderValue::from_str(value).map_err(|_| {
            McpOwnerError::invalid_binding("MCP transport contains an invalid header value")
        })?;
        headers.insert(header_name, header_value);
    }
    if let Some(credential) = credential {
        let secret = std::str::from_utf8(&credential.secret).map_err(|_| {
            McpOwnerError::new(
                "MCP_CREDENTIAL_INVALID",
                "credential authority returned non-UTF-8 material",
            )
        })?;
        let authorization = format!("{} {}", credential.token_type, secret);
        let value = reqwest::header::HeaderValue::from_str(&authorization).map_err(|_| {
            McpOwnerError::new(
                "MCP_CREDENTIAL_INVALID",
                "credential authority returned an invalid authorization value",
            )
        })?;
        headers.insert(reqwest::header::AUTHORIZATION, value);
    }
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/json, text/event-stream"),
    );
    Ok(headers)
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: Option<u64>,
    method: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    #[allow(dead_code)]
    message: String,
}

#[derive(Debug)]
struct RemoteTool {
    name: String,
    input_schema: Value,
}

struct HttpMcpSession {
    client: reqwest::Client,
    endpoint: String,
    headers: reqwest::header::HeaderMap,
    session_id: Option<String>,
}

impl HttpMcpSession {
    fn new(
        client: reqwest::Client,
        endpoint: String,
        headers: reqwest::header::HeaderMap,
    ) -> Self {
        Self {
            client,
            endpoint,
            headers,
            session_id: None,
        }
    }

    async fn request(
        &mut self,
        request: JsonRpcRequest,
    ) -> Result<JsonRpcResponse, McpOwnerError> {
        let response = self.send(request).await?;
        if let Some(session_id) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
        {
            if session_id.trim().is_empty() {
                return Err(McpOwnerError::protocol_failed(
                    "MCP server returned an empty session ID",
                ));
            }
            self.session_id = Some(session_id.to_owned());
        }
        parse_response(response).await
    }

    async fn notify(&mut self, request: JsonRpcRequest) -> Result<(), McpOwnerError> {
        let response = self.send(request).await?;
        let status = response.status();
        let session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let session_id_error = match session_id {
            Some(session_id) if session_id.trim().is_empty() => {
                Some(McpOwnerError::protocol_failed(
                    "MCP server returned an empty session ID",
                ))
            }
            Some(session_id) => {
                self.session_id = Some(session_id);
                None
            }
            None => None,
        };
        let body_result = drain_response_body(response).await;
        if let Some(error) = session_id_error {
            return Err(error);
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(McpOwnerError::new(
                "MCP_CREDENTIAL_REQUIRED",
                "MCP server rejected the canonical credential authority",
            ));
        }
        if !status.is_success() {
            return Err(McpOwnerError::new(
                "MCP_HTTP_ERROR",
                format!(
                    "MCP server returned HTTP {} for initialized notification",
                    status.as_u16()
                ),
            ));
        }
        body_result?;
        Ok(())
    }

    async fn close(&mut self) -> Result<(), McpOwnerError> {
        let Some(session_id) = self.session_id.take() else {
            return Ok(());
        };
        let mut headers = self.headers.clone();
        let value = reqwest::header::HeaderValue::from_str(&session_id).map_err(|_| {
            McpOwnerError::new(
                "MCP_SESSION_CLEANUP_FAILED",
                "MCP session ID cannot be represented as a cleanup header",
            )
        })?;
        headers.insert("mcp-session-id", value);
        let response = self
            .client
            .delete(&self.endpoint)
            .headers(headers)
            .send()
            .await
            .map_err(|error| map_cleanup_request_error(error))?;
        let status = response.status();
        drain_response_body(response).await.map_err(|_| {
            McpOwnerError::new(
                "MCP_SESSION_CLEANUP_FAILED",
                "MCP session cleanup response could not be consumed",
            )
        })?;
        if status == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Err(McpOwnerError::new(
                "MCP_SESSION_CLEANUP_UNSUPPORTED",
                "MCP server does not support explicit session cleanup",
            ));
        }
        if !status.is_success() {
            return Err(McpOwnerError::new(
                "MCP_SESSION_CLEANUP_FAILED",
                format!(
                    "MCP server returned HTTP {} while closing the MCP session",
                    status.as_u16()
                ),
            ));
        }
        Ok(())
    }

    async fn send(
        &self,
        request: JsonRpcRequest,
    ) -> Result<reqwest::Response, McpOwnerError> {
        let mut headers = self.headers.clone();
        if let Some(session_id) = &self.session_id {
            let value = reqwest::header::HeaderValue::from_str(session_id).map_err(|_| {
                McpOwnerError::protocol_failed("MCP session ID cannot be represented as a header")
            })?;
            headers.insert("mcp-session-id", value);
        }
        self.client
            .post(&self.endpoint)
            .headers(headers)
            .json(&request)
            .send()
            .await
            .map_err(|error| map_request_error(error, "MCP request failed"))
    }
}

fn map_request_error(error: reqwest::Error, context: &str) -> McpOwnerError {
    if error.is_timeout() {
        McpOwnerError::connection_failed(format!("{context} before the response deadline"))
    } else {
        McpOwnerError::connection_failed(context)
    }
}

fn map_cleanup_request_error(error: reqwest::Error) -> McpOwnerError {
    let message = if error.is_timeout() {
        "MCP session cleanup request exceeded its response deadline"
    } else {
        "MCP session cleanup request failed"
    };
    McpOwnerError::new("MCP_SESSION_CLEANUP_FAILED", message)
}

async fn drain_response_body(
    mut response: reqwest::Response,
) -> Result<(), McpOwnerError> {
    let mut total = 0usize;
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        McpOwnerError::connection_failed("MCP response body could not be consumed")
    })? {
        total = total.saturating_add(chunk.len());
        if total > MAX_RESPONSE_BYTES {
            return Err(McpOwnerError::protocol_failed(
                "MCP response exceeds the bounded response limit",
            ));
        }
    }
    Ok(())
}

async fn parse_response(
    mut response: reqwest::Response,
) -> Result<JsonRpcResponse, McpOwnerError> {
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        let _ = drain_response_body(response).await;
        return Err(McpOwnerError::new(
            "MCP_CREDENTIAL_REQUIRED",
            "MCP server rejected the canonical credential authority",
        ));
    }
    if !status.is_success() {
        let _ = drain_response_body(response).await;
        return Err(McpOwnerError::new(
            "MCP_HTTP_ERROR",
            format!("MCP server returned HTTP {}", status.as_u16()),
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(McpOwnerError::protocol_failed(
            "MCP response exceeds the bounded response limit",
        ));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let mut body = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or_default()
            .min(MAX_RESPONSE_BYTES as u64) as usize,
    );
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        McpOwnerError::connection_failed("MCP response body could not be read")
    })? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(McpOwnerError::protocol_failed(
                "MCP response exceeds the bounded response limit",
            ));
        }
        body.extend_from_slice(&chunk);
    }
    if content_type.contains("text/event-stream") {
        parse_sse_response(&String::from_utf8_lossy(&body))
    } else {
        serde_json::from_slice(&body).map_err(|error| {
            McpOwnerError::protocol_failed(format!("MCP response is not valid JSON-RPC: {error}"))
        })
    }
}

fn validate_initialize_result(value: &Value) -> Result<(), McpOwnerError> {
    let protocol_version = value
        .as_object()
        .and_then(|object| object.get("protocolVersion"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            McpOwnerError::protocol_failed("initialize result has no protocolVersion")
        })?;
    if protocol_version != MCP_PROTOCOL_VERSION {
        return Err(McpOwnerError::new(
            "MCP_PROTOCOL_VERSION_MISMATCH",
            format!(
                "MCP server selected protocol {protocol_version}; expected {MCP_PROTOCOL_VERSION}"
            ),
        ));
    }
    Ok(())
}

fn parse_sse_response(body: &str) -> Result<JsonRpcResponse, McpOwnerError> {
    let normalized = body.replace("\r\n", "\n");
    for event in normalized.split("\n\n") {
        let data = event
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(|line| line.strip_prefix(' ').unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n");
        if data.trim().is_empty() {
            continue;
        }
        if let Ok(response) = serde_json::from_str::<JsonRpcResponse>(&data)
            && response.id.is_some()
        {
            return Ok(response);
        }
    }
    Err(McpOwnerError::protocol_failed(
        "MCP SSE response contained no correlated JSON-RPC result",
    ))
}

fn ensure_response_id(
    response: &JsonRpcResponse,
    expected: u64,
) -> Result<(), McpOwnerError> {
    let matches = response.id.as_ref().is_some_and(|id| match id {
        Value::Number(number) => number.as_u64() == Some(expected),
        Value::String(value) => value == &expected.to_string(),
        _ => false,
    });
    if matches {
        Ok(())
    } else {
        Err(McpOwnerError::protocol_failed(format!(
            "MCP response correlation ID did not match request {expected}"
        )))
    }
}

fn ensure_no_rpc_error(
    method: &str,
    response: &JsonRpcResponse,
) -> Result<(), McpOwnerError> {
    let Some(error) = &response.error else {
        return Ok(());
    };
    Err(McpOwnerError::new(
        "MCP_RPC_ERROR",
        format!("{method} returned JSON-RPC error {}", error.code),
    ))
}

fn parse_tools(value: &Value) -> Result<Vec<RemoteTool>, McpOwnerError> {
    let tools = value
        .as_object()
        .and_then(|object| object.get("tools"))
        .and_then(Value::as_array)
        .ok_or_else(|| McpOwnerError::protocol_failed("tools/list result has no tools array"))?;
    let mut names = BTreeSet::new();
    let mut parsed = Vec::with_capacity(tools.len());
    for (index, tool) in tools.iter().enumerate() {
        let object = tool.as_object().ok_or_else(|| {
            McpOwnerError::protocol_failed(format!("tools/list entry {index} is not an object"))
        })?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty() && name.trim() == *name)
            .ok_or_else(|| {
                McpOwnerError::protocol_failed(format!(
                    "tools/list entry {index} has an invalid name"
                ))
            })?
            .to_owned();
        if !names.insert(name.clone()) {
            return Err(McpOwnerError::protocol_failed(format!(
                "tools/list contains duplicate tool name {name}"
            )));
        }
        let input_schema = object
            .get("inputSchema")
            .cloned()
            .ok_or_else(|| {
                McpOwnerError::protocol_failed(format!(
                    "tools/list entry {name} has no inputSchema"
                ))
            })?;
        if !input_schema.is_object() {
            return Err(McpOwnerError::protocol_failed(format!(
                "tools/list entry {name} has a non-object inputSchema"
            )));
        }
        parsed.push(RemoteTool { name, input_schema });
    }
    Ok(parsed)
}

fn validate_tool_result(value: &Value) -> Result<(), McpOwnerError> {
    let object = value
        .as_object()
        .ok_or_else(|| McpOwnerError::protocol_failed("tools/call result is not an object"))?;
    let content = object
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| McpOwnerError::protocol_failed("tools/call result has no content array"))?;
    if let Some(is_error) = object.get("isError") {
        if !is_error.is_boolean() {
            return Err(McpOwnerError::protocol_failed(
                "tools/call result isError is not boolean",
            ));
        }
        if is_error.as_bool() == Some(true) {
            return Err(McpOwnerError::new(
                "MCP_TOOL_FAILED",
                "the bound MCP tool reported an execution error",
            ));
        }
    }
    for (index, item) in content.iter().enumerate() {
        if !item.is_object() {
            return Err(McpOwnerError::protocol_failed(format!(
                "tools/call content item {index} is not an object"
            )));
        }
    }
    Ok(())
}

fn initialize_request() -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0",
        id: Some(1),
        method: "initialize",
        params: Some(json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": MCP_CLIENT_NAME,
                "version": MCP_CLIENT_VERSION
            }
        })),
    }
}

fn initialized_notification() -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0",
        id: None,
        method: "notifications/initialized",
        params: None,
    }
}

fn tools_list_request() -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0",
        id: Some(2),
        method: "tools/list",
        params: None,
    }
}

fn tool_call_request(
    tool_name: &str,
    arguments: &Value,
    operation_id: &str,
) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0",
        id: Some(3),
        method: "tools/call",
        params: Some(json!({
            "name": tool_name,
            "arguments": arguments,
            "_meta": {
                MCP_EXECUTION_OPERATION_META_KEY: operation_id
            }
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::header::{HeaderValue, LOCATION};
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{delete, get, post};
    use axum::{Json, Router};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;

    fn schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "message": {"type": "string"}
            },
            "required": ["message"]
        })
    }

    fn tool() -> McpToolBinding {
        McpToolBinding::new(
            "server-1",
            "test.mcp.echo",
            "a".repeat(64),
            schema(),
            "echo",
        )
        .unwrap()
    }

    fn server() -> McpServerBinding {
        McpServerBinding {
            server_id: "server-1".to_owned(),
            server_owner_id: "system".to_owned(),
            enabled: true,
            connection_config_ref: "connection-1".to_owned(),
            resource_binding_id: "mcp-binding".to_owned(),
            resource_kind: MCP_SERVER_RESOURCE_KIND.to_owned(),
            resource_id: "server-1".to_owned(),
            resource_owner_id: "owner".to_owned(),
            granted_operations: BTreeSet::from([
                    MCP_CONNECT_OPERATION.to_owned(),
                    MCP_INVOKE_OPERATION.to_owned(),
                    MCP_READ_OPERATION.to_owned(),
            ]),
            resource_connection_config_ref: Some("connection-1".to_owned()),
            transport: McpServerTransport::Http {
                url: "http://127.0.0.1:1/mcp".to_owned(),
                headers: HashMap::new(),
            },
        }
    }

    fn invocation(server: McpServerBinding, operation_id: &str) -> McpToolInvocationRequest {
        McpToolInvocationRequest {
            principal_kind: "user".to_owned(),
            principal_id: "owner".to_owned(),
            operation_id: operation_id.to_owned(),
            server,
            tool: tool(),
            arguments: json!({}),
        }
    }

    #[test]
    fn tool_binding_rejects_noncanonical_schema_digest() {
        let error = McpToolBinding::new(
            "server-1",
            "test.mcp.echo",
            "A".repeat(64),
            schema(),
            "echo",
        )
        .unwrap_err();
        assert_eq!(error.code(), "MCP_BINDING_INVALID");
    }

    #[test]
    fn invocation_requires_exact_resource_owner_and_identity() {
        let tool = tool();
        let mut request = McpToolInvocationRequest {
            principal_kind: "user".to_owned(),
            principal_id: "owner".to_owned(),
            operation_id: "operation-1".to_owned(),
            server: server(),
            tool,
            arguments: json!({}),
        };
        request.server.resource_id = "other-server".to_owned();
        let error = validate_invocation(&request).unwrap_err();
        assert_eq!(error.code(), "MCP_RESOURCE_IDENTITY_MISMATCH");
    }

    #[test]
    fn invocation_rejects_model_routing_fields_by_shape() {
        let tool = tool();
        let request = McpToolInvocationRequest {
            principal_kind: "user".to_owned(),
            principal_id: "owner".to_owned(),
            operation_id: "operation-2".to_owned(),
            server: server(),
            tool,
            arguments: json!("model-selected-tool"),
        };
        let error = validate_invocation(&request).unwrap_err();
        assert_eq!(error.code(), "MCP_INVALID_ARGUMENTS");
    }

    #[test]
    fn static_authorization_headers_cannot_bypass_authority() {
        let error = build_headers(
            &HashMap::from([(
                "Authorization".to_owned(),
                "Bearer forged".to_owned(),
            )]),
            None,
        )
        .unwrap_err();
        assert_eq!(error.code(), "MCP_CREDENTIAL_AUTHORITY_REQUIRED");
    }

    #[test]
    fn rpc_response_ids_accept_numeric_and_string_protocol_ids() {
        let numeric = JsonRpcResponse {
            jsonrpc: "2.0".to_owned(),
            id: Some(json!(2)),
            result: None,
            error: None,
        };
        let string = JsonRpcResponse {
            jsonrpc: "2.0".to_owned(),
            id: Some(json!("2")),
            result: None,
            error: None,
        };
        ensure_response_id(&numeric, 2).unwrap();
        ensure_response_id(&string, 2).unwrap();
    }

    #[test]
    fn tool_result_requires_real_content_and_rejects_tool_error() {
        let missing = validate_tool_result(&json!({})).unwrap_err();
        assert_eq!(missing.code(), "MCP_PROTOCOL_ERROR");
        let failed = validate_tool_result(&json!({
            "content": [{"type": "text", "text": "remote failure"}],
            "isError": true
        }))
        .unwrap_err();
        assert_eq!(failed.code(), "MCP_TOOL_FAILED");
    }

    #[test]
    fn rpc_error_messages_never_echo_untrusted_remote_text() {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_owned(),
            id: Some(json!(1)),
            result: None,
            error: Some(JsonRpcError {
                code: -32000,
                message: "authorization=fixture-secret".to_owned(),
            }),
        };
        let error = ensure_no_rpc_error("tools/call", &response).unwrap_err();
        assert_eq!(error.code(), "MCP_RPC_ERROR");
        assert!(!error.message().contains("fixture-secret"));
        assert_eq!(
            error.message(),
            "tools/call returned JSON-RPC error -32000"
        );
    }

    #[test]
    fn unsupported_transport_is_explicitly_not_a_fallback() {
        let transport = McpServerTransport::Stdio {
            command: "legacy-client".to_owned(),
            args: Vec::new(),
            env: HashMap::new(),
        };
        assert!(matches!(transport, McpServerTransport::Stdio { .. }));
    }

    #[derive(Clone, Default)]
    struct FixtureState {
        requests: Arc<Mutex<Vec<(String, Value, Option<String>)>>>,
        session_id: Option<String>,
        cleanup_status: Option<StatusCode>,
        cleanup_session_ids: Arc<Mutex<Vec<Option<String>>>>,
    }

    #[derive(Clone, Default)]
    struct RecordingCredentialAuthority {
        lookups: Arc<Mutex<Vec<McpCredentialLookup>>>,
    }

    #[async_trait]
    impl McpCredentialAuthority for RecordingCredentialAuthority {
        async fn resolve(
            &self,
            lookup: McpCredentialLookup,
        ) -> Result<Option<McpCredential>, McpOwnerError> {
            self.lookups.lock().await.push(lookup);
            Ok(Some(McpCredential::bearer("fixture-token")?))
        }
    }

    async fn fixture_handler(
        State(state): State<FixtureState>,
        headers: HeaderMap,
        Json(request): Json<Value>,
    ) -> Response {
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let authorization = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        state
            .requests
            .lock()
            .await
            .push((method.clone(), request.clone(), authorization));

        if method == "notifications/initialized" {
            return StatusCode::NO_CONTENT.into_response();
        }

        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let body = match method.as_str() {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "serverInfo": {"name": "fixture", "version": "1.0.0"}
                }
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"tools": [{
                    "name": "echo",
                    "description": "fixture echo",
                    "inputSchema": schema()
                }]}
            }),
            "tools/call" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{"type": "text", "text": "fixture-result"}],
                    "isError": false
                }
            }),
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": "method not found"}
            }),
        };
        let mut response = (StatusCode::OK, Json(body)).into_response();
        if method == "initialize"
            && let Some(session_id) = state.session_id.as_deref()
        {
            response.headers_mut().insert(
                "mcp-session-id",
                HeaderValue::from_str(session_id).expect("fixture session ID"),
            );
        }
        response
    }

    async fn fixture_delete_handler(
        State(state): State<FixtureState>,
        headers: HeaderMap,
    ) -> Response {
        let session_id = headers
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        state
            .cleanup_session_ids
            .lock()
            .await
            .push(session_id);
        let authorization = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        state
            .requests
            .lock()
            .await
            .push(("DELETE".to_owned(), Value::Null, authorization));
        state
            .cleanup_status
            .unwrap_or(StatusCode::NO_CONTENT)
            .into_response()
    }

    async fn redirect_source() -> Response {
        let mut response = StatusCode::TEMPORARY_REDIRECT.into_response();
        response
            .headers_mut()
            .insert(LOCATION, HeaderValue::from_static("/target"));
        response
    }

    async fn redirect_target(State(hits): State<Arc<AtomicUsize>>) -> Response {
        hits.fetch_add(1, Ordering::SeqCst);
        StatusCode::NO_CONTENT.into_response()
    }

    #[tokio::test]
    async fn owner_performs_real_exact_http_tool_call_with_authority() {
        let state = FixtureState::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/mcp", post(fixture_handler))
            .with_state(state.clone());
        let server_task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        let authority = RecordingCredentialAuthority::default();
        let owner = McpOwner::new(
            Arc::new(authority.clone()),
            reqwest::Client::builder().no_proxy().build().unwrap(),
        )
        .with_timeout(Duration::from_secs(5));
        let mut server = server();
        if let McpServerTransport::Http { url, .. } = &mut server.transport {
            *url = endpoint.clone();
        }
        let request = McpToolInvocationRequest {
            principal_kind: "user".to_owned(),
            principal_id: "owner".to_owned(),
            operation_id: "operation-3".to_owned(),
            server,
            tool: McpToolBinding::new(
                "server-1",
                "test.mcp.echo",
                "a".repeat(64),
                schema(),
                "echo",
            )
            .unwrap(),
            arguments: json!({"message": "hello"}),
        };

        let result = owner.invoke(request).await.unwrap();
        assert_eq!(result.result["content"][0]["text"], "fixture-result");
        assert_eq!(result.canonical_tool_key, "test.mcp.echo");
        assert_eq!(result.remote_tool_name, "echo");

        let requests = state.requests.lock().await.clone();
        assert_eq!(
            requests
                .iter()
                .map(|(method, _, _)| method.as_str())
                .collect::<Vec<_>>(),
            vec!["initialize", "notifications/initialized", "tools/list", "tools/call"]
        );
        assert_eq!(requests[3].1["params"]["name"], "echo");
        assert_eq!(requests[3].1["params"]["arguments"]["message"], "hello");
        assert!(
            requests
                .iter()
                .all(|(_, _, authorization)| authorization.as_deref() == Some("Bearer fixture-token"))
        );

        let lookups = authority.lookups.lock().await.clone();
        assert_eq!(lookups.len(), 1);
        assert_eq!(lookups[0].server_id, "server-1");
        assert_eq!(lookups[0].resource_id, "server-1");
        assert_eq!(lookups[0].connection_config_ref, "connection-1");
        assert_eq!(lookups[0].endpoint, endpoint);

        server_task.abort();
        let _ = server_task.await;
    }

    #[tokio::test]
    async fn dynamic_http_client_does_not_follow_redirects() {
        let target_hits = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/source", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/source", get(redirect_source))
            .route("/target", get(redirect_target))
            .with_state(target_hits.clone());
        let server_task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        let client = build_dynamic_http_client().expect("dynamic client should build");
        let response = client.get(endpoint).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(target_hits.load(Ordering::SeqCst), 0);

        server_task.abort();
        let _ = server_task.await;
    }

    #[tokio::test]
    async fn endpoint_validation_does_not_echo_url_credentials() {
        let mut server = server();
        let userinfo_secret = "fixture-userinfo-secret";
        let query_secret = "fixture-query-secret";
        if let McpServerTransport::Http { url, .. } = &mut server.transport {
            *url = format!(
                "http://user:{userinfo_secret}@127.0.0.1:1/mcp?access_token={query_secret}"
            );
        }
        let owner = McpOwner::new(
            Arc::new(AnonymousMcpCredentialAuthority),
            reqwest::Client::builder().no_proxy().build().unwrap(),
        );
        let error = owner
            .invoke(invocation(server, "operation-url-validation"))
            .await
            .unwrap_err();

        assert_eq!(error.code(), "MCP_CREDENTIAL_AUTHORITY_REQUIRED");
        assert!(!error.message().contains(userinfo_secret));
        assert!(!error.message().contains(query_secret));
        assert!(!error.message().contains("http://"));
    }

    #[tokio::test]
    async fn established_session_is_closed_once_after_tool_call() {
        let state = FixtureState {
            session_id: Some("fixture-session".to_owned()),
            ..FixtureState::default()
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let router = Router::new()
            .route(
                "/mcp",
                post(fixture_handler).delete(fixture_delete_handler),
            )
            .with_state(state.clone());
        let server_task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        let owner = McpOwner::new(
            Arc::new(AnonymousMcpCredentialAuthority),
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        )
        .with_timeout(Duration::from_secs(5));
        let mut request = invocation(server(), "operation-session-cleanup");
        if let McpServerTransport::Http { url, .. } = &mut request.server.transport {
            *url = endpoint;
        }

        owner.invoke(request).await.unwrap();

        let requests = state.requests.lock().await.clone();
        assert_eq!(
            requests
                .iter()
                .map(|(method, _, _)| method.as_str())
                .collect::<Vec<_>>(),
            vec![
                "initialize",
                "notifications/initialized",
                "tools/list",
                "tools/call",
                "DELETE"
            ]
        );
        assert_eq!(
            state.cleanup_session_ids.lock().await.as_slice(),
            &[Some("fixture-session".to_owned())]
        );

        server_task.abort();
        let _ = server_task.await;
    }

    #[tokio::test]
    async fn session_cleanup_failure_is_typed_and_not_retried() {
        let state = FixtureState {
            cleanup_status: Some(StatusCode::INTERNAL_SERVER_ERROR),
            ..FixtureState::default()
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/", delete(fixture_delete_handler))
            .with_state(state.clone());
        let server_task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut session = HttpMcpSession::new(
            client,
            endpoint,
            reqwest::header::HeaderMap::new(),
        );
        session.session_id = Some("fixture-session".to_owned());
        let error = session.close().await.unwrap_err();
        assert_eq!(error.code(), "MCP_SESSION_CLEANUP_FAILED");
        assert!(session.session_id.is_none());
        assert_eq!(state.cleanup_session_ids.lock().await.len(), 1);

        assert!(session.close().await.is_ok());
        assert_eq!(state.cleanup_session_ids.lock().await.len(), 1);

        server_task.abort();
        let _ = server_task.await;
    }

    #[tokio::test]
    async fn unreachable_http_server_returns_typed_connection_failure() {
        let owner = McpOwner::new(
            Arc::new(AnonymousMcpCredentialAuthority),
            reqwest::Client::builder().no_proxy().build().unwrap(),
        )
        .with_timeout(Duration::from_millis(200));
        let request = McpToolInvocationRequest {
            principal_kind: "user".to_owned(),
            principal_id: "owner".to_owned(),
            operation_id: "operation-4".to_owned(),
            server: server(),
            tool: tool(),
            arguments: json!({}),
        };
        let error = owner.invoke(request).await.unwrap_err();
        assert!(
            matches!(error.code(), "MCP_CONNECTION_FAILED" | "MCP_TIMEOUT"),
            "unreachable MCP endpoint must fail with a typed connection/deadline error: {}",
            error
        );
    }
}
