//! Application-owned MCP execution adapter for the Wave 2 host.
//!
//! The caller supplies every routing and authorization fact that was frozen at
//! admission time. This module does not look up a server, derive a tool from a
//! capability ID, consult Gateway, or fall back to the legacy MCP runtime.

use std::collections::{BTreeSet, HashMap};
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
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;

const MCP_CONNECTORS_PACKAGE_ID: &str = "nomifun.mcp-connectors";
const MCP_CONNECTORS_MOUNT_ID: &str = "domain-mcp-connectors";

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

/// The v4 package-config projection that carries non-secret MCP transport and
/// catalog facts. Identity/ownership/revision remain authoritative in the
/// dedicated `mcp_servers` and `mcp_tool_materializations` tables.
///
/// This is intentionally an owning-package config rather than a second
/// database schema or a resource `typed_parameters` escape hatch.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct McpRuntimeCatalogConfig {
    #[serde(default)]
    pub servers: Vec<McpRuntimeServerConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct McpRuntimeServerConfig {
    pub server_id: String,
    pub connection_config_ref: String,
    pub enabled: bool,
    pub transport: McpRuntimeTransportConfig,
    pub tools: Vec<McpRuntimeToolConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct McpRuntimeToolConfig {
    pub canonical_tool_key: String,
    pub remote_tool_name: String,
    pub input_schema: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum McpRuntimeTransportConfig {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    Sse {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

impl McpRuntimeTransportConfig {
    fn into_transport(self) -> McpServerTransport {
        match self {
            Self::Stdio { command, args, env } => {
                McpServerTransport::Stdio { command, args, env }
            }
            Self::Sse { url, headers } => McpServerTransport::Sse { url, headers },
            Self::Http { url, headers } => McpServerTransport::Http { url, headers },
        }
    }
}

impl McpRuntimeCatalogConfig {
    fn validate(&self) -> Result<(), Wave2HostPortError> {
        let mut server_ids = BTreeSet::new();
        for server in &self.servers {
            if server.server_id.trim().is_empty()
                || server.server_id != server.server_id.trim()
                || !server_ids.insert(server.server_id.clone())
            {
                return Err(mcp_source_error(
                    "MCP_CATALOG_INVALID",
                    "MCP runtime catalog contains a duplicate or malformed server identity",
                ));
            }
            if server.connection_config_ref.trim().is_empty()
                || server.connection_config_ref != server.connection_config_ref.trim()
            {
                return Err(mcp_source_error(
                    "MCP_CATALOG_INVALID",
                    "MCP runtime catalog contains an empty or malformed connection reference",
                ));
            }
            let mut tool_keys = BTreeSet::new();
            for tool in &server.tools {
                if tool.canonical_tool_key.trim().is_empty()
                    || tool.canonical_tool_key != tool.canonical_tool_key.trim()
                    || !tool_keys.insert(tool.canonical_tool_key.clone())
                {
                    return Err(mcp_source_error(
                        "MCP_CATALOG_INVALID",
                        "MCP runtime catalog contains a duplicate or malformed tool identity",
                    ));
                }
                if tool.remote_tool_name.trim().is_empty()
                    || tool.remote_tool_name != tool.remote_tool_name.trim()
                    || tool.remote_tool_name.chars().any(char::is_control)
                    || !tool.input_schema.is_object()
                {
                    return Err(mcp_source_error(
                        "MCP_CATALOG_INVALID",
                        "MCP runtime catalog contains malformed remote tool facts",
                    ));
                }
            }
        }
        Ok(())
    }
}

/// A source backed by the canonical Fresh-v4 pool.
///
/// The source deliberately reads only the v4 MCP tables and the MCP package
/// config row. It never opens the legacy `mcp_servers` repository or asks a
/// Gateway/Conversation service to discover a tool.
#[derive(Clone, Debug)]
pub(crate) struct SqliteMcpRuntimeBindingSource {
    pool: SqlitePool,
}

impl SqliteMcpRuntimeBindingSource {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl McpRuntimeBindingSource for SqliteMcpRuntimeBindingSource {
    async fn resolve(
        &self,
        lock: &ResolvedMcpToolLock,
        resource_binding: &TypedResourceBinding,
        principal: &PrincipalRef,
    ) -> Result<ResolvedMcpRuntimeBinding, Wave2HostPortError> {
        let server_id = lock.server_id.as_ref();
        let row: Option<(String, String, i64)> = sqlx::query_as(
            "SELECT owner_user_id, connection_config_ref, catalog_revision \
             FROM mcp_servers WHERE server_id = ?",
        )
        .bind(server_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| {
            mcp_source_error(
                "MCP_CATALOG_UNAVAILABLE",
                format!("MCP server catalog could not be read: {error}"),
            )
        })?;
        let Some((server_owner_id, connection_config_ref, _catalog_revision)) = row else {
            return Err(mcp_source_error(
                "MCP_BINDING_NOT_FOUND",
                format!("MCP server {server_id} is not present in the v4 catalog"),
            ));
        };

        let mapping: Option<(String, String, i64)> = sqlx::query_as(
            "SELECT schema_hash, capability_id, materialization_revision \
             FROM mcp_tool_materializations \
             WHERE server_id = ? AND canonical_tool_key = ?",
        )
        .bind(server_id)
        .bind(lock.canonical_tool_key.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| {
            mcp_source_error(
                "MCP_CATALOG_UNAVAILABLE",
                format!("MCP tool materialization could not be read: {error}"),
            )
        })?;
        let Some((schema_hash, capability_id, materialization_revision)) = mapping else {
            return Err(mcp_source_error(
                "MCP_MAPPING_NOT_FOUND",
                format!(
                    "MCP mapping {server_id}/{} is not present in the v4 catalog",
                    lock.canonical_tool_key.as_ref()
                ),
            ));
        };
        let materialization_revision = u64::try_from(materialization_revision).map_err(|_| {
            mcp_source_error(
                "MCP_CATALOG_INVALID",
                "MCP materialization revision is negative",
            )
        })?;
        if schema_hash != lock.schema_digest.as_ref()
            || capability_id != lock.capability_id.as_ref()
            || materialization_revision != lock.materialization_revision
        {
            return Err(mcp_source_error(
                "MCP_MATERIALIZATION_MISMATCH",
                "v4 MCP materialization does not match the frozen Snapshot lock",
            ));
        }

        let config_json: Option<String> = sqlx::query_scalar(
            "SELECT config_json FROM plugin_configs \
             WHERE package_id = ? AND mount_id = ?",
        )
        .bind(MCP_CONNECTORS_PACKAGE_ID)
        .bind(MCP_CONNECTORS_MOUNT_ID)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| {
            mcp_source_error(
                "MCP_CATALOG_UNAVAILABLE",
                format!("MCP package config could not be read: {error}"),
            )
        })?;
        let Some(config_json) = config_json else {
            return Err(mcp_source_error(
                "MCP_BINDING_UNAVAILABLE",
                "MCP package has no runtime catalog configuration",
            ));
        };
        let config: McpRuntimeCatalogConfig =
            serde_json::from_str(&config_json).map_err(|error| {
                mcp_source_error(
                    "MCP_CATALOG_INVALID",
                    format!("MCP package runtime catalog is invalid: {error}"),
                )
            })?;
        config.validate()?;
        let server = config
            .servers
            .iter()
            .find(|server| server.server_id == server_id)
            .ok_or_else(|| {
                mcp_source_error(
                    "MCP_BINDING_UNAVAILABLE",
                    format!("MCP server {server_id} has no runtime transport configuration"),
                )
            })?;
        let tool = server
            .tools
            .iter()
            .find(|tool| tool.canonical_tool_key == lock.canonical_tool_key.as_ref())
            .ok_or_else(|| {
                mcp_source_error(
                    "MCP_MAPPING_NOT_FOUND",
                    format!(
                        "MCP mapping {server_id}/{} has no stored remote tool facts",
                        lock.canonical_tool_key.as_ref()
                    ),
                )
            })?;
        if resource_binding.connection_config_ref.as_ref().map(AsRef::as_ref)
            != Some(connection_config_ref.as_str())
        {
            return Err(mcp_source_error(
                "MCP_CONNECTION_CONFIG_MISMATCH",
                "MCP resource binding does not match the v4 server connection reference",
            ));
        }
        if server.connection_config_ref != connection_config_ref {
            return Err(mcp_source_error(
                "MCP_CONNECTION_CONFIG_MISMATCH",
                "MCP runtime catalog and v4 server row use different connection references",
            ));
        }
        if server.server_id.trim().is_empty()
            || !server.enabled
            || server.server_id != server_id
        {
            return Err(mcp_source_error(
                "MCP_SERVER_DISABLED",
                "the exact v4 MCP server runtime configuration is disabled",
            ));
        }
        if server_owner_id.trim().is_empty()
            || (server_owner_id != "system" && server_owner_id != principal.principal_id)
        {
            return Err(mcp_source_error(
                "MCP_SERVER_OWNER_MISMATCH",
                "the exact v4 MCP server belongs to a different owner",
            ));
        }

        Ok(ResolvedMcpRuntimeBinding {
            server: McpServerBindingFacts {
                server_id: lock.server_id.clone(),
                server_owner_id,
                enabled: server.enabled,
                connection_config_ref: ConnectionConfigRef::from(connection_config_ref),
                transport: server.transport.clone().into_transport(),
            },
            remote_tool: McpRemoteToolFacts {
                remote_tool_name: tool.remote_tool_name.clone(),
                input_schema: tool.input_schema.clone(),
            },
        })
    }
}

/// Small explicit source used by focused host tests and by embedders that
/// already own a validated catalog. It is intentionally keyed by the exact
/// server/tool identity; no capability or model input is consulted.
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub(crate) struct StaticMcpRuntimeBindingSource {
    bindings: Arc<HashMap<(String, String), ResolvedMcpRuntimeBinding>>,
}

#[cfg(test)]
impl StaticMcpRuntimeBindingSource {
    pub(crate) fn for_mapping(
        server_id: impl Into<String>,
        canonical_tool_key: impl Into<String>,
        binding: ResolvedMcpRuntimeBinding,
    ) -> Self {
        let mut source = Self::default();
        let mut bindings = HashMap::new();
        bindings.insert(
            (server_id.into(), canonical_tool_key.into()),
            binding,
        );
        source.bindings = Arc::new(bindings);
        source
    }
}

#[cfg(test)]
#[async_trait]
impl McpRuntimeBindingSource for StaticMcpRuntimeBindingSource {
    async fn resolve(
        &self,
        lock: &ResolvedMcpToolLock,
        _resource_binding: &TypedResourceBinding,
        _principal: &PrincipalRef,
    ) -> Result<ResolvedMcpRuntimeBinding, Wave2HostPortError> {
        self.bindings
            .get(&(
                lock.server_id.as_ref().to_owned(),
                lock.canonical_tool_key.as_ref().to_owned(),
            ))
            .cloned()
            .ok_or_else(|| {
                mcp_source_error(
                    "MCP_BINDING_NOT_FOUND",
                    "the exact MCP mapping is absent from the runtime source",
                )
            })
    }
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
    pub(crate) fn new(owner: Arc<McpOwner>) -> Self {
        Self { owner }
    }

    /// Invoke the exact frozen mapping and return the validated MCP result.
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

fn mcp_source_error(
    code: impl Into<String>,
    message: impl Into<String>,
) -> Wave2HostPortError {
    Wave2HostPortError::new(code, message)
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
    use nomifun_agent_contracts::FRESH_V4_BASELINE_SQL;
    use sqlx::sqlite::SqlitePoolOptions;
    use tokio::sync::Mutex;

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

    fn input(endpoint: &str) -> McpOwnerInvocationInput {
        let schema = schema();
        McpOwnerInvocationInput {
            mcp_tool_lock: ResolvedMcpToolLock {
                server_id: McpServerId::from("server-1"),
                canonical_tool_key: "vendor.echo".into(),
                capability_id: "vendor.echo.capability".into(),
                schema_digest: digest_payload(&schema).expect("schema digest"),
                materialization_revision: 7,
            },
            server: McpServerBindingFacts {
                server_id: McpServerId::from("server-1"),
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
                resource_id: "server-1".into(),
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

        assert_eq!(request.server.server_id, "server-1");
        assert_eq!(request.server.resource_binding_id, "mcp-binding-1");
        assert_eq!(request.server.resource_id, "server-1");
        assert_eq!(request.tool.server_id, "server-1");
        assert_eq!(request.tool.canonical_tool_key, "vendor.echo");
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
            return StatusCode::NO_CONTENT.into_response();
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

    async fn fresh_v4_pool_for_source(
        endpoint: &str,
        schema: &Value,
    ) -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("fresh v4 test pool");
        sqlx::raw_sql(FRESH_V4_BASELINE_SQL)
            .execute(&pool)
            .await
            .expect("fresh v4 schema");

        let schema_digest = digest_payload(schema).expect("schema digest");
        sqlx::query(
            "INSERT INTO plugin_packages \
             (package_id, package_version, manifest_json, manifest_digest, display_json) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind("nomifun.mcp-connectors")
        .bind("1.0.0")
        .bind("{}")
        .bind("a".repeat(64))
        .bind("{}")
        .execute(&pool)
        .await
        .expect("MCP package");
        sqlx::query(
            "INSERT INTO plugin_mounts \
             (mount_id, package_id, package_version, source_json, desired_state, \
              effective_state, criticality) \
             VALUES (?, ?, ?, ?, 'enabled', 'active', 'required')",
        )
        .bind("domain-mcp-connectors")
        .bind("nomifun.mcp-connectors")
        .bind("1.0.0")
        .bind("{}")
        .execute(&pool)
        .await
        .expect("MCP mount");
        sqlx::query(
            "INSERT INTO capability_definitions \
             (capability_id, capability_version, package_id, package_version, \
              manifest_json, manifest_digest) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind("mcp.tool_proxy")
        .bind("1.0.0")
        .bind("nomifun.mcp-connectors")
        .bind("1.0.0")
        .bind("{}")
        .bind("b".repeat(64))
        .execute(&pool)
        .await
        .expect("MCP capability");
        sqlx::query(
            "INSERT INTO mcp_servers \
             (server_id, owner_user_id, connection_config_ref, catalog_revision) \
             VALUES (?, ?, ?, ?)",
        )
        .bind("server-1")
        .bind("system")
        .bind("connection-1")
        .bind(4_i64)
        .execute(&pool)
        .await
        .expect("MCP server");
        sqlx::query(
            "INSERT INTO mcp_tool_materializations \
             (server_id, canonical_tool_key, schema_hash, capability_id, \
              capability_version, materialization_revision, package_id, package_version) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("server-1")
        .bind("vendor.echo")
        .bind(schema_digest.as_ref())
        .bind("mcp.tool_proxy")
        .bind("1.0.0")
        .bind(1_i64)
        .bind("nomifun.mcp-connectors")
        .bind("1.0.0")
        .execute(&pool)
        .await
        .expect("MCP mapping");

        let config = McpRuntimeCatalogConfig {
            servers: vec![McpRuntimeServerConfig {
                server_id: "server-1".to_owned(),
                connection_config_ref: "connection-1".to_owned(),
                enabled: true,
                transport: McpRuntimeTransportConfig::Http {
                    url: endpoint.to_owned(),
                    headers: HashMap::new(),
                },
                tools: vec![McpRuntimeToolConfig {
                    canonical_tool_key: "vendor.echo".to_owned(),
                    remote_tool_name: "remote.echo".to_owned(),
                    input_schema: schema.clone(),
                }],
            }],
        };
        sqlx::query(
            "INSERT INTO plugin_configs (package_id, mount_id, config_json, revision) \
             VALUES (?, ?, ?, ?)",
        )
        .bind("nomifun.mcp-connectors")
        .bind("domain-mcp-connectors")
        .bind(serde_json::to_string(&config).expect("MCP config JSON"))
        .bind(1_i64)
        .execute(&pool)
        .await
        .expect("MCP runtime config");
        pool
    }

    fn source_resource_binding() -> TypedResourceBinding {
        TypedResourceBinding {
            binding_id: "mcp-binding-1".into(),
            resource_kind: MCP_SERVER_RESOURCE_KIND.into(),
            resource_id: "server-1".into(),
            owner_id: "owner-1".to_owned(),
            operations: BTreeSet::from([
                MCP_CONNECT_OPERATION.to_owned(),
                MCP_INVOKE_OPERATION.to_owned(),
            ]),
            connection_config_ref: Some(ConnectionConfigRef::from("connection-1")),
            typed_parameters: Default::default(),
        }
    }

    fn source_lock(schema: &Value) -> ResolvedMcpToolLock {
        ResolvedMcpToolLock {
            server_id: McpServerId::from("server-1"),
            canonical_tool_key: "vendor.echo".into(),
            capability_id: "mcp.tool_proxy".into(),
            schema_digest: digest_payload(schema).expect("schema digest"),
            materialization_revision: 1,
        }
    }

    #[tokio::test]
    async fn sqlite_source_resolves_only_exact_v4_server_mapping_and_tool_facts() {
        let schema = schema();
        let pool = fresh_v4_pool_for_source("http://127.0.0.1:1/mcp", &schema).await;
        let source = SqliteMcpRuntimeBindingSource::new(pool.clone());
        let resolved = source
            .resolve(
                &source_lock(&schema),
                &source_resource_binding(),
                &principal(),
            )
            .await
            .expect("exact v4 MCP source resolution");

        assert_eq!(resolved.server.server_id, McpServerId::from("server-1"));
        assert_eq!(resolved.server.server_owner_id, "system");
        assert_eq!(
            resolved.server.connection_config_ref,
            ConnectionConfigRef::from("connection-1")
        );
        assert_eq!(resolved.remote_tool.remote_tool_name, "remote.echo");
        assert_eq!(resolved.remote_tool.input_schema, schema);
        pool.close().await;
    }

    #[tokio::test]
    async fn sqlite_source_rejects_snapshot_mapping_drift_before_owner_execution() {
        let schema = schema();
        let pool = fresh_v4_pool_for_source("http://127.0.0.1:1/mcp", &schema).await;
        let source = SqliteMcpRuntimeBindingSource::new(pool.clone());
        let mut lock = source_lock(&schema);
        lock.canonical_tool_key = "vendor.other".into();
        let error = source
            .resolve(&lock, &source_resource_binding(), &principal())
            .await
            .expect_err("drifted MCP mapping must fail closed");
        assert_eq!(error.code, "MCP_MAPPING_NOT_FOUND");
        pool.close().await;
    }

    #[tokio::test]
    async fn sqlite_source_facts_reach_the_real_owner_tool_call() {
        let state = FixtureState::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fixture listener");
        let endpoint = format!(
            "http://{}/mcp",
            listener.local_addr().expect("fixture address")
        );
        let router = Router::new()
            .route("/mcp", post(fixture_handler))
            .with_state(state.clone());
        let server_task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        let schema = schema();
        let pool = fresh_v4_pool_for_source(&endpoint, &schema).await;
        let source = SqliteMcpRuntimeBindingSource::new(pool.clone());
        let lock = source_lock(&schema);
        let resource = source_resource_binding();
        let resolved = source
            .resolve(&lock, &resource, &principal())
            .await
            .expect("resolve exact v4 runtime facts");
        let owner = McpOwner::new(
            Arc::new(nomifun_mcp::AnonymousMcpCredentialAuthority),
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("fixture HTTP client"),
        );
        let adapter = McpOwnerAdapter::new(Arc::new(owner));
        let result = adapter
            .invoke(McpOwnerInvocationInput {
                mcp_tool_lock: lock,
                server: resolved.server,
                resource_binding: resource,
                remote_tool: resolved.remote_tool,
                principal: principal(),
                operation_id: OperationId::from("source-owner-operation"),
                arguments: StrictJsonValue(serde_json::json!({"message": "hello"})),
            })
            .await
            .expect("v4 source-backed MCP owner invocation");
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

        pool.close().await;
        server_task.abort();
        let _ = server_task.await;
    }
}
