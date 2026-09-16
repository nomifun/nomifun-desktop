//! Current-product MCP execution port. This is not the Fresh-v4 SQL adapter.
//! The product catalog belongs to the installation owner (its rows have no
//! user_id column). Every call still needs an exact Snapshot lock and grant.
//! Catalog publication/admission is deliberately separate from this owner.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ConnectionConfigRef, McpToolKey, PrincipalRef, ResolvedMcpToolLock, StrictJsonValue,
    TypedResourceBinding, digest_payload,
};
use nomifun_agent_domain_wave2::Wave2HostPortError;
use nomifun_common::AppError;
use nomifun_db::IMcpServerRepository;
use nomifun_mcp::{McpOwner, McpServer, OAuthMcpCredentialAuthority};

use super::agent_wave2_mcp::{
    McpOwnerAdapter, McpOwnerInvocationInput, McpRemoteToolFacts, McpRuntimeBindingSource,
    McpServerBindingFacts, ResolvedMcpRuntimeBinding, build_mcp_tool_invocation_request,
};

const MAX_CATALOG_BYTES: usize = 1024 * 1024;
const MAX_TOOLS: usize = 256;
const MAX_UNCERTAIN_SESSIONS: usize = 1024;

fn error(code: &str, message: &str) -> Wave2HostPortError {
    Wave2HostPortError::new(code, message)
}

/// Versioned identity rule for a future product catalog materializer. It must
/// use this exact key, schema digest and revision 1 in the published mapping.
/// Never split a model name or use arguments to resolve a remote tool.
pub(crate) fn canonical_tool_key(
    server_id: &str,
    remote_name: &str,
) -> Result<McpToolKey, Wave2HostPortError> {
    let server_id = nomifun_api_types::McpServerId::parse(server_id.to_owned()).map_err(|_| {
        error(
            "MCP_BINDING_INVALID",
            "MCP server identity is not a canonical UUIDv7",
        )
    })?;
    nomifun_mcp::canonical_mcp_tool_capability_id(&server_id, remote_name)
        .map(McpToolKey::from)
        .map_err(|_| {
            error(
                "MCP_BINDING_INVALID",
                "MCP identity cannot be canonicalized",
            )
        })
}

pub(crate) struct NomiCoreMcpRuntimeBindingSource {
    repository: Arc<dyn IMcpServerRepository>,
    installation_owner: Arc<str>,
}

impl NomiCoreMcpRuntimeBindingSource {
    async fn resource_server(
        &self,
        principal: &PrincipalRef,
        resource: &TypedResourceBinding,
    ) -> Result<nomifun_mcp::McpServerBinding, Wave2HostPortError> {
        if principal.principal_kind != "user"
            || principal.principal_id != self.installation_owner.as_ref()
            || resource.owner_id != principal.principal_id
            || resource.resource_kind.as_ref() != "mcp_server"
            || resource.binding_id.as_ref().is_empty()
            || !resource.typed_parameters.is_empty()
            || !resource.operations.contains("connect")
            || !resource.operations.contains("read")
        {
            return Err(error(
                "MCP_RESOURCE_OPERATION_DENIED",
                "MCP resource access requires the exact owner and connect/read binding",
            ));
        }
        let row = self
            .repository
            .find_by_id(resource.resource_id.as_ref())
            .await
            .map_err(|_| {
                error(
                    "MCP_CATALOG_UNAVAILABLE",
                    "MCP server could not be resolved",
                )
            })?
            .ok_or_else(|| {
                error(
                    "MCP_SERVER_NOT_FOUND",
                    "The exact MCP server no longer exists",
                )
            })?;
        if row.mcp_server_id != resource.resource_id.as_ref()
            || row.deleted_at.is_some()
            || !row.enabled
            || row.transport_config.len() > 256 * 1024
        {
            return Err(error(
                "MCP_SERVER_DISABLED",
                "The exact MCP server is not available",
            ));
        }
        let config = format!("mcp-server:{}@{}", row.mcp_server_id, row.updated_at);
        if resource.connection_config_ref.as_ref().map(AsRef::as_ref) != Some(config.as_str()) {
            return Err(error(
                "MCP_CONNECTION_CONFIG_STALE",
                "MCP configuration changed after resource admission",
            ));
        }
        // Do not parse/require an unrelated tools catalog for resources-only servers.
        let transport =
            nomifun_mcp::McpServerTransport::from_db(&row.transport_type, &row.transport_config)
                .map_err(|_| {
                    error(
                        "MCP_SERVER_CONFIG_INVALID",
                        "MCP transport configuration is unsupported",
                    )
                })?;
        Ok(nomifun_mcp::McpServerBinding {
            server_id: row.mcp_server_id,
            server_owner_id: self.installation_owner.to_string(),
            enabled: true,
            connection_config_ref: config.clone(),
            resource_binding_id: resource.binding_id.as_ref().to_owned(),
            resource_kind: "mcp_server".into(),
            resource_id: resource.resource_id.as_ref().to_owned(),
            resource_owner_id: resource.owner_id.clone(),
            granted_operations: resource.operations.clone(),
            resource_connection_config_ref: Some(config),
            transport,
        })
    }
}

