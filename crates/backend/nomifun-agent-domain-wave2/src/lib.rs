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
    CapabilityContributions, CapabilityId, CapabilityKind, CapabilityManifest, CapabilityRef,
    CanonicalErrorCode, CanonicalSchemaRef, CancellationDescriptor,
    CorrelationId, DeclaredServiceViewDescriptor, EffectClass, ExactRoleContractRef,
    ExecutionRoleId, HostPortBindingDescriptor, HostPortId, HostPortRef, IdempotencyKey,
    InProcessEntrypointMetadata,
    LocalizedMetadata, ManagedTaskRegistrationDescriptor, PackageContributions,
    PackageId, PackageManifest, PackageRef, PlatformConstraint, PluginBootCriticality,
    PluginBootState, PluginContextDescriptor, PluginDesiredState, PluginEffectiveState,
    PluginIdentityDescriptor, PluginMountId, PluginRegistrarDescriptor,
    PluginRegistrarOperation, PluginRegistrationMetadata, PluginSourceKind,
    PluginSourceMetadata, PluginStateCompareAndSwapOutcome, PluginStateEntry,
    PluginStateHandleDescriptor, PluginStateMethod, OperationId, PrincipalRef,
    ExactRoleProviderRef, ResolvedSnapshotRef, ResourceKind, RoleContractKey, RoleContractManifest,
    RoleMemberContract, RoleMemberRequirement, RoleProviderContribution,
    RoleProviderMemberContribution, RuntimeTarget, ScopeKey, StateKey, StrictJsonValue,
    ToolPresentationKind, TypedResourceBindings, ValidatedPluginConfig, VersionString,
    CAPABILITY_UNAVAILABLE_ON_PLATFORM, PRESET_RESOURCE_NOT_BOUND, RESOURCE_OWNER_MISMATCH,
    digest_payload,
};
use nomifun_agent_kernel::{
    CapabilityHandler, CapabilityInvocationContext, ContextContributionFactory,
    HostPluginStateApi, KernelError, PluginRegistration, PluginStateError,
    PluginStateHandle, ResourceProviderFactory,
    ResourceProviderRequest, ResourceProviderResult, ResolvedRoleMemberContext,
    ContextContributionRequest, ContextContributionResult,
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
pub const BROWSER_EXECUTION_ROLE_ID: &str = "system.browser_use";
pub const COMPUTER_EXECUTION_ROLE_ID: &str = "system.computer_use";

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
    "browser.render_content",
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

pub const ALL_CAPABILITY_IDS: [&str; 42] = [
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
    "browser.render_content",
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
pub const TARGET_CAPABILITY_IDS: [&str; 42] = ALL_CAPABILITY_IDS;

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

/// Canonical error codes used by the typed Wave 2 host boundary.
///
/// `CAPABILITY_UNAVAILABLE` is intentionally distinct from
/// `CAPABILITY_UNAVAILABLE_ON_PLATFORM`: the former means that the owning
/// application adapter is not currently mounted, while the latter means that
/// the capability is not part of the host's declared platform surface.
pub const CAPABILITY_UNAVAILABLE: &str = "CAPABILITY_UNAVAILABLE";
pub const INVALID_PAYLOAD: &str = "INVALID_PAYLOAD";
pub const RESOURCE_NOT_FOUND: &str = "RESOURCE_NOT_FOUND";

/// The narrow state surface exposed to a Wave 2 owner.
///
/// The owner can inspect its Kernel-authorized namespace and perform an
/// atomic compare-and-swap transition, but it cannot receive a pool or a
/// service locator. Read/modify/write state transitions must use CAS.
#[derive(Clone)]
pub struct Wave2StateHandle(PluginStateHandle);

impl fmt::Debug for Wave2StateHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Wave2StateHandle")
            .field("descriptor", self.descriptor())
            .finish()
    }
}

impl PartialEq for Wave2StateHandle {
    fn eq(&self, other: &Self) -> bool {
        self.descriptor() == other.descriptor()
    }
}

impl Eq for Wave2StateHandle {}

impl Wave2StateHandle {
    fn new(handle: PluginStateHandle) -> Self {
        Self(handle)
    }

    pub fn descriptor(&self) -> &PluginStateHandleDescriptor {
        self.0.descriptor()
    }

    pub async fn get(
        &self,
        scope_key: &ScopeKey,
        state_key: &StateKey,
    ) -> Result<Option<PluginStateEntry>, PluginStateError> {
        self.0.get(scope_key, state_key).await
    }

