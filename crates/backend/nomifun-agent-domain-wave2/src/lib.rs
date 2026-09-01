//! Bundled Wave 2 coding-extension capability registrations.
//!
//! The package inventory in this crate is deliberately limited to the
//! extension surface around Coding: workspace/filesystem/process/terminal/VCS,
//! SSH, Browser, Computer/A11y, and MCP/connectors.  The exact native Coding
//! surface is owned by the Runtime contract and is not re-declared here.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nomifun_agent_contracts::{
    ActionId, AgentSessionId, ArtifactEnvelope, CapabilityActionDescriptor,
    CapabilityContributions, CapabilityId, CapabilityKind, CapabilityManifest,
    CanonicalErrorCode, CanonicalSchemaRef, CancellationDescriptor,
    CorrelationId, DeclaredServiceViewDescriptor, EffectClass, HostPortBindingDescriptor,
    HostPortId, HostPortRef, IdempotencyKey, InProcessEntrypointMetadata,
    LocalizedMetadata, ManagedTaskRegistrationDescriptor, PackageContributions,
    PackageId, PackageManifest, PackageRef, PlatformConstraint, PluginBootCriticality,
    PluginBootState, PluginContextDescriptor, PluginDesiredState, PluginEffectiveState,
    PluginIdentityDescriptor, PluginMountId, PluginRegistrarDescriptor,
    PluginRegistrarOperation, PluginRegistrationMetadata, PluginSourceKind,
    PluginSourceMetadata, PluginStateHandleDescriptor, PluginStateMethod,
    OperationId, PrincipalRef, ResolvedSnapshotRef, ResourceKind, RuntimeTarget, ScopeKey,
    StrictJsonValue, ToolPresentationKind, TypedResourceBindings, ValidatedPluginConfig,
    VersionString, CAPABILITY_UNAVAILABLE_ON_PLATFORM, digest_payload,
};
use nomifun_agent_kernel::{
    CapabilityHandler, CapabilityInvocationContext, KernelError, PluginRegistration,
};

pub const CONTRACT_VERSION: &str = "1.0.0";
pub const VERSION: &str = CONTRACT_VERSION;
pub const PACKAGE_VERSION: &str = CONTRACT_VERSION;

pub const WORKSPACE_EXECUTION_PACKAGE_ID: &str = "nomifun.workspace-execution";
pub const SSH_PACKAGE_ID: &str = "nomifun.ssh";
pub const MCP_CONNECTORS_PACKAGE_ID: &str = "nomifun.mcp-connectors";
pub const BROWSER_PACKAGE_ID: &str = "nomifun.browser";
pub const COMPUTER_A11Y_PACKAGE_ID: &str = "nomifun.computer-a11y";

pub const WORKSPACE_EXECUTION_MOUNT_ID: &str = "domain-workspace-execution";
pub const SSH_MOUNT_ID: &str = "domain-ssh";
pub const MCP_CONNECTORS_MOUNT_ID: &str = "domain-mcp-connectors";
pub const BROWSER_MOUNT_ID: &str = "domain-browser";
pub const COMPUTER_A11Y_MOUNT_ID: &str = "domain-computer-a11y";

pub const PACKAGE_IDS: [&str; 5] = [
    WORKSPACE_EXECUTION_PACKAGE_ID,
    SSH_PACKAGE_ID,
    MCP_CONNECTORS_PACKAGE_ID,
    BROWSER_PACKAGE_ID,
    COMPUTER_A11Y_PACKAGE_ID,
];
pub const TARGET_PACKAGE_IDS: [&str; 5] = PACKAGE_IDS;

pub const WORKSPACE_EXECUTION_CAPABILITY_IDS: &[&str] = &[
    "fs.read",
    "fs.search",
    "fs.write",
    "fs.patch",
    "fs.delete",
    "fs.watch",
    "fs.snapshot",
    "workspace.bind",
    "workspace.artifacts",
    "vcs.status",
    "vcs.diff",
    "vcs.stage",
    "vcs.commit",
    "vcs.push",
    "process.exec",
    "process.session",
    "terminal.pty",
];

pub const SSH_CAPABILITY_IDS: &[&str] = &[
    "ssh.connect",
    "ssh.fs.read",
    "ssh.fs.write",
    "ssh.exec",
    "ssh.sudo",
];

pub const MCP_CONNECTORS_CAPABILITY_IDS: &[&str] = &[
    "mcp.connect",
    "mcp.tool_proxy",
    "mcp.resource",
    "mcp.oauth",
    "connector.data.read",
    "connector.data.write",
];

pub const BROWSER_CAPABILITY_IDS: &[&str] = &[
    "browser.identity",
    "browser.observe",
    "browser.navigate",
    "browser.act",
    "browser.download",
    "browser.upload",
    "browser.evaluate",
    "browser.site_memory",
    "browser.takeover",
];

pub const COMPUTER_A11Y_CAPABILITY_IDS: &[&str] = &[
    "computer.observe",
    "computer.input",
    "computer.launch",
    "a11y.observe",
];

pub const ALL_CAPABILITY_IDS: [&str; 41] = [
    "fs.read",
    "fs.search",
    "fs.write",
    "fs.patch",
    "fs.delete",
    "fs.watch",
    "fs.snapshot",
    "workspace.bind",
    "workspace.artifacts",
    "vcs.status",
    "vcs.diff",
    "vcs.stage",
    "vcs.commit",
    "vcs.push",
    "process.exec",
    "process.session",
    "terminal.pty",
    "ssh.connect",
    "ssh.fs.read",
    "ssh.fs.write",
    "ssh.exec",
    "ssh.sudo",
    "mcp.connect",
    "mcp.tool_proxy",
    "mcp.resource",
    "mcp.oauth",
    "connector.data.read",
    "connector.data.write",
    "browser.identity",
    "browser.observe",
    "browser.navigate",
    "browser.act",
    "browser.download",
    "browser.upload",
    "browser.evaluate",
    "browser.site_memory",
    "browser.takeover",
    "computer.observe",
    "computer.input",
    "computer.launch",
    "a11y.observe",
];
pub const TARGET_CAPABILITY_IDS: [&str; 41] = ALL_CAPABILITY_IDS;

pub const TARGET_CAPABILITY_FAMILIES: [&str; 11] = [
    "browser",
    "computer",
    "external-mcp",
    "filesystem",
    "process",
    "remote-execution",
    "review-ci",
    "ssh",
    "terminal",
    "vcs",
    "workspace",
];