#[async_trait]
impl McpRuntimeBindingSource for NomiCoreMcpRuntimeBindingSource {
    async fn resolve(
        &self,
        lock: &ResolvedMcpToolLock,
        resource: &TypedResourceBinding,
        principal: &PrincipalRef,
    ) -> Result<ResolvedMcpRuntimeBinding, Wave2HostPortError> {
        if principal.principal_kind != "user"
            || principal.principal_id != self.installation_owner.as_ref()
            || resource.owner_id != principal.principal_id
        {
            return Err(error(
                "MCP_RESOURCE_OWNER_MISMATCH",
                "MCP catalog requires the installation owner",
            ));
        }
        if resource.resource_kind.as_ref() != "mcp_server"
            || resource.resource_id.as_ref() != lock.server_id.as_ref()
            || resource.binding_id.as_ref().is_empty()
            || !resource.typed_parameters.is_empty()
            || !resource.operations.contains("connect")
            || !resource.operations.contains("invoke")
            || lock.materialization_revision
                != nomifun_mcp::MCP_TOOL_MATERIALIZATION_REVISION
        {
            return Err(error(
                "MCP_BINDING_INVALID",
                "MCP mapping has no exact supported resource grant",
            ));
        }
        let row = self
            .repository
            .find_by_id(lock.server_id.as_ref())
            .await
            .map_err(|_| error("MCP_CATALOG_UNAVAILABLE", "MCP catalog could not be read"))?
            .ok_or_else(|| {
                error(
                    "MCP_SERVER_NOT_FOUND",
                    "the exact MCP server no longer exists",
                )
            })?;
        if row.mcp_server_id != lock.server_id.as_ref() || row.deleted_at.is_some() || !row.enabled
        {
            return Err(error(
                "MCP_SERVER_DISABLED",
                "the exact MCP server is not active",
            ));
        }
        let config_ref = ConnectionConfigRef::from(format!(
            "mcp-server:{}@{}",
            row.mcp_server_id, row.updated_at
        ));
        if resource.connection_config_ref.as_ref() != Some(&config_ref) {
            return Err(error(
                "MCP_CONNECTION_CONFIG_STALE",
                "MCP configuration changed after resource resolution",
            ));
        }
        if row.transport_config.len() > 256 * 1024
            || row
                .tools
                .as_ref()
                .is_some_and(|tools| tools.len() > MAX_CATALOG_BYTES)
        {
            return Err(error(
                "MCP_CATALOG_TOO_LARGE",
                "MCP catalog exceeds the execution bound",
            ));
        }
        let server = McpServer::from_row(row).map_err(|_| {
            error(
                "MCP_SERVER_CONFIG_INVALID",
                "MCP configuration or tool catalog is malformed",
            )
        })?;
        if server.tools.len() > MAX_TOOLS {
            return Err(error(
                "MCP_CATALOG_TOO_LARGE",
                "MCP catalog exceeds 256 tools",
            ));
        }
        let mut names = std::collections::BTreeSet::new();
        let mut selected = None;
        for tool in server.tools {
            if tool.name.is_empty()
                || tool.name.len() > 256
                || tool.name.trim() != tool.name
                || tool.name.chars().any(char::is_control)
                || !names.insert(tool.name.clone())
            {
                return Err(error(
                    "MCP_CATALOG_INVALID",
                    "MCP tool identities are malformed or duplicated",
                ));
            }
            if canonical_tool_key(lock.server_id.as_ref(), &tool.name)? != lock.canonical_tool_key {
                continue;
            }
            let schema = tool
                .input_schema
                .filter(serde_json::Value::is_object)
                .ok_or_else(|| {
                    error(
                        "MCP_SCHEMA_MISMATCH",
                        "the exact MCP tool has no object schema",
                    )
                })?;
            if digest_payload(&schema)
                .map_err(|_| error("MCP_SCHEMA_MISMATCH", "MCP schema cannot be canonicalized"))?
                != lock.schema_digest
            {
                return Err(error(
                    "MCP_SCHEMA_MISMATCH",
                    "MCP catalog schema differs from the Snapshot lock",
                ));
            }
            selected = Some(McpRemoteToolFacts {
                remote_tool_name: tool.name,
                input_schema: schema,
            });
        }
        Ok(ResolvedMcpRuntimeBinding {
            server: McpServerBindingFacts {
                server_id: lock.server_id.clone(),
                server_owner_id: self.installation_owner.to_string(),
                enabled: true,
                connection_config_ref: config_ref,
                transport: server.transport,
            },
            remote_tool: selected.ok_or_else(|| {
                error("MCP_TOOL_NOT_FOUND", "the exact frozen MCP tool is absent")
            })?,
        })
    }
}

