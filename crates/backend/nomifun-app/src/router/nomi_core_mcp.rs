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
use nomifun_mcp::{McpOwner, McpServer};

use super::agent_wave2_mcp::{
    McpOwnerAdapter, McpOwnerInvocationInput, McpRemoteToolFacts, McpRuntimeBindingSource,
    McpServerBindingFacts, ResolvedMcpRuntimeBinding, build_mcp_tool_invocation_request,
};

const MAX_CATALOG_BYTES: usize = 1024 * 1024;
const MAX_TOOLS: usize = 256;
const MAX_ACTIVE_SESSIONS: usize = 1024;

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
        // Do not parse/require an unrelated tools catalog for resources-only servers.
        let transport =
            nomifun_mcp::McpServerTransport::from_db(&row.transport_type, &row.transport_config)
                .map_err(|_| {
                    error(
                        "MCP_SERVER_CONFIG_INVALID",
                        "MCP transport configuration is unsupported",
                    )
                })?;
        let server_id = nomifun_api_types::McpServerId::parse(row.mcp_server_id.clone())
            .map_err(|_| error("MCP_SERVER_CONFIG_INVALID", "MCP server identity is invalid"))?;
        let config = nomifun_mcp::canonical_mcp_connection_config_ref(&server_id, &transport)
            .map_err(|_| error("MCP_SERVER_CONFIG_INVALID", "MCP connection cannot be canonicalized"))?;
        if resource.connection_config_ref.as_ref().map(AsRef::as_ref) != Some(config.as_str()) {
            return Err(error(
                "MCP_CONNECTION_CONFIG_STALE",
                "MCP configuration changed after resource admission",
            ));
        }
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
        let config_ref = nomifun_mcp::canonical_mcp_connection_config_ref(
            &server.mcp_server_id,
            &server.transport,
        )
        .map(ConnectionConfigRef::from)
        .map_err(|_| error("MCP_SERVER_CONFIG_INVALID", "MCP connection cannot be canonicalized"))?;
        if resource.connection_config_ref.as_ref() != Some(&config_ref) {
            return Err(error(
                "MCP_CONNECTION_CONFIG_STALE",
                "MCP configuration changed after resource resolution",
            ));
        }
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
/// This registry owns in-flight invocation exclusion only. Uncertain outcomes
/// remain in the canonical Effect chain, including after a task is dropped.
/// An exact instance identity prevents old cleanup from releasing a successor.
struct InvocationGuard<'a> {
    sessions: &'a Mutex<BTreeMap<SessionKey, Arc<()>>>,
    key: SessionKey,
    invocation: Arc<()>,
}

impl Drop for InvocationGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.sessions.lock()
            && sessions.get(&self.key)
                .is_some_and(|current| Arc::ptr_eq(current, &self.invocation))
        {
            sessions.remove(&self.key);
        }
    }
}

pub(crate) struct NomiCoreMcpHost {
    receipts: super::mcp_effect_receipts::McpEffectReceipts,
    source: NomiCoreMcpRuntimeBindingSource,
    owner: McpOwnerAdapter,
    sessions: Mutex<BTreeMap<SessionKey, Arc<()>>>,
}