/// Browser is release-defined on all four desktop host targets.  Headless
/// hosts are intentionally absent from this set.
pub const BROWSER_DESKTOP_HOST_TARGETS: &[&str] = &[
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
];

/// The first release does not claim a full Computer surface on Linux Desktop.
/// Linux Desktop and all headless hosts therefore fail closed with the
/// canonical platform-unavailable error.
pub const COMPUTER_DESKTOP_HOST_TARGETS: &[&str] = &[
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
];

pub const DESKTOP_HOST_SURFACES: &[&str] = &["desktop"];
pub const AGENT_SURFACES: &[&str] = &["desktop", "headless"];
pub const BROWSER_COMPUTER_SURFACES: &[&str] = &["desktop"];

const WORKSPACE_RESOURCE: &[&str] = &["workspace"];
const PROCESS_RESOURCE: &[&str] = &["process_session"];
const TERMINAL_RESOURCE: &[&str] = &["terminal"];
const MCP_RESOURCE: &[&str] = &["mcp_server"];
const SSH_RESOURCE: &[&str] = &["ssh_host"];
const BROWSER_RESOURCE: &[&str] = &["browser"];
const COMPUTER_RESOURCE: &[&str] = &["computer"];
const ARTIFACT_RESOURCES: &[&str] = &["workspace"];
const CONNECTOR_RESOURCES: &[&str] = &["mcp_server"];

const PLUGIN_CANCEL_PORT: &str = "host.plugin.cancel";
const PLUGIN_TASKS_PORT: &str = "host.plugin.tasks";

/// The single host port used by action-bearing Wave 2 capabilities.
///
/// The domain crate owns capability metadata and invocation validation.  The
/// host owns filesystem, process, SSH, MCP, Browser, and Computer facts and
/// must provide the real action result through this port.
pub const WAVE2_CAPABILITY_HOST_PORT_ID: &str = "host.wave2.capability.invoke";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave2HostContext {
    pub principal: PrincipalRef,
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub idempotency_key: IdempotencyKey,
    pub correlation_id: CorrelationId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub registry_generation: u64,
    pub capability_id: CapabilityId,
    pub action_id: ActionId,
    pub resource_bindings: TypedResourceBindings,
}

/// A family-typed action operation for the host adapter.
///
/// Capability and action identity remain in [`Wave2HostContext`].  The
/// variant prevents a host adapter from treating every Wave 2 action as an
/// untyped generic success path while leaving capability-specific JSON
/// decoding to the owning host service.
#[derive(Clone, Debug, PartialEq)]
pub enum Wave2CapabilityOperation {
    WorkspaceExecution { input: StrictJsonValue },
    Ssh { input: StrictJsonValue },
    McpConnectors { input: StrictJsonValue },
    Browser { input: StrictJsonValue },
    ComputerA11y { input: StrictJsonValue },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave2HostRequest {
    pub context: Wave2HostContext,
    pub operation: Wave2CapabilityOperation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave2HostPortError {
    pub code: String,
    pub message: String,
}

impl Wave2HostPortError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new("WAVE2_HOST_PORT_UNAVAILABLE", message)
    }
}

impl fmt::Display for Wave2HostPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Wave2HostPortError {}

/// Host-owned implementation boundary for action-bearing Wave 2 capabilities.
pub trait Wave2HostPort: Send + Sync {
    fn invoke<'a>(
        &'a self,
        request: Wave2HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a>>;
}

struct UnconfiguredWave2HostPort;

impl Wave2HostPort for UnconfiguredWave2HostPort {
    fn invoke<'a>(
        &'a self,
        request: Wave2HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a>>
    {
        Box::pin(async move {
            Err(Wave2HostPortError::unavailable(format!(
                "no production host adapter is bound for {}",
                request.context.capability_id.as_ref()
            )))
        })
    }
}

/// Return the default adapter used by metadata-only compositions.
///
/// It deliberately fails closed; it never fabricates an action result.
pub fn unconfigured_host_port() -> Arc<dyn Wave2HostPort> {
    Arc::new(UnconfiguredWave2HostPort)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlatformScope {
    Any,
    BrowserDesktop,
    ComputerDesktop,
}

#[derive(Clone, Copy)]
struct CapabilityDefinition {
    id: &'static str,
    kind: CapabilityKind,
    effect_class: Option<EffectClass>,
    resource_kinds: &'static [&'static str],
    platform_scope: PlatformScope,
}

impl CapabilityDefinition {
    const fn tool(
        id: &'static str,
        effect_class: EffectClass,
        resource_kinds: &'static [&'static str],
    ) -> Self {
        Self {
            id,
            kind: CapabilityKind::Tool,
            effect_class: Some(effect_class),
            resource_kinds,
            platform_scope: PlatformScope::Any,
        }
    }

    const fn context(
        id: &'static str,
        resource_kinds: &'static [&'static str],
        platform_scope: PlatformScope,
    ) -> Self {
        Self {
            id,
            kind: CapabilityKind::ContextContributor,
            effect_class: None,
            resource_kinds,
            platform_scope,
        }
    }

    const fn resource_provider(
        id: &'static str,
        resource_kinds: &'static [&'static str],
        platform_scope: PlatformScope,
    ) -> Self {
        Self {
            id,
            kind: CapabilityKind::ResourceProvider,
            effect_class: None,
            resource_kinds,
            platform_scope,
        }
    }

    const fn event_source(
        id: &'static str,
        resource_kinds: &'static [&'static str],
    ) -> Self {
        Self {
            id,
            kind: CapabilityKind::EventSource,
            effect_class: None,
            resource_kinds,
            platform_scope: PlatformScope::Any,
        }
    }

    const fn transport(
        id: &'static str,
        resource_kinds: &'static [&'static str],
    ) -> Self {
        Self {
            id,
            kind: CapabilityKind::Transport,
            effect_class: None,
            resource_kinds,
            platform_scope: PlatformScope::Any,
        }
    }

    const fn is_tool(self) -> bool {
        self.effect_class.is_some()
    }
}

#[derive(Clone, Copy)]
struct PackageDefinition {
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    mount_id: &'static str,
    capabilities: &'static [CapabilityDefinition],
}