type SessionKey = (String, String);
#[derive(Default)]
struct SessionState {
    active: bool,
    uncertain: bool,
}

/// A dropped invocation cannot become an apparent cleanup success. Healthy
/// entries are removed; uncertain entries remain fenced in memory as well as
/// in the durable owner receipts used by cross-boot reconciliation.
struct InvocationGuard<'a> {
    sessions: &'a Mutex<BTreeMap<SessionKey, SessionState>>,
    key: SessionKey,
    settled: bool,
}

impl InvocationGuard<'_> {
    fn settle(mut self, uncertain: bool) {
        if let Ok(mut sessions) = self.sessions.lock() {
            if uncertain {
                if let Some(state) = sessions.get_mut(&self.key) {
                    state.active = false;
                    state.uncertain = true;
                }
            } else {
                sessions.remove(&self.key);
            }
            self.settled = true;
        }
    }
}

impl Drop for InvocationGuard<'_> {
    fn drop(&mut self) {
        if !self.settled
            && let Ok(mut sessions) = self.sessions.lock()
        {
            if let Some(state) = sessions.get_mut(&self.key) {
                state.active = false;
                state.uncertain = true;
            }
        }
    }
}

pub(crate) struct NomiCoreMcpHost {
    receipts: super::mcp_effect_receipts::McpEffectReceipts,
    source: NomiCoreMcpRuntimeBindingSource,
    owner: McpOwnerAdapter,
    sessions: Mutex<BTreeMap<SessionKey, SessionState>>,
}

/// Only MCP authority facts cross this boundary; the remote owner never
/// receives a Plugin state handle or unrelated lifecycle/service authority.
pub(crate) struct McpExecutionContext {
    principal: PrincipalRef,
    agent_session_id: nomifun_agent_contracts::AgentSessionId,
    operation_id: nomifun_agent_contracts::OperationId,
    capability_id: nomifun_agent_contracts::CapabilityId,
    resource_bindings: Vec<TypedResourceBinding>,
    mcp_tool_lock: Option<ResolvedMcpToolLock>,
}
impl From<nomifun_agent_kernel::CapabilityInvocationContext> for McpExecutionContext {
    fn from(context: nomifun_agent_kernel::CapabilityInvocationContext) -> Self {
        Self {
            principal: context.principal,
            agent_session_id: context.agent_session_id,
            operation_id: context.operation_id,
            capability_id: context.capability_id,
            resource_bindings: context.resource_bindings,
            mcp_tool_lock: context.mcp_tool_lock,
        }
    }
}

