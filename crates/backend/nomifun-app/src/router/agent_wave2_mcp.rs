//! Application-owned MCP execution adapter for the Wave 2 host.
//!
//! The caller supplies every routing and authorization fact that was frozen at
//! admission time. This module does not look up a server, derive a tool from a
//! capability ID, consult Gateway, or fall back to the legacy MCP runtime.

use std::sync::Arc;

use nomifun_agent_contracts::{
    ConnectionConfigRef, McpServerId, OperationId, PrincipalRef, ResolvedMcpToolLock,
    StrictJsonValue, TypedResourceBinding, digest_payload,
};
use nomifun_agent_domain_wave2::Wave2HostPortError;
use nomifun_mcp::{
    MCP_CONNECT_OPERATION, MCP_INVOKE_OPERATION, MCP_SERVER_RESOURCE_KIND, McpOwner,
    McpOwnerError, McpServerBinding as OwnerMcpServerBinding, McpServerTransport,
    McpToolBinding, McpToolInvocationRequest,
};
use async_trait::async_trait;
use serde_json::Value;

/// The non-secret, application-owned facts needed to execute one materialized
/// MCP mapping. These facts are resolved from the v4 catalog after the
/// Snapshot lock and resource binding have already been admitted.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedMcpRuntimeBinding {
    pub server: McpServerBindingFacts,
    pub remote_tool: McpRemoteToolFacts,
}

/// Resolve exact MCP runtime facts without exposing a database or service bag
/// to the capability host.
#[async_trait]
pub(crate) trait McpRuntimeBindingSource: Send + Sync {
    async fn resolve(
        &self,
        lock: &ResolvedMcpToolLock,
        resource_binding: &TypedResourceBinding,
        principal: &PrincipalRef,
    ) -> Result<ResolvedMcpRuntimeBinding, Wave2HostPortError>;
}

/// Host-resolved facts for the exact MCP server selected for one invocation.
///
/// These facts are intentionally separate from [`TypedResourceBinding`]:
/// resource authorization and server transport/configuration are different
/// contracts, and neither may be reconstructed from model input.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct McpServerBindingFacts {
    pub server_id: McpServerId,
    pub server_owner_id: String,
    pub enabled: bool,
    pub connection_config_ref: ConnectionConfigRef,
    pub transport: McpServerTransport,
}

/// Frozen protocol facts for the remote tool behind a canonical MCP mapping.
///
/// `remote_tool_name` and `input_schema` are materialized catalog facts. They
/// are never accepted from the model-facing arguments object.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct McpRemoteToolFacts {
    pub remote_tool_name: String,
    pub input_schema: Value,
}

/// Complete application input required to invoke one frozen MCP mapping.
///
/// `lock` is passed explicitly by the Agent Snapshot/host context. The adapter
/// never searches a mapping by `capability_id`; the lock, server facts, and
/// resource binding must agree exactly before the owner is called.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct McpOwnerInvocationInput {
    pub mcp_tool_lock: ResolvedMcpToolLock,
    pub server: McpServerBindingFacts,
    pub resource_binding: TypedResourceBinding,
    pub remote_tool: McpRemoteToolFacts,
    pub principal: PrincipalRef,
    pub operation_id: OperationId,
    pub arguments: StrictJsonValue,
}