const WORKSPACE_EXECUTION_CAPABILITIES: &[CapabilityDefinition] = &[
    CapabilityDefinition::tool("fs.read", EffectClass::ReadLocal, WORKSPACE_RESOURCE),
    CapabilityDefinition::tool("fs.search", EffectClass::ReadLocal, WORKSPACE_RESOURCE),
    CapabilityDefinition::tool("fs.write", EffectClass::WriteDurable, WORKSPACE_RESOURCE),
    CapabilityDefinition::tool("fs.patch", EffectClass::WriteReversible, WORKSPACE_RESOURCE),
    CapabilityDefinition::tool("fs.delete", EffectClass::Destructive, WORKSPACE_RESOURCE),
    CapabilityDefinition::event_source("fs.watch", &[]),
    CapabilityDefinition::tool("fs.snapshot", EffectClass::ReadLocal, WORKSPACE_RESOURCE),
    CapabilityDefinition::resource_provider(
        "workspace.bind",
        WORKSPACE_RESOURCE,
        PlatformScope::Any,
    ),
    CapabilityDefinition::resource_provider(
        "workspace.artifacts",
        ARTIFACT_RESOURCES,
        PlatformScope::Any,
    ),
    CapabilityDefinition::tool("vcs.status", EffectClass::ReadLocal, WORKSPACE_RESOURCE),
    CapabilityDefinition::tool("vcs.diff", EffectClass::ReadLocal, WORKSPACE_RESOURCE),
    CapabilityDefinition::tool("vcs.stage", EffectClass::WriteReversible, WORKSPACE_RESOURCE),
    CapabilityDefinition::tool("vcs.commit", EffectClass::WriteDurable, WORKSPACE_RESOURCE),
    CapabilityDefinition::tool(
        "vcs.push",
        EffectClass::ExternalTransmit,
        WORKSPACE_RESOURCE,
    ),
    CapabilityDefinition::tool(
        "process.exec",
        EffectClass::ExecuteLocal,
        PROCESS_RESOURCE,
    ),
    CapabilityDefinition::resource_provider(
        "process.session",
        PROCESS_RESOURCE,
        PlatformScope::Any,
    ),
    CapabilityDefinition::resource_provider("terminal.pty", TERMINAL_RESOURCE, PlatformScope::Any),
];

const SSH_CAPABILITIES: &[CapabilityDefinition] = &[
    CapabilityDefinition::resource_provider("ssh.connect", SSH_RESOURCE, PlatformScope::Any),
    CapabilityDefinition::tool("ssh.fs.read", EffectClass::ReadSensitive, SSH_RESOURCE),
    CapabilityDefinition::tool("ssh.fs.write", EffectClass::WriteDurable, SSH_RESOURCE),
    CapabilityDefinition::tool("ssh.exec", EffectClass::ExecuteLocal, SSH_RESOURCE),
    CapabilityDefinition::tool("ssh.sudo", EffectClass::ExecuteLocal, SSH_RESOURCE),
];

const MCP_CONNECTORS_CAPABILITIES: &[CapabilityDefinition] = &[
    CapabilityDefinition::transport("mcp.connect", &[]),
    CapabilityDefinition::tool(
        "mcp.tool_proxy",
        EffectClass::ExternalTransmit,
        MCP_RESOURCE,
    ),
    CapabilityDefinition::resource_provider("mcp.resource", MCP_RESOURCE, PlatformScope::Any),
    CapabilityDefinition::transport("mcp.oauth", &[]),
    CapabilityDefinition::tool(
        "connector.data.read",
        EffectClass::ReadSensitive,
        CONNECTOR_RESOURCES,
    ),
    CapabilityDefinition::tool(
        "connector.data.write",
        EffectClass::WriteDurable,
        CONNECTOR_RESOURCES,
    ),
];

const BROWSER_CAPABILITIES: &[CapabilityDefinition] = &[
    CapabilityDefinition::resource_provider(
        "browser.identity",
        BROWSER_RESOURCE,
        PlatformScope::BrowserDesktop,
    ),
    CapabilityDefinition::context(
        "browser.observe",
        BROWSER_RESOURCE,
        PlatformScope::BrowserDesktop,
    ),
    CapabilityDefinition::browser_tool("browser.navigate", EffectClass::ExternalTransmit),
    CapabilityDefinition::browser_tool("browser.act", EffectClass::WriteReversible),
    CapabilityDefinition::browser_tool("browser.download", EffectClass::WriteDurable),
    CapabilityDefinition::browser_tool("browser.upload", EffectClass::ExternalTransmit),
    CapabilityDefinition::browser_tool("browser.evaluate", EffectClass::ExecuteLocal),
    CapabilityDefinition::context(
        "browser.site_memory",
        BROWSER_RESOURCE,
        PlatformScope::BrowserDesktop,
    ),
    CapabilityDefinition::browser_tool("browser.takeover", EffectClass::WriteReversible),
];

const COMPUTER_A11Y_CAPABILITIES: &[CapabilityDefinition] = &[
    CapabilityDefinition::context(
        "computer.observe",
        COMPUTER_RESOURCE,
        PlatformScope::ComputerDesktop,
    ),
    CapabilityDefinition::computer_tool("computer.input", EffectClass::Physical),
    CapabilityDefinition::computer_tool("computer.launch", EffectClass::ExecuteLocal),
    CapabilityDefinition::context(
        "a11y.observe",
        COMPUTER_RESOURCE,
        PlatformScope::ComputerDesktop,
    ),
];

impl CapabilityDefinition {
    const fn browser_tool(id: &'static str, effect_class: EffectClass) -> Self {
        Self {
            id,
            kind: CapabilityKind::Tool,
            effect_class: Some(effect_class),
            resource_kinds: BROWSER_RESOURCE,
            platform_scope: PlatformScope::BrowserDesktop,
        }
    }

    const fn computer_tool(id: &'static str, effect_class: EffectClass) -> Self {
        Self {
            id,
            kind: CapabilityKind::Tool,
            effect_class: Some(effect_class),
            resource_kinds: COMPUTER_RESOURCE,
            platform_scope: PlatformScope::ComputerDesktop,
        }
    }
}