/// Only MCP authority facts cross this boundary; the remote owner never
/// receives a Plugin state handle or unrelated lifecycle/service authority.
pub(crate) struct McpExecutionContext {
    principal: PrincipalRef,
    agent_session_id: nomifun_agent_contracts::AgentSessionId,
    turn_id: nomifun_agent_contracts::OperationId,
    resolved_snapshot_ref: nomifun_agent_contracts::ResolvedSnapshotRef,
    control: nomifun_agent_kernel::InvocationControl,
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
            turn_id: context.turn_id,
            resolved_snapshot_ref: context.resolved_snapshot_ref,
            control: context.control,
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
        control: nomifun_agent_kernel::InvocationControl,
        turn_id: nomifun_agent_contracts::OperationId,
        snapshot: nomifun_agent_contracts::ResolvedSnapshotRef,
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
        self.receipts.ensure_settled(&principal.principal_id, session.as_ref()).await
            .map_err(|_| error("MCP_SESSION_UNCERTAIN", "MCP has an unresolved canonical outcome"))?;
        let receipt = match self
            .receipts
            .begin(
                &principal.principal_id,
                session.as_ref(),
                operation_id.as_ref(),
                super::mcp_effect_receipts::MCP_SERVER_RESOURCE_EFFECT,
                &turn_id,
                &snapshot,
            )
            .await
        {
            Ok(receipt) => receipt,
            Err(_) => {
                // No protocol owner was entered. A potentially committed SQL
                // reservation remains durably fenced, but local budget/input
                // rejection must not invent an in-memory remote transaction.
                drop(guard);
                return Err(error(
                    "MCP_EFFECT_RECEIPT_UNAVAILABLE",
                    "MCP resource transaction has no durable turn authority",
                ));
            }
        };
        let outcome = self.owner.resource_controlled(request, control).await;
        // The original task owns protocol shutdown and durable settlement.
        // Cleanup uncertainty preserves any observed result in that receipt.
        self.receipts.settle(receipt, &outcome).await.map_err(|_| {
            error("MCP_EFFECT_SETTLEMENT_UNCERTAIN", "MCP resource settlement is unproven")
        })?;
        drop(guard);
        project_owner_outcome(outcome)
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
        if sessions.contains_key(&key) || sessions.len() >= MAX_ACTIVE_SESSIONS {
            return Err(error(
                "MCP_SESSION_UNCERTAIN",
                "MCP session is busy or the active invocation bound is exhausted",
            ));
        }
        let invocation = Arc::new(());
        sessions.insert(key.clone(), invocation.clone());
        Ok(InvocationGuard {
            sessions: &self.sessions,
            key,
            invocation,
        })
    }

    pub(crate) fn for_services(services: &crate::services::AppServices) -> Self {
        let pool = services.database.pool().clone();
        Self {
            receipts: super::mcp_effect_receipts::McpEffectReceipts::new(pool.clone()),
            source: NomiCoreMcpRuntimeBindingSource {
                repository: services.app_mcp_sources.clone(),
                installation_owner: services.authoritative_user_id.clone(),
            },
            owner: McpOwnerAdapter::new(Arc::new(McpOwner::new_dynamic(services.mcp_auth.clone()))),
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
        self.receipts.ensure_settled(&context.principal.principal_id, context.agent_session_id.as_ref()).await
            .map_err(|_| error("MCP_SESSION_UNCERTAIN", "MCP has an unresolved canonical outcome"))?;
        let receipt = match self
            .receipts
            .begin(
                &context.principal.principal_id,
                context.agent_session_id.as_ref(),
                context.operation_id.as_ref(),
                context.capability_id.as_ref(),
                &context.turn_id,
                &context.resolved_snapshot_ref,
            )
            .await
        {
            Ok(receipt) => receipt,
            Err(_) => {
                // No owner was entered; keep possible pending SQL evidence,
                // but do not invent an in-memory unknown remote transaction.
                drop(guard);
                return Err(error(
                    "MCP_EFFECT_RECEIPT_UNAVAILABLE",
                    "MCP dispatch has no durable live-turn authority",
                ));
            }
        };
        let outcome = match self.owner.invoke_controlled(input, context.control).await {
            Ok(outcome) => outcome,
            Err(failure) => local_rejection(failure),
        };
        self.receipts.settle(receipt, &outcome).await.map_err(|_| {
            error("MCP_EFFECT_SETTLEMENT_UNCERTAIN", "MCP result could not be durably settled; do not replay")
        })?;
        drop(guard);
        project_owner_outcome(outcome).and_then(super::agent_wave2_mcp::project_mcp_tool_result)
    }

    /// Observation after the engine tool task group has joined. Never clear
    /// an unresolved effect merely because there is no local process left.
    pub(crate) async fn ensure_settled(&self, owner: &str, session: &str) -> Result<(), AppError> {
        self.ensure_memory_settled(owner, session)?;
        self.receipts.ensure_settled(owner, session).await
    }

    fn ensure_memory_settled(&self, owner: &str, session: &str) -> Result<(), AppError> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| AppError::Conflict("MCP session state is poisoned".into()))?;
        if sessions.contains_key(&(owner.to_owned(), session.to_owned())) {
            return Err(AppError::Conflict(
                "MCP execution or cleanup is still owned by an active task".into(),
            ));
        }
        Ok(())
    }
}