/// Build the owner request from already admitted application facts.
///
/// This function is pure and performs all checks that can be made without
/// contacting the remote server. In particular, a mismatched lock/resource or
/// schema cannot reach the network.
pub(crate) fn build_mcp_tool_invocation_request(
    input: McpOwnerInvocationInput,
) -> Result<McpToolInvocationRequest, Wave2HostPortError> {
    let McpOwnerInvocationInput {
        mcp_tool_lock,
        server,
        resource_binding,
        remote_tool,
        principal,
        operation_id,
        arguments,
    } = input;

    validate_principal(&principal)?;
    validate_operation_id(&operation_id)?;
    validate_arguments(&arguments)?;
    validate_remote_tool(&mcp_tool_lock, &remote_tool)?;
    validate_server_and_resource(
        &mcp_tool_lock,
        &server,
        &resource_binding,
        &principal,
    )?;

    let tool = McpToolBinding::new(
        mcp_tool_lock.server_id.as_ref().to_owned(),
        mcp_tool_lock.canonical_tool_key.as_ref().to_owned(),
        mcp_tool_lock.schema_digest.as_ref().to_owned(),
        remote_tool.input_schema,
        remote_tool.remote_tool_name,
    )
    .map_err(map_mcp_owner_error)?;

    let connection_config_ref = server.connection_config_ref.as_ref().to_owned();
    let resource_connection_config_ref = resource_binding
        .connection_config_ref
        .as_ref()
        .map(|value| value.as_ref().to_owned());

    Ok(McpToolInvocationRequest {
        principal_kind: principal.principal_kind,
        principal_id: principal.principal_id,
        operation_id: operation_id.as_ref().to_owned(),
        server: OwnerMcpServerBinding {
            server_id: server.server_id.as_ref().to_owned(),
            server_owner_id: server.server_owner_id,
            enabled: server.enabled,
            connection_config_ref,
            resource_binding_id: resource_binding.binding_id.as_ref().to_owned(),
            resource_kind: resource_binding.resource_kind.as_ref().to_owned(),
            resource_id: resource_binding.resource_id.as_ref().to_owned(),
            resource_owner_id: resource_binding.owner_id,
            granted_operations: resource_binding.operations,
            resource_connection_config_ref,
            transport: server.transport,
        },
        tool,
        arguments: arguments.0,
    })
}

/// Application-owned adapter over [`McpOwner`].
///
/// The owner is the only execution path. There is deliberately no Gateway,
/// legacy client, discovery callback, or retrying alternate owner here.
#[derive(Clone)]
pub(crate) struct McpOwnerAdapter {
    owner: Arc<McpOwner>,
}

impl McpOwnerAdapter {
    pub(crate) async fn resource(&self, request: nomifun_mcp::McpResourceRequest) -> Result<StrictJsonValue, Wave2HostPortError> {
        let result = self.owner.resource(request).await.map_err(map_mcp_owner_error)?;
        serde_json::to_value(result).map(StrictJsonValue)
            .map_err(|_| Wave2HostPortError::unavailable("MCP resource result cannot be encoded"))
    }

    pub(crate) fn new(owner: Arc<McpOwner>) -> Self {
        Self { owner }
    }

    /// Invoke the exact frozen mapping and return the validated MCP result.
    /// Ok may contain isError=true: persist that observed result first, then
    /// use project_mcp_tool_result for the canonical execution-error channel.
    ///
    /// The lock, server facts, resource binding, remote tool facts, principal,
    /// operation ID, and model arguments are all explicit fields of
    /// [`McpOwnerInvocationInput`].
    pub(crate) async fn invoke(
        &self,
        input: McpOwnerInvocationInput,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let request = build_mcp_tool_invocation_request(input)?;
        let result = self
            .owner
            .invoke(request)
            .await
            .map_err(map_mcp_owner_error)?;
        Ok(StrictJsonValue(result.result))
    }
}

/// Project only an already validated owner result, AFTER recording any required
/// durable settlement. A returned failure is not an unknown owner transaction.
pub(crate) fn project_mcp_tool_result(result: StrictJsonValue) -> Result<StrictJsonValue, Wave2HostPortError> {
    if result.0.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(Wave2HostPortError::new("MCP_TOOL_RETURNED_FAILURE",
            "The bound MCP tool returned failure and completed protocol cleanup. Inspect its recorded observation; do not infer no effects, rollback, or permission to automatically replay it."));
    }
    Ok(result)
}

fn validate_principal(principal: &PrincipalRef) -> Result<(), Wave2HostPortError> {
    if principal.principal_kind.trim().is_empty()
        || principal.principal_id.trim().is_empty()
    {
        return Err(mcp_adapter_error(
            "MCP_BINDING_INVALID",
            "MCP invocation requires a non-empty principal",
        ));
    }
    Ok(())
}