const PACKAGE_DEFINITIONS: &[PackageDefinition] = &[
    PackageDefinition {
        id: WORKSPACE_EXECUTION_PACKAGE_ID,
        display_name: "Workspace & Execution",
        description: "Filesystem, workspace, process, terminal, and VCS capabilities.",
        mount_id: WORKSPACE_EXECUTION_MOUNT_ID,
        capabilities: WORKSPACE_EXECUTION_CAPABILITIES,
    },
    PackageDefinition {
        id: SSH_PACKAGE_ID,
        display_name: "SSH",
        description: "Typed remote filesystem and process capabilities over SSH.",
        mount_id: SSH_MOUNT_ID,
        capabilities: SSH_CAPABILITIES,
    },
    PackageDefinition {
        id: MCP_CONNECTORS_PACKAGE_ID,
        display_name: "MCP & Connectors",
        description: "MCP transports, resources, tool projection, and connectors.",
        mount_id: MCP_CONNECTORS_MOUNT_ID,
        capabilities: MCP_CONNECTORS_CAPABILITIES,
    },
    PackageDefinition {
        id: BROWSER_PACKAGE_ID,
        display_name: "Browser",
        description: "Browser identity, observation, navigation, and interaction.",
        mount_id: BROWSER_MOUNT_ID,
        capabilities: BROWSER_CAPABILITIES,
    },
    PackageDefinition {
        id: COMPUTER_A11Y_PACKAGE_ID,
        display_name: "Computer & Accessibility",
        description: "Desktop Computer and accessibility observation capabilities.",
        mount_id: COMPUTER_A11Y_MOUNT_ID,
        capabilities: COMPUTER_A11Y_CAPABILITIES,
    },
];

/// Build the complete Wave 2 bundled registration inventory with a
/// fail-closed host adapter.
pub fn registrations() -> Result<Vec<PluginRegistration>, String> {
    registrations_with_host_port(unconfigured_host_port())
}

/// Build the complete Wave 2 bundled registration inventory with an
/// application-owned action host port.
pub fn registrations_with_host_port(
    action_host_port: Arc<dyn Wave2HostPort>,
) -> Result<Vec<PluginRegistration>, String> {
    let mut registrations = Vec::with_capacity(PACKAGE_DEFINITIONS.len());
    let mut packages = BTreeSet::new();
    let mut mounts = BTreeSet::new();
    let mut capabilities = BTreeSet::new();

    for package in PACKAGE_DEFINITIONS {
        if !packages.insert(package.id) {
            return Err(format!("duplicate Wave 2 package {}", package.id));
        }
        if !mounts.insert(package.mount_id) {
            return Err(format!("duplicate Wave 2 mount {}", package.mount_id));
        }
        let registration = build_registration(package, Arc::clone(&action_host_port))?;
        for capability in &registration
            .metadata
            .manifest
            .payload
            .contributions
            .capabilities
        {
            if !capabilities.insert(capability.id.clone()) {
                return Err(format!(
                    "duplicate Wave 2 capability {}",
                    capability.id.as_ref()
                ));
            }
        }
        registrations.push(registration);
    }

    if capabilities.len() != ALL_CAPABILITY_IDS.len() {
        return Err(format!(
            "Wave 2 capability inventory has {} entries; expected {}",
            capabilities.len(),
            ALL_CAPABILITY_IDS.len()
        ));
    }

    Ok(registrations)
}

fn build_registration(
    package: &PackageDefinition,
    action_host_port: Arc<dyn Wave2HostPort>,
) -> Result<PluginRegistration, String> {
    let package_ref = PackageRef {
        id: PackageId::from(package.id),
        version: VersionString::from(CONTRACT_VERSION),
    };
    let mut capability_manifests = Vec::with_capacity(package.capabilities.len());
    let mut handlers = Vec::new();

    for definition in package.capabilities {
        let capability = build_capability(&package_ref, *definition)?;
        if definition.is_tool() {
            let capability_id = CapabilityId::from(definition.id);
            handlers.push((
                capability_id.clone(),
                ActionId::from(format!("{}.invoke", definition.id)),
                definition
                    .resource_kinds
                    .iter()
                    .map(|resource_kind| ResourceKind::from(*resource_kind))
                    .collect(),
            ));
        }
        capability_manifests.push(capability);
    }

    let config_schema = object_schema();
    let manifest = PackageManifest {
        schema_version: VersionString::from(CONTRACT_VERSION),
        host_contract_version: VersionString::from(CONTRACT_VERSION),
        package_id: package_ref.id.clone(),
        package_version: package_ref.version.clone(),
        display: localized(package.display_name, package.description),
        package_dependencies: Vec::new(),
        requires_runtime_features: Vec::new(),
        config_schema: config_schema.clone(),
        provides_services: Vec::new(),
        requires_services: Vec::new(),
        entrypoint: InProcessEntrypointMetadata {
            entrypoint_profile: "trusted-in-process".to_owned(),
            entrypoint_id: format!("{}.entrypoint", package.id),
            contract_version: VersionString::from(CONTRACT_VERSION),
        },
        contributions: PackageContributions {
            capabilities: capability_manifests,
            skills: Vec::new(),
            mcp_tools: Vec::new(),
        },
    };

    let source = PluginSourceMetadata {
        source_kind: PluginSourceKind::Bundled,
        source_identity: package.id.to_owned(),
        source_digest: None,
    };
    let identity = PluginIdentityDescriptor {
        package: package_ref.clone(),
        mount_id: PluginMountId::from(package.mount_id),
    };
    let cancellation_port = host_port(PLUGIN_CANCEL_PORT);
    let tasks_port = host_port(PLUGIN_TASKS_PORT);
    let action_port = host_port(WAVE2_CAPABILITY_HOST_PORT_ID);
    let has_action_handler = !handlers.is_empty();
    let mut declared_host_ports =
        BTreeSet::from([cancellation_port.id.clone(), tasks_port.id.clone()]);
    if has_action_handler {
        declared_host_ports.insert(action_port.id.clone());
    }
    let host_port_bindings = if has_action_handler {
        vec![host_port_binding()?]
    } else {
        Vec::new()
    };
    let metadata = PluginRegistrationMetadata {
        manifest: ArtifactEnvelope::new(manifest)
            .map_err(|error| format!("build {} manifest: {error}", package.id))?,
        mount_id: identity.mount_id.clone(),
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
                PluginRegistrarOperation::BindHostPort,
                PluginRegistrarOperation::ContributeCapability,
            ]),
            declared_capability_ids: package
                .capabilities
                .iter()
                .map(|definition| CapabilityId::from(definition.id))
                .collect(),
            declared_skill_ids: BTreeSet::new(),
            declared_mcp_tool_keys: BTreeSet::new(),
            declared_service_keys: BTreeSet::new(),
            declared_host_ports,
        },
        context: PluginContextDescriptor {
            identity: identity.clone(),
            source,
            validated_config: ValidatedPluginConfig {
                schema_digest: digest_payload(&config_schema)
                    .map_err(|error| format!("digest {} config: {error}", package.id))?,
                config_revision: 1,
                value: empty_object(),
            },
            state: PluginStateHandleDescriptor {
                package_id: package_ref.id.clone(),
                mount_id: identity.mount_id.clone(),
                methods: PluginStateMethod::REQUIRED.into_iter().collect(),
            },
            declared_services: DeclaredServiceViewDescriptor::default(),
            host_ports: host_port_bindings,
            typed_command_ports: Vec::new(),
            domain_outbox_ports: Vec::new(),
            cancellation: CancellationDescriptor {
                cancellation_port,
                scope_key: ScopeKey::from(format!("mount:{}", package.mount_id)),
            },
            managed_task_registration: ManagedTaskRegistrationDescriptor {
                registrar_port: tasks_port,
                scope_key: ScopeKey::from(format!("mount:{}", package.mount_id)),
            },
        },
    };

    let mut registration = PluginRegistration::new(metadata);
    for (capability_id, action_id, required_resource_kinds) in handlers {
        registration
            .add_capability_handler(
                capability_id.clone(),
                Arc::new(Wave2CapabilityHandler {
                    capability_id,
                    action_id,
                    required_resource_kinds,
                    host_port: Arc::clone(&action_host_port),
                }),
            )
            .map_err(|error| format!("register {} handler: {error}", package.id))?;
    }
    Ok(registration)
}

