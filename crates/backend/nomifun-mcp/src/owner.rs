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
const MAX_CATALOG_PAGES: u64 = 32;
const MAX_CATALOG_TOOLS: usize = 1024;
const MAX_CURSOR_BYTES: usize = 4096;

#[path = "owner_stream.rs"]
mod stream;
#[path = "owner_legacy_sse.rs"]
mod legacy_sse;
#[path = "owner_stdio.rs"]
mod stdio;
#[path = "owner_discovery.rs"]
mod discovery;
#[path = "owner_resources.rs"]
mod resources;
#[path = "owner_resource_template.rs"]
mod resource_template;

pub use resources::{McpResourceFailure, McpResourceOperation, McpResourceRequest, McpResourceResult};

pub(crate) use discovery::discover_network;
pub(crate) use stdio::discover as discover_stdio;

/// A typed error emitted by the MCP owner.
///
/// The message never contains credential material. The code is stable enough
/// for the central host to map it into its canonical host-port error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpOwnerError {
    code: String,
    message: String,
    authentication_challenge: Option<String>,
    http_status: Option<u16>,
}

impl McpOwnerError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            code: code.into(),
            message: sanitize_diagnostic(&message),
            authentication_challenge: None,
            http_status: None,
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn authentication_challenge(&self) -> Option<&str> {
        self.authentication_challenge.as_deref()
    }

    pub(crate) fn http_status(&self) -> Option<u16> {
        self.http_status
    }

    fn credential_required(headers: &reqwest::header::HeaderMap) -> Self {
        let mut error = Self::new("MCP_CREDENTIAL_REQUIRED", "MCP server rejected the canonical credential authority");
        error.authentication_challenge = headers.get("www-authenticate")
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.len() <= 4096 && !value.chars().any(char::is_control))
            .map(sanitize_diagnostic);
        error
    }

    fn invalid_binding(message: impl Into<String>) -> Self {
        Self::new("MCP_BINDING_INVALID", message)
    }

    fn connection_failed(message: impl Into<String>) -> Self {
        Self::new("MCP_CONNECTION_FAILED", message)
    }

    fn http_error(status: reqwest::StatusCode, message: impl Into<String>) -> Self {
        let mut error = Self::new("MCP_HTTP_ERROR", message);
        error.http_status = Some(status.as_u16());
        error
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
    /// All three transports share frozen tool admission. Local stdio processes
    /// use the platform tree owner, never a legacy Nomi/Gateway fallback.
    pub async fn invoke(
        &self,
        request: McpToolInvocationRequest,
    ) -> Result<McpToolInvocationResult, McpOwnerError> {
        validate_invocation(&request)?;
        let (url, static_headers, legacy) = match &request.server.transport {
            McpServerTransport::Http { url, headers } => (url.clone(), headers.clone(), false),
            McpServerTransport::Sse { url, headers } => (url.clone(), headers.clone(), true),
            McpServerTransport::Stdio { command, args, env } => {
                let deadline = Instant::now() + self.timeout;
                let transport = stdio::StdioTransport::launch(command, args, env, deadline).await?;
                return self.invoke_session(&request, McpSession::from_stdio(transport), deadline).await;
            }
        };
        validate_http_endpoint(&url)?;
        // OAuth storage keys use the exact configured endpoint. URL parsing
        // may add a slash/remove a default port; do not silently look up a
        // different credential key after validating the bound URL.
        self.invoke_http(&request, url, static_headers, legacy).await
    }

    async fn invoke_http(
        &self,
        request: &McpToolInvocationRequest,
        endpoint: String,
        static_headers: HashMap<String, String>,
        legacy: bool,
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
        let mut session = McpSession::new(http_client, endpoint, headers);
        session.legacy_mode = legacy;
        self.invoke_session(request, session, deadline).await
    }

    async fn invoke_session(
        &self,
        request: &McpToolInvocationRequest,
        mut session: McpSession,
        deadline: Instant,
    ) -> Result<McpToolInvocationResult, McpOwnerError> {
        // Launching a configured local program can itself mutate external
        // state before tools/call. Tree cleanup is not rollback.
        let local_process_started = session.stdio.is_some();
        let mut call_started = false;
        let transaction = async {
            session.initialize().await?;

            let call_id = session.require_exact_tool(&request.tool).await?;

            // A failed request from this point can already have applied an
            // effect. Do not replay it as though execution never started.
            call_started = true;
            let call = session
                .request(tool_call_request(
                    call_id,
                    &request.tool.remote_tool_name,
                    &request.arguments,
                    &request.operation_id,
                ))
                .await?;
            ensure_response_id(&call, call_id)?;
            if call.jsonrpc != "2.0" || call.result.is_some() == call.error.is_some() {
                return Err(McpOwnerError::protocol_failed("tools/call must return exactly one JSON-RPC result or error"));
            }
            let result = if let Some(error) = call.error {
                // A correlated RPC rejection is an observed failed call, not
                // a lost response. Do not echo untrusted RPC message/data.
                // Publication still waits for the independent cleanup below.
                json!({"isError":true,"content":[{"type":"text","text":format!(
                    "The bound MCP tool returned JSON-RPC error {}. This is a returned failure, not proof of no effects or rollback. Do not automatically replay it.", error.code)}]})
            } else {
                call.result.ok_or_else(|| {
                    McpOwnerError::protocol_failed("tools/call response has no result")
                })?
            };
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
        let cleanup_budget = if session.stdio.is_some() { stdio::CLEANUP_TIMEOUT } else { SESSION_CLEANUP_TIMEOUT };
        let cleanup = timeout(cleanup_budget, session.close())
            .await
            .unwrap_or_else(|_| {
                Err(McpOwnerError::new(
                    "MCP_SESSION_CLEANUP_FAILED",
                    "MCP session cleanup exceeded its bounded deadline",
                ))
            });

        match (outcome, cleanup) {
            (Ok(result), Ok(_)) => Ok(result),
            // An earlier protocol failure cannot hide cleanup uncertainty.
            (_, Err(cleanup_error)) => Err(McpOwnerError::new(
                "MCP_SESSION_CLEANUP_FAILED",
                format!("MCP transaction cleanup is unproven ({})", cleanup_error.code()),
            )),
            (Err(operation_error), Ok(_)) if call_started || local_process_started => Err(McpOwnerError::new(
                "MCP_OUTCOME_UNKNOWN",
                format!("MCP tool or local server may have applied effects; do not replay ({})", operation_error.code()),
            )),
            (Err(operation_error), Ok(_)) => Err(operation_error),
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
    validate_connection_authority(&request.principal_kind, &request.principal_id,
        &request.operation_id, &request.server, MCP_INVOKE_OPERATION)?;
    validate_tool_binding(&request.tool)?;
    if request.server.server_id != request.tool.server_id {
        return Err(McpOwnerError::new("MCP_SERVER_IDENTITY_MISMATCH",
            "server binding and MCP tool mapping refer to different servers"));
    }
    if !request.arguments.is_object() {
        return Err(McpOwnerError::new("MCP_INVALID_ARGUMENTS", "MCP tools/call arguments must be a JSON object"));
    }
    let argument_bytes = serde_json::to_vec(&request.arguments).map_err(|_| {
        McpOwnerError::new("MCP_INVALID_ARGUMENTS", "MCP arguments could not be serialized")
    })?;
    if argument_bytes.len() > MAX_ARGUMENT_BYTES {
        return Err(McpOwnerError::new("MCP_INVALID_ARGUMENTS", "MCP arguments exceed the byte limit"));
    }
    Ok(())
}

/// Tool invoke and resource read use the same exact connection/owner checks,
/// but neither operation grant implies the other.
fn validate_connection_authority(
    principal_kind: &str, principal_id: &str, operation_id: &str,
    server: &McpServerBinding, required_operation: &str,
) -> Result<(), McpOwnerError> {
    if principal_kind.trim().is_empty()
        || principal_id.trim().is_empty()
    {
        return Err(McpOwnerError::invalid_binding(
            "MCP invocation requires a non-empty principal",
        ));
    }
    if operation_id.is_empty()
        || operation_id.len() > MAX_OPERATION_ID_BYTES
        || !operation_id
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
    if server.server_id.trim().is_empty()
        || server.server_owner_id.trim().is_empty()
        || server.connection_config_ref.trim().is_empty()
    {
        return Err(McpOwnerError::invalid_binding(
            "MCP server binding contains an empty identity or connection reference",
        ));
    }
    if !server.enabled {
        return Err(McpOwnerError::new(
            "MCP_SERVER_DISABLED",
            "the exact MCP server binding is disabled",
        ));
    }
    if server.server_owner_id != "system"
        && server.server_owner_id != principal_id
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
    if server.resource_owner_id != principal_id {
        return Err(McpOwnerError::new(
            "MCP_RESOURCE_OWNER_MISMATCH",
            "MCP resource binding belongs to a different principal",
        ));
    }
    for operation in [MCP_CONNECT_OPERATION, required_operation] {
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
                | "mcp-protocol-version"
                | "content-type"
                | "accept"
                | "host"
                | "content-length"
                | "transfer-encoding"
                | "connection"
                | "upgrade"
                | "trailer"
                | "te"
                | "proxy-connection"
                | "last-event-id"
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
        let mut value = reqwest::header::HeaderValue::from_str(&authorization).map_err(|_| {
            McpOwnerError::new(
                "MCP_CREDENTIAL_INVALID",
                "credential authority returned an invalid authorization value",
            )
        })?;
        value.set_sensitive(true);
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
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    method: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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

struct McpSession {
    // Private constructors choose exactly one transport. HTTP fields are never
    // used for stdio, which does not require constructing a network client.
    client: Option<reqwest::Client>,
    endpoint: String,
    headers: reqwest::header::HeaderMap,
    session_id: Option<String>,
    // One bounded request namespace across initialize, catalog pages and call.
    server_request_ids: BTreeSet<String>,
    legacy_mode: bool,
    legacy: Option<legacy_sse::LegacyStream>,
    stdio: Option<stdio::StdioTransport>,
    resources_supported: bool,
}

impl McpSession {
    async fn initialize(&mut self) -> Result<(), McpOwnerError> {
        if self.legacy_mode {
            let (stream, endpoint) = legacy_sse::LegacyStream::connect(self.http_client()?, &self.endpoint, &self.headers).await?;
            self.endpoint = endpoint;
            self.legacy = Some(stream);
        }
        let mut initialization = initialize_request();
        if self.legacy_mode {
            if let Some(params) = initialization.params.as_mut() {
                params["protocolVersion"] = json!(legacy_sse::PROTOCOL_VERSION);
            }
        }
        let response = self.request(initialization).await?;
        ensure_no_rpc_error("initialize", &response)?;
        let result = response.result.as_ref().ok_or_else(|| McpOwnerError::protocol_failed("initialize response has no result"))?;
        if self.legacy_mode {
            validate_initialize_result_for(result, legacy_sse::PROTOCOL_VERSION)?;
        } else {
            validate_initialize_result(result)?;
        }
        self.resources_supported = match result.get("capabilities").and_then(|value| value.get("resources")) {
            None => false,
            Some(Value::Object(_)) => true,
            Some(_) => return Err(McpOwnerError::protocol_failed("initialize resources capability is malformed")),
        };
        self.notify(initialized_notification()).await
    }

    /// Validate every bounded page, including pages after a match, so an
    /// incomplete catalog or duplicate name cannot silently select a tool.
    /// All pages share the invocation's outer deadline; none resets it.
    async fn require_exact_tool(&mut self, expected: &McpToolBinding) -> Result<u64, McpOwnerError> {
        self.read_catalog(Some(expected)).await.map(|(id, _)| id)
    }

    // Discovery and frozen execution validate the same complete catalog.
    // Execution does not retain raw descriptions or other discovery metadata.
    async fn read_catalog(&mut self, expected: Option<&McpToolBinding>) -> Result<(u64, Vec<Value>), McpOwnerError> {
        let mut cursor = None;
        let mut cursors = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut total_bytes = 0usize;
        let mut found = false;
        let mut catalog = Vec::new();
        for page in 0..MAX_CATALOG_PAGES {
            let id = 2 + page;
            let response = self.request(tools_list_request(id, cursor.as_deref())).await?;
            ensure_response_id(&response, id)?;
            ensure_no_rpc_error("tools/list", &response)?;
            let value = response.result.ok_or_else(|| McpOwnerError::protocol_failed("tools/list response has no result"))?;
            total_bytes = total_bytes.saturating_add(serde_json::to_vec(&value)
                .map_err(|_| McpOwnerError::protocol_failed("tools/list catalog is not serializable"))?.len());
            if total_bytes > MAX_RESPONSE_BYTES {
                return Err(McpOwnerError::protocol_failed("tools/list catalog exceeds the aggregate byte limit"));
            }
            for tool in parse_tools(&value)? {
                if names.len() >= MAX_CATALOG_TOOLS || !names.insert(tool.name.clone()) {
                    return Err(McpOwnerError::protocol_failed("tools/list catalog is oversized or contains duplicate names"));
                }
                if let Some(expected) = expected.filter(|expected| tool.name == expected.remote_tool_name) {
                    if tool.input_schema != expected.input_schema {
                        return Err(McpOwnerError::new("MCP_SCHEMA_MISMATCH", "advertised schema differs from the frozen tool schema"));
                    }
                    found = true;
                }
            }
            if expected.is_none() {
                catalog.extend(value.get("tools").and_then(Value::as_array)
                    .ok_or_else(|| McpOwnerError::protocol_failed("tools/list has no tools array"))?.iter().cloned());
            }
            match value.get("nextCursor") {
                None | Some(Value::Null) => {
                    return if found || expected.is_none() { Ok((id + 1, catalog)) } else {
                        Err(McpOwnerError::new("MCP_TOOL_NOT_FOUND", "the complete MCP catalog does not advertise the frozen tool"))
                    };
                }
                Some(Value::String(next)) if !next.is_empty() && next.len() <= MAX_CURSOR_BYTES => {
                    if !cursors.insert(next.clone()) {
                        return Err(McpOwnerError::protocol_failed("tools/list returned a repeated pagination cursor"));
                    }
                    cursor = Some(next.clone());
                }
                _ => return Err(McpOwnerError::protocol_failed("tools/list returned an invalid pagination cursor")),
            }
        }
        Err(McpOwnerError::protocol_failed("tools/list exceeded the pagination limit"))
    }

    fn new(
        client: reqwest::Client,
        endpoint: String,
        headers: reqwest::header::HeaderMap,
    ) -> Self {
        Self {
            client: Some(client),
            endpoint,
            headers,
            session_id: None,
            server_request_ids: BTreeSet::new(),
            legacy_mode: false,
            legacy: None,
            stdio: None,
            resources_supported: false,
        }
    }

    fn from_stdio(transport: stdio::StdioTransport) -> Self {
        Self {
            client: None,
            endpoint: String::new(),
            headers: reqwest::header::HeaderMap::new(),
            session_id: None,
            server_request_ids: BTreeSet::new(),
            legacy_mode: false,
            legacy: None,
            stdio: Some(transport),
            resources_supported: false,
        }
    }

    fn http_client(&self) -> Result<&reqwest::Client, McpOwnerError> {
        self.client.as_ref().ok_or_else(|| McpOwnerError::protocol_failed("MCP transaction has no HTTP transport"))
    }

    async fn request(
        &mut self,
        request: JsonRpcRequest,
    ) -> Result<JsonRpcResponse, McpOwnerError> {
        let initializing = request.method == "initialize";
        let expected = request.id.ok_or_else(|| McpOwnerError::protocol_failed("MCP request has no correlation ID"))?;
        if let Some(transport) = &self.stdio {
            let writer = transport.writer();
            let frame = stdio::encode(&request)?;
            let (_, result) = tokio::try_join!(stdio::write_frame(writer, frame), stdio::read_response(self, expected))?;
            return Ok(result);
        }
        if self.legacy_mode {
            // Read the event stream while waiting for POST acknowledgment.
            // Otherwise a large response can backpressure the server before
            // it sends its 202, deadlocking the two HTTP connections.
            let post = self.post_payload(&request, initializing)?.send();
            let ack = async {
                let response = post.await.map_err(|error| map_request_error(error, "MCP legacy request failed"))?;
                legacy_sse::accept_ack(response).await
            };
            let (_, result) = tokio::try_join!(ack, legacy_sse::read_response(self, expected))?;
            return Ok(result);
        }
        let response = self.send(request).await?;
        self.capture_session_id(response.headers(), initializing)?;
        let result = parse_response(response, expected, self).await?;
        if result.jsonrpc != "2.0" || result.result.is_some() == result.error.is_some() {
            return Err(McpOwnerError::protocol_failed("MCP response must contain exactly one JSON-RPC 2.0 result or error"));
        }
        ensure_response_id(&result, expected)?;
        Ok(result)
    }

    async fn notify(&mut self, request: JsonRpcRequest) -> Result<(), McpOwnerError> {
        if let Some(transport) = &self.stdio {
            return stdio::write_frame(transport.writer(), stdio::encode(&request)?).await;
        }
        let response = self.send(request).await?;
        self.accept_empty_ack(response).await
    }

    // Replies never recurse into the SSE parser. Streamable HTTP requires an
    // empty 202; legacy SSE also permits a bounded uninterpreted textual body.
    async fn accept_empty_ack(&mut self, mut response: reqwest::Response) -> Result<(), McpOwnerError> {
        if self.legacy_mode { return legacy_sse::accept_ack(response).await; }
        let status = response.status();
        let session_id_error = self.capture_session_id(response.headers(), false).err();
        if let Some(error) = session_id_error {
            return Err(error);
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(McpOwnerError::credential_required(response.headers()));
        }
        if status != reqwest::StatusCode::ACCEPTED {
            return Err(McpOwnerError::http_error(
                status,
                format!(
                    "MCP server returned HTTP {} instead of an empty 202 acknowledgment",
                    status.as_u16()
                ),
            ));
        }
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            McpOwnerError::connection_failed("MCP acknowledgment could not be read")
        })? {
            if !chunk.is_empty() {
                return Err(McpOwnerError::protocol_failed("MCP acknowledgment must have an empty body"));
            }
        }
        Ok(())
    }

    async fn reply_to_server(&mut self, request: &Value) -> Result<(), McpOwnerError> {
        let id = request.get("id").filter(|id| match id {
            Value::String(value) => !value.is_empty() && value.len() <= 1024,
            Value::Number(value) => value.is_i64() || value.is_u64(),
            _ => false,
        }).ok_or_else(|| McpOwnerError::protocol_failed("MCP server request has an invalid ID"))?;
        let key = serde_json::to_string(id).map_err(|_| McpOwnerError::protocol_failed("MCP server request ID is not serializable"))?;
        if self.server_request_ids.len() >= 64 || !self.server_request_ids.insert(key) {
            return Err(McpOwnerError::protocol_failed("MCP server requests exceed the bound or reuse an ID"));
        }
        let method = request.get("method").and_then(Value::as_str)
            .ok_or_else(|| McpOwnerError::protocol_failed("MCP server request has no method"))?;
        let valid_params = request.get("params").is_none_or(Value::is_object);
        let reply = if !valid_params {
            serde_json::json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32602,"message":"Invalid params"}})
        } else if method == "ping" {
            serde_json::json!({"jsonrpc":"2.0", "id":id, "result":{}})
        } else {
            // No sampling, elicitation, roots or custom client capability is
            // advertised. A remote method is never a new authority grant.
            serde_json::json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32601,"message":"Client method not supported"}})
        };
        if let Some(transport) = &self.stdio {
            return stdio::write_frame(transport.writer(), stdio::encode(&reply)?).await;
        }
        let response = self.send_payload(&reply, false).await?;
        self.accept_empty_ack(response).await
    }

    fn capture_session_id(&mut self, headers: &reqwest::header::HeaderMap, initializing: bool) -> Result<(), McpOwnerError> {
        let Some(value) = headers.get("mcp-session-id") else { return Ok(()); };
        let id = value.to_str().ok().filter(|id| !id.is_empty() && id.len() <= 1024
            && id.bytes().all(|byte| byte.is_ascii_graphic()))
            .ok_or_else(|| McpOwnerError::new("MCP_SESSION_CLEANUP_FAILED", "MCP server returned an unusable session identity"))?;
        if initializing && self.session_id.is_none() {
            self.session_id = Some(id.to_owned());
        } else if self.session_id.as_deref() != Some(id) {
            // Keep the original identity for cleanup; never redirect later
            // calls or DELETE to a newly supplied session header.
            return Err(McpOwnerError::new("MCP_SESSION_CLEANUP_FAILED", "MCP server changed its initialized session identity"));
        }
        Ok(())
    }

    async fn close(&mut self) -> Result<(), McpOwnerError> {
        if let Some(transport) = &mut self.stdio {
            return transport.close().await;
        }
        if self.legacy_mode {
            // Legacy SSE defines no DELETE/session-termination protocol.
            // Release this transaction's stream; this proves neither remote
            // service shutdown nor reversal of the acknowledged tool effect.
            self.legacy.take();
            return Ok(());
        }
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
        headers.insert("mcp-protocol-version", reqwest::header::HeaderValue::from_static(MCP_PROTOCOL_VERSION));
        let response = self
            .http_client()?
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
        let initializing = request.method == "initialize";
        self.send_payload(&request, initializing).await
    }

    async fn send_payload<T: Serialize + Sync>(
        &self,
        payload: &T,
        initializing: bool,
    ) -> Result<reqwest::Response, McpOwnerError> {
        self.post_payload(payload, initializing)?
            .send().await.map_err(|error| map_request_error(error, "MCP request failed"))
    }

    fn post_payload<T: Serialize + Sync>(&self, payload: &T, initializing: bool) -> Result<reqwest::RequestBuilder, McpOwnerError> {
        let mut headers = self.headers.clone();
        if !initializing && !self.legacy_mode {
            headers.insert("mcp-protocol-version", reqwest::header::HeaderValue::from_static(MCP_PROTOCOL_VERSION));
        }
        if let Some(session_id) = &self.session_id {
            let value = reqwest::header::HeaderValue::from_str(session_id).map_err(|_| {
                McpOwnerError::protocol_failed("MCP session ID cannot be represented as a header")
            })?;
            headers.insert("mcp-session-id", value);
        }
        Ok(self.http_client()?
            .post(&self.endpoint)
            .headers(headers)
            .json(payload))
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
    expected: u64,
    session: &mut McpSession,
) -> Result<JsonRpcResponse, McpOwnerError> {
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        // Keep the bounded challenge even if an error body would never end.
        // Dropping that body closes it; the known session is cleaned separately.
        return Err(McpOwnerError::credential_required(response.headers()));
    }
    if !status.is_success() {
        let _ = drain_response_body(response).await;
        return Err(McpOwnerError::http_error(
            status,
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
    if content_type.split(';').next().is_some_and(|kind| kind.trim().eq_ignore_ascii_case("text/event-stream")) {
        return stream::read_response(response, expected, session).await;
    }
    if !content_type.split(';').next().is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json")) {
        return Err(McpOwnerError::protocol_failed("MCP response has an unsupported content type"));
    }
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
    let value = serde_json::from_slice(&body).map_err(|_| {
        McpOwnerError::protocol_failed("MCP response is not valid JSON-RPC")
    })?;
    decode_rpc_response(value)
}

fn decode_rpc_response(value: Value) -> Result<JsonRpcResponse, McpOwnerError> {
    // Presence, not Option deserialization, decides the envelope: error:null
    // alongside result is still an illegal second outcome field.
    let result = value.get("result").cloned();
    let error = value.get("error");
    if result.is_some() == error.is_some() || error.is_some_and(Value::is_null) {
        return Err(McpOwnerError::protocol_failed("MCP response must contain exactly one result or error"));
    }
    let mut response: JsonRpcResponse = serde_json::from_value(value)
        .map_err(|_| McpOwnerError::protocol_failed("MCP response envelope is malformed"))?;
    response.result = result;
    Ok(response)
}

fn validate_initialize_result(value: &Value) -> Result<(), McpOwnerError> {
    validate_initialize_result_for(value, MCP_PROTOCOL_VERSION)
}

fn validate_initialize_result_for(value: &Value, expected: &str) -> Result<(), McpOwnerError> {
    let protocol_version = value
        .as_object()
        .and_then(|object| object.get("protocolVersion"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            McpOwnerError::protocol_failed("initialize result has no protocolVersion")
        })?;
    if protocol_version != expected {
        return Err(McpOwnerError::new(
            "MCP_PROTOCOL_VERSION_MISMATCH",
            format!(
                "MCP server selected an unsupported protocol; expected {expected}"
            ),
        ));
    }
    Ok(())
}

fn ensure_response_id(
    response: &JsonRpcResponse,
    expected: u64,
) -> Result<(), McpOwnerError> {
    let matches = response.id.as_ref().is_some_and(|id| match id {
        Value::Number(number) => number.as_u64() == Some(expected),
        // IDs are typed: the string 2 cannot acknowledge numeric request 2.
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
    if tools.len() > MAX_CATALOG_TOOLS {
        return Err(McpOwnerError::protocol_failed("tools/list page exceeds the tool count bound"));
    }
    let mut names = BTreeSet::new();
    let mut parsed = Vec::with_capacity(tools.len());
    for (index, tool) in tools.iter().enumerate() {
        let object = tool.as_object().ok_or_else(|| {
            McpOwnerError::protocol_failed(format!("tools/list entry {index} is not an object"))
        })?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty() && name.trim() == *name
                && name.len() <= 256 && !name.chars().any(char::is_control))
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
        // isError describes execution failure, not malformed protocol or
        // missing completion. Validate the content even for a failed result;
        // the host settles the observation before projecting it as failure.
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

fn tools_list_request(id: u64, cursor: Option<&str>) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0",
        id: Some(id),
        method: "tools/list",
        params: cursor.map(|cursor| json!({"cursor": cursor})),
    }
}

fn tool_call_request(
    id: u64,
    tool_name: &str,
    arguments: &Value,
    operation_id: &str,
) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0",
        id: Some(id),
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
    fn rpc_response_ids_preserve_the_request_id_type() {
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
        assert!(ensure_response_id(&string, 2).is_err());
    }

    #[test]
    fn tool_result_requires_content_and_accepts_reported_failure() {
        let missing = validate_tool_result(&json!({})).unwrap_err();
        assert_eq!(missing.code(), "MCP_PROTOCOL_ERROR");
        validate_tool_result(&json!({
            "content": [{"type": "text", "text": "remote failure"}],
            "isError": true
        }))
        .expect("a structured tool failure is a valid observed result");
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
    fn stdio_configuration_is_an_explicit_transport() {
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
            return StatusCode::ACCEPTED.into_response();
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
        let mut session = McpSession::new(
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