impl NomiCoreMcpHost {
    /// Called only after the shared Session host checks the exact frozen
    /// `mcp_server` binding. Resource reads are binding-derived and never
    /// depend on an Agent-authorable connection/resource capability.
    pub(crate) async fn resource(
        &self,
        principal: PrincipalRef,
        session: nomifun_agent_contracts::AgentSessionId,
        operation_id: nomifun_agent_contracts::OperationId,
        resource: TypedResourceBinding,
        operation: nomifun_mcp::McpResourceOperation,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let server = self.source.resource_server(&principal, &resource).await?;
        let digest = digest_payload(&("nomi-mcp-resource-v1", &principal, &session, &operation_id))
            .map_err(|_| {
                error(
                    "MCP_OPERATION_ID_INVALID",
                    "MCP resource operation identity is invalid",
                )
            })?;
        let request = nomifun_mcp::McpResourceRequest {
            principal_kind: principal.principal_kind.clone(),
            principal_id: principal.principal_id.clone(),
            operation_id: format!("nomi-mcp-resource-{}", digest.as_ref()),
            server,
            operation,
        };
        request.validate().map_err(|_| {
            error(
                "MCP_RESOURCE_REQUEST_INVALID",
                "MCP resource request is invalid",
            )
        })?;
        let guard = self.claim_session(&principal.principal_id, session.as_ref())?;
        let receipt = match self
            .receipts
            .begin(
                &principal.principal_id,
                session.as_ref(),
                operation_id.as_ref(),
                super::mcp_effect_receipts::MCP_SERVER_RESOURCE_EFFECT,
            )
            .await
        {
            Ok(receipt) => receipt,
            Err(_) => {
                // No protocol owner was entered. A potentially committed SQL
                // reservation remains durably fenced, but local budget/input
                // rejection must not invent an in-memory remote transaction.
                guard.settle(false);
                return Err(error(
                    "MCP_EFFECT_RECEIPT_UNAVAILABLE",
                    "MCP resource transaction has no durable turn authority",
                ));
            }
        };
        let result = self.owner.resource(request).await;
        // Ok includes typed resource rejection after successful cleanup.
        // Settlement records the observed return, never successful data or rollback.
        if let Ok(value) = &result {
            self.receipts.settle(receipt, &value.0).await.map_err(|_| {
                error(
                    "MCP_EFFECT_SETTLEMENT_UNCERTAIN",
                    "MCP resource settlement is unproven",
                )
            })?;
        }
        guard.settle(result.is_err());
        result
    }