fn build_capability(
    package: &PackageRef,
    definition: CapabilityDefinition,
) -> Result<CapabilityManifest, String> {
    let actions = if definition.is_tool() {
        vec![CapabilityActionDescriptor {
            action_id: ActionId::from(format!("{}.invoke", definition.id)),
            input_schema: schema_ref(definition.id, "input")?,
            output_schema: schema_ref(definition.id, "output")?,
            effect_class: definition
                .effect_class
                .expect("tool definitions always carry an effect class"),
            presentation: ToolPresentationKind::FunctionTool,
        }]
    } else {
        Vec::new()
    };
    let context_schema_refs = (definition.kind == CapabilityKind::ContextContributor)
        .then(|| schema_ref(definition.id, "context"))
        .transpose()?
        .into_iter()
        .collect();
    let event_schema_refs = (definition.kind == CapabilityKind::EventSource)
        .then(|| schema_ref(definition.id, "event"))
        .transpose()?
        .into_iter()
        .collect();

    let supported_surfaces = match definition.platform_scope {
        PlatformScope::Any => AGENT_SURFACES,
        PlatformScope::BrowserDesktop | PlatformScope::ComputerDesktop => {
            BROWSER_COMPUTER_SURFACES
        }
    };

    Ok(CapabilityManifest {
        id: CapabilityId::from(definition.id),
        version: VersionString::from(CONTRACT_VERSION),
        kind: definition.kind,
        package: package.clone(),
        display: localized(
            definition.id,
            "Bundled Wave 2 coding-extension capability.",
        ),
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: supported_surfaces
            .iter()
            .map(|surface| (*surface).to_owned())
            .collect(),
        requires_runtime_features: Vec::new(),
        supported_platforms: platform_constraints(definition.platform_scope),
        config_schema: object_schema(),
        contributions: CapabilityContributions {
            actions,
            context_schema_refs,
            event_schema_refs,
            resource_kinds: definition
                .resource_kinds
                .iter()
                .map(|resource_kind| ResourceKind::from(*resource_kind))
                .collect(),
            host_ports: definition
                .is_tool()
                .then(|| host_port(WAVE2_CAPABILITY_HOST_PORT_ID))
                .into_iter()
                .collect(),
        },
    })
}

struct Wave2CapabilityHandler {
    capability_id: CapabilityId,
    action_id: ActionId,
    required_resource_kinds: BTreeSet<ResourceKind>,
    host_port: Arc<dyn Wave2HostPort>,
}

