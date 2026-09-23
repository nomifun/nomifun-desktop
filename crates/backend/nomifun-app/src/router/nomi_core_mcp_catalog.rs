//! Source-owned materializer for the current product's MCP tool catalog.
//! Each remote tool gets a distinct canonical capability, never a model-side
//! server/name router. Only non-secret catalog facts enter registration data.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::*;
use nomifun_agent_kernel::{
    CapabilityHandler, CapabilityInvocationContext, KernelError, MaterializedRegistry,
    PluginRegistration,
};
use nomifun_common::AppError;
use nomifun_db::IMcpServerRepository;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::nomi_core_mcp::canonical_tool_key;
use super::nomi_core_wave2::NomiCoreWave2Host;

const VERSION: &str = "1.0.0";
const PREFIX: &str = nomifun_mcp::MCP_TOOL_CAPABILITY_PREFIX;
pub(crate) const MAX_SESSION_SERVERS: usize = 16;

pub(crate) fn is_product_tool(id: &str) -> bool {
    nomifun_mcp::is_namespaced_mcp_tool_capability(id)
}

fn failure() -> AppError {
    AppError::Conflict(
        "MCP frozen catalog is malformed, stale, or exceeds its supported bounds".into(),
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FrozenMcpTool {
    pub lock: ResolvedMcpToolLock,
    pub connection_config_ref: ConnectionConfigRef,
    pub remote_tool_name: String,
    pub input_schema: Value,
    pub display_name: String,
    pub description: String,
}

impl FrozenMcpTool {
    fn suffix(&self) -> Result<&str, AppError> {
        if !is_product_tool(self.lock.capability_id.as_ref()) {
            return Err(failure());
        }
        Ok(&self.lock.capability_id.as_ref()[PREFIX.len()..])
    }
    fn package(&self) -> Result<PackageRef, AppError> {
        Ok(PackageRef {
            id: format!("nomifun.mcp.tool.{}", self.suffix()?).into(),
            version: VERSION.into(),
        })
    }
    fn mount(&self) -> Result<AgentModuleId, AppError> {
        Ok(format!("mcp-tool-{}", self.suffix()?).into())
    }
    pub(crate) fn action(&self) -> ActionId {
        nomifun_mcp::canonical_mcp_tool_action_id(self.lock.capability_id.as_ref())
            .expect("FrozenMcpTool validates the namespaced capability identity")
            .into()
    }
    pub(crate) fn input_ref(&self) -> CanonicalSchemaRef {
        format!(
            "schema://{}/input@1#{}",
            self.lock.capability_id.as_ref(),
            self.lock.schema_digest.as_ref()
        )
        .into()
    }
    fn validator(&self) -> Result<jsonschema::Validator, AppError> {
        if self.lock.materialization_revision
            != nomifun_mcp::MCP_TOOL_MATERIALIZATION_REVISION
            || self.remote_tool_name.is_empty()
            || self.remote_tool_name.len() > 256
            || self.remote_tool_name.trim() != self.remote_tool_name
            || self.remote_tool_name.chars().any(char::is_control)
            || !self.input_schema.is_object()
            || self.display_name.len() > 256
            || self.description.len() > 4096
            || serde_json::to_vec(&self.input_schema)
                .map_err(|_| failure())?
                .len()
                > 64 * 1024
            || canonical_tool_key(self.lock.server_id.as_ref(), &self.remote_tool_name)
                .map_err(|_| failure())?
                != self.lock.canonical_tool_key
            || self.lock.capability_id.as_ref() != self.lock.canonical_tool_key.as_ref()
            || digest_payload(&self.input_schema).map_err(|_| failure())? != self.lock.schema_digest
        {
            return Err(failure());
        }
        self.suffix()?;
        // Resolve schema references through a denying retriever. Do not scan
        // arbitrary JSON keys: a property or const value named $ref is data.
        jsonschema::options()
            .with_retriever(NoExternalSchema)
            .build(&self.input_schema)
            .map_err(|_| failure())
    }
}

struct NoExternalSchema;
impl jsonschema::Retrieve for NoExternalSchema {
    fn retrieve(
        &self,
        _uri: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err("MCP schemas cannot retrieve external resources".into())
    }
}

/// Bootstrap materialization. Ordinary MCP configuration never loads engine
/// code. Catalog refresh must use the existing serialized registry publisher.
pub(crate) async fn load_registrations(
    repository: &dyn IMcpServerRepository,
    host: Arc<NomiCoreWave2Host>,
) -> Result<Vec<PluginRegistration>, AppError> {
    let rows = repository.list().await.map_err(|_| failure())?;
    if rows.len() > 256 {
        return Err(failure());
    }
    let mut registrations = Vec::new();
    for row in rows {
        if row.deleted_at.is_some()
            || !row.enabled
            || !matches!(row.transport_type.as_str(), "http" | "sse" | "stdio")
        {
            continue;
        }
        let server_id = row.mcp_server_id.clone();
        match server_tools(row) {
            Ok(tools) => {
                if registrations.len() + tools.len() > 1024 {
                    return Err(failure());
                }
                // Build a server atomically; one malformed tool cannot publish
                // a misleading partial server catalog.
                let ready = tools
                    .into_iter()
                    .map(|tool| registration(tool, host.clone()))
                    .collect::<Result<Vec<_>, _>>();
                match ready {
                    Ok(ready) => registrations.extend(ready),
                    Err(_) => tracing::warn!(
                        server_id,
                        "MCP server catalog unavailable: invalid tool contract"
                    ),
                }
            }
            Err(_) => tracing::warn!(
                server_id,
                "MCP server catalog unavailable: invalid persisted catalog"
            ),
        }
    }
    registrations.sort_by(|left, right| left.metadata.mount_id.cmp(&right.metadata.mount_id));
    Ok(registrations)
}

pub(super) fn server_tools(
    row: nomifun_db::models::McpServerRow,
) -> Result<Vec<FrozenMcpTool>, AppError> {
    if row.deleted_at.is_some()
        || !row.enabled
        || !matches!(row.transport_type.as_str(), "http" | "sse" | "stdio")
        || row.transport_config.len() > 256 * 1024
        || row
            .tools
            .as_ref()
            .is_some_and(|tools| tools.len() > 1024 * 1024)
    {
        return Err(failure());
    }
    let config_ref = ConnectionConfigRef::from(format!(
        "mcp-server:{}@{}",
        row.mcp_server_id, row.updated_at
    ));
    let server = nomifun_mcp::McpServer::from_row(row).map_err(|_| failure())?;
    if server.tools.len() > 256 {
        return Err(failure());
    }
    let mut names = BTreeSet::new();
    let mut result = Vec::new();
    for tool in server.tools {
        if !names.insert(tool.name.clone()) {
            return Err(failure());
        }
        let input_schema = tool.input_schema.ok_or_else(failure)?;
        let key =
            canonical_tool_key(server.mcp_server_id.as_ref(), &tool.name).map_err(|_| failure())?;
        let descriptor = FrozenMcpTool {
            lock: ResolvedMcpToolLock {
                server_id: server.mcp_server_id.as_ref().to_owned().into(),
                canonical_tool_key: key.clone(),
                capability_id: key.as_ref().to_owned().into(),
                schema_digest: digest_payload(&input_schema).map_err(|_| failure())?,
                materialization_revision: nomifun_mcp::MCP_TOOL_MATERIALIZATION_REVISION,
            },
            connection_config_ref: config_ref.clone(),
            display_name: format!("{} / {}", server.name, tool.name),
            description: tool.description.unwrap_or_default(),
            remote_tool_name: tool.name,
            input_schema,
        };
        descriptor.validator()?;
        result.push(descriptor);
    }
    Ok(result)
}

fn display(name: &str, description: &str) -> LocalizedMetadata {
    LocalizedMetadata {
        name: name.into(),
        description: description.into(),
        localized_names: BTreeMap::new(),
        localized_descriptions: BTreeMap::new(),
    }
}
fn port(id: &str) -> HostPortRef {
    HostPortRef {
        id: id.into(),
        version: VERSION.into(),
    }
}

fn registration(
    tool: FrozenMcpTool,
    host: Arc<NomiCoreWave2Host>,
) -> Result<PluginRegistration, AppError> {
    let validator = Arc::new(tool.validator()?);
    let package = tool.package()?;
    let mount = tool.mount()?;
    let capability_id = tool.lock.capability_id.clone();
    let value = serde_json::to_value(&tool).map_err(|_| failure())?;
    // This immutable config carries only public tool metadata. Its const
    // schema makes the descriptor part of the package's artifact identity.
    let config_schema = StrictJsonValue(json!({"const": value}));
    let output_schema = digest_payload(&json!({"type":"object"})).map_err(|_| failure())?;
    let owner_port = port("host.nomi.mcp");
    let manifest = PackageManifest {
        schema_version: VERSION.into(),
        host_contract_version: VERSION.into(),
        package_id: package.id.clone(),
        package_version: package.version.clone(),
        display: display(&tool.display_name, &tool.description),
        package_dependencies: Vec::new(),
        requires_runtime_features: Vec::new(),
        config_schema: config_schema.clone(),
        provides_services: Vec::new(),
        requires_services: Vec::new(),
        entrypoint: InProcessEntrypointMetadata {
            entrypoint_profile: "trusted-in-process".into(),
            entrypoint_id: format!("{}.entrypoint", package.id.as_ref()),
            contract_version: VERSION.into(),
        }
        .into(),
        contributions: PackageContributions {
            capabilities: vec![CapabilityManifest {
                id: capability_id.clone(),
                contribution_id: format!("capability:{}", capability_id.as_ref()).into(),
                kind: CapabilityKind::Tool,
                package: package.clone(),
                display: display(&tool.display_name, &tool.description),
                requires: Vec::new(),
                conflicts: Vec::new(),
                supported_surfaces: capability_module_surface_declarations(
                    ["desktop", "headless"],
                    [CapabilityConsumer::Agent],
                    CapabilityAuthoringPolicy::Direct,
                ),
                requires_runtime_features: Vec::new(),
                supported_platforms: vec![PlatformConstraint::Any],
                config_schema: StrictJsonValue(
                    json!({"type":"object", "additionalProperties":false}),
                ),
                contributions: CapabilityContributions {
                    context_phase: Default::default(),
                    ui_slot: None,
                    actions: vec![CapabilityActionDescriptor {
                        action_id: tool.action(),
                        input_schema: tool.input_ref(),
                        output_schema: format!(
                            "schema://{}/output@1#{}",
                            capability_id.as_ref(),
                            output_schema.as_ref()
                        )
                        .into(),
                        effect_class: EffectClass::ExternalTransmit,
                        presentation: ToolPresentationKind::FunctionTool,
                    }],
                    context_schema_refs: Vec::new(),
                    event_schema_refs: Vec::new(),
                    resource_kinds: BTreeSet::from(["mcp_server".into()]),
                    host_ports: vec![owner_port.clone()],
                },
            }],
            skills: Vec::new(),
            role_contracts: Vec::new(),
            role_providers: Vec::new(),
            mcp_tools: vec![McpToolCapabilityMapping {
                package: package.clone(),
                server_id: tool.lock.server_id.clone(),
                canonical_tool_key: tool.lock.canonical_tool_key.clone(),
                schema_digest: tool.lock.schema_digest.clone(),
                capability: CapabilityRef {
                    id: capability_id.clone(),
                },
                materialization_version: VERSION.into(),
            }],
        },
    };
    let source = PluginSourceMetadata {
        source_kind: PluginSourceKind::Bundled,
        source_identity: package.id.as_ref().to_owned(),
        source_digest: None,
    };
    let identity = PluginIdentityDescriptor {
        package: package.clone(),
        mount_id: mount.clone(),
    };
    let cancel = port("host.plugin.cancel");
    let tasks = port("host.plugin.tasks");
    let scope = ScopeKey::from(format!("mount:{}", mount.as_ref()));
    let metadata = PluginRegistrationMetadata {
        manifest: ArtifactEnvelope::new(manifest).map_err(|_| failure())?,
        mount_id: mount.clone(),
        source: source.clone(),
        boot_state: PluginBootState {
            criticality: PluginBootCriticality::Required,
            desired_state: PluginDesiredState::Enabled,
            effective_state: PluginEffectiveState::Active,
            diagnostic_code: None,
        },
        registrar: PluginRegistrarDescriptor {
            identity: identity.clone(),
            allowed_operations: BTreeSet::from([
                PluginRegistrarOperation::ContributeCapability,
                PluginRegistrarOperation::ContributeMcpToolMapping,
                PluginRegistrarOperation::BindHostPort,
            ]),
            declared_capability_ids: BTreeSet::from([capability_id.clone()]),
            declared_skill_ids: BTreeSet::new(),
            declared_mcp_tool_keys: BTreeSet::from([tool.lock.canonical_tool_key.clone()]),
            declared_role_ids: BTreeSet::new(),
            declared_service_keys: BTreeSet::new(),
            declared_host_ports: BTreeSet::from([
                cancel.id.clone(),
                tasks.id.clone(),
                owner_port.id.clone(),
            ]),
        },
        context: PluginContextDescriptor {
            identity,
            source,
            validated_config: ValidatedPluginConfig {
                schema_digest: digest_payload(&config_schema).map_err(|_| failure())?,
                config_revision: 1,
                value: StrictJsonValue(value),
            },
            state: PluginStateHandleDescriptor {
                package_id: package.id,
                mount_id: mount,
                methods: PluginStateMethod::REQUIRED.into_iter().collect(),
            },
            declared_services: DeclaredServiceViewDescriptor::default(),
            host_ports: vec![HostPortBindingDescriptor {
                port: owner_port,
                request_schema: tool.input_ref(),
                response_schema: format!(
                    "schema://{}/output@1#{}",
                    capability_id.as_ref(),
                    output_schema.as_ref()
                )
                .into(),
            }],
            typed_command_ports: Vec::new(),
            domain_outbox_ports: Vec::new(),
            cancellation: CancellationDescriptor {
                cancellation_port: cancel,
                scope_key: scope.clone(),
            },
            managed_task_registration: ManagedTaskRegistrationDescriptor {
                registrar_port: tasks,
                scope_key: scope,
            },
        },
    };
    let mut registration = PluginRegistration::new(metadata);
    registration
        .add_capability_handler(
            capability_id,
            Arc::new(ToolHandler {
                tool,
                validator,
                host,
            }),
        )
        .map_err(|_| failure())?;
    Ok(registration)
}

struct ToolHandler {
    tool: FrozenMcpTool,
    validator: Arc<jsonschema::Validator>,
    host: Arc<NomiCoreWave2Host>,
}

#[async_trait]
impl CapabilityHandler for ToolHandler {
    async fn invoke(
        &self,
        context: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError> {
        let [resource] = context.resource_bindings.as_slice() else {
            return Err(KernelError::capability_execution_failed(
                "PRESET_RESOURCE_NOT_BOUND",
                "MCP tool needs one exact server binding",
            ));
        };
        if context.capability_id != self.tool.lock.capability_id
            || context.action_id != self.tool.action()
            || context.mcp_tool_lock.as_ref() != Some(&self.tool.lock)
            || resource.connection_config_ref.as_ref() != Some(&self.tool.connection_config_ref)
            || !input.0.is_object()
            || !self.validator.is_valid(&input.0)
        {
            return Err(KernelError::capability_execution_failed(
                "MCP_BINDING_INVALID",
                "MCP call differs from its frozen mapping, connection, or input schema",
            ));
        }
        self.host
            .invoke_mcp_tool(context, input)
            .await
            .map_err(|error| {
                KernelError::capability_execution_failed(error.canonical_code(), error.message)
            })
    }
}

/// Read the exact descriptor already materialized in the current Registry.
/// No database refresh or global tool discovery occurs during projection.
pub(crate) fn frozen_tool(
    registry: &MaterializedRegistry,
    selected: &ResolvedCapability,
) -> Result<FrozenMcpTool, AppError> {
    let capability = registry
        .capability(&selected.capability.id)
        .ok_or_else(failure)?;
    if capability.source.source_kind != PluginSourceKind::Bundled
        || selected.contribution_lock.source_kind != ContributionSourceKind::McpBinding
        || capability.contribution_lock != selected.contribution_lock
        || capability.schema_digest != selected.schema_digest
    {
        return Err(failure());
    }
    let metadata = registry
        .plugins
        .get(&capability.mount_id)
        .ok_or_else(failure)?;
    let tool: FrozenMcpTool =
        serde_json::from_value(metadata.context.validated_config.value.0.clone())
            .map_err(|_| failure())?;
    if tool.lock.capability_id != selected.capability.id
        || tool.package()? != capability.manifest.package
        || tool.mount()? != capability.mount_id
        || registry
            .mcp_for_capability(&selected.capability.id)
            .is_none_or(|mapping| {
                mapping.mapping.server_id != tool.lock.server_id
                    || mapping.mapping.canonical_tool_key != tool.lock.canonical_tool_key
                    || mapping.mapping.schema_digest != tool.lock.schema_digest
            })
    {
        return Err(failure());
    }
    tool.validator()?;
    Ok(tool)
}

pub(crate) fn validate_resources(
    compiled: &nomifun_agent_kernel::CompiledSnapshot,
    registry: &MaterializedRegistry,
    principal: &PrincipalRef,
) -> Result<(), AppError> {
    let resources = compiled
        .resource_bindings()
        .iter()
        .filter(|resource| resource.resource_kind.as_ref() == "mcp_server")
        .collect::<Vec<_>>();
    if resources.len() > MAX_SESSION_SERVERS
        || resources
            .iter()
            .map(|resource| &resource.resource_id)
            .collect::<BTreeSet<_>>()
            .len()
            != resources.len()
        || resources
            .iter()
            .map(|resource| &resource.binding_id)
            .collect::<BTreeSet<_>>()
            .len()
            != resources.len()
        || resources.iter().any(|resource| {
            resource.resource_id.as_ref().is_empty()
                || resource.resource_id.as_ref().len() > 256
                || resource.resource_id.as_ref().chars().any(char::is_control)
                || resource.owner_id != principal.principal_id
                || !resource.operations.contains("connect")
                || !resource.operations.contains("read")
                || !resource.typed_parameters.is_empty()
                || resource.connection_config_ref.is_none()
        })
    {
        return Err(failure());
    }
    let locks = &compiled.content().mcp_tool_locks;
    if locks.iter().any(|lock| {
        nomifun_api_types::McpServerId::parse(lock.server_id.as_ref().to_owned()).is_err()
            || !is_product_tool(lock.capability_id.as_ref())
            || lock.canonical_tool_key.as_ref() != lock.capability_id.as_ref()
            || lock.materialization_revision
                != nomifun_mcp::MCP_TOOL_MATERIALIZATION_REVISION
    }) {
        return Err(failure());
    }
    let lock_capability_ids = locks
        .iter()
        .map(|lock| lock.capability_id.clone())
        .collect::<BTreeSet<_>>();
    let selected_capability_ids = compiled
        .content()
        .enabled_capabilities
        .iter()
        .filter(|selected| is_product_tool(selected.capability.id.as_ref()))
        .map(|selected| selected.capability.id.clone())
        .collect::<BTreeSet<_>>();
    if lock_capability_ids.len() != locks.len()
        || selected_capability_ids != lock_capability_ids
    {
        return Err(failure());
    }
    let expected = locks
        .iter()
        .map(|lock| lock.server_id.as_ref())
        .collect::<BTreeSet<_>>();
    let actual_ids = resources
        .iter()
        .map(|resource| resource.resource_id.as_ref())
        .collect::<BTreeSet<_>>();
    if !expected.is_subset(&actual_ids) {
        return Err(failure());
    }
    for resource in &resources {
        let mut operations = BTreeSet::from(["connect".to_owned(), "read".to_owned()]);
        if expected.contains(resource.resource_id.as_ref()) {
            operations.insert("invoke".to_owned());
        }
        if resource.operations != operations {
            return Err(failure());
        }
    }
    for selected in &compiled.content().enabled_capabilities {
        if !is_product_tool(selected.capability.id.as_ref()) {
            continue;
        }
        let tool = frozen_tool(registry, selected)?;
        if !compiled.content().mcp_tool_locks.contains(&tool.lock) {
            return Err(failure());
        }
        let resources = compiled
            .resource_bindings()
            .iter()
            .filter(|resource| {
                resource.resource_kind.as_ref() == "mcp_server"
                    && resource.resource_id.as_ref() == tool.lock.server_id.as_ref()
            })
            .collect::<Vec<_>>();
        let [resource] = resources.as_slice() else {
            return Err(failure());
        };
        if resource.owner_id != principal.principal_id
            || compiled
                .policy(&selected.capability.id)
                .is_none_or(|policy| {
                    policy.resource_binding_ids != BTreeSet::from([resource.binding_id.clone()])
                })
            || resource.resource_id.as_ref() != tool.lock.server_id.as_ref()
            || resource.connection_config_ref.as_ref() != Some(&tool.connection_config_ref)
            || !resource.operations.contains("connect")
            || !resource.operations.contains("invoke")
            || !resource.typed_parameters.is_empty()
        {
            return Err(failure());
        }
    }
    Ok(())
}

pub(crate) fn settlement_witness(
    host: Arc<NomiCoreWave2Host>,
    owner: String,
    session: String,
) -> Arc<dyn nomifun_ai_agent::engine_effect_scope::EngineEffectSettlement> {
    Arc::new(McpSettlement {
        host,
        owner,
        session,
    })
}

/// Shared by engines using the frozen per-tool lane. Session overlays may
/// project these servers, never add authority beyond the Agent Snapshot.
pub(crate) fn validate_product_session_selection(
    snapshot: &ResolvedSnapshotEnvelope,
    resources: &[TypedResourceBinding],
    extra: &Value,
) -> Result<(), AppError> {
    if !snapshot.content.mcp_tool_locks.is_empty()
        || resources
            .iter()
            .any(|resource| resource.resource_kind.as_ref() == "mcp_server")
    {
        validate_session_selection(snapshot, resources, extra)?;
    }
    Ok(())
}

pub(crate) fn validate_session_selection(
    snapshot: &ResolvedSnapshotEnvelope,
    resources: &[TypedResourceBinding],
    extra: &Value,
) -> Result<(), AppError> {
    let lock_ids = snapshot
        .content
        .mcp_tool_locks
        .iter()
        .map(|lock| lock.capability_id.clone())
        .collect::<BTreeSet<_>>();
    let selected_tool_ids = snapshot
        .content
        .enabled_capabilities
        .iter()
        .filter(|selected| is_product_tool(selected.capability.id.as_ref()))
        .map(|selected| selected.capability.id.clone())
        .collect::<BTreeSet<_>>();
    if lock_ids.len() != snapshot.content.mcp_tool_locks.len()
        || selected_tool_ids != lock_ids
        || snapshot.content.mcp_tool_locks.iter().any(|lock| {
            nomifun_api_types::McpServerId::parse(lock.server_id.as_ref().to_owned()).is_err()
                || !is_product_tool(lock.capability_id.as_ref())
                || lock.canonical_tool_key.as_ref() != lock.capability_id.as_ref()
                || lock.materialization_revision
                    != nomifun_mcp::MCP_TOOL_MATERIALIZATION_REVISION
        })
    {
        return Err(failure());
    }
    let expected_tools = snapshot
        .content
        .mcp_tool_locks
        .iter()
        .map(|lock| lock.server_id.as_ref())
        .collect::<BTreeSet<_>>();
    // Resource-only servers have no frozen tool mapping. Their authority is
    // the typed platform binding itself, never an authorable generic switch
    // switch or an extra/alias value.
    let selected = resources
        .iter()
        .filter(|resource| resource.resource_kind.as_ref() == "mcp_server")
        .collect::<Vec<_>>();
    let expected = selected
        .iter()
        .map(|resource| resource.resource_id.as_ref())
        .collect::<BTreeSet<_>>();
    let binding_ids = selected
        .iter()
        .map(|resource| &resource.binding_id)
        .collect::<BTreeSet<_>>();
    if selected.len() > MAX_SESSION_SERVERS
        || expected.len() != selected.len()
        || binding_ids.len() != selected.len()
        || !expected_tools.is_subset(&expected)
        || selected.iter().any(|resource| {
            resource.resource_id.as_ref().is_empty()
                || resource.resource_id.as_ref().len() > 256
                || resource.resource_id.as_ref().chars().any(char::is_control)
                || !resource.operations.contains("connect")
                || !resource.operations.contains("read")
                || resource.connection_config_ref.is_none()
                || !resource.typed_parameters.is_empty()
        })
    {
        return Err(failure());
    }
    for resource in &selected {
        let mut operations = BTreeSet::from(["connect".to_owned(), "read".to_owned()]);
        if expected_tools.contains(resource.resource_id.as_ref()) {
            operations.insert("invoke".to_owned());
        }
        if resource.operations != operations {
            return Err(failure());
        }
    }
    if expected.len() > MAX_SESSION_SERVERS {
        return Err(failure());
    }
    for key in ["mcp_server_ids", "selected_mcp_server_ids"] {
        if let Some(value) = extra.get(key) {
            let values = value.as_array().ok_or_else(failure)?;
            let actual = values
                .iter()
                .map(|value| value.as_str().ok_or_else(failure))
                .collect::<Result<BTreeSet<_>, _>>()?;
            if actual != expected || actual.len() != values.len() {
                return Err(AppError::Conflict(
                    "MCP overlay differs from the Agent's exact frozen server mapping".into(),
                ));
            }
        }
    }
    if let Some(value) = extra.get("mcp_servers") {
        let names = value.as_array().ok_or_else(failure)?;
        if names.len() != expected.len()
            || names.iter().any(|name| {
                !name
                    .as_str()
                    .is_some_and(|name| !name.is_empty() && name.len() <= 256)
            })
        {
            return Err(AppError::Conflict(
                "MCP name projection differs from the frozen server selection".into(),
            ));
        }
    }
    Ok(())
}

struct McpSettlement {
    host: Arc<NomiCoreWave2Host>,
    owner: String,
    session: String,
}

#[async_trait]
impl nomifun_ai_agent::engine_effect_scope::EngineEffectSettlement for McpSettlement {
    async fn ensure_settled(&self) -> Result<(), AppError> {
        self.host
            .ensure_mcp_settled(&self.owner, &self.session)
            .await
    }
    async fn ensure_source_replay_safe(&self, source: &str) -> Result<(), AppError> {
        self.host
            .ensure_mcp_source_replay_safe(&self.owner, &self.session, source)
            .await
    }
}

pub(crate) fn recovery_context(
    host: Arc<NomiCoreWave2Host>,
    owner: String,
    session: String,
) -> Arc<dyn nomifun_ai_agent::ContextContributor> {
    Arc::new(McpSettlement {
        host,
        owner,
        session,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVER_ID: &str = "0195f7c0-7b6a-7c21-8f4a-1234567890ab";

    fn frozen_tool() -> FrozenMcpTool {
        let input_schema = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {"query": {"type": "string"}},
            "required": ["query"]
        });
        let server_id = nomifun_api_types::McpServerId::parse(SERVER_ID).unwrap();
        let capability_id =
            nomifun_mcp::canonical_mcp_tool_capability_id(&server_id, "lookup").unwrap();
        FrozenMcpTool {
            lock: ResolvedMcpToolLock {
                server_id: SERVER_ID.into(),
                canonical_tool_key: capability_id.clone().into(),
                capability_id: capability_id.into(),
                schema_digest: digest_payload(&input_schema).unwrap(),
                materialization_revision: nomifun_mcp::MCP_TOOL_MATERIALIZATION_REVISION,
            },
            connection_config_ref: format!("mcp-server:{SERVER_ID}@1").into(),
            remote_tool_name: "lookup".into(),
            input_schema,
            display_name: "Server / lookup".into(),
            description: "Lookup one value".into(),
        }
    }

    #[test]
    fn one_remote_tool_publishes_one_exact_direct_action_without_runtime_authority() {
        let tool = frozen_tool();
        let registration = registration(tool.clone(), Arc::new(NomiCoreWave2Host::default()))
            .unwrap();
        let manifest = &registration.metadata.manifest.payload;
        assert!(manifest.requires_runtime_features.is_empty());
        assert!(manifest.package_dependencies.is_empty());
        assert_eq!(manifest.contributions.capabilities.len(), 1);
        assert_eq!(manifest.contributions.mcp_tools.len(), 1);
        assert!(manifest.contributions.skills.is_empty());
        assert!(manifest.contributions.role_contracts.is_empty());
        assert!(manifest.contributions.role_providers.is_empty());
        let capability = &manifest.contributions.capabilities[0];
        assert_eq!(capability.id, tool.lock.capability_id);
        assert_eq!(capability.authoring_policy().unwrap(), CapabilityAuthoringPolicy::Direct);
        assert_eq!(capability.contributions.actions.len(), 1);
        assert_eq!(capability.contributions.actions[0].action_id, tool.action());
        assert_eq!(
            capability.contributions.resource_kinds,
            BTreeSet::from([ResourceKind::from("mcp_server")])
        );
        let mapping = &manifest.contributions.mcp_tools[0];
        assert_eq!(mapping.server_id, tool.lock.server_id);
        assert_eq!(mapping.canonical_tool_key, tool.lock.canonical_tool_key);
        assert_eq!(mapping.schema_digest, tool.lock.schema_digest);
        assert_eq!(mapping.capability.id, tool.lock.capability_id);
        assert_eq!(registration.handler_ids(), BTreeSet::from([tool.lock.capability_id]));
    }

    #[test]
    fn broad_proxy_identity_cannot_be_materialized_as_a_remote_tool() {
        let mut tool = frozen_tool();
        let broad_proxy = concat!("mcp", ".", "tool_proxy");
        tool.lock.canonical_tool_key = broad_proxy.into();
        tool.lock.capability_id = broad_proxy.into();
        assert!(tool.validator().is_err());
    }
}

#[async_trait]
impl nomifun_ai_agent::ContextContributor for McpSettlement {
    async fn pre_turn_context(&self) -> Option<String> {
        self.host
            .mcp_recovery_context(&self.owner, &self.session)
            .await
            .ok()
            .flatten()
    }
    async fn pre_turn_context_for_turn_result(
        &self,
        _: &nomifun_ai_agent::TurnContext,
    ) -> Result<Option<String>, String> {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.host.mcp_recovery_context(&self.owner, &self.session),
        )
        .await
        .map_err(|_| "MCP_EFFECT_CONTEXT_TIMEOUT".to_owned())?
        .map_err(|_| "MCP_EFFECT_CONTEXT_UNAVAILABLE".to_owned())
    }
    fn label(&self) -> &str {
        "platform_mcp_effect_history"
    }
}