fn local_rejection(failure: Wave2HostPortError) -> nomifun_mcp::McpInvocationOutcome<StrictJsonValue> {
    let code = failure.canonical_code().as_ref().to_owned();
    nomifun_mcp::McpInvocationOutcome {
        result: Err(nomifun_mcp::McpOwnerError::new(code, failure.message)),
        execution: nomifun_mcp::McpExecutionFacts {
            request_dispatched: false, local_process_started: false, cancellation_requested: false,
        },
        cleanup: nomifun_mcp::McpCleanupFacts {
            local: nomifun_mcp::McpCleanupStatus::NotRequired,
            remote: nomifun_mcp::McpCleanupStatus::NotRequired,
            error: None,
        },
    }
}

fn project_owner_outcome(
    outcome: nomifun_mcp::McpInvocationOutcome<StrictJsonValue>,
) -> Result<StrictJsonValue, Wave2HostPortError> {
    if matches!(outcome.cleanup.local, nomifun_mcp::McpCleanupStatus::Unproven)
        || matches!(outcome.cleanup.remote, nomifun_mcp::McpCleanupStatus::Unproven)
    {
        return Err(error("MCP_SESSION_CLEANUP_FAILED",
            "MCP owner cleanup is unproven. Any returned observation was recorded with the original effect; do not replay the call."));
    }
    outcome.result.map_err(super::agent_wave2_mcp::map_mcp_owner_error)
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    use nomifun_db::{CreateMcpServerParams, SqliteMcpServerRepository, UpdateMcpServerParams};
    use std::collections::BTreeSet;

    #[tokio::test]
    async fn unchanged_probe_and_metadata_preserve_exact_connection_but_transport_edit_is_stale() {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let repository = Arc::new(SqliteMcpServerRepository::new(db.pool().clone()));
        let tools = serde_json::json!([{
            "name": "lookup", "description": "Lookup one value",
            "input_schema": {"type": "object", "additionalProperties": false}
        }]).to_string();
        let row = repository.create(CreateMcpServerParams {
            name: "test-server", description: None, enabled: true,
            transport_type: "http", transport_config: r#"{"url":"https://example.test/mcp"}"#,
            tools: Some(&tools), original_json: None, builtin: false,
        }).await.unwrap();
        let descriptor = super::super::nomi_core_mcp_catalog::server_tools(row.clone()).unwrap().remove(0);
        let principal = PrincipalRef { principal_kind: "user".into(), principal_id: "owner".into() };
        let resource = TypedResourceBinding {
            binding_id: "mcp-binding".into(), resource_kind: "mcp_server".into(),
            resource_id: row.mcp_server_id.clone().into(), owner_id: principal.principal_id.clone(),
            operations: BTreeSet::from(["connect".into(), "read".into(), "invoke".into()]),
            connection_config_ref: Some(descriptor.connection_config_ref.clone()),
            typed_parameters: BTreeMap::new(),
        };
        let source = NomiCoreMcpRuntimeBindingSource {
            repository: repository.clone(), installation_owner: "owner".into(),
        };
        source.resolve(&descriptor.lock, &resource, &principal).await.unwrap();
        assert!(repository.update_probe_if_revision(
            &row.mcp_server_id, row.updated_at, "connected", Some(row.updated_at + 1), Some(&tools),
        ).await.unwrap());
        let probed = repository.find_by_id(&row.mcp_server_id).await.unwrap().unwrap();
        assert!(probed.updated_at > row.updated_at);
        assert_eq!(super::super::nomi_core_mcp_catalog::server_tools(probed).unwrap()[0].connection_config_ref,
            descriptor.connection_config_ref);
        source.resolve(&descriptor.lock, &resource, &principal).await.unwrap();
        source.resource_server(&principal, &resource).await.unwrap();

        repository.update(&row.mcp_server_id, UpdateMcpServerParams {
            description: Some(Some("Updated presentation")), ..Default::default()
        }).await.unwrap();
        source.resolve(&descriptor.lock, &resource, &principal).await.unwrap();
        source.resource_server(&principal, &resource).await.unwrap();

        repository.update(&row.mcp_server_id, UpdateMcpServerParams {
            transport_config: Some(r#"{"url":"https://changed.test/mcp"}"#), ..Default::default()
        }).await.unwrap();
        assert_eq!(source.resolve(&descriptor.lock, &resource, &principal).await.unwrap_err().canonical_code().as_ref(),
            "MCP_CONNECTION_CONFIG_STALE");
        assert_eq!(source.resource_server(&principal, &resource).await.unwrap_err().canonical_code().as_ref(),
            "MCP_CONNECTION_CONFIG_STALE");
    }
}

#[cfg(test)]
#[path = "nomi_core_mcp_outcome_tests.rs"]
mod outcome_tests;