impl CapabilityHandler for Wave2CapabilityHandler {
    fn invoke<'life0, 'async_trait>(
        &'life0 self,
        context: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<StrictJsonValue, KernelError>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: Sync + 'async_trait,
    {
        Box::pin(async move {
            if context.capability_id != self.capability_id
                || context.action_id != self.action_id
            {
                return Err(KernelError::ActionNotDeclared {
                    capability_id: context.capability_id,
                    action_id: context.action_id,
                });
            }
            if !input.0.is_object() {
                return Err(KernelError::CapabilityExecution {
                    reason: format!(
                        "{} input must be a JSON object",
                        self.capability_id.as_ref()
                    ),
                });
            }

            let mut binding_ids = BTreeSet::new();
            for binding in &context.resource_bindings {
                if binding.binding_id.as_ref().is_empty()
                    || binding.resource_id.as_ref().is_empty()
                {
                    return Err(KernelError::CapabilityExecution {
                        reason: format!(
                            "{} requires non-empty binding and resource IDs",
                            self.capability_id.as_ref()
                        ),
                    });
                }
                if !binding_ids.insert(binding.binding_id.clone()) {
                    return Err(KernelError::CapabilityExecution {
                        reason: format!(
                            "{} received duplicate resource binding {}",
                            self.capability_id.as_ref(),
                            binding.binding_id.as_ref()
                        ),
                    });
                }
                if binding.owner_id != context.principal.principal_id {
                    return Err(KernelError::ResourceOwnerMismatch {
                        binding_id: binding.binding_id.clone(),
                    });
                }
            }

            for resource_kind in &self.required_resource_kinds {
                if !context
                    .resource_bindings
                    .iter()
                    .any(|binding| &binding.resource_kind == resource_kind)
                {
                    return Err(KernelError::CapabilityResourceNotBound {
                        capability_id: self.capability_id.clone(),
                        resource_kind: resource_kind.as_ref().to_owned(),
                    });
                }
            }

            let operation = operation_for(&self.capability_id, input)?;
            self.host_port
                .invoke(Wave2HostRequest {
                    context: Wave2HostContext {
                        principal: context.principal,
                        agent_session_id: context.agent_session_id,
                        operation_id: context.operation_id,
                        idempotency_key: context.idempotency_key,
                        correlation_id: context.correlation_id,
                        resolved_snapshot_ref: context.resolved_snapshot_ref,
                        registry_generation: context.registry_generation,
                        capability_id: self.capability_id.clone(),
                        action_id: context.action_id,
                        resource_bindings: context.resource_bindings,
                    },
                    operation,
                })
                .await
                .map_err(|error| KernelError::CapabilityExecution {
                    reason: error.to_string(),
                })
        })
    }
}

fn operation_for(
    capability_id: &CapabilityId,
    input: StrictJsonValue,
) -> Result<Wave2CapabilityOperation, KernelError> {
    let operation = match capability_id.as_ref() {
        "fs.read"
        | "fs.search"
        | "fs.write"
        | "fs.patch"
        | "fs.delete"
        | "fs.snapshot"
        | "vcs.status"
        | "vcs.diff"
        | "vcs.stage"
        | "vcs.commit"
        | "vcs.push"
        | "process.exec" => Wave2CapabilityOperation::WorkspaceExecution { input },
        "ssh.fs.read" | "ssh.fs.write" | "ssh.exec" | "ssh.sudo" => {
            Wave2CapabilityOperation::Ssh { input }
        }
        "mcp.tool_proxy" | "connector.data.read" | "connector.data.write" => {
            Wave2CapabilityOperation::McpConnectors { input }
        }
        "browser.navigate"
        | "browser.act"
        | "browser.download"
        | "browser.upload"
        | "browser.evaluate"
        | "browser.takeover" => Wave2CapabilityOperation::Browser { input },
        "computer.input" | "computer.launch" => {
            Wave2CapabilityOperation::ComputerA11y { input }
        }
        _ => {
            return Err(KernelError::CapabilityExecution {
                reason: format!(
                    "{} does not expose an action host operation",
                    capability_id.as_ref()
                ),
            });
        }
    };
    Ok(operation)
}

/// Return the complete canonical Wave 2 Capability ID set.
pub fn capability_ids() -> BTreeSet<CapabilityId> {
    PACKAGE_DEFINITIONS
        .iter()
        .flat_map(|package| package.capabilities.iter())
        .map(|definition| CapabilityId::from(definition.id))
        .collect()
}

/// Return the target inventory capability set under the conventional API
/// name used by the other domain-wave crates.
pub fn target_capability_ids() -> BTreeSet<CapabilityId> {
    capability_ids()
}

/// Return the five target inventory package IDs.
pub fn package_ids() -> BTreeSet<PackageId> {
    PACKAGE_IDS
        .iter()
        .map(|package_id| PackageId::from(*package_id))
        .collect()
}

/// Return the declared typed resource kinds for a Capability.
pub fn required_resource_kinds(capability_id: &str) -> Option<BTreeSet<ResourceKind>> {
    definition_for(capability_id).map(|definition| {
        definition
            .resource_kinds
            .iter()
            .map(|resource_kind| ResourceKind::from(*resource_kind))
            .collect()
    })
}

/// The action identity used by each Tool contribution.
pub fn action_id(capability_id: &str) -> Option<ActionId> {
    definition_for(capability_id)
        .filter(|definition| definition.is_tool())
        .map(|_| ActionId::from(format!("{}.invoke", capability_id)))
}

/// Check a Capability against its release-time host target/surface metadata.
///
/// A failed check is represented by the canonical typed Kernel error.  No
/// alternate execution mode is introduced for unavailable Browser or
/// Computer hosts.
pub fn check_platform_availability(
    capability_id: &CapabilityId,
    host_target: &RuntimeTarget,
    host_surface: &str,
) -> Result<(), KernelError> {
    let Some(definition) = definition_for(capability_id.as_ref()) else {
        return Err(KernelError::CapabilityExecution {
            reason: format!("unknown Wave 2 capability {}", capability_id.as_ref()),
        });
    };

    if platform_supported(
        definition.platform_scope,
        host_target.as_ref(),
        host_surface,
    ) {
        Ok(())
    } else {
        Err(KernelError::CapabilityUnavailableOnPlatform {
            capability_id: capability_id.clone(),
            target: host_target.as_ref().to_owned(),
            surface: host_surface.to_owned(),
        })
    }
}

/// Return a boolean availability result for callers that need a preflight
/// projection without losing the typed error API above.
pub fn is_available_on_platform(
    capability_id: &str,
    host_target: &str,
    host_surface: &str,
) -> Result<bool, String> {
    let Some(definition) = definition_for(capability_id) else {
        return Err(format!("unknown Wave 2 capability {capability_id}"));
    };
    Ok(platform_supported(
        definition.platform_scope,
        host_target,
        host_surface,
    ))
}

/// Return the canonical error code for headless Browser/Computer failures.
pub fn unavailable_on_platform_code() -> CanonicalErrorCode {
    CanonicalErrorCode::from(CAPABILITY_UNAVAILABLE_ON_PLATFORM)
}

/// Build one package registration from the same factory used by
/// [`registrations`].
pub fn workspace_execution_registration() -> Result<PluginRegistration, String> {
    build_registration(&PACKAGE_DEFINITIONS[0], unconfigured_host_port())
}

/// Build the bundled SSH registration.
pub fn ssh_registration() -> Result<PluginRegistration, String> {
    build_registration(&PACKAGE_DEFINITIONS[1], unconfigured_host_port())
}

/// Build the bundled MCP/connectors registration.
pub fn mcp_connectors_registration() -> Result<PluginRegistration, String> {
    build_registration(&PACKAGE_DEFINITIONS[2], unconfigured_host_port())
}

/// Build the bundled Browser registration.
pub fn browser_registration() -> Result<PluginRegistration, String> {
    build_registration(&PACKAGE_DEFINITIONS[3], unconfigured_host_port())
}

/// Build the bundled Computer/A11y registration.
pub fn computer_a11y_registration() -> Result<PluginRegistration, String> {
    build_registration(&PACKAGE_DEFINITIONS[4], unconfigured_host_port())
}

fn definition_for(capability_id: &str) -> Option<CapabilityDefinition> {
    PACKAGE_DEFINITIONS
        .iter()
        .flat_map(|package| package.capabilities.iter())
        .copied()
        .find(|definition| definition.id == capability_id)
}

fn platform_supported(scope: PlatformScope, host_target: &str, host_surface: &str) -> bool {
    match scope {
        PlatformScope::Any => true,
        PlatformScope::BrowserDesktop => {
            host_surface == "desktop"
                && BROWSER_DESKTOP_HOST_TARGETS
                    .iter()
                    .any(|target| *target == host_target)
        }
        PlatformScope::ComputerDesktop => {
            host_surface == "desktop"
                && COMPUTER_DESKTOP_HOST_TARGETS
                    .iter()
                    .any(|target| *target == host_target)
        }
    }
}

fn platform_constraints(scope: PlatformScope) -> Vec<PlatformConstraint> {
    match scope {
        PlatformScope::Any => vec![PlatformConstraint::Any],
        PlatformScope::BrowserDesktop => target_constraint(BROWSER_DESKTOP_HOST_TARGETS),
        PlatformScope::ComputerDesktop => target_constraint(COMPUTER_DESKTOP_HOST_TARGETS),
    }
}

fn target_constraint(targets: &[&str]) -> Vec<PlatformConstraint> {
    vec![PlatformConstraint::Targets {
        host_targets: targets
            .iter()
            .map(|target| RuntimeTarget::from(*target))
            .collect(),
        host_surfaces: BTreeSet::from(["desktop".to_owned()]),
    }]
}

fn schema_ref(capability_id: &str, role: &str) -> Result<CanonicalSchemaRef, String> {
    let schema = match role {
        "input" | "request" => open_object_schema(),
        "output" | "response" => output_schema(),
        _ => object_schema(),
    };
    let digest = digest_payload(&schema)
        .map_err(|error| format!("digest {capability_id} {role} schema: {error}"))?;
    Ok(CanonicalSchemaRef::from(format!(
        "schema://{capability_id}/{role}@1#{}",
        digest.as_ref()
    )))
}

fn object_schema() -> StrictJsonValue {
    let mut schema = empty_object();
    let object = schema
        .0
        .as_object_mut()
        .expect("empty_object always returns an object");
    object.insert("type".to_owned(), "object".to_owned().into());
    object.insert("additionalProperties".to_owned(), false.into());
    schema
}

fn open_object_schema() -> StrictJsonValue {
    let mut schema = empty_object();
    let object = schema
        .0
        .as_object_mut()
        .expect("empty_object always returns an object");
    object.insert("type".to_owned(), "object".to_owned().into());
    object.insert("additionalProperties".to_owned(), true.into());
    schema
}

fn output_schema() -> StrictJsonValue {
    // The owning host service defines the operation result.  The registration
    // only requires an object and does not publish a fabricated result shape.
    open_object_schema()
}

fn empty_object() -> StrictJsonValue {
    let mut value = nomifun_agent_contracts::remote_binding_protocol_fixture()
        .open
        .request
        .initial_input
        .expect("the canonical Remote fixture supplies an object value")
        .0;
    value
        .as_object_mut()
        .expect("the canonical Remote fixture input is an object")
        .clear();
    StrictJsonValue(value)
}

fn localized(name: &str, description: &str) -> LocalizedMetadata {
    LocalizedMetadata {
        name: name.to_owned(),
        description: description.to_owned(),
        localized_names: Default::default(),
        localized_descriptions: Default::default(),
    }
}

fn host_port(id: &str) -> HostPortRef {
    HostPortRef {
        id: HostPortId::from(id),
        version: VersionString::from(CONTRACT_VERSION),
    }
}

fn host_port_binding() -> Result<HostPortBindingDescriptor, String> {
    Ok(HostPortBindingDescriptor {
        port: host_port(WAVE2_CAPABILITY_HOST_PORT_ID),
        request_schema: schema_ref(WAVE2_CAPABILITY_HOST_PORT_ID, "request")?,
        response_schema: schema_ref(WAVE2_CAPABILITY_HOST_PORT_ID, "response")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_kernel::{
        InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy,
        Materializer,
    };

    #[test]
    fn registrations_have_unique_inventory_ids_and_exact_handler_coverage() {
        let registrations = registrations().expect("Wave 2 registrations");
        let package_ids = registrations
            .iter()
            .map(|registration| {
                registration
                    .metadata
                    .manifest
                    .payload
                    .package_id
                    .as_ref()
                    .to_owned()
            })
            .collect::<BTreeSet<_>>();
        let actual_capability_ids = registrations
            .iter()
            .flat_map(|registration| {
                registration
                    .metadata
                    .manifest
                    .payload
                    .contributions
                    .capabilities
                    .iter()
                    .map(|capability| capability.id.clone())
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(
            package_ids,
            PACKAGE_IDS
                .iter()
                .map(|package_id| (*package_id).to_owned())
                .collect()
        );
        assert_eq!(actual_capability_ids, capability_ids());
        assert_eq!(actual_capability_ids.len(), ALL_CAPABILITY_IDS.len());
        assert_eq!(
            actual_capability_ids,
            TARGET_CAPABILITY_IDS
                .iter()
                .map(|capability_id| CapabilityId::from(*capability_id))
                .collect()
        );
        assert!(actual_capability_ids
            .iter()
            .all(|capability| !capability.as_ref().starts_with("coding.")));

        for registration in &registrations {
            assert!(
                registration
                    .metadata
                    .manifest
                    .verify()
                    .expect("manifest digest")
            );
            let manifest = &registration.metadata.manifest.payload;
            let action_capabilities = manifest
                .contributions
                .capabilities
                .iter()
                .filter(|capability| !capability.contributions.actions.is_empty())
                .map(|capability| capability.id.clone())
                .collect::<BTreeSet<_>>();
            assert_eq!(registration.handler_ids(), action_capabilities);
            for capability in &manifest.contributions.capabilities {
                if capability.kind == CapabilityKind::Tool {
                    assert_eq!(capability.contributions.actions.len(), 1);
                    assert_eq!(
                        action_id(capability.id.as_ref()),
                        Some(capability.contributions.actions[0].action_id.clone())
                    );
                    assert_eq!(
                        capability.contributions.host_ports,
                        vec![host_port(WAVE2_CAPABILITY_HOST_PORT_ID)]
                    );
                } else {
                    assert!(capability.contributions.actions.is_empty());
                    assert!(capability.contributions.host_ports.is_empty());
                }
            }
            assert!(manifest
                .contributions
                .capabilities
                .iter()
                .any(|capability| capability.kind == CapabilityKind::Tool));
            assert!(registration
                .metadata
                .context
                .host_ports
                .iter()
                .any(|binding| {
                    binding.port.id == HostPortId::from(WAVE2_CAPABILITY_HOST_PORT_ID)
                }));
            assert!(registration
                .metadata
                .registrar
                .declared_host_ports
                .contains(&HostPortId::from(WAVE2_CAPABILITY_HOST_PORT_ID)));
        }
    }

    #[test]
    fn registrations_materialize_and_publish_through_the_kernel_contract() {
        let registrations = registrations().expect("Wave 2 registrations");
        let materialized = Materializer::materialize(
            &MaterializationPolicy::stable(CONTRACT_VERSION),
            &registrations,
            1,
        )
        .expect("Wave 2 metadata materializes");
        assert_eq!(materialized.packages.len(), 5);
        assert_eq!(materialized.capabilities.len(), 41);
        assert_eq!(
            required_resource_kinds("process.exec"),
            Some(BTreeSet::from([ResourceKind::from("process_session")]))
        );
        assert_eq!(
            required_resource_kinds("terminal.pty"),
            Some(BTreeSet::from([ResourceKind::from("terminal")]))
        );

        let registry = KernelRegistry::new(
            MaterializationPolicy::stable(CONTRACT_VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .expect("kernel registry");
        registry
            .replace_all(registrations)
            .expect("all action handlers are declared");
    }

    #[test]
    fn every_action_capability_maps_to_a_typed_host_operation() {
        for definition in PACKAGE_DEFINITIONS
            .iter()
            .flat_map(|package| package.capabilities.iter())
            .filter(|definition| definition.is_tool())
        {
            let capability_id = CapabilityId::from(definition.id);
            assert!(
                operation_for(&capability_id, empty_object()).is_ok(),
                "{} must have a host operation",
                definition.id
            );
        }
    }

    #[test]
    fn unconfigured_action_host_returns_a_typed_unavailable_error() {
        use std::task::{Context, Poll, Waker};

        let host_port = unconfigured_host_port();
        let future = host_port.invoke(Wave2HostRequest {
                context: Wave2HostContext {
                    principal: PrincipalRef {
                        principal_kind: "user".to_owned(),
                        principal_id: "wave2-test-owner".to_owned(),
                    },
                    agent_session_id: AgentSessionId::from("wave2-test-session"),
                    operation_id: OperationId::from("wave2-test-operation"),
                    idempotency_key: IdempotencyKey::from("wave2-test-idempotency"),
                    correlation_id: CorrelationId::from("wave2-test-correlation"),
                    resolved_snapshot_ref: ResolvedSnapshotRef {
                        snapshot_id: "snapshot".into(),
                        snapshot_digest: "digest".into(),
                    },
                    registry_generation: 1,
                    capability_id: CapabilityId::from("fs.read"),
                    action_id: ActionId::from("fs.read.invoke"),
                    resource_bindings: Vec::new(),
                },
                operation: Wave2CapabilityOperation::WorkspaceExecution {
                    input: empty_object(),
                },
            });
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = std::pin::pin!(future);
        let result = match future.as_mut().poll(&mut context) {
            Poll::Ready(result) => result.expect_err("unconfigured Wave 2 actions must fail closed"),
            Poll::Pending => panic!("unconfigured Wave 2 adapter must fail immediately"),
        };
        assert_eq!(result.code, "WAVE2_HOST_PORT_UNAVAILABLE");
        assert_eq!(
            result.message,
            "no production host adapter is bound for fs.read"
        );
    }

    #[test]
    fn browser_and_computer_platform_metadata_fail_closed_on_headless_hosts() {
        let registrations = registrations().expect("Wave 2 registrations");
        let materialized = Materializer::materialize(
            &MaterializationPolicy::stable(CONTRACT_VERSION),
            &registrations,
            1,
        )
        .expect("Wave 2 metadata materializes");

        for capability_id in BROWSER_CAPABILITY_IDS {
            let capability = materialized
                .capability(&CapabilityId::from(*capability_id))
                .expect("Browser capability");
            assert!(capability.manifest.supported_platforms.iter().all(
                |constraint| matches!(
                    constraint,
                    PlatformConstraint::Targets { host_surfaces, .. }
                        if host_surfaces == &BTreeSet::from(["desktop".to_owned()])
                )
            ));
            assert_eq!(
                capability.manifest.supported_surfaces,
                BTreeSet::from(["desktop".to_owned()])
            );
            assert!(check_platform_availability(
                &CapabilityId::from(*capability_id),
                &RuntimeTarget::from("x86_64-unknown-linux-gnu"),
                "desktop",
            )
            .is_ok());
            let error = check_platform_availability(
                &CapabilityId::from(*capability_id),
                &RuntimeTarget::from("x86_64-unknown-linux-gnu"),
                "headless",
            )
            .expect_err("headless Browser must be unavailable");
            assert_eq!(error.canonical_code(), unavailable_on_platform_code());
        }

        for capability_id in COMPUTER_A11Y_CAPABILITY_IDS {
            let capability = materialized
                .capability(&CapabilityId::from(*capability_id))
                .expect("Computer/A11y capability");
            assert!(capability.manifest.supported_platforms.iter().all(
                |constraint| matches!(
                    constraint,
                    PlatformConstraint::Targets { host_surfaces, .. }
                        if host_surfaces == &BTreeSet::from(["desktop".to_owned()])
                )
            ));
            assert_eq!(
                capability.manifest.supported_surfaces,
                BTreeSet::from(["desktop".to_owned()])
            );
            for (target, surface) in [
                ("x86_64-unknown-linux-gnu", "desktop"),
                ("x86_64-unknown-linux-gnu", "headless"),
            ] {
                let error = check_platform_availability(
                    &CapabilityId::from(*capability_id),
                    &RuntimeTarget::from(target),
                    surface,
                )
                .expect_err("unsupported Computer host must be unavailable");
                assert_eq!(error.canonical_code(), unavailable_on_platform_code());
            }
            assert!(check_platform_availability(
                &CapabilityId::from(*capability_id),
                &RuntimeTarget::from("x86_64-pc-windows-msvc"),
                "desktop",
            )
            .is_ok());
        }
    }
}