    pub async fn compare_and_swap(
        &self,
        scope_key: &ScopeKey,
        state_key: &StateKey,
        expected_revision: u64,
        state_format_version: &VersionString,
        value: Option<StrictJsonValue>,
    ) -> Result<PluginStateCompareAndSwapOutcome, PluginStateError> {
        self.0
            .compare_and_swap(
                scope_key,
                state_key,
                expected_revision,
                state_format_version,
                value,
            )
            .await
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave2HostContext {
    /// The authenticated principal used for owner checks; adapters must not
    /// infer identity from input payloads or resource IDs.
    pub principal: PrincipalRef,
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub idempotency_key: IdempotencyKey,
    pub correlation_id: CorrelationId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub registry_generation: u64,
    pub capability_id: CapabilityId,
    pub action_id: ActionId,
    pub role_provider: Option<ExactRoleProviderRef>,
    /// The frozen, authorization-bearing host bindings selected for this
    /// invocation. The application resolves these bindings; the adapter
    /// receives them without any pool or service-bag access.
    /// The state handle is similarly scoped to the mounted package and is
    /// the only state surface exposed to the adapter.
    pub state: Wave2StateHandle,
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

/// Exact capability-to-operation mapping exposed to composable host
/// adapters.
///
/// [`Wave2CapabilityOperation`] remains the compatibility envelope consumed by
/// the existing application host. This enum is the stricter contract for new
/// owners: each action has its own variant, so a dispatch implementation cannot
/// accidentally handle `fs.delete` as `fs.read` or silently treat an
/// unsupported action as a successful generic operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Wave2TypedCapabilityOperation {
    FsRead { input: StrictJsonValue },
    FsSearch { input: StrictJsonValue },
    FsWrite { input: StrictJsonValue },
    FsPatch { input: StrictJsonValue },
    FsDelete { input: StrictJsonValue },
    FsSnapshot { input: StrictJsonValue },
    VcsStatus { input: StrictJsonValue },
    VcsDiff { input: StrictJsonValue },
    VcsStage { input: StrictJsonValue },
    VcsCommit { input: StrictJsonValue },
    VcsPush { input: StrictJsonValue },
    ProcessExec { input: StrictJsonValue },
    SshFsRead { input: StrictJsonValue },
    SshFsWrite { input: StrictJsonValue },
    SshExec { input: StrictJsonValue },
    SshSudo { input: StrictJsonValue },
    McpToolProxy { input: StrictJsonValue },
    ConnectorDataRead { input: StrictJsonValue },
    ConnectorDataWrite { input: StrictJsonValue },
    BrowserNavigate { input: StrictJsonValue },
    BrowserAct { input: StrictJsonValue },
    BrowserRenderContent { input: StrictJsonValue },
    BrowserDownload { input: StrictJsonValue },
    BrowserUpload { input: StrictJsonValue },
    BrowserEvaluate { input: StrictJsonValue },
    BrowserTakeover { input: StrictJsonValue },
    ComputerInput { input: StrictJsonValue },
    ComputerLaunch { input: StrictJsonValue },
}

impl Wave2TypedCapabilityOperation {
    pub fn capability_id(&self) -> &'static str {
        match self {
            Self::FsRead { .. } => "fs.read",
            Self::FsSearch { .. } => "fs.search",
            Self::FsWrite { .. } => "fs.write",
            Self::FsPatch { .. } => "fs.patch",
            Self::FsDelete { .. } => "fs.delete",
            Self::FsSnapshot { .. } => "fs.snapshot",
            Self::VcsStatus { .. } => "vcs.status",
            Self::VcsDiff { .. } => "vcs.diff",
            Self::VcsStage { .. } => "vcs.stage",
            Self::VcsCommit { .. } => "vcs.commit",
            Self::VcsPush { .. } => "vcs.push",
            Self::ProcessExec { .. } => "process.exec",
            Self::SshFsRead { .. } => "ssh.fs.read",
            Self::SshFsWrite { .. } => "ssh.fs.write",
            Self::SshExec { .. } => "ssh.exec",
            Self::SshSudo { .. } => "ssh.sudo",
            Self::McpToolProxy { .. } => "mcp.tool_proxy",
            Self::ConnectorDataRead { .. } => "connector.data.read",
            Self::ConnectorDataWrite { .. } => "connector.data.write",
            Self::BrowserNavigate { .. } => "browser.navigate",
            Self::BrowserAct { .. } => "browser.act",
            Self::BrowserRenderContent { .. } => "browser.render_content",
            Self::BrowserDownload { .. } => "browser.download",
            Self::BrowserUpload { .. } => "browser.upload",
            Self::BrowserEvaluate { .. } => "browser.evaluate",
            Self::BrowserTakeover { .. } => "browser.takeover",
            Self::ComputerInput { .. } => "computer.input",
            Self::ComputerLaunch { .. } => "computer.launch",
        }
    }

    fn family(&self) -> Wave2CapabilityOperation {
        match self {
            Self::FsRead { input }
            | Self::FsSearch { input }
            | Self::FsWrite { input }
            | Self::FsPatch { input }
            | Self::FsDelete { input }
            | Self::FsSnapshot { input }
            | Self::VcsStatus { input }
            | Self::VcsDiff { input }
            | Self::VcsStage { input }
            | Self::VcsCommit { input }
            | Self::VcsPush { input }
            | Self::ProcessExec { input } => {
                Wave2CapabilityOperation::WorkspaceExecution { input: input.clone() }
            }
            Self::SshFsRead { input }
            | Self::SshFsWrite { input }
            | Self::SshExec { input }
            | Self::SshSudo { input } => Wave2CapabilityOperation::Ssh { input: input.clone() },
            Self::McpToolProxy { input }
            | Self::ConnectorDataRead { input }
            | Self::ConnectorDataWrite { input } => {
                Wave2CapabilityOperation::McpConnectors { input: input.clone() }
            }
            Self::BrowserNavigate { input }
            | Self::BrowserAct { input }
            | Self::BrowserRenderContent { input }
            | Self::BrowserDownload { input }
            | Self::BrowserUpload { input }
            | Self::BrowserEvaluate { input }
            | Self::BrowserTakeover { input } => {
                Wave2CapabilityOperation::Browser { input: input.clone() }
            }
            Self::ComputerInput { input } | Self::ComputerLaunch { input } => {
                Wave2CapabilityOperation::ComputerA11y { input: input.clone() }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave2HostRequest {
    pub context: Wave2HostContext,
    pub operation: Wave2CapabilityOperation,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave2TypedHostRequest {
    pub context: Wave2HostContext,
    pub operation: Wave2TypedCapabilityOperation,
}

impl Wave2HostRequest {
    /// Validate the compatibility envelope and project it to the exact
    /// operation contract used by a new owner-backed adapter.
    pub fn into_typed(self) -> Result<Wave2TypedHostRequest, Wave2HostPortError> {
        let expected_action = action_id(self.context.capability_id.as_ref()).ok_or_else(|| {
            Wave2HostPortError::new(
                CAPABILITY_UNAVAILABLE,
                format!(
                    "{} does not expose an action host operation",
                    self.context.capability_id.as_ref()
                ),
            )
        })?;
        if self.context.action_id != expected_action {
            return Err(Wave2HostPortError::new(
                "ACTION_NOT_DECLARED",
                format!(
                    "{} received action {} instead of {}",
                    self.context.capability_id.as_ref(),
                    self.context.action_id.as_ref(),
                    expected_action.as_ref()
                ),
            ));
        }
        let input = match &self.operation {
            Wave2CapabilityOperation::WorkspaceExecution { input }
            | Wave2CapabilityOperation::Ssh { input }
            | Wave2CapabilityOperation::McpConnectors { input }
            | Wave2CapabilityOperation::Browser { input }
            | Wave2CapabilityOperation::ComputerA11y { input } => input.clone(),
        };
        let typed = typed_operation_for(&self.context.capability_id, input)
            .map_err(|error| Wave2HostPortError::new("ACTION_NOT_DECLARED", error.to_string()))?;
        if typed.family() != self.operation {
            return Err(Wave2HostPortError::new(
                "ACTION_OPERATION_MISMATCH",
                format!(
                    "{} was paired with the wrong typed host operation family",
                    self.context.capability_id.as_ref()
                ),
            ));
        }
        validate_action_resource_bindings(
            &self.context.capability_id,
            &self.context.principal,
            &self.context.resource_bindings,
        )
        .map_err(|error| {
            Wave2HostPortError::new(error.canonical_code().as_ref().to_owned(), error.to_string())
        })?;
        Ok(Wave2TypedHostRequest { context: self.context, operation: typed })
    }
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
        Self::new(CAPABILITY_UNAVAILABLE, message)
    }

    pub fn invalid_payload(message: impl Into<String>) -> Self {
        Self::new(INVALID_PAYLOAD, message)
    }

    pub fn resource_not_bound(message: impl Into<String>) -> Self {
        Self::new(PRESET_RESOURCE_NOT_BOUND, message)
    }

    pub fn owner_mismatch(message: impl Into<String>) -> Self {
        Self::new(RESOURCE_OWNER_MISMATCH, message)
    }

    pub fn platform_unavailable(message: impl Into<String>) -> Self {
        Self::new(CAPABILITY_UNAVAILABLE_ON_PLATFORM, message)
    }

    pub fn canonical_code(&self) -> CanonicalErrorCode {
        CanonicalErrorCode::from(self.code.clone())
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

/// Trusted facts projected to non-action Role members.
#[derive(Clone)]
pub struct Wave2RoleMemberContext {
    pub principal: PrincipalRef,
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub correlation_id: CorrelationId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub registry_generation: u64,
    pub capability_id: CapabilityId,
    pub role_provider: ExactRoleProviderRef,
    pub state_scope_key: ScopeKey,
    pub state: Wave2StateHandle,
    pub resource_bindings: TypedResourceBindings,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Wave2ContextCapabilityOperation {
    BrowserObserve,
    BrowserSiteMemory,
    ComputerObserve,
    A11yObserve,
}

impl Wave2ContextCapabilityOperation {
    pub fn capability_id(&self) -> &'static str {
        match self {
            Self::BrowserObserve => "browser.observe",
            Self::BrowserSiteMemory => "browser.site_memory",
            Self::ComputerObserve => "computer.observe",
            Self::A11yObserve => "a11y.observe",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Wave2ResourceCapabilityOperation {
    BrowserIdentity,
}

impl Wave2ResourceCapabilityOperation {
    pub fn capability_id(&self) -> &'static str {
        match self {
            Self::BrowserIdentity => "browser.identity",
        }
    }
}

#[derive(Clone)]
pub struct Wave2ContextHostRequest {
    pub context: Wave2RoleMemberContext,
    pub operation: Wave2ContextCapabilityOperation,
    pub schema_ref: CanonicalSchemaRef,
}

#[derive(Clone)]
pub struct Wave2ResourceHostRequest {
    pub context: Wave2RoleMemberContext,
    pub operation: Wave2ResourceCapabilityOperation,
}

pub trait Wave2ContextHostPort: Send + Sync {
    fn contribute<'a>(
        &'a self,
        request: Wave2ContextHostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<ContextContributionResult, Wave2HostPortError>>
                + Send
                + 'a,
        >,
    >;
}

pub trait Wave2ResourceHostPort: Send + Sync {
    fn acquire<'a>(
        &'a self,
        request: Wave2ResourceHostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<ResourceProviderResult, Wave2HostPortError>>
                + Send
                + 'a,
        >,
    >;
}

/// An exact-operation adapter used by [`Wave2HostPortDispatcher`].
pub trait Wave2TypedOperationAdapter: Send + Sync {
    fn supports(&self, operation: &Wave2TypedCapabilityOperation) -> bool;

    fn invoke<'a>(
        &'a self,
        request: Wave2TypedHostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a>>;
}

/// Compose independently owned capability owners behind one Wave 2 host port.
///
/// Dispatch is first-match and fail-closed. An adapter is selected only by its
/// exact typed operation. The dispatcher never retries another owner after an
/// adapter has accepted an operation, so an owner remains authoritative for
/// its side effects.
pub struct Wave2HostPortDispatcher {
    adapters: Vec<Arc<dyn Wave2TypedOperationAdapter>>,
}

impl Wave2HostPortDispatcher {
    pub fn new(adapters: Vec<Arc<dyn Wave2TypedOperationAdapter>>) -> Self {
        Self { adapters }
    }

    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    pub fn push(&mut self, adapter: Arc<dyn Wave2TypedOperationAdapter>) {
        self.adapters.push(adapter);
    }
}

impl Wave2HostPort for Wave2HostPortDispatcher {
    fn invoke<'a>(
        &'a self,
        request: Wave2HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a>> {
        let typed = match request.into_typed() {
            Ok(typed) => typed,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        let Some(adapter) = self
            .adapters
            .iter()
            .find(|adapter| adapter.supports(&typed.operation))
        else {
            let capability_id = typed.context.capability_id.clone();
            return Box::pin(async move {
                Err(Wave2HostPortError::unavailable(format!(
                    "no canonical application owner is wired for {}",
                    capability_id.as_ref()
                )))
            });
        };
        adapter.invoke(typed)
    }
}

struct ClosureWave2TypedOperationAdapter<S, F> {
    supports: S,
    dispatch: F,
}

impl<S, F> ClosureWave2TypedOperationAdapter<S, F> {
    fn new(supports: S, dispatch: F) -> Self {
        Self { supports, dispatch }
    }
}

impl<S, F, Fut> Wave2TypedOperationAdapter for ClosureWave2TypedOperationAdapter<S, F>
where
    S: Fn(&Wave2TypedCapabilityOperation) -> bool + Send + Sync + 'static,
    F: Fn(Wave2TypedHostRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'static,
{
    fn supports(&self, operation: &Wave2TypedCapabilityOperation) -> bool {
        (self.supports)(operation)
    }

    fn invoke<'a>(
        &'a self,
        request: Wave2TypedHostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a>> {
        let dispatch = &self.dispatch;
        Box::pin(async move { dispatch(request).await })
    }
}

/// Build one exact-operation adapter from closures for central app
/// composition without exposing this crate's private handler types.
pub fn typed_operation_adapter<S, F, Fut>(
    supports: S,
    dispatch: F,
) -> Arc<dyn Wave2TypedOperationAdapter>
where
    S: Fn(&Wave2TypedCapabilityOperation) -> bool + Send + Sync + 'static,
    F: Fn(Wave2TypedHostRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'static,
{
    Arc::new(ClosureWave2TypedOperationAdapter::new(supports, dispatch))
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

struct UnconfiguredWave2ContextHostPort;

impl Wave2ContextHostPort for UnconfiguredWave2ContextHostPort {
    fn contribute<'a>(
        &'a self,
        request: Wave2ContextHostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<ContextContributionResult, Wave2HostPortError>>
                + Send
                + 'a,
        >,
    >
    {
        Box::pin(async move {
            Err(Wave2HostPortError::unavailable(format!(
                "no production context owner is bound for {}",
                request.context.capability_id.as_ref()
            )))
        })
    }
}

struct UnconfiguredWave2ResourceHostPort;

impl Wave2ResourceHostPort for UnconfiguredWave2ResourceHostPort {
    fn acquire<'a>(
        &'a self,
        request: Wave2ResourceHostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<ResourceProviderResult, Wave2HostPortError>>
                + Send
                + 'a,
        >,
    >
    {
        Box::pin(async move {
            Err(Wave2HostPortError::unavailable(format!(
                "no production resource owner is bound for {}",
                request.context.capability_id.as_ref()
            )))
        })
    }
}

pub fn unconfigured_context_host_port() -> Arc<dyn Wave2ContextHostPort> {
    Arc::new(UnconfiguredWave2ContextHostPort)
}

pub fn unconfigured_resource_host_port() -> Arc<dyn Wave2ResourceHostPort> {
    Arc::new(UnconfiguredWave2ResourceHostPort)
}

#[derive(Clone)]
pub struct Wave2RoleHostPorts {
    pub actions: Arc<dyn Wave2HostPort>,
    pub browser_actions: Arc<dyn Wave2HostPort>,
    pub computer_actions: Arc<dyn Wave2HostPort>,
    pub browser_contexts: Arc<dyn Wave2ContextHostPort>,
    pub computer_contexts: Arc<dyn Wave2ContextHostPort>,
    pub browser_resources: Arc<dyn Wave2ResourceHostPort>,
}

impl Wave2RoleHostPorts {
    pub fn with_actions(actions: Arc<dyn Wave2HostPort>) -> Self {
        Self {
            browser_actions: Arc::clone(&actions),
            computer_actions: Arc::clone(&actions),
            actions,
            browser_contexts: unconfigured_context_host_port(),
            computer_contexts: unconfigured_context_host_port(),
            browser_resources: unconfigured_resource_host_port(),
        }
    }

    fn action_port(&self, package_id: &str) -> Arc<dyn Wave2HostPort> {
        match package_id {
            BROWSER_PACKAGE_ID => Arc::clone(&self.browser_actions),
            COMPUTER_A11Y_PACKAGE_ID => Arc::clone(&self.computer_actions),
            _ => Arc::clone(&self.actions),
        }
    }

    fn context_port(
        &self,
        role_id: &ExecutionRoleId,
    ) -> Arc<dyn Wave2ContextHostPort> {
        match role_id.as_ref() {
            BROWSER_EXECUTION_ROLE_ID => Arc::clone(&self.browser_contexts),
            COMPUTER_EXECUTION_ROLE_ID => Arc::clone(&self.computer_contexts),
            _ => unconfigured_context_host_port(),
        }
    }

    fn resource_port(
        &self,
        role_id: &ExecutionRoleId,
    ) -> Arc<dyn Wave2ResourceHostPort> {
        match role_id.as_ref() {
            BROWSER_EXECUTION_ROLE_ID => Arc::clone(&self.browser_resources),
            _ => unconfigured_resource_host_port(),
        }
    }
}

struct Wave2ContextFactory {
    role_id: ExecutionRoleId,
    capability_id: CapabilityId,
    host_port: Arc<dyn Wave2ContextHostPort>,
}

#[async_trait::async_trait]
impl ContextContributionFactory for Wave2ContextFactory {
    async fn contribute(
        &self,
        request: ContextContributionRequest,
    ) -> Result<ContextContributionResult, KernelError> {
        if request.context.provider_lock.provider.role.key.role_id != self.role_id
            || request.context.member_id != self.capability_id
        {
            return Err(KernelError::RoleProviderMemberUnavailable {
                role_id: self.role_id.clone(),
                capability_id: self.capability_id.clone(),
            });
        }
        let operation = match self.capability_id.as_ref() {
            "browser.observe" => Wave2ContextCapabilityOperation::BrowserObserve,
            "browser.site_memory" => Wave2ContextCapabilityOperation::BrowserSiteMemory,
            "computer.observe" => Wave2ContextCapabilityOperation::ComputerObserve,
            "a11y.observe" => Wave2ContextCapabilityOperation::A11yObserve,
            _ => {
                return Err(KernelError::RoleProviderMemberUnavailable {
                    role_id: self.role_id.clone(),
                    capability_id: self.capability_id.clone(),
                });
            }
        };
        self.host_port
            .contribute(Wave2ContextHostRequest {
                context: role_member_context(request.context)?,
                operation,
                schema_ref: request.schema_ref,
            })
            .await
            .map_err(|error| KernelError::CapabilityExecution {
                reason: error.to_string(),
            })
    }
}

struct Wave2ResourceFactory {
    role_id: ExecutionRoleId,
    capability_id: CapabilityId,
    host_port: Arc<dyn Wave2ResourceHostPort>,
}

#[async_trait::async_trait]
impl ResourceProviderFactory for Wave2ResourceFactory {
    async fn acquire(
        &self,
        request: ResourceProviderRequest,
    ) -> Result<ResourceProviderResult, KernelError> {
        if request.context.provider_lock.provider.role.key.role_id != self.role_id
            || request.context.member_id != self.capability_id
        {
            return Err(KernelError::RoleProviderMemberUnavailable {
                role_id: self.role_id.clone(),
                capability_id: self.capability_id.clone(),
            });
        }
        let operation = match self.capability_id.as_ref() {
            "browser.identity" => Wave2ResourceCapabilityOperation::BrowserIdentity,
            _ => {
                return Err(KernelError::RoleProviderMemberUnavailable {
                    role_id: self.role_id.clone(),
                    capability_id: self.capability_id.clone(),
                });
            }
        };
        self.host_port
            .acquire(Wave2ResourceHostRequest {
                context: role_member_context(request.context)?,
                operation,
            })
            .await
            .map_err(|error| KernelError::CapabilityExecution {
                reason: error.to_string(),
            })
    }
}

fn role_member_context(
    context: ResolvedRoleMemberContext,
) -> Result<Wave2RoleMemberContext, KernelError> {
    let agent_session_id = context
        .agent_session_id
        .ok_or_else(|| KernelError::CapabilityExecution {
            reason: "role member context requires an AgentSession".to_owned(),
        })?;
    let resolved_snapshot_ref = context
        .resolved_snapshot_ref
        .ok_or_else(|| KernelError::CapabilityExecution {
            reason: "role member context requires a frozen Snapshot".to_owned(),
        })?;
    Ok(Wave2RoleMemberContext {
        principal: context.principal,
        agent_session_id,
        operation_id: context.operation_id,
        correlation_id: context.correlation_id,
        resolved_snapshot_ref,
        registry_generation: context.registry_generation,
        capability_id: context.member_id,
        role_provider: context.provider_lock.provider,
        state_scope_key: context.state_scope_key,
        state: Wave2StateHandle::new(context.mount.state),
        resource_bindings: context.resource_bindings,
    })
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
    CapabilityDefinition::browser_tool(
        "browser.render_content",
        EffectClass::ExternalTransmit,
    ),
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
    registrations_with_role_host_ports(Wave2RoleHostPorts::with_actions(action_host_port))
}

/// Build the complete Wave 2 registration inventory with independently typed
/// action, context, and resource host ports.
pub fn registrations_with_role_host_ports(
    role_host_ports: Wave2RoleHostPorts,
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
        let registration = build_registration(package, role_host_ports.clone())?;
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
    role_host_ports: Wave2RoleHostPorts,
) -> Result<PluginRegistration, String> {
    let action_host_port = role_host_ports.action_port(package.id);
    let package_ref = PackageRef {
        id: PackageId::from(package.id),
        version: VersionString::from(CONTRACT_VERSION),
    };
    let mut capability_manifests = Vec::with_capacity(package.capabilities.len());
    let mut handlers = Vec::new();
    let mut role_handlers = Vec::new();

    for definition in package.capabilities {
        let capability = build_capability(&package_ref, *definition)?;
        if definition.is_tool() {
            let capability_id = CapabilityId::from(definition.id);
            if let Some(role_id) = role_id_for_capability(definition.id) {
                role_handlers.push((
                    role_id,
                    capability_id.clone(),
                    ActionId::from(format!("{}.invoke", definition.id)),
                ));
            } else {
                handlers.push((
                    capability_id.clone(),
                    ActionId::from(format!("{}.invoke", definition.id)),
                ));
            }
        }
        capability_manifests.push(capability);
    }

    let role_contracts = role_contracts_for_package(package, &capability_manifests)?;
    let role_providers = role_providers_for_package(package, &role_contracts)?;
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
            role_contracts: role_contracts.clone(),
            role_providers,
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
    let has_action_handler = !handlers.is_empty() || !role_handlers.is_empty();
    let has_role_handler = !role_handlers.is_empty()
        || package.capabilities.iter().any(|definition| {
            role_id_for_capability(definition.id).is_some()
                && matches!(
                    definition.kind,
                    CapabilityKind::ContextContributor | CapabilityKind::ResourceProvider
                )
        });
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
            ])
            .into_iter()
            .chain(
                has_role_handler
                    .then_some(PluginRegistrarOperation::ContributeRoleProvider),
            )
            .collect(),
            declared_capability_ids: package
                .capabilities
                .iter()
                .map(|definition| CapabilityId::from(definition.id))
                .collect(),
            declared_skill_ids: BTreeSet::new(),
            declared_mcp_tool_keys: BTreeSet::new(),
            declared_role_ids: role_contracts
                .iter()
                .map(|contract| contract.key.role_id.clone())
                .collect(),
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
    for (capability_id, action_id) in handlers {
        registration
            .add_capability_handler(
                capability_id.clone(),
                Arc::new(Wave2CapabilityHandler {
                    capability_id,
                    action_id,
                    host_port: Arc::clone(&action_host_port),
                }),
            )
            .map_err(|error| format!("register {} handler: {error}", package.id))?;
    }
    for (role_id, capability_id, action_id) in role_handlers {
        registration
            .add_role_action_handler(
                role_id,
                capability_id.clone(),
                Arc::new(Wave2CapabilityHandler {
                    capability_id,
                    action_id,
                    host_port: Arc::clone(&action_host_port),
                }),
            )
            .map_err(|error| format!("register {} role handler: {error}", package.id))?;
    }
    for definition in package.capabilities {
        let Some(role_id) = role_id_for_capability(definition.id) else {
            continue;
        };
        let capability_id = CapabilityId::from(definition.id);
        match definition.kind {
            CapabilityKind::ContextContributor => {
                registration
                    .add_role_context_factory(
                        role_id.clone(),
                        capability_id.clone(),
                        Arc::new(Wave2ContextFactory {
                            host_port: role_host_ports.context_port(&role_id),
                            role_id,
                            capability_id,
                        }),
                    )
                    .map_err(|error| {
                        format!("register {} context factory: {error}", package.id)
                    })?;
            }
            CapabilityKind::ResourceProvider => {
                registration
                    .add_role_resource_factory(
                        role_id.clone(),
                        capability_id.clone(),
                        Arc::new(Wave2ResourceFactory {
                            host_port: role_host_ports.resource_port(&role_id),
                            role_id,
                            capability_id,
                        }),
                    )
                    .map_err(|error| {
                        format!("register {} resource factory: {error}", package.id)
                    })?;
            }
            _ => {}
        }
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
            presentation: if definition.id == "browser.render_content" {
                ToolPresentationKind::Hidden
            } else {
                ToolPresentationKind::FunctionTool
            },
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

    let supported_surfaces = match role_id_for_capability(definition.id).as_ref().map(AsRef::as_ref)
    {
        Some(BROWSER_EXECUTION_ROLE_ID) => AGENT_SURFACES,
        Some(COMPUTER_EXECUTION_ROLE_ID) => BROWSER_COMPUTER_SURFACES,
        _ => match definition.platform_scope {
            PlatformScope::Any => AGENT_SURFACES,
            PlatformScope::BrowserDesktop | PlatformScope::ComputerDesktop => {
                BROWSER_COMPUTER_SURFACES
            }
        },
    };
    let supported_platforms = if role_id_for_capability(definition.id).is_some() {
        vec![PlatformConstraint::Any]
    } else {
        platform_constraints(definition.platform_scope)
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
        supported_platforms,
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

            validate_action_resource_bindings(
                &self.capability_id,
                &context.principal,
                &context.resource_bindings,
            )?;

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
                        role_provider: context
                            .role_provider
                            .as_ref()
                            .map(|provider| provider.provider.clone()),
                        state: Wave2StateHandle::new(context.state),
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
    Ok(typed_operation_for(capability_id, input)?.family())
}

/// Map an action capability to its exact typed host operation.
///
/// This is deliberately exhaustive over the action inventory. Non-action
/// capabilities (providers, contexts, transports, and event sources) return an
/// error instead of being admitted to an action dispatcher.
pub fn typed_operation_for(
    capability_id: &CapabilityId,
    input: StrictJsonValue,
) -> Result<Wave2TypedCapabilityOperation, KernelError> {
    let operation = match capability_id.as_ref() {
        "fs.read" => Wave2TypedCapabilityOperation::FsRead { input },
        "fs.search" => Wave2TypedCapabilityOperation::FsSearch { input },
        "fs.write" => Wave2TypedCapabilityOperation::FsWrite { input },
        "fs.patch" => Wave2TypedCapabilityOperation::FsPatch { input },
        "fs.delete" => Wave2TypedCapabilityOperation::FsDelete { input },
        "fs.snapshot" => Wave2TypedCapabilityOperation::FsSnapshot { input },
        "vcs.status" => Wave2TypedCapabilityOperation::VcsStatus { input },
        "vcs.diff" => Wave2TypedCapabilityOperation::VcsDiff { input },
        "vcs.stage" => Wave2TypedCapabilityOperation::VcsStage { input },
        "vcs.commit" => Wave2TypedCapabilityOperation::VcsCommit { input },
        "vcs.push" => Wave2TypedCapabilityOperation::VcsPush { input },
        "process.exec" => Wave2TypedCapabilityOperation::ProcessExec { input },
        "ssh.fs.read" => Wave2TypedCapabilityOperation::SshFsRead { input },
        "ssh.fs.write" => Wave2TypedCapabilityOperation::SshFsWrite { input },
        "ssh.exec" => Wave2TypedCapabilityOperation::SshExec { input },
        "ssh.sudo" => Wave2TypedCapabilityOperation::SshSudo { input },
        "mcp.tool_proxy" => Wave2TypedCapabilityOperation::McpToolProxy { input },
        "connector.data.read" => Wave2TypedCapabilityOperation::ConnectorDataRead { input },
        "connector.data.write" => Wave2TypedCapabilityOperation::ConnectorDataWrite { input },
        "browser.navigate" => Wave2TypedCapabilityOperation::BrowserNavigate { input },
        "browser.act" => Wave2TypedCapabilityOperation::BrowserAct { input },
        "browser.render_content" => {
            Wave2TypedCapabilityOperation::BrowserRenderContent { input }
        }
        "browser.download" => Wave2TypedCapabilityOperation::BrowserDownload { input },
        "browser.upload" => Wave2TypedCapabilityOperation::BrowserUpload { input },
        "browser.evaluate" => Wave2TypedCapabilityOperation::BrowserEvaluate { input },
        "browser.takeover" => Wave2TypedCapabilityOperation::BrowserTakeover { input },
        "computer.input" => Wave2TypedCapabilityOperation::ComputerInput { input },
        "computer.launch" => Wave2TypedCapabilityOperation::ComputerLaunch { input },
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

/// Return the operation that must be granted by the bound resource for an
/// action capability.
///
/// The official Coding resource defaults grant `read`, `write`, and `execute`;
/// destructive filesystem actions therefore consume the workspace `write`
/// grant rather than inventing a separate `delete` permission.
pub fn required_resource_operation(capability_id: &CapabilityId) -> Option<&'static str> {
    match capability_id.as_ref() {
        "fs.read" | "fs.search" | "fs.snapshot" | "vcs.status" | "vcs.diff" => Some("read"),
        "fs.write" | "fs.patch" | "fs.delete" | "vcs.stage" | "vcs.commit" | "vcs.push"
        | "ssh.fs.write" | "connector.data.write" => Some("write"),
        "process.exec" | "ssh.exec" | "ssh.sudo" => Some("execute"),
        "ssh.fs.read" | "connector.data.read" => Some("read"),
        "mcp.tool_proxy" => Some("invoke"),
        "browser.navigate" | "browser.upload" | "browser.render_content" => Some("navigate"),
        "browser.act" | "browser.takeover" => Some("interact"),
        "browser.download" => Some("download"),
        "browser.evaluate" => Some("evaluate"),
        "computer.input" => Some("input"),
        "computer.launch" => Some("launch"),
        _ => None,
    }
}

/// Validate the authorization-bearing resource projection before an action is
/// handed to an owner.
///
/// This is intentionally reusable by a central host dispatcher. It validates
/// binding identity, principal ownership, required resource cardinality, and
/// the operation grant. It does not resolve a resource or expose an
/// application pool.
pub fn validate_action_resource_bindings(
    capability_id: &CapabilityId,
    principal: &PrincipalRef,
    bindings: &TypedResourceBindings,
) -> Result<(), KernelError> {
    let Some(definition) = definition_for(capability_id.as_ref()) else {
        return Err(KernelError::CapabilityExecution {
            reason: format!("unknown Wave 2 capability {}", capability_id.as_ref()),
        });
    };
    if !definition.is_tool() {
        return Err(KernelError::CapabilityExecution {
            reason: format!("{} is not an action capability", capability_id.as_ref()),
        });
    }

    let declared_resource_kinds = definition
        .resource_kinds
        .iter()
        .map(|kind| ResourceKind::from(*kind))
        .collect::<BTreeSet<_>>();
    let mut binding_ids = BTreeSet::new();
    for binding in bindings {
        if binding.binding_id.as_ref().is_empty() || binding.resource_id.as_ref().is_empty() {
            return Err(KernelError::CapabilityExecution {
                reason: format!(
                    "{} requires non-empty binding and resource IDs",
                    capability_id.as_ref()
                ),
            });
        }
        if !binding_ids.insert(binding.binding_id.clone()) {
            return Err(KernelError::CapabilityExecution {
                reason: format!(
                    "{} received duplicate resource binding {}",
                    capability_id.as_ref(),
                    binding.binding_id.as_ref()
                ),
            });
        }
        if binding.owner_id != principal.principal_id {
            return Err(KernelError::ResourceOwnerMismatch {
                binding_id: binding.binding_id.clone(),
            });
        }
        if !declared_resource_kinds.contains(&binding.resource_kind) {
            return Err(KernelError::UnexpectedResourceBinding {
                capability_id: capability_id.clone(),
                binding_id: binding.binding_id.clone(),
                resource_kind: binding.resource_kind.as_ref().to_owned(),
            });
        }
    }

    for resource_kind in definition
        .resource_kinds
        .iter()
        .map(|kind| ResourceKind::from(*kind))
    {
        let matching_bindings = bindings
            .iter()
            .filter(|binding| binding.resource_kind == resource_kind)
            .collect::<Vec<_>>();
        if matching_bindings.is_empty() {
            return Err(KernelError::CapabilityResourceNotBound {
                capability_id: capability_id.clone(),
                resource_kind: resource_kind.as_ref().to_owned(),
            });
        }
        if matching_bindings.len() > 1 {
            return Err(KernelError::CapabilityExecution {
                reason: format!(
                    "{} requires exactly one {} resource binding",
                    capability_id.as_ref(),
                    resource_kind.as_ref()
                ),
            });
        }
        if let Some(required_operation) = required_resource_operation(capability_id) {
            if !matching_bindings[0].operations.contains(required_operation) {
                return Err(KernelError::CapabilityResourceNotBound {
                    capability_id: capability_id.clone(),
                    resource_kind: format!(
                        "{} (operation {})",
                        resource_kind.as_ref(),
                        required_operation
                    ),
                });
            }
        }
    }
    Ok(())
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
    build_registration(
        &PACKAGE_DEFINITIONS[0],
        Wave2RoleHostPorts::with_actions(unconfigured_host_port()),
    )
}

/// Build the bundled SSH registration.
pub fn ssh_registration() -> Result<PluginRegistration, String> {
    build_registration(
        &PACKAGE_DEFINITIONS[1],
        Wave2RoleHostPorts::with_actions(unconfigured_host_port()),
    )
}

/// Build the bundled MCP/connectors registration.
pub fn mcp_connectors_registration() -> Result<PluginRegistration, String> {
    build_registration(
        &PACKAGE_DEFINITIONS[2],
        Wave2RoleHostPorts::with_actions(unconfigured_host_port()),
    )
}

/// Build the bundled Browser registration.
pub fn browser_registration() -> Result<PluginRegistration, String> {
    build_registration(
        &PACKAGE_DEFINITIONS[3],
        Wave2RoleHostPorts::with_actions(unconfigured_host_port()),
    )
}

/// Build the bundled Computer/A11y registration.
pub fn computer_a11y_registration() -> Result<PluginRegistration, String> {
    build_registration(
        &PACKAGE_DEFINITIONS[4],
        Wave2RoleHostPorts::with_actions(unconfigured_host_port()),
    )
}

fn role_id_for_capability(capability_id: &str) -> Option<ExecutionRoleId> {
    if capability_id.starts_with("browser.") {
        Some(ExecutionRoleId::from(BROWSER_EXECUTION_ROLE_ID))
    } else if capability_id.starts_with("computer.") || capability_id == "a11y.observe" {
        Some(ExecutionRoleId::from(COMPUTER_EXECUTION_ROLE_ID))
    } else {
        None
    }
}

fn role_contracts_for_package(
    package: &PackageDefinition,
    capabilities: &[CapabilityManifest],
) -> Result<Vec<RoleContractManifest>, String> {
    let role_id = match package.id {
        BROWSER_PACKAGE_ID => BROWSER_EXECUTION_ROLE_ID,
        COMPUTER_A11Y_PACKAGE_ID => COMPUTER_EXECUTION_ROLE_ID,
        _ => return Ok(Vec::new()),
    };
    let member_ids = match role_id {
        BROWSER_EXECUTION_ROLE_ID => [
            ("browser.observe", RoleMemberRequirement::Required),
            ("browser.navigate", RoleMemberRequirement::Required),
            ("browser.act", RoleMemberRequirement::Required),
            ("browser.identity", RoleMemberRequirement::Optional),
            ("browser.render_content", RoleMemberRequirement::Optional),
            ("browser.download", RoleMemberRequirement::Optional),
            ("browser.upload", RoleMemberRequirement::Optional),
            ("browser.evaluate", RoleMemberRequirement::Optional),
            ("browser.site_memory", RoleMemberRequirement::Optional),
            ("browser.takeover", RoleMemberRequirement::Optional),
        ]
        .as_slice(),
        COMPUTER_EXECUTION_ROLE_ID => [
            ("computer.observe", RoleMemberRequirement::Required),
            ("computer.input", RoleMemberRequirement::Required),
            ("computer.launch", RoleMemberRequirement::Optional),
            ("a11y.observe", RoleMemberRequirement::Optional),
        ]
        .as_slice(),
        _ => &[],
    };
    let mut by_id = capabilities
        .iter()
        .map(|capability| (capability.id.as_ref(), capability))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut members = Vec::with_capacity(member_ids.len());
    for (capability_id, requirement) in member_ids {
        let capability = by_id.remove(capability_id).ok_or_else(|| {
            format!(
                "role {} references missing capability {}",
                role_id, capability_id
            )
        })?;
        members.push(RoleMemberContract {
            capability: CapabilityRef {
                id: capability.id.clone(),
                version: capability.version.clone(),
            },
            capability_manifest_digest: digest_payload(capability)
                .map_err(|error| format!("digest role member {capability_id}: {error}"))?,
            requirement: *requirement,
        });
    }
    Ok(vec![RoleContractManifest {
        key: RoleContractKey {
            role_id: ExecutionRoleId::from(role_id),
            contract_version: VersionString::from(CONTRACT_VERSION),
        },
        members,
        serialized_target_resource_kind: (role_id == COMPUTER_EXECUTION_ROLE_ID)
            .then(|| ResourceKind::from("computer")),
    }])
}

fn role_providers_for_package(
    package: &PackageDefinition,
    contracts: &[RoleContractManifest],
) -> Result<Vec<RoleProviderContribution>, String> {
    let Some(contract) = contracts.first() else {
        return Ok(Vec::new());
    };
    let supported_platforms = match contract.key.role_id.as_ref() {
        BROWSER_EXECUTION_ROLE_ID => platform_constraints(PlatformScope::BrowserDesktop),
        COMPUTER_EXECUTION_ROLE_ID => platform_constraints(PlatformScope::ComputerDesktop),
        _ => vec![PlatformConstraint::Any],
    };
    let mut members = std::collections::BTreeMap::new();
    for member in &contract.members {
        let required_resource_kinds = definition_for(member.capability.id.as_ref())
            .map(|definition| {
                definition
                    .resource_kinds
                    .iter()
                    .map(|kind| ResourceKind::from(*kind))
                    .collect()
            })
            .unwrap_or_default();
        members.insert(
            member.capability.id.clone(),
            RoleProviderMemberContribution {
                supported_platforms: supported_platforms.clone(),
                required_resource_kinds,
            },
        );
    }
    Ok(vec![RoleProviderContribution {
        role: ExactRoleContractRef {
            key: contract.key.clone(),
            contract_digest: digest_payload(contract)
                .map_err(|error| format!("digest {} role contract: {error}", package.id))?,
        },
        display: localized(
            match contract.key.role_id.as_ref() {
                BROWSER_EXECUTION_ROLE_ID => "Browser Use",
                COMPUTER_EXECUTION_ROLE_ID => "Computer Use",
                _ => "System Capability",
            },
            "Bundled first-party execution-role provider.",
        ),
        members,
    }])
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
        PlatformScope::Any => AGENT_SURFACES.iter().any(|surface| *surface == host_surface),
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
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::task::{Context, Poll, Waker};

    use super::*;
    use serde_json::json;
    use nomifun_agent_kernel::{
        AgentPresetCompiler, CapabilityInvocationRequest, CompileRequest, CompilerEnvironment,
        ContextContributionFactory, ContextContributionRequest, ContextContributionResult,
        InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy, Materializer,
        ResourceHandle, ResourceHandleIdentity, ResourceProviderFactory, ResourceProviderRequest,
        ResourceProviderResult, RoleMemberAdmission, RoleMemberInvocationRequest,
        SessionCapabilityState,
    };
    use nomifun_agent_contracts::{
        AgentPresetId, AgentPresetRevision, AgentPresetRevisionPayload, CapabilityExposure,
        CapabilityRef, CapabilitySelection, DigestHex, PresetRevisionRef, ResourceBindingId,
        RoleProviderSelection, RuntimeProfileKind, UserId, TypedResourceBinding,
    };

    struct StateCaptureHostPort {
        captured: Arc<Mutex<Option<Wave2StateHandle>>>,
    }

    struct AlternateBrowserHandler {
        captured_mount: Arc<Mutex<Option<String>>>,
    }

    struct AlternateBrowserContextFactory;

    #[async_trait::async_trait]
    impl ContextContributionFactory for AlternateBrowserContextFactory {
        async fn contribute(
            &self,
            request: ContextContributionRequest,
        ) -> Result<ContextContributionResult, KernelError> {
            Ok(ContextContributionResult {
                value: Some(StrictJsonValue(json!({
                    "provider_mount": request.context.mount.identity.mount_id,
                    "capability": request.context.member_id,
                }))),
            })
        }
    }

    struct AlternateBrowserResourceFactory {
        releases: Arc<AtomicUsize>,
    }

    struct AlternateBrowserResourceHandle {
        identity: ResourceHandleIdentity,
        releases: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl ResourceHandle for AlternateBrowserResourceHandle {
        fn identity(&self) -> &ResourceHandleIdentity {
            &self.identity
        }

        async fn release(&self) -> Result<(), KernelError> {
            self.releases.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl ResourceProviderFactory for AlternateBrowserResourceFactory {
        async fn acquire(
            &self,
            request: ResourceProviderRequest,
        ) -> Result<ResourceProviderResult, KernelError> {
            let binding = request
                .context
                .resource_bindings
                .first()
                .ok_or_else(|| KernelError::ResourceBindingMissing {
                    binding_id: ResourceBindingId::from("browser"),
                })?;
            Ok(ResourceProviderResult {
                handle: Arc::new(AlternateBrowserResourceHandle {
                    identity: ResourceHandleIdentity {
                        binding_id: binding.binding_id.clone(),
                        resource_kind: binding.resource_kind.clone(),
                        resource_id: binding.resource_id.clone(),
                    },
                    releases: Arc::clone(&self.releases),
                }),
            })
        }
    }

    impl CapabilityHandler for AlternateBrowserHandler {
        fn invoke<'life0, 'async_trait>(
            &'life0 self,
            context: CapabilityInvocationContext,
            _input: StrictJsonValue,
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
            let captured_mount = Arc::clone(&self.captured_mount);
            Box::pin(async move {
                *captured_mount.lock().expect("alternate provider capture") =
                    Some(context.state.descriptor().mount_id.as_ref().to_owned());
                Ok(empty_object())
            })
        }
    }

    impl Wave2HostPort for StateCaptureHostPort {
        fn invoke<'a>(
            &'a self,
            request: Wave2HostRequest,
        ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a>>
        {
            let captured = Arc::clone(&self.captured);
            let state = request.context.state;
            Box::pin(std::future::ready({
                *captured.lock().expect("state capture mutex") = Some(state);
                Ok(empty_object())
            }))
        }
    }

    fn poll_ready<F: Future>(future: F) -> F::Output {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = std::pin::pin!(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("test future must complete without an executor"),
        }
    }

    fn test_state_handle() -> Wave2StateHandle {
        static HANDLE: OnceLock<Wave2StateHandle> = OnceLock::new();
        HANDLE.get_or_init(capture_state_handle).clone()
    }

    fn alternate_browser_registration_with_capture(
        captured_mount: Arc<Mutex<Option<String>>>,
        releases: Arc<AtomicUsize>,
    ) -> PluginRegistration {
        let first_party = browser_registration().expect("first-party Browser registration");
        let provider = first_party.metadata.manifest.payload.contributions.role_providers[0].clone();
        let package = PackageRef {
            id: PackageId::from("fixture.browser-provider"),
            version: VersionString::from(CONTRACT_VERSION),
        };
        let mount_id = PluginMountId::from("fixture-browser-provider");
        let source = PluginSourceMetadata {
            source_kind: PluginSourceKind::TestFixture,
            source_identity: "fixture.browser-provider".to_owned(),
            source_digest: None,
        };
        let config_schema = object_schema();
        let cancellation_port = host_port("host.plugin.cancel");
        let task_port = host_port("host.plugin.tasks");
        let manifest = PackageManifest {
            schema_version: VersionString::from(CONTRACT_VERSION),
            host_contract_version: VersionString::from(CONTRACT_VERSION),
            package_id: package.id.clone(),
            package_version: package.version.clone(),
            display: localized("Alternate Browser", "Test-only Browser role provider."),
            package_dependencies: Vec::new(),
            requires_runtime_features: Vec::new(),
            config_schema: config_schema.clone(),
            provides_services: Vec::new(),
            requires_services: Vec::new(),
            entrypoint: InProcessEntrypointMetadata {
                entrypoint_profile: "trusted-in-process".to_owned(),
                entrypoint_id: "fixture.browser-provider.entrypoint".to_owned(),
                contract_version: VersionString::from(CONTRACT_VERSION),
            },
            contributions: PackageContributions {
                capabilities: Vec::new(),
                skills: Vec::new(),
                mcp_tools: Vec::new(),
                role_contracts: Vec::new(),
                role_providers: vec![provider.clone()],
            },
        };
        let identity = PluginIdentityDescriptor {
            package: package.clone(),
            mount_id: mount_id.clone(),
        };
        let metadata = PluginRegistrationMetadata {
            manifest: ArtifactEnvelope::new(manifest).expect("alternate Browser manifest"),
            mount_id: mount_id.clone(),
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
                    PluginRegistrarOperation::ContributeRoleProvider,
                    PluginRegistrarOperation::BindHostPort,
                ]),
                declared_capability_ids: BTreeSet::new(),
                declared_skill_ids: BTreeSet::new(),
                declared_mcp_tool_keys: BTreeSet::new(),
                declared_role_ids: BTreeSet::from([provider.role.key.role_id.clone()]),
                declared_service_keys: BTreeSet::new(),
                declared_host_ports: BTreeSet::from([
                    cancellation_port.id.clone(),
                    task_port.id.clone(),
                ]),
            },
            context: PluginContextDescriptor {
                identity,
                source,
                validated_config: ValidatedPluginConfig {
                    schema_digest: digest_payload(&config_schema).expect("config digest"),
                    config_revision: 1,
                    value: empty_object(),
                },
                state: PluginStateHandleDescriptor {
                    package_id: package.id,
                    mount_id: mount_id.clone(),
                    methods: PluginStateMethod::REQUIRED.into_iter().collect(),
                },
                declared_services: DeclaredServiceViewDescriptor::default(),
                host_ports: Vec::new(),
                typed_command_ports: Vec::new(),
                domain_outbox_ports: Vec::new(),
                cancellation: CancellationDescriptor {
                    cancellation_port,
                    scope_key: ScopeKey::from("mount:fixture-browser-provider"),
                },
                managed_task_registration: ManagedTaskRegistrationDescriptor {
                    registrar_port: task_port,
                    scope_key: ScopeKey::from("mount:fixture-browser-provider"),
                },
            },
        };
        let mut registration = PluginRegistration::new(metadata);
        for capability_id in provider.members.keys() {
            let Some(definition) = definition_for(capability_id.as_ref()) else {
                continue;
            };
            match definition.kind {
                CapabilityKind::Tool => {
                    registration
                        .add_role_action_handler(
                            provider.role.key.role_id.clone(),
                            capability_id.clone(),
                            Arc::new(AlternateBrowserHandler {
                                captured_mount: Arc::clone(&captured_mount),
                            }),
                        )
                        .expect("alternate Browser role handler");
                }
                CapabilityKind::ContextContributor => {
                    registration
                        .add_role_context_factory(
                            provider.role.key.role_id.clone(),
                            capability_id.clone(),
                            Arc::new(AlternateBrowserContextFactory),
                        )
                        .expect("alternate Browser context factory");
                }
                CapabilityKind::ResourceProvider => {
                    registration
                        .add_role_resource_factory(
                            provider.role.key.role_id.clone(),
                            capability_id.clone(),
                            Arc::new(AlternateBrowserResourceFactory {
                                releases: Arc::clone(&releases),
                            }),
                        )
                        .expect("alternate Browser resource factory");
                }
                _ => {}
            }
        }
        registration
    }

    fn capture_state_handle() -> Wave2StateHandle {
        let captured = Arc::new(Mutex::new(None));
        let registry = KernelRegistry::new(
            MaterializationPolicy::stable(CONTRACT_VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .expect("kernel registry");
        let materialized = registry
            .replace_all(
                registrations_with_host_port(Arc::new(StateCaptureHostPort {
                    captured: Arc::clone(&captured),
                }))
                .expect("Wave 2 registrations"),
            )
            .expect("publish Wave 2 registrations");

        let principal = nomifun_agent_contracts::PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "wave2-state-owner".to_owned(),
        };
        let binding = TypedResourceBinding {
            binding_id: ResourceBindingId::from("wave2-state-workspace"),
            resource_kind: nomifun_agent_contracts::ResourceKind::from("workspace"),
            resource_id: nomifun_agent_contracts::ResourceId::from("wave2-state-resource"),
            owner_id: principal.principal_id.clone(),
            operations: BTreeSet::from(["read".to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        };
        let action = action_id("fs.read").expect("fs.read action");
        let payload = AgentPresetRevisionPayload {
            schema_version: VersionString::from(CONTRACT_VERSION),
            surfaces: BTreeSet::from(["desktop".to_owned()]),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            initial_capabilities: vec![CapabilitySelection {
                capability: CapabilityRef {
                    id: CapabilityId::from("fs.read"),
                    version: VersionString::from(CONTRACT_VERSION),
                },
                required: true,
                exposure: CapabilityExposure::Advertised,
                action_allowlist: BTreeSet::from([action.clone()]),
                resource_binding_refs: vec![binding.binding_id.clone()],
                destination_constraints: BTreeSet::new(),
                context_budget_override: None,
                tool_budget_override: None,
                config: empty_object(),
            }],
            on_demand_capabilities: Vec::new(),
            skill_bindings: Vec::new(),
            resource_bindings: vec![binding],
            system_role_provider_overrides: BTreeMap::new(),
            persona: "Wave 2 state test".to_owned(),
            instructions: "Invoke the selected capability.".to_owned(),
            context_policy: empty_object(),
            execution_constraints: empty_object(),
            runtime_budget: empty_object(),
        };
        let revision = AgentPresetRevision {
            reference: PresetRevisionRef {
                preset_id: AgentPresetId::from("wave2-state-test"),
                revision: 1,
                revision_digest: digest_payload(&payload).expect("revision digest"),
            },
            payload,
            created_by: UserId::from(principal.principal_id.clone()),
            created_at_ms: 1,
            reason: None,
        };
        let snapshot = AgentPresetCompiler::compile(
            &materialized,
            &CompilerEnvironment {
                resolver_version: VersionString::from(CONTRACT_VERSION),
                required_runtime_protocol_version: VersionString::from(CONTRACT_VERSION),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: DigestHex::from("runtime"),
                available_runtime_features: BTreeSet::new(),
                installation_role_bindings: BTreeMap::new(),
                canonical_schema_manifest_digest: DigestHex::from("schema"),
                target_contribution_manifest_digest: DigestHex::from("target"),
                host_target: RuntimeTarget::from("windows-desktop-x64"),
                host_surface: "desktop".to_owned(),
                availability_evidence_revision: "wave2-state-test".to_owned(),
            },
            CompileRequest {
                revision,
                principal: principal.clone(),
                scene: "wave2-state-test".to_owned(),
                surface: "desktop".to_owned(),
                audience: "test".to_owned(),
                created_at_ms: 2,
                resolver_run_id: OperationId::from("wave2-state-resolve"),
            },
        )
        .expect("compile selected capability");
        let active = SessionCapabilityState::new(&snapshot)
            .snapshot()
            .expect("initial active set");
        let result = poll_ready(registry.invoke(
            &snapshot,
            &active,
            CapabilityInvocationRequest {
                principal: principal.clone(),
                session_owner: principal,
                agent_session_id: AgentSessionId::from("wave2-state-session"),
                operation_id: OperationId::from("wave2-state-operation"),
                idempotency_key: IdempotencyKey::from("wave2-state-idempotency"),
                correlation_id: CorrelationId::from("wave2-state-correlation"),
                resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                active_set_generation: active.generation,
                capability_id: CapabilityId::from("fs.read"),
                action_id: action,
                resource_binding_ids: BTreeSet::from([ResourceBindingId::from(
                    "wave2-state-workspace",
                )]),
                state_scope_key: ScopeKey::from("package:wave2-state-test"),
                input: empty_object(),
            },
        ));
        result.expect("state projection invocation");
        captured
            .lock()
            .expect("state capture mutex")
            .take()
            .expect("host adapter received the state handle")
    }

    #[test]
    fn kernel_projects_package_and_mount_scoped_state_handle_to_host_adapter() {
        let state = capture_state_handle();
        assert_eq!(
            state.descriptor().package_id.as_ref(),
            WORKSPACE_EXECUTION_PACKAGE_ID
        );
        assert_eq!(
            state.descriptor().mount_id.as_ref(),
            WORKSPACE_EXECUTION_MOUNT_ID
        );
        assert_eq!(
            state.descriptor().methods,
            PluginStateMethod::REQUIRED.into_iter().collect()
        );
    }

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
            let role_capabilities = manifest
                .contributions
                .role_contracts
                .iter()
                .flat_map(|contract| {
                    contract
                        .members
                        .iter()
                        .map(|member| member.capability.id.clone())
                })
                .collect::<BTreeSet<_>>();
            let ordinary_action_capabilities = action_capabilities
                .difference(&role_capabilities)
                .cloned()
                .collect::<BTreeSet<_>>();
            assert_eq!(registration.handler_ids(), ordinary_action_capabilities);
            let role_action_capabilities = registration
                .role_action_handler_ids()
                .into_iter()
                .map(|(_, capability_id)| capability_id)
                .collect::<BTreeSet<_>>();
            assert_eq!(
                role_action_capabilities,
                action_capabilities
                    .intersection(&role_capabilities)
                    .cloned()
                    .collect()
            );
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
        assert_eq!(materialized.capabilities.len(), 42);
        assert_eq!(materialized.role_contracts.len(), 2);
        assert_eq!(materialized.role_providers.len(), 2);
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
    fn explicit_browser_provider_lock_dispatches_once_without_fallback() {
        let first_party = browser_registration().expect("first-party Browser registration");
        let captured_mount = Arc::new(Mutex::new(None));
        let releases = Arc::new(AtomicUsize::new(0));
        let alternate =
            alternate_browser_registration_with_capture(
                Arc::clone(&captured_mount),
                Arc::clone(&releases),
            );
        let registry = KernelRegistry::new(
            MaterializationPolicy::stable_with_test_fixtures(CONTRACT_VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .expect("kernel registry");
        let materialized = registry
            .replace_all(vec![first_party, alternate])
            .expect("publish two Browser providers");
        let role_id = ExecutionRoleId::from(BROWSER_EXECUTION_ROLE_ID);
        let contract = materialized
            .role_contract(&role_id)
            .expect("Browser role contract");
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "browser-provider-owner".to_owned(),
        };
        let binding = TypedResourceBinding {
            binding_id: ResourceBindingId::from("browser-provider-binding"),
            resource_kind: ResourceKind::from("browser"),
            resource_id: nomifun_agent_contracts::ResourceId::from("browser-target"),
            owner_id: principal.principal_id.clone(),
            operations: BTreeSet::from([
                "navigate".to_owned(),
                "observe".to_owned(),
            ]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        };
        let revision = |overrides: BTreeMap<ExecutionRoleId, RoleProviderSelection>| {
            let payload = AgentPresetRevisionPayload {
                schema_version: VersionString::from(CONTRACT_VERSION),
                surfaces: BTreeSet::from(["desktop".to_owned()]),
                model_route_refs: BTreeMap::new(),
                chat_route_records: BTreeMap::new(),
                initial_capabilities: vec![
                    CapabilitySelection {
                        capability: CapabilityRef {
                            id: CapabilityId::from("browser.navigate"),
                            version: VersionString::from(CONTRACT_VERSION),
                        },
                        required: true,
                        exposure: CapabilityExposure::Advertised,
                        action_allowlist: BTreeSet::from([ActionId::from(
                            "browser.navigate.invoke",
                        )]),
                        resource_binding_refs: vec![binding.binding_id.clone()],
                        destination_constraints: BTreeSet::new(),
                        context_budget_override: None,
                        tool_budget_override: None,
                        config: empty_object(),
                    },
                    CapabilitySelection {
                        capability: CapabilityRef {
                            id: CapabilityId::from("browser.observe"),
                            version: VersionString::from(CONTRACT_VERSION),
                        },
                        required: true,
                        exposure: CapabilityExposure::Hidden,
                        action_allowlist: BTreeSet::new(),
                        resource_binding_refs: vec![binding.binding_id.clone()],
                        destination_constraints: BTreeSet::new(),
                        context_budget_override: None,
                        tool_budget_override: None,
                        config: empty_object(),
                    },
                    CapabilitySelection {
                        capability: CapabilityRef {
                            id: CapabilityId::from("browser.identity"),
                            version: VersionString::from(CONTRACT_VERSION),
                        },
                        required: true,
                        exposure: CapabilityExposure::Hidden,
                        action_allowlist: BTreeSet::new(),
                        resource_binding_refs: vec![binding.binding_id.clone()],
                        destination_constraints: BTreeSet::new(),
                        context_budget_override: None,
                        tool_budget_override: None,
                        config: empty_object(),
                    },
                ],
                on_demand_capabilities: Vec::new(),
                skill_bindings: Vec::new(),
                resource_bindings: vec![binding.clone()],
                system_role_provider_overrides: overrides,
                persona: "Browser provider fixture".to_owned(),
                instructions: "Navigate with the selected Browser provider.".to_owned(),
                context_policy: empty_object(),
                execution_constraints: empty_object(),
                runtime_budget: empty_object(),
            };
            AgentPresetRevision {
                reference: PresetRevisionRef {
                    preset_id: AgentPresetId::from("browser-provider-fixture"),
                    revision: 1,
                    revision_digest: digest_payload(&payload).expect("revision digest"),
                },
                payload,
                created_by: UserId::from(principal.principal_id.clone()),
                created_at_ms: 1,
                reason: None,
            }
        };
        let environment = CompilerEnvironment {
            resolver_version: VersionString::from(CONTRACT_VERSION),
            required_runtime_protocol_version: VersionString::from(CONTRACT_VERSION),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: DigestHex::from("runtime"),
            available_runtime_features: BTreeSet::new(),
            installation_role_bindings: BTreeMap::new(),
            canonical_schema_manifest_digest: DigestHex::from("schema"),
            target_contribution_manifest_digest: DigestHex::from("target"),
            host_target: RuntimeTarget::from("x86_64-pc-windows-msvc"),
            host_surface: "desktop".to_owned(),
            availability_evidence_revision: "browser-provider-test".to_owned(),
        };
        assert!(matches!(
            AgentPresetCompiler::compile(
                &materialized,
                &environment,
                CompileRequest {
                    revision: revision(BTreeMap::new()),
                    principal: principal.clone(),
                    scene: "test".to_owned(),
                    surface: "desktop".to_owned(),
                    audience: "test".to_owned(),
                    created_at_ms: 2,
                    resolver_run_id: OperationId::from("browser-provider-no-selection"),
                },
            ),
            Err(KernelError::RoleProviderNotBound { .. })
        ));

        let selected = RoleProviderSelection {
            role: ExactRoleContractRef {
                key: contract.manifest.key.clone(),
                contract_digest: contract.contract_digest.clone(),
            },
            provider_mount_id: PluginMountId::from("fixture-browser-provider"),
        };
        let compiled = AgentPresetCompiler::compile(
            &materialized,
            &environment,
            CompileRequest {
                revision: revision(BTreeMap::from([(role_id.clone(), selected)])),
                principal: principal.clone(),
                scene: "test".to_owned(),
                surface: "desktop".to_owned(),
                audience: "test".to_owned(),
                created_at_ms: 2,
                resolver_run_id: OperationId::from("browser-provider-selected"),
            },
        )
        .expect("compile selected alternate Browser provider");
        assert_eq!(
            compiled
                .role_provider(&role_id)
                .expect("frozen Browser provider")
                .provider
                .mount_id
                .as_ref(),
            "fixture-browser-provider"
        );
        let active = SessionCapabilityState::new(&compiled)
            .snapshot()
            .expect("initial active set");
        poll_ready(registry.invoke(
            &compiled,
            &active,
            CapabilityInvocationRequest {
                principal: principal.clone(),
                session_owner: principal.clone(),
                agent_session_id: AgentSessionId::from("browser-provider-session"),
                operation_id: OperationId::from("browser-provider-invoke"),
                idempotency_key: IdempotencyKey::from("browser-provider-invoke"),
                correlation_id: CorrelationId::from("browser-provider-invoke"),
                resolved_snapshot_ref: compiled.snapshot_ref().clone(),
                active_set_generation: active.generation,
                capability_id: CapabilityId::from("browser.navigate"),
                action_id: ActionId::from("browser.navigate.invoke"),
                resource_binding_ids: BTreeSet::from([binding.binding_id.clone()]),
                state_scope_key: ScopeKey::from("session:browser-provider"),
                input: empty_object(),
            },
        ))
        .expect("alternate Browser invocation");
        assert_eq!(
            captured_mount
                .lock()
                .expect("alternate provider capture")
                .as_deref(),
            Some("fixture-browser-provider")
        );
        let role_member_request = |capability_id: &str| RoleMemberInvocationRequest {
            principal: principal.clone(),
            session_owner: principal.clone(),
            operation_id: OperationId::from(format!("{capability_id}:operation")),
            correlation_id: CorrelationId::from(format!("{capability_id}:correlation")),
            capability_id: CapabilityId::from(capability_id),
            resource_binding_ids: BTreeSet::from([binding.binding_id.clone()]),
            state_scope_key: ScopeKey::from("session:browser-provider"),
            admission: RoleMemberAdmission::Agent {
                agent_session_id: AgentSessionId::from("browser-provider-session"),
                resolved_snapshot_ref: compiled.snapshot_ref().clone(),
                active_set_generation: active.generation,
            },
        };
        let context = poll_ready(registry.contribute_role_context(
            &compiled,
            &active,
            role_member_request("browser.observe"),
        ))
        .expect("alternate Browser context");
        assert_eq!(
            context.value.expect("context value").0["provider_mount"],
            "fixture-browser-provider"
        );

        let first_handle = poll_ready(registry.acquire_role_resource(
            &compiled,
            &active,
            role_member_request("browser.identity"),
        ))
        .expect("first Browser resource acquisition");
        let replay_handle = poll_ready(registry.acquire_role_resource(
            &compiled,
            &active,
            role_member_request("browser.identity"),
        ))
        .expect("replayed Browser resource acquisition");
        assert!(Arc::ptr_eq(&first_handle.handle, &replay_handle.handle));
        assert_eq!(releases.load(Ordering::Acquire), 1);
        poll_ready(registry.release_role_resources(&ScopeKey::from(
            "session:browser-provider",
        )))
        .expect("release Browser resource");
        assert_eq!(releases.load(Ordering::Acquire), 2);
    }

    #[test]
    fn every_action_capability_maps_to_a_typed_host_operation() {
        assert!(matches!(
            typed_operation_for(&CapabilityId::from("fs.read"), empty_object()),
            Ok(Wave2TypedCapabilityOperation::FsRead { .. })
        ));
        assert!(matches!(
            typed_operation_for(&CapabilityId::from("fs.write"), empty_object()),
            Ok(Wave2TypedCapabilityOperation::FsWrite { .. })
        ));
        assert!(matches!(
            typed_operation_for(&CapabilityId::from("fs.delete"), empty_object()),
            Ok(Wave2TypedCapabilityOperation::FsDelete { .. })
        ));

        for definition in PACKAGE_DEFINITIONS
            .iter()
            .flat_map(|package| package.capabilities.iter())
            .filter(|definition| definition.is_tool())
        {
            let capability_id = CapabilityId::from(definition.id);
            let typed = typed_operation_for(&capability_id, empty_object())
                .expect("action capabilities must have an exact typed operation");
            assert_eq!(typed.capability_id(), definition.id);
            assert!(
                operation_for(&capability_id, empty_object()).is_ok(),
                "{} must have a host operation",
                definition.id
            );
        }
    }

    #[test]
    fn non_action_capabilities_cannot_enter_the_host_dispatch_contract() {
        let error = typed_operation_for(&CapabilityId::from("workspace.bind"), empty_object())
            .expect_err("resource providers must not become action operations");
        assert!(error.to_string().contains("does not expose an action host operation"));
    }

    #[test]
    fn host_request_rejects_wrong_family_and_missing_or_unauthorized_bindings() {
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "owner".to_owned(),
        };
        let binding = nomifun_agent_contracts::TypedResourceBinding {
            binding_id: "workspace-binding".into(),
            resource_kind: "workspace".into(),
            resource_id: "workspace-resource".into(),
            owner_id: "owner".to_owned(),
            operations: BTreeSet::from(["read".to_owned()]),
            connection_config_ref: None,
            typed_parameters: Default::default(),
        };
        let context = Wave2HostContext {
            principal: principal.clone(),
            agent_session_id: AgentSessionId::from("session"),
            operation_id: OperationId::from("operation"),
            idempotency_key: IdempotencyKey::from("idempotency"),
            correlation_id: CorrelationId::from("correlation"),
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: "snapshot".into(),
                snapshot_digest: "digest".into(),
            },
            registry_generation: 7,
            capability_id: CapabilityId::from("fs.read"),
            action_id: ActionId::from("fs.read.invoke"),
            role_provider: None,
            state: test_state_handle(),
            resource_bindings: vec![binding.clone()],
        };
        let wrong_family = match (Wave2HostRequest {
            context: context.clone(),
            operation: Wave2CapabilityOperation::Ssh {
                input: empty_object(),
            },
        })
        .into_typed()
        {
            Ok(_) => panic!("fs.read cannot be routed through the SSH family"),
            Err(error) => error,
        };
        assert_eq!(wrong_family.code, "ACTION_OPERATION_MISMATCH");

        let missing_binding = match (Wave2HostRequest {
            context: Wave2HostContext {
                resource_bindings: Vec::new(),
                ..context.clone()
            },
            operation: Wave2CapabilityOperation::WorkspaceExecution {
                input: empty_object(),
            },
        })
        .into_typed()
        {
            Ok(_) => panic!("owner adapters must not receive an unbound action"),
            Err(error) => error,
        };
        assert_eq!(missing_binding.code, PRESET_RESOURCE_NOT_BOUND);

        let wrong_owner = match (Wave2HostRequest {
            context: Wave2HostContext {
                principal: PrincipalRef {
                    principal_id: "different-owner".to_owned(),
                    ..principal
                },
                resource_bindings: vec![binding.clone()],
                ..context.clone()
            },
            operation: Wave2CapabilityOperation::WorkspaceExecution {
                input: empty_object(),
            },
        })
        .into_typed()
        {
            Ok(_) => panic!("owner adapters must not receive another principal's binding"),
            Err(error) => error,
        };
        assert_eq!(wrong_owner.code, RESOURCE_OWNER_MISMATCH);

        let unexpected_binding = match (Wave2HostRequest {
            context: Wave2HostContext {
                resource_bindings: vec![
                    binding.clone(),
                    nomifun_agent_contracts::TypedResourceBinding {
                        binding_id: "ssh-binding".into(),
                        resource_kind: "ssh_host".into(),
                        resource_id: "ssh-resource".into(),
                        owner_id: "owner".to_owned(),
                        operations: BTreeSet::from(["read".to_owned()]),
                        connection_config_ref: None,
                        typed_parameters: Default::default(),
                    },
                ],
                ..context.clone()
            },
            operation: Wave2CapabilityOperation::WorkspaceExecution {
                input: empty_object(),
            },
        })
         .into_typed()
        {
            Ok(_) => panic!("owner adapters must not receive undeclared resource bindings"),
            Err(error) => error,
        };
        assert_eq!(unexpected_binding.code, PRESET_RESOURCE_NOT_BOUND);
        assert!(unexpected_binding.message.contains("ssh_host"));

        let missing_grant = match (Wave2HostRequest {
            context: Wave2HostContext {
                resource_bindings: vec![nomifun_agent_contracts::TypedResourceBinding {
                    operations: BTreeSet::new(),
                    ..binding
                }],
                ..context
            },
            operation: Wave2CapabilityOperation::WorkspaceExecution {
                input: empty_object(),
            },
        })
        .into_typed()
        {
            Ok(_) => panic!("owner adapters must not receive a binding without read grant"),
            Err(error) => error,
        };
        assert_eq!(missing_grant.code, PRESET_RESOURCE_NOT_BOUND);
    }

    #[test]
    fn composed_typed_adapter_receives_exact_operation_and_authorization_projection() {
        use std::sync::Mutex;
        use std::task::{Context, Poll, Waker};

        let seen = Arc::new(Mutex::new(None));
        let seen_by_adapter = Arc::clone(&seen);
        let adapter = typed_operation_adapter(
            |operation| matches!(operation, Wave2TypedCapabilityOperation::FsRead { .. }),
            move |request| {
                let seen = Arc::clone(&seen_by_adapter);
                std::future::ready({
                    let is_exact = matches!(
                        request.operation,
                        Wave2TypedCapabilityOperation::FsRead { .. }
                    );
                    let authorization = (
                        request.context.principal.principal_id,
                        request.context.registry_generation,
                        request.context.resource_bindings[0].resource_id.clone(),
                    );
                    *seen.lock().expect("test adapter mutex") =
                        Some((is_exact, authorization));
                    Ok(empty_object())
                })
            },
        );
        let dispatcher = Wave2HostPortDispatcher::new(vec![adapter]);
        let request = Wave2HostRequest {
            context: Wave2HostContext {
                principal: PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: "owner".to_owned(),
                },
                agent_session_id: AgentSessionId::from("session"),
                operation_id: OperationId::from("operation"),
                idempotency_key: IdempotencyKey::from("idempotency"),
                correlation_id: CorrelationId::from("correlation"),
                resolved_snapshot_ref: ResolvedSnapshotRef {
                    snapshot_id: "snapshot".into(),
                    snapshot_digest: "digest".into(),
                },
                registry_generation: 11,
                capability_id: CapabilityId::from("fs.read"),
                action_id: ActionId::from("fs.read.invoke"),
                role_provider: None,
                state: test_state_handle(),
                resource_bindings: vec![nomifun_agent_contracts::TypedResourceBinding {
                    binding_id: "binding".into(),
                    resource_kind: "workspace".into(),
                    resource_id: "resource".into(),
                    owner_id: "owner".to_owned(),
                    operations: BTreeSet::from(["read".to_owned()]),
                    connection_config_ref: None,
                    typed_parameters: Default::default(),
                }],
            },
            operation: Wave2CapabilityOperation::WorkspaceExecution {
                input: empty_object(),
            },
        };
        let future = dispatcher.invoke(request);
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = std::pin::pin!(future);
        assert!(matches!(
            future.as_mut().poll(&mut context),
            Poll::Ready(Ok(_))
        ));
        assert_eq!(
            *seen.lock().expect("test adapter mutex"),
            Some((
                true,
                (
                    "owner".to_owned(),
                    11,
                    nomifun_agent_contracts::ResourceId::from("resource"),
                )
            ))
        );

        let empty_dispatcher = Wave2HostPortDispatcher::empty();
        let unavailable_future = empty_dispatcher.invoke(Wave2HostRequest {
                context: Wave2HostContext {
                    capability_id: CapabilityId::from("fs.write"),
                    action_id: ActionId::from("fs.write.invoke"),
                    resource_bindings: vec![nomifun_agent_contracts::TypedResourceBinding {
                        binding_id: "binding".into(),
                        resource_kind: "workspace".into(),
                        resource_id: "resource".into(),
                        owner_id: "owner".to_owned(),
                        operations: BTreeSet::from(["write".to_owned()]),
                        connection_config_ref: None,
                        typed_parameters: Default::default(),
                    }],
                    ..Wave2HostContext {
                        principal: PrincipalRef {
                            principal_kind: "user".to_owned(),
                            principal_id: "owner".to_owned(),
                        },
                        agent_session_id: AgentSessionId::from("session"),
                        operation_id: OperationId::from("operation"),
                        idempotency_key: IdempotencyKey::from("idempotency"),
                        correlation_id: CorrelationId::from("correlation"),
                        resolved_snapshot_ref: ResolvedSnapshotRef {
                            snapshot_id: "snapshot".into(),
                            snapshot_digest: "digest".into(),
                        },
                        registry_generation: 11,
                        capability_id: CapabilityId::from("fs.write"),
                        action_id: ActionId::from("fs.write.invoke"),
                        role_provider: None,
                        state: test_state_handle(),
                        resource_bindings: Vec::new(),
                    }
                },
                operation: Wave2CapabilityOperation::WorkspaceExecution {
                    input: empty_object(),
                },
            });
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut unavailable_future = std::pin::pin!(unavailable_future);
        let unavailable = match unavailable_future.as_mut().poll(&mut context) {
            Poll::Ready(result) => result.expect_err("unsupported owner-backed action must fail closed"),
            Poll::Pending => panic!("unsupported owner-backed action must fail immediately"),
        };
        assert_eq!(unavailable.code, CAPABILITY_UNAVAILABLE);
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
                    role_provider: None,
                    state: test_state_handle(),
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
        assert_eq!(result.code, CAPABILITY_UNAVAILABLE);
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
            assert_eq!(
                capability.manifest.supported_platforms,
                vec![PlatformConstraint::Any]
            );
            assert_eq!(
                capability.manifest.supported_surfaces,
                BTreeSet::from(["desktop".to_owned(), "headless".to_owned()])
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
            assert_eq!(
                capability.manifest.supported_platforms,
                vec![PlatformConstraint::Any]
            );
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

        for (role_id, expected_targets) in [
            (
                BROWSER_EXECUTION_ROLE_ID,
                BROWSER_DESKTOP_HOST_TARGETS,
            ),
            (
                COMPUTER_EXECUTION_ROLE_ID,
                COMPUTER_DESKTOP_HOST_TARGETS,
            ),
        ] {
            let provider = materialized
                .role_provider(
                    &ExecutionRoleId::from(role_id),
                    &PluginMountId::from(if role_id == BROWSER_EXECUTION_ROLE_ID {
                        BROWSER_MOUNT_ID
                    } else {
                        COMPUTER_A11Y_MOUNT_ID
                    }),
                )
                .expect("first-party role provider");
            for member in provider.contribution.members.values() {
                assert!(member.supported_platforms.iter().all(|constraint| {
                    matches!(
                        constraint,
                        PlatformConstraint::Targets {
                            host_targets,
                            host_surfaces,
                        } if host_targets
                            == &expected_targets
                                .iter()
                                .map(|target| RuntimeTarget::from(*target))
                                .collect()
                            && host_surfaces == &BTreeSet::from(["desktop".to_owned()])
                    )
                }));
            }
        }
    }
}