fn validate_operation_id(operation_id: &OperationId) -> Result<(), Wave2HostPortError> {
    let value = operation_id.as_ref();
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(mcp_adapter_error(
            "MCP_OPERATION_ID_INVALID",
            "MCP operation ID must contain 1..=128 visible ASCII bytes",
        ));
    }
    Ok(())
}

fn validate_arguments(arguments: &StrictJsonValue) -> Result<(), Wave2HostPortError> {
    if !arguments.0.is_object() {
        return Err(mcp_adapter_error(
            "MCP_INVALID_ARGUMENTS",
            "MCP tools/call arguments must be a JSON object",
        ));
    }
    Ok(())
}

fn validate_remote_tool(
    lock: &ResolvedMcpToolLock,
    remote_tool: &McpRemoteToolFacts,
) -> Result<(), Wave2HostPortError> {
    validate_materialized_tool_identity(lock)?;
    if lock.server_id.as_ref().trim().is_empty()
        || lock.canonical_tool_key.as_ref().trim().is_empty()
        || lock.capability_id.as_ref().trim().is_empty()
    {
        return Err(mcp_adapter_error(
            "MCP_BINDING_INVALID",
            "frozen MCP mapping contains an empty identity",
        ));
    }
    if !remote_tool.input_schema.is_object() {
        return Err(mcp_adapter_error(
            "MCP_BINDING_INVALID",
            "frozen MCP tool input schema must be an object",
        ));
    }
    if remote_tool.remote_tool_name.trim().is_empty()
        || remote_tool.remote_tool_name.trim() != remote_tool.remote_tool_name
        || remote_tool
            .remote_tool_name
            .chars()
            .any(char::is_control)
    {
        return Err(mcp_adapter_error(
            "MCP_BINDING_INVALID",
            "frozen MCP remote tool name is empty or malformed",
        ));
    }
    let server_id = nomifun_api_types::McpServerId::parse(lock.server_id.as_ref().to_owned())
        .map_err(|_| {
            mcp_adapter_error(
                "MCP_BINDING_INVALID",
                "frozen MCP server identity is not a canonical UUIDv7",
            )
        })?;
    let expected_capability = nomifun_mcp::canonical_mcp_tool_capability_id(
        &server_id,
        &remote_tool.remote_tool_name,
    )
    .map_err(|_| {
        mcp_adapter_error(
            "MCP_BINDING_INVALID",
            "frozen MCP server/tool identity is malformed",
        )
    })?;
    if expected_capability != lock.capability_id.as_ref() {
        return Err(mcp_adapter_error(
            "MCP_MATERIALIZATION_MISMATCH",
            "frozen MCP capability does not match its exact server/tool identity",
        ));
    }

    let computed_digest = digest_payload(&remote_tool.input_schema).map_err(|error| {
        mcp_adapter_error(
            "MCP_SCHEMA_MISMATCH",
            format!("frozen MCP tool schema could not be canonicalized: {error}"),
        )
    })?;
    if computed_digest != lock.schema_digest {
        return Err(mcp_adapter_error(
            "MCP_SCHEMA_MISMATCH",
            format!(
                "frozen MCP tool schema does not match lock digest {}",
                lock.schema_digest.as_ref()
            ),
        ));
    }
    Ok(())
}

fn validate_materialized_tool_identity(
    lock: &ResolvedMcpToolLock,
) -> Result<(), Wave2HostPortError> {
    if nomifun_api_types::McpServerId::parse(lock.server_id.as_ref().to_owned()).is_err()
        || !nomifun_mcp::is_namespaced_mcp_tool_capability(lock.capability_id.as_ref())
        || lock.canonical_tool_key.as_ref() != lock.capability_id.as_ref()
        || lock.materialization_revision != nomifun_mcp::MCP_TOOL_MATERIALIZATION_REVISION
    {
        return Err(mcp_adapter_error(
            "MCP_MATERIALIZATION_MISMATCH",
            "MCP tools require one namespaced per-tool capability and exact materialization revision",
        ));
    }
    Ok(())
}