    fn claim_session(
        &self,
        principal: &str,
        session: &str,
    ) -> Result<InvocationGuard<'_>, Wave2HostPortError> {
        let key = (principal.to_owned(), session.to_owned());
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| error("MCP_SESSION_UNCERTAIN", "MCP session state is poisoned"))?;
        if sessions.contains_key(&key) || sessions.len() >= MAX_UNCERTAIN_SESSIONS {
            return Err(error(
                "MCP_SESSION_UNCERTAIN",
                "MCP session is busy, fenced, or uncertainty bound exhausted",
            ));
        }
        sessions.insert(
            key.clone(),
            SessionState {
                active: true,
                uncertain: false,
            },
        );
        Ok(InvocationGuard {
            sessions: &self.sessions,
            key,
            settled: false,
        })
    }

    pub(crate) fn for_services(services: &crate::services::AppServices) -> Self {
        let pool = services.database.pool().clone();
        let oauth = Arc::new(nomifun_mcp::McpOAuthService::new_dynamic(Arc::new(
            nomifun_db::SqliteOAuthTokenRepository::new(pool.clone()),
        )));
        Self {
            receipts: super::mcp_effect_receipts::McpEffectReceipts::new(pool.clone()),
            source: NomiCoreMcpRuntimeBindingSource {
                repository: Arc::new(nomifun_db::SqliteMcpServerRepository::new(pool)),
                installation_owner: services.authoritative_user_id.clone(),
            },
            owner: McpOwnerAdapter::new(Arc::new(McpOwner::new_dynamic(Arc::new(
                OAuthMcpCredentialAuthority::new(oauth),
            )))),
            sessions: Mutex::new(BTreeMap::new()),
        }
    }

    pub(crate) async fn invoke(
        &self,
        context: McpExecutionContext,
        arguments: StrictJsonValue,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let lock = context
            .mcp_tool_lock
            .as_ref()
            .filter(|lock| {
                lock.capability_id == context.capability_id
                    && super::nomi_core_mcp_catalog::is_product_tool(
                        context.capability_id.as_ref(),
                    )
                    && context.capability_id.as_ref() == lock.canonical_tool_key.as_ref()
            })
            .ok_or_else(|| {
                error(
                    "MCP_MAPPING_NOT_FROZEN",
                    "MCP execution requires an exact supported Snapshot mapping",
                )
            })?;
        let [resource] = context.resource_bindings.as_slice() else {
            return Err(error(
                "PRESET_RESOURCE_NOT_BOUND",
                "MCP execution requires one exact resource binding",
            ));
        };
        let resolved = self
            .source
            .resolve(lock, resource, &context.principal)
            .await?;
        // Hash all identity components, never truncate operation IDs. The
        // platform journal retains the original operation/call association.
        let operation = digest_payload(&(
            "nomi-mcp-operation-v1",
            &context.principal,
            &context.agent_session_id,
            &context.operation_id,
        ))
        .map_err(|_| {
            error(
                "MCP_OPERATION_ID_INVALID",
                "MCP operation cannot be canonicalized",
            )
        })?;
        let input = McpOwnerInvocationInput {
            mcp_tool_lock: lock.clone(),
            server: resolved.server,
            resource_binding: resource.clone(),
            remote_tool: resolved.remote_tool,
            principal: context.principal.clone(),
            operation_id: format!("nomi-mcp-{}", operation.as_ref()).into(),
            arguments,
        };
        // Fail malformed local inputs before opening a potentially effectful
        // owner transaction. No workspace binding is involved in this path.
        build_mcp_tool_invocation_request(input.clone())?;
        let guard = self.claim_session(
            &context.principal.principal_id,
            context.agent_session_id.as_ref(),
        )?;
        let receipt = match self
            .receipts
            .begin(
                &context.principal.principal_id,
                context.agent_session_id.as_ref(),
                context.operation_id.as_ref(),
                context.capability_id.as_ref(),
            )
            .await
        {
            Ok(receipt) => receipt,
            Err(_) => {
                // No owner was entered; keep possible pending SQL evidence,
                // but do not invent an in-memory unknown remote transaction.
                guard.settle(false);
                return Err(error(
                    "MCP_EFFECT_RECEIPT_UNAVAILABLE",
                    "MCP dispatch has no durable live-turn authority",
                ));
            }
        };
        let result = self.owner.invoke(input).await;
        if let Ok(value) = &result {
            self.receipts.settle(receipt, &value.0).await.map_err(|_| {
                error(
                    "MCP_EFFECT_SETTLEMENT_UNCERTAIN",
                    "MCP result could not be durably settled; do not replay",
                )
            })?;
        }
        // Conservatively fence any owner failure, even initialize/credential
        // failures: a transport may have allocated an unseen remote session or
        // started a local process. Local catalog/input rejection preceded this guard.
        // Narrower retry admission needs an explicit no-effect receipt.
        guard.settle(result.is_err());
        result.and_then(super::agent_wave2_mcp::project_mcp_tool_result)
    }

    /// Observation after the engine tool task group has joined. Never clear
    /// an unresolved effect merely because there is no local process left.
    pub(crate) async fn ensure_settled(&self, owner: &str, session: &str) -> Result<(), AppError> {
        self.ensure_memory_settled(owner, session)?;
        self.receipts.ensure_settled(owner, session).await
    }

    pub(crate) async fn ensure_source_replay_safe(
        &self,
        owner: &str,
        session: &str,
        source: &str,
    ) -> Result<(), AppError> {
        self.ensure_memory_settled(owner, session)?;
        self.receipts
            .ensure_source_replay_safe(owner, session, source)
            .await
    }

    pub(crate) async fn recovery_context(
        &self,
        owner: &str,
        session: &str,
    ) -> Result<Option<String>, AppError> {
        self.ensure_memory_settled(owner, session)?;
        self.receipts.recovery_context(owner, session).await
    }

    fn ensure_memory_settled(&self, owner: &str, session: &str) -> Result<(), AppError> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| AppError::Conflict("MCP session state is poisoned".into()))?;
        if sessions
            .get(&(owner.to_owned(), session.to_owned()))
            .is_some_and(|state| state.active || state.uncertain)
        {
            return Err(AppError::Conflict(
                "MCP execution or cleanup has no settled outcome; Session remains fenced".into(),
            ));
        }
        Ok(())
    }
}