fn validate_server_and_resource(
    lock: &ResolvedMcpToolLock,
    server: &McpServerBindingFacts,
    resource: &TypedResourceBinding,
    principal: &PrincipalRef,
) -> Result<(), Wave2HostPortError> {
    if server.server_id != lock.server_id {
        return Err(mcp_adapter_error(
            "MCP_SERVER_IDENTITY_MISMATCH",
            "server facts and frozen MCP mapping refer to different servers",
        ));
    }
    if server.server_owner_id.trim().is_empty()
        || server.connection_config_ref.as_ref().trim().is_empty()
    {
        return Err(mcp_adapter_error(
            "MCP_BINDING_INVALID",
            "MCP server facts contain an empty owner or connection reference",
        ));
    }
    if !server.enabled {
        return Err(mcp_adapter_error(
            "MCP_SERVER_DISABLED",
            "the exact MCP server binding is disabled",
        ));
    }
    if server.server_owner_id != "system"
        && server.server_owner_id != principal.principal_id
    {
        return Err(mcp_adapter_error(
            "MCP_SERVER_OWNER_MISMATCH",
            "the exact MCP server belongs to a different owner",
        ));
    }

    if resource.binding_id.as_ref().trim().is_empty()
        || resource.resource_id.as_ref().trim().is_empty()
        || resource.owner_id.trim().is_empty()
    {
        return Err(mcp_adapter_error(
            "MCP_BINDING_INVALID",
            "MCP resource binding contains an empty identity",
        ));
    }
    if resource.resource_kind.as_ref() != MCP_SERVER_RESOURCE_KIND {
        return Err(mcp_adapter_error(
            "MCP_BINDING_INVALID",
            "MCP invocation requires an mcp_server resource binding",
        ));
    }
    if resource.resource_id.as_ref() != lock.server_id.as_ref() {
        return Err(mcp_adapter_error(
            "MCP_RESOURCE_IDENTITY_MISMATCH",
            "resource binding does not identify the exact frozen MCP server",
        ));
    }
    if resource.owner_id != principal.principal_id {
        return Err(mcp_adapter_error(
            "MCP_RESOURCE_OWNER_MISMATCH",
            "MCP resource binding belongs to a different principal",
        ));
    }
    for operation in [MCP_CONNECT_OPERATION, MCP_INVOKE_OPERATION] {
        if !resource.operations.contains(operation) {
            return Err(mcp_adapter_error(
                "MCP_RESOURCE_OPERATION_DENIED",
                format!("MCP resource binding does not grant {operation}"),
            ));
        }
    }
    if resource.connection_config_ref.as_ref().map(AsRef::as_ref)
        != Some(server.connection_config_ref.as_ref())
    {
        return Err(mcp_adapter_error(
            "MCP_CONNECTION_CONFIG_MISMATCH",
            "resource and server facts use different connection references",
        ));
    }
    if !resource.typed_parameters.is_empty() {
        return Err(mcp_adapter_error(
            "MCP_BINDING_INVALID",
            "MCP resource binding contains unconsumed typed parameters",
        ));
    }
    Ok(())
}

fn mcp_adapter_error(
    code: impl Into<String>,
    message: impl Into<String>,
) -> Wave2HostPortError {
    Wave2HostPortError::new(code, message)
}

/// Preserve the MCP owner's stable typed error code at the Wave 2 boundary.
///
/// `McpOwner` bounds and redacts its diagnostic messages before returning them;
/// this adapter does not replace those codes with a generic success or
/// fallback result.
pub(crate) fn map_mcp_owner_error(error: McpOwnerError) -> Wave2HostPortError {
    Wave2HostPortError::new(
        error.code().to_owned(),
        format!("canonical MCP owner failed: {}", error.message()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::{BTreeSet, HashMap};

    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Response};
    use axum::routing::post;
    use axum::{Json, Router};
    use tokio::sync::Mutex;

    const SERVER_ID: &str = "0195f7c0-7b6a-7c21-8f4a-1234567890ab";

    fn schema() -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "message": {"type": "string"}
            },
            "required": ["message"]
        })
    }

    fn principal() -> PrincipalRef {
        PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "owner-1".to_owned(),
        }
    }

    fn capability_id() -> String {
        let server_id = nomifun_api_types::McpServerId::parse(SERVER_ID).unwrap();
        nomifun_mcp::canonical_mcp_tool_capability_id(&server_id, "remote.echo")
            .expect("canonical fixture MCP identity")
    }

    fn input(endpoint: &str) -> McpOwnerInvocationInput {
        let schema = schema();
        McpOwnerInvocationInput {
            mcp_tool_lock: ResolvedMcpToolLock {
                server_id: McpServerId::from(SERVER_ID),
                canonical_tool_key: capability_id().into(),
                capability_id: capability_id().into(),
                schema_digest: digest_payload(&schema).expect("schema digest"),
                materialization_revision: nomifun_mcp::MCP_TOOL_MATERIALIZATION_REVISION,
            },
            server: McpServerBindingFacts {
                server_id: McpServerId::from(SERVER_ID),
                server_owner_id: "system".to_owned(),
                enabled: true,
                connection_config_ref: ConnectionConfigRef::from("connection-1"),
                transport: McpServerTransport::Http {
                    url: endpoint.to_owned(),
                    headers: HashMap::new(),
                },
            },
            resource_binding: TypedResourceBinding {
                binding_id: "mcp-binding-1".into(),
                resource_kind: MCP_SERVER_RESOURCE_KIND.into(),
                resource_id: SERVER_ID.into(),
                owner_id: "owner-1".to_owned(),
                operations: BTreeSet::from([
                    MCP_CONNECT_OPERATION.to_owned(),
                    MCP_INVOKE_OPERATION.to_owned(),
                ]),
                connection_config_ref: Some(ConnectionConfigRef::from("connection-1")),
                typed_parameters: Default::default(),
            },
            remote_tool: McpRemoteToolFacts {
                remote_tool_name: "remote.echo".to_owned(),
                input_schema: schema,
            },
            principal: principal(),
            operation_id: "operation-1".into(),
            arguments: StrictJsonValue(serde_json::json!({"message": "hello"})),
        }
    }

    #[test]
    fn request_builder_preserves_exact_mapping_and_resource_identity() {
        let request = build_mcp_tool_invocation_request(input("http://127.0.0.1:1/mcp"))
            .expect("valid exact MCP input");

        assert_eq!(request.server.server_id, SERVER_ID);
        assert_eq!(request.server.resource_binding_id, "mcp-binding-1");
        assert_eq!(request.server.resource_id, SERVER_ID);
        assert_eq!(request.tool.server_id, SERVER_ID);
        assert_eq!(request.tool.canonical_tool_key, capability_id());
        assert_eq!(request.tool.remote_tool_name, "remote.echo");
        assert_eq!(request.arguments, serde_json::json!({"message": "hello"}));

        let mut wrong_resource = input("http://127.0.0.1:1/mcp");
        wrong_resource.resource_binding.resource_id = "other-server".into();
        let error = build_mcp_tool_invocation_request(wrong_resource)
            .expect_err("a different resource must be rejected before execution");
        assert_eq!(error.code, "MCP_RESOURCE_IDENTITY_MISMATCH");
    }

    #[test]
    fn request_builder_rejects_schema_drift_from_the_frozen_lock() {
        let mut drifted = input("http://127.0.0.1:1/mcp");
        drifted.remote_tool.input_schema["properties"]["message"] =
            serde_json::json!({"type": "integer"});

        let error = build_mcp_tool_invocation_request(drifted)
            .expect_err("remote schema facts must match the frozen lock digest");
        assert_eq!(error.code, "MCP_SCHEMA_MISMATCH");
    }

    #[test]
    fn request_builder_rejects_the_retired_generic_proxy_identity() {
        let mut legacy = input("http://127.0.0.1:1/mcp");
        let broad_proxy = concat!("mcp", ".", "tool_proxy");
        legacy.mcp_tool_lock.canonical_tool_key = broad_proxy.into();
        legacy.mcp_tool_lock.capability_id = broad_proxy.into();

        let error = build_mcp_tool_invocation_request(legacy)
            .expect_err("the broad proxy must not be accepted as a per-tool mapping");
        assert_eq!(error.code, "MCP_MATERIALIZATION_MISMATCH");
    }

    #[derive(Clone, Default)]
    struct FixtureState {
        requests: Arc<Mutex<Vec<Value>>>,
    }

    async fn fixture_handler(
        State(state): State<FixtureState>,
        Json(request): Json<Value>,
    ) -> Response {
        state.requests.lock().await.push(request.clone());
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if method == "notifications/initialized" {
            return StatusCode::ACCEPTED.into_response();
        }

        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let body = match method {
            "initialize" => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "serverInfo": {"name": "fixture", "version": "1.0.0"}
                }
            }),
            "tools/list" => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": [{
                        "name": "remote.echo",
                        "inputSchema": schema()
                    }]
                }
            }),
            "tools/call" => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{"type": "text", "text": "fixture-result"}],
                    "isError": false
                }
            }),
            _ => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": "method not found"}
            }),
        };
        (StatusCode::OK, Json(body)).into_response()
    }

    #[tokio::test]
    async fn adapter_uses_frozen_remote_name_and_model_arguments() {
        let state = FixtureState::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fixture listener");
        let endpoint = format!("http://{}/mcp", listener.local_addr().expect("fixture address"));
        let router = Router::new()
            .route("/mcp", post(fixture_handler))
            .with_state(state.clone());
        let server_task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        let owner = McpOwner::new(
            Arc::new(nomifun_mcp::AnonymousMcpCredentialAuthority),
            reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("fixture HTTP client"),
        );
        let adapter = McpOwnerAdapter::new(Arc::new(owner));
        let result = adapter
            .invoke(input(&endpoint))
            .await
            .expect("canonical MCP invocation");
        assert_eq!(result.0["content"][0]["text"], "fixture-result");

        let requests = state.requests.lock().await.clone();
        assert_eq!(
            requests
                .iter()
                .filter_map(|request| request.get("method").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            vec![
                "initialize",
                "notifications/initialized",
                "tools/list",
                "tools/call"
            ]
        );
        let call = requests
            .iter()
            .find(|request| request.get("method") == Some(&Value::String("tools/call".to_owned())))
            .expect("tools/call request");
        assert_eq!(call["params"]["name"], "remote.echo");
        assert_eq!(
            call["params"]["arguments"],
            serde_json::json!({"message": "hello"})
        );

        server_task.abort();
        let _ = server_task.await;
    }

    #[tokio::test]
    async fn adapter_preserves_typed_connection_failure() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("temporary listener");
        let endpoint = format!("http://{}/mcp", listener.local_addr().expect("temporary address"));
        drop(listener);

        let owner = McpOwner::new(
            Arc::new(nomifun_mcp::AnonymousMcpCredentialAuthority),
            reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("fixture HTTP client"),
        )
        .with_timeout(std::time::Duration::from_millis(200));
        let adapter = McpOwnerAdapter::new(Arc::new(owner));
        let error = adapter
            .invoke(input(&endpoint))
            .await
            .expect_err("unreachable MCP endpoint must fail");

        assert!(
            matches!(error.code.as_str(), "MCP_CONNECTION_FAILED" | "MCP_TIMEOUT"),
            "expected a typed connection/deadline error, got {error}"
        );
        assert!(error.message.starts_with("canonical MCP owner failed:"));
    }

}
