//! Nomi adapter for ordinary Plugin Tool capabilities.
//!
//! The adapter consumes one exact compiled Agent Snapshot and the shared
//! Kernel registry. It does not resolve the latest Catalog, accept model-owned
//! capability identity, or use the non-Agent operation API.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use nomi_protocol::events::ToolCategory;
use nomi_agent::context_contributor::{ContextContributor, TurnContext};
use nomi_tools::{
    Tool, ToolExecutionContext,
    registry::{DeferredToolState, ToolRegistry},
};
use nomi_types::tool::{JsonSchema, ToolResult};
use nomifun_agent_contracts::{
    ActionId, AgentSessionId, CapabilityActionDescriptor, CapabilityConsumer,
    CapabilityId, CapabilityKind, CapabilityManifest, CanonicalErrorCode,
    CanonicalSchemaRef, ContributionSourceKind,
    CorrelationId, DigestHex, EffectClass, IdempotencyKey, OperationId,
    PluginSourceKind, PrincipalRef, ResolvedCapability,
    ResolvedSnapshotRef, ScopeKey,
    StrictJsonValue, ToolPresentationKind, canonical_json_bytes,
    digest_payload,
};
use nomifun_agent_kernel::{
    CapabilityAccessRequest, CapabilityInvocationRequest, CompiledSnapshot,
    KernelError, KernelRegistry,
    MaterializedCapability, MaterializedRegistry, SessionCapabilityState,
};
use nomifun_common::AppError;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::plugin_tool_error_projection::model_safe_tool_error;

#[path = "plugin_context.rs"]
mod context;

const PROVIDER_NAME_PREFIX: &str = "plugin__";
const PROVIDER_NAME_SEPARATOR: &str = "__";
const PROVIDER_NAME_MAX_BYTES: usize = 64;
const PROVIDER_NAME_HASH_HEX_BYTES: usize = 20;
const MAX_INITIAL_CAPABILITY_CONTEXT_BYTES: usize = 64 * 1024;

/// Capability shapes actually consumed by the Nomi managed-Plugin adapter.
/// Catalog availability uses the same predicate as runtime materialization;
/// declaring a kind in a Package alone does not make it executable by Nomi.
/// Source, exact artifact, runtime readiness and authorization are checked by
/// their existing owners, not granted by this shape check.
pub fn supports_nomi_plugin_capability(manifest: &CapabilityManifest) -> bool {
    manifest.supports_consumer(CapabilityConsumer::Agent)
        && match manifest.kind {
            CapabilityKind::Tool => manifest
                .contributions
                .actions
                .iter()
                .any(|action| action.presentation == ToolPresentationKind::FunctionTool)
                || crate::tool_discovery::supports(manifest),
            CapabilityKind::ContextContributor => {
                manifest.contributions.context_schema_refs.len() == 1
            }
            _ => false,
        }
}

tokio::task_local! {
    static CURRENT_NOMI_PLUGIN_TOOL_SESSION: Option<NomiPluginToolSession>;
}

#[derive(Debug, Error)]
pub enum NomiPluginToolError {
    #[error("Hosted effect outcome is unproven: {0}")]
    OutcomeUnknown(String),
    #[error("Nomi Plugin Tool contract error: {0}")]
    Contract(String),
    #[error("Nomi Plugin Tool schema {reference:?} could not be resolved: {reason}")]
    Schema {
        reference: CanonicalSchemaRef,
        reason: String,
    },
    #[error("Nomi Plugin Tool Kernel invocation failed: {0}")]
    Kernel(#[from] KernelError),
}

/// Host-owned resolver for the JSON Schema named by a canonical action ref.
///
/// Implementations must read the same immutable schema repository that
/// produced the materialized Capability. A Plugin manifest or model input is
/// not a schema authority.
#[async_trait]
pub trait NomiPluginToolSchemaResolver: Send + Sync {
    async fn resolve(
        &self,
        capability: &ResolvedCapability,
        reference: &CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String>;
}

/// Host-owned canonical schema source for explicitly admitted bundled Kernel
/// Tools.
///
/// Admission and schema lookup are intentionally separate. The admission
/// object below locks an exact materialized target before any Session exists;
/// this resolver only returns schema bytes for that already-approved target.
/// Implementations must derive those bytes from the same bundled wave contract
/// that produced the registration. They must never synthesize a permissive
/// fallback schema from a capability ID or a model request.
#[async_trait]
pub trait NomiPlatformBuiltinToolSchemaResolver: Send + Sync {
    async fn resolve(
        &self,
        capability: &ResolvedCapability,
        reference: &CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String>;
}

/// Exact capability-ID router for canonical schemas owned by independent
/// bundled wave modules.
#[derive(Clone, Default)]
pub struct NomiPlatformBuiltinToolSchemaRouter {
    routes: Arc<
        BTreeMap<CapabilityId, Arc<dyn NomiPlatformBuiltinToolSchemaResolver>>,
    >,
}

impl fmt::Debug for NomiPlatformBuiltinToolSchemaRouter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NomiPlatformBuiltinToolSchemaRouter")
            .field("capability_ids", &self.routes.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl NomiPlatformBuiltinToolSchemaRouter {
    pub fn new(
        owners: impl IntoIterator<
            Item = (
                BTreeSet<CapabilityId>,
                Arc<dyn NomiPlatformBuiltinToolSchemaResolver>,
            ),
        >,
    ) -> Result<Self, NomiPluginToolError> {
        let mut routes = BTreeMap::new();
        for (capability_ids, resolver) in owners {
            for capability_id in capability_ids {
                if routes
                    .insert(capability_id.clone(), Arc::clone(&resolver))
                    .is_some()
                {
                    return Err(NomiPluginToolError::Contract(format!(
                        "bundled schema owner for {} is registered more than once",
                        capability_id.as_ref()
                    )));
                }
            }
        }
        Ok(Self {
            routes: Arc::new(routes),
        })
    }

    pub fn capability_ids(&self) -> BTreeSet<CapabilityId> {
        self.routes.keys().cloned().collect()
    }
}

#[async_trait]
impl NomiPlatformBuiltinToolSchemaResolver
    for NomiPlatformBuiltinToolSchemaRouter
{
    async fn resolve(
        &self,
        capability: &ResolvedCapability,
        reference: &CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String> {
        let resolver = self.routes.get(&capability.capability.id).ok_or_else(
            || {
                format!(
                    "no bundled schema owner is admitted for {}",
                    capability.capability.id.as_ref()
                )
            },
        )?;
        resolver.resolve(capability, reference).await
    }
}

/// Exact, composition-time approval for bundled Kernel Tools exposed to one
/// Nomi host.
///
/// Merely being `PluginSourceKind::Bundled` is not executable evidence. The
/// caller must explicitly approve every capability from the materialized
/// registry produced by its real host-backed wave registrations. Construction
/// rejects metadata-only declarative placeholders (they have no capability
/// host port), TestFixture sources, non-Agent/non-Function Tools, and any ID
/// still owned by Nomi's native registry.
#[derive(Clone)]
pub struct NomiPlatformBuiltinToolAdmission {
    targets: Arc<BTreeMap<CapabilityId, MaterializedCapability>>,
    schema_resolver: Arc<dyn NomiPlatformBuiltinToolSchemaResolver>,
    mcp_targets: Arc<BTreeMap<CapabilityId, (MaterializedCapability, StrictJsonValue)>>,
}

impl fmt::Debug for NomiPlatformBuiltinToolAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NomiPlatformBuiltinToolAdmission")
            .field(
                "capability_ids",
                &self.targets.keys().collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl NomiPlatformBuiltinToolAdmission {
    /// Lock an explicit set of host-backed bundled Tool targets.
    ///
    /// `native_capability_ids` is the authoritative set still registered by
    /// Nomi itself. Keeping it explicit makes a native/Kernel double route a
    /// composition error instead of silently exposing two tools for one
    /// capability.
    pub fn from_registry(
        registry: &MaterializedRegistry,
        approved_capability_ids: BTreeSet<CapabilityId>,
        native_capability_ids: BTreeSet<CapabilityId>,
        schema_resolver: Arc<dyn NomiPlatformBuiltinToolSchemaResolver>,
    ) -> Result<Self, NomiPluginToolError> {
        if let Some(duplicate) = approved_capability_ids
            .intersection(&native_capability_ids)
            .next()
        {
            return Err(NomiPluginToolError::Contract(format!(
                "bundled Kernel Tool {} is also owned by Nomi's native registry",
                duplicate.as_ref()
            )));
        }

        let mut targets = BTreeMap::new();
        for capability_id in approved_capability_ids {
            let capability = registry.capability(&capability_id).ok_or_else(|| {
                NomiPluginToolError::Contract(format!(
                    "approved bundled Kernel Tool {} is not materialized",
                    capability_id.as_ref()
                ))
            })?;
            validate_platform_builtin_tool_target(capability)?;
            targets.insert(capability_id, capability.clone());
        }
        Ok(Self {
            targets: Arc::new(targets),
            schema_resolver,
            mcp_targets: Arc::new(BTreeMap::new()),
        })
    }

    /// Supplement this Session's host approval with exact frozen MCP tools.
    /// The product host must first validate each mapping/resource/descriptor;
    /// schemas here are already resolved data, never a remote discovery hook.
    /// This does not add IDs to the PlatformBuiltin approval set.
    pub fn with_mcp_tools(
        mut self,
        registry: &MaterializedRegistry,
        schemas: BTreeMap<CapabilityId, StrictJsonValue>,
    ) -> Result<Self, NomiPluginToolError> {
        if !self.mcp_targets.is_empty() || schemas.len() > 1024 {
            return Err(NomiPluginToolError::Contract("MCP approval is already installed or exceeds its bound".into()));
        }
        let mut targets = BTreeMap::new();
        for (id, schema) in schemas {
            let target = registry.capability(&id).ok_or_else(|| NomiPluginToolError::Contract("MCP target is absent".into()))?;
            let [action] = target.manifest.contributions.actions.as_slice() else {
                return Err(NomiPluginToolError::Contract("MCP target needs one exact action".into()));
            };
            if target.source.source_kind != PluginSourceKind::Bundled
                || target.contribution_lock.source_kind != ContributionSourceKind::McpBinding
                || target.manifest.kind != CapabilityKind::Tool
                || !target.manifest.supports_consumer(CapabilityConsumer::Agent)
                || action.presentation != ToolPresentationKind::FunctionTool
                || registry.mcp_for_capability(&id).is_none()
                || self.targets.contains_key(&id)
            {
                return Err(NomiPluginToolError::Contract("MCP target lacks exact bundled function-tool provenance".into()));
            }
            validate_canonical_input_schema(&action.input_schema, &schema)?;
            targets.insert(id, (target.clone(), schema));
        }
        self.mcp_targets = Arc::new(targets);
        Ok(self)
    }

    pub fn approved_capability_ids(&self) -> BTreeSet<CapabilityId> {
        self.targets.keys().cloned().collect()
    }

    fn target_for(
        &self,
        resolved: &ResolvedCapability,
    ) -> Result<Option<&MaterializedCapability>, NomiPluginToolError> {
        let Some(target) = self.targets.get(&resolved.capability.id) else {
            return Ok(None);
        };
        validate_exact_target(resolved, target)?;
        Ok(Some(target))
    }
}

/// Exact, composition-time approval for bundled ContextContributors.
///
/// Enabled ContextContributors are admitted independently from ordinary Tools
/// and contribute to the system prompt through their exact owner boundary.
#[derive(Clone, Debug)]
pub struct NomiPlatformBuiltinContextAdmission {
    targets: Arc<BTreeMap<CapabilityId, MaterializedCapability>>,
}

impl NomiPlatformBuiltinContextAdmission {
    pub fn from_registry(
        registry: &MaterializedRegistry,
        approved_capability_ids: BTreeSet<CapabilityId>,
        native_capability_ids: BTreeSet<CapabilityId>,
    ) -> Result<Self, NomiPluginToolError> {
        if let Some(duplicate) = approved_capability_ids
            .intersection(&native_capability_ids)
            .next()
        {
            return Err(NomiPluginToolError::Contract(format!(
                "bundled Kernel ContextContributor {} is also owned by Nomi's native context path",
                duplicate.as_ref()
            )));
        }

        let mut targets = BTreeMap::new();
        for capability_id in approved_capability_ids {
            let capability = registry.capability(&capability_id).ok_or_else(|| {
                NomiPluginToolError::Contract(format!(
                    "approved bundled Kernel ContextContributor {} is not materialized",
                    capability_id.as_ref()
                ))
            })?;
            validate_platform_builtin_context_target(registry, capability)?;
            targets.insert(capability_id, capability.clone());
        }
        Ok(Self {
            targets: Arc::new(targets),
        })
    }

    pub fn approved_capability_ids(&self) -> BTreeSet<CapabilityId> {
        self.targets.keys().cloned().collect()
    }

    fn target_for(
        &self,
        resolved: &ResolvedCapability,
    ) -> Result<Option<&MaterializedCapability>, NomiPluginToolError> {
        let Some(target) = self.targets.get(&resolved.capability.id) else {
            return Ok(None);
        };
        validate_exact_target(resolved, target)?;
        Ok(Some(target))
    }
}

fn validate_platform_builtin_context_target(
    registry: &MaterializedRegistry,
    capability: &MaterializedCapability,
) -> Result<(), NomiPluginToolError> {
    let manifest = &capability.manifest;
    if capability.source.source_kind != PluginSourceKind::Bundled
        || capability.contribution_lock.source_kind
            != ContributionSourceKind::PlatformBuiltin
        || capability.contribution_lock.mount_id.is_some()
    {
        return Err(NomiPluginToolError::Contract(format!(
            "approved Kernel ContextContributor {} is not an exact bundled PlatformBuiltin",
            manifest.id.as_ref()
        )));
    }
    if manifest.kind != CapabilityKind::ContextContributor
        || !manifest.supports_consumer(CapabilityConsumer::Agent)
        || manifest.contributions.context_schema_refs.len() != 1
    {
        return Err(NomiPluginToolError::Contract(format!(
            "approved PlatformBuiltin {} is not an Agent ContextContributor with one canonical schema",
            manifest.id.as_ref()
        )));
    }
    let registration = registry
        .plugins
        .get(&capability.mount_id)
        .ok_or_else(|| NomiPluginToolError::Contract(format!(
            "approved PlatformBuiltin {} has no owning registration",
            manifest.id.as_ref()
        )))?;
    if registration.context.host_ports.is_empty() {
        return Err(NomiPluginToolError::Contract(format!(
            "approved PlatformBuiltin {} has no typed host binding; metadata-only context placeholders cannot be exposed to Nomi",
            manifest.id.as_ref()
        )));
    }
    Ok(())
}

fn lifecycle_schema_ref(
    manifest: &CapabilityManifest,
) -> Result<Option<CanonicalSchemaRef>, NomiPluginToolError> {
    let refs = match manifest.kind {
        CapabilityKind::EventSource => &manifest.contributions.event_schema_refs,
        CapabilityKind::TurnMiddleware => {
            &manifest.contributions.context_schema_refs
        }
        CapabilityKind::Transport
        | CapabilityKind::ResourceProvider
        | CapabilityKind::BackgroundService => return Ok(None),
        _ => {
            return Err(NomiPluginToolError::Contract(format!(
                "{} is not a supported Nomi lifecycle capability",
                manifest.id.as_ref()
            )));
        }
    };
    let [schema_ref] = refs.as_slice() else {
        return Err(NomiPluginToolError::Contract(format!(
            "lifecycle capability {} must declare exactly one canonical schema for its kind",
            manifest.id.as_ref()
        )));
    };
    Ok(Some(schema_ref.clone()))
}

/// Exact host invocation for a bundled non-Tool Session lifecycle
/// contribution.
#[derive(Clone, Debug)]
pub struct NomiPlatformBuiltinLifecycleInvocation {
    pub principal: PrincipalRef,
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub correlation_id: CorrelationId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub registry_generation: u64,
    pub registry_digest: DigestHex,
    pub capability: ResolvedCapability,
    pub state_scope_key: ScopeKey,
    pub resource_bindings: Vec<nomifun_agent_contracts::TypedResourceBinding>,
    pub schema_ref: Option<CanonicalSchemaRef>,
    pub turn_input: StrictJsonValue,
}

#[async_trait]
pub trait NomiPlatformBuiltinLifecycleInvoker: Send + Sync {
    async fn activate(
        &self,
        request: NomiPlatformBuiltinLifecycleInvocation,
    ) -> Result<StrictJsonValue, String>;

    async fn context_contributor(
        &self,
        _request: NomiPlatformBuiltinLifecycleInvocation,
    ) -> Result<Option<Arc<dyn ContextContributor>>, String> {
        Ok(None)
    }
}

/// Exact admission for bundled Event/Transport/TurnMiddleware lifecycle
/// owners. Registration metadata alone is insufficient; every admitted ID is
/// locked to the current materialized target and one host-owned invoker.
#[derive(Clone)]
pub struct NomiPlatformBuiltinLifecycleAdmission {
    targets: Arc<BTreeMap<CapabilityId, MaterializedCapability>>,
    invoker: Arc<dyn NomiPlatformBuiltinLifecycleInvoker>,
}

impl fmt::Debug for NomiPlatformBuiltinLifecycleAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NomiPlatformBuiltinLifecycleAdmission")
            .field("capability_ids", &self.targets.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl NomiPlatformBuiltinLifecycleAdmission {
    pub fn from_registry(
        registry: &MaterializedRegistry,
        approved_capability_ids: BTreeSet<CapabilityId>,
        native_capability_ids: BTreeSet<CapabilityId>,
        invoker: Arc<dyn NomiPlatformBuiltinLifecycleInvoker>,
    ) -> Result<Self, NomiPluginToolError> {
        if let Some(duplicate) = approved_capability_ids
            .intersection(&native_capability_ids)
            .next()
        {
            return Err(NomiPluginToolError::Contract(format!(
                "bundled lifecycle capability {} is also owned by Nomi's native lifecycle",
                duplicate.as_ref()
            )));
        }
        let mut targets = BTreeMap::new();
        for capability_id in approved_capability_ids {
            let capability = registry.capability(&capability_id).ok_or_else(|| {
                NomiPluginToolError::Contract(format!(
                    "approved bundled lifecycle capability {} is not materialized",
                    capability_id.as_ref()
                ))
            })?;
            validate_platform_builtin_lifecycle_target(registry, capability)?;
            targets.insert(capability_id, capability.clone());
        }
        Ok(Self {
            targets: Arc::new(targets),
            invoker,
        })
    }

    pub fn approved_capability_ids(&self) -> BTreeSet<CapabilityId> {
        self.targets.keys().cloned().collect()
    }

    fn target_for(
        &self,
        resolved: &ResolvedCapability,
    ) -> Result<Option<&MaterializedCapability>, NomiPluginToolError> {
        let Some(target) = self.targets.get(&resolved.capability.id) else {
            return Ok(None);
        };
        validate_exact_target(resolved, target)?;
        Ok(Some(target))
    }
}

fn validate_platform_builtin_lifecycle_target(
    registry: &MaterializedRegistry,
    capability: &MaterializedCapability,
) -> Result<(), NomiPluginToolError> {
    let manifest = &capability.manifest;
    if capability.source.source_kind != PluginSourceKind::Bundled
        || capability.contribution_lock.source_kind
            != ContributionSourceKind::PlatformBuiltin
        || capability.contribution_lock.mount_id.is_some()
    {
        return Err(NomiPluginToolError::Contract(format!(
            "approved lifecycle capability {} is not an exact bundled PlatformBuiltin",
            manifest.id.as_ref()
        )));
    }
    if !manifest.supports_consumer(CapabilityConsumer::Agent)
        || !matches!(
            manifest.kind,
            CapabilityKind::EventSource
                | CapabilityKind::Transport
                | CapabilityKind::TurnMiddleware
                | CapabilityKind::ResourceProvider
                | CapabilityKind::BackgroundService
        )
    {
        return Err(NomiPluginToolError::Contract(format!(
            "approved PlatformBuiltin {} is not an Agent lifecycle capability",
            manifest.id.as_ref()
        )));
    }
    let registration = registry
        .plugins
        .get(&capability.mount_id)
        .ok_or_else(|| NomiPluginToolError::Contract(format!(
            "approved lifecycle capability {} has no owning registration",
            manifest.id.as_ref()
        )))?;
    if registration.context.host_ports.is_empty() {
        return Err(NomiPluginToolError::Contract(format!(
            "approved lifecycle capability {} has no typed host binding",
            manifest.id.as_ref()
        )));
    }
    Ok(())
}

fn validate_platform_builtin_tool_target(
    capability: &MaterializedCapability,
) -> Result<(), NomiPluginToolError> {
    let manifest = &capability.manifest;
    if capability.source.source_kind != PluginSourceKind::Bundled
        || capability.contribution_lock.source_kind
            != ContributionSourceKind::PlatformBuiltin
        || capability.contribution_lock.mount_id.is_some()
    {
        return Err(NomiPluginToolError::Contract(format!(
            "approved Kernel Tool {} is not an exact bundled PlatformBuiltin",
            manifest.id.as_ref()
        )));
    }
    if manifest.kind != CapabilityKind::Tool
        || !manifest.supports_consumer(CapabilityConsumer::Agent)
        || !manifest
            .contributions
            .actions
            .iter()
            .any(|action| {
                action.presentation == ToolPresentationKind::FunctionTool
            })
    {
        return Err(NomiPluginToolError::Contract(format!(
            "approved PlatformBuiltin {} is not an Agent FunctionTool",
            manifest.id.as_ref()
        )));
    }
    if manifest.contributions.host_ports.is_empty() {
        return Err(NomiPluginToolError::Contract(format!(
            "approved PlatformBuiltin {} has no typed capability host port; metadata-only placeholders cannot be exposed to Nomi",
            manifest.id.as_ref()
        )));
    }
    Ok(())
}

/// Host-owned resolver for schemas exported by a Plugin Product Active Release.
///
/// The owner is passed separately because Plugin Product release storage is
/// owner-scoped. The resolver must verify the exact release/catalog facts in
/// the supplied snapshot projection before returning schema bytes.
#[async_trait]
pub trait NomiPluginProductToolSchemaResolver: Send + Sync {
    async fn resolve(
        &self,
        owner: &PrincipalRef,
        capability: &ResolvedCapability,
        reference: &CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String>;
}

/// Authoritative request used by the app-owned session provider.
///
/// The provider receives only first-class owner/session identity and must load
/// the persisted Binding, Snapshot and schemas itself. No client-supplied
/// Mount, Artifact, action, or schema field is accepted here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NomiPluginToolSessionRequest {
    pub owner_id: String,
    pub conversation_id: String,
}

/// Late composition seam for Nomi-core's Conversation-backed AgentSession.
///
/// A production implementation resolves the exact persisted Nomi session
/// binding and delegates materialization to [`KernelNomiPluginToolSession`].
#[async_trait]
pub trait NomiPluginToolSessionProvider: Send + Sync {
    /// Read-only cold discovery. Implementations must not delegate this to
    /// resolve(): session materialization can execute Context and acquire resources.
    async fn discover_skill_commands(
        &self,
        _request: NomiPluginToolSessionRequest,
    ) -> Result<Vec<nomifun_api_types::SlashCommandItem>, AppError> {
        Ok(Vec::new())
    }

    async fn resolve(
        &self,
        request: NomiPluginToolSessionRequest,
    ) -> Result<Option<NomiPluginToolSession>, AppError>;
}

pub(crate) async fn with_nomi_plugin_tool_session<F>(
    session: Option<NomiPluginToolSession>,
    future: F,
) -> F::Output
where
    F: Future,
{
    CURRENT_NOMI_PLUGIN_TOOL_SESSION
        .scope(session, future)
        .await
}

pub(crate) fn current_nomi_plugin_tool_session(
) -> Option<NomiPluginToolSession> {
    CURRENT_NOMI_PLUGIN_TOOL_SESSION
        .try_with(Clone::clone)
        .ok()
        .flatten()
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct NomiPluginToolActionIdentity {
    resolved_snapshot_ref: ResolvedSnapshotRef,
    resolved_capability: ResolvedCapability,
    action: CapabilityActionDescriptor,
    input_schema_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct NomiPluginProductToolActionIdentity {
    resolved_snapshot_ref: ResolvedSnapshotRef,
    resolved_capability: ResolvedCapability,
    action: CapabilityActionDescriptor,
    input_schema_digest: DigestHex,
}

/// One provider-visible action derived from an exact Plugin capability.
#[derive(Clone, Debug, PartialEq)]
pub struct NomiPluginToolAction {
    provider_name: String,
    activation_identity: String,
    artifact_identity: String,
    description: String,
    input_schema: StrictJsonValue,
    identity: NomiPluginToolActionIdentity,
}

impl NomiPluginToolAction {
    pub fn provider_name(&self) -> &str {
        &self.provider_name
    }

    pub fn activation_identity(&self) -> &str {
        &self.activation_identity
    }

    /// Semantic identity used only for artifact-output classification.
    /// Unlike `activation_identity`, this excludes package provenance fields
    /// such as `artifact_digest` that would make every Plugin Tool look like
    /// an artifact producer.
    pub fn artifact_identity(&self) -> &str {
        &self.artifact_identity
    }

    pub fn capability_id(&self) -> &CapabilityId {
        &self.identity.resolved_capability.capability.id
    }

    pub fn action_id(&self) -> &ActionId {
        &self.identity.action.action_id
    }

    pub fn input_schema(&self) -> &StrictJsonValue {
        &self.input_schema
    }

}

/// One provider-visible action derived from an exact Plugin Product Active
/// Release capability.
#[derive(Clone, Debug, PartialEq)]
pub struct NomiPluginProductToolAction {
    provider_name: String,
    activation_identity: String,
    artifact_identity: String,
    description: String,
    input_schema: StrictJsonValue,
    identity: NomiPluginProductToolActionIdentity,
}

impl NomiPluginProductToolAction {
    pub fn provider_name(&self) -> &str {
        &self.provider_name
    }

    pub fn activation_identity(&self) -> &str {
        &self.activation_identity
    }

    pub fn artifact_identity(&self) -> &str {
        &self.artifact_identity
    }

    pub fn capability_id(&self) -> &CapabilityId {
        &self.identity.resolved_capability.capability.id
    }

    pub fn action_id(&self) -> &ActionId {
        &self.identity.action.action_id
    }

    pub fn input_schema(&self) -> &StrictJsonValue {
        &self.input_schema
    }

}

/// Invocation identity created by the Nomi engine, never by model arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct NomiPluginToolInvocation {
    identity: NomiPluginToolActionIdentity,
    operation_id: OperationId,
    idempotency_key: IdempotencyKey,
    correlation_id: CorrelationId,
    input: StrictJsonValue,
}

#[async_trait]
pub trait NomiPluginToolInvoker: Send + Sync {
    async fn preflight(&self, _request: NomiPluginToolInvocation) -> Result<(), NomiPluginToolError> {
        Err(NomiPluginToolError::Contract("Plugin Tool does not provide read-only hook admission".into()))
    }
    async fn invoke(
        &self,
        request: NomiPluginToolInvocation,
    ) -> Result<StrictJsonValue, NomiPluginToolError>;
}

/// One invocation's cooperative cancellation signal. The retained owner task
/// outlives its waiter and continues recording the actual outcome.
#[derive(Clone, Debug, Default)]
pub struct NomiPluginProductCallCancellation(Arc<std::sync::atomic::AtomicBool>);

impl PartialEq for NomiPluginProductCallCancellation {
    fn eq(&self, other: &Self) -> bool { Arc::ptr_eq(&self.0, &other.0) }
}
impl NomiPluginProductCallCancellation {
    pub fn cancel(&self) { self.0.store(true, std::sync::atomic::Ordering::Release); }
    pub fn is_canceled(&self) -> bool { self.0.load(std::sync::atomic::Ordering::Acquire) }
    pub fn shared_flag(&self) -> Arc<std::sync::atomic::AtomicBool> { self.0.clone() }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NomiPluginProductToolInvocation {
    cancellation: NomiPluginProductCallCancellation,
    identity: NomiPluginProductToolActionIdentity,
    operation_id: OperationId,
    idempotency_key: IdempotencyKey,
    correlation_id: CorrelationId,
    input: StrictJsonValue,
}

impl NomiPluginProductToolInvocation {
    pub fn cancellation(&self) -> &NomiPluginProductCallCancellation { &self.cancellation }

    pub fn capability(&self) -> &ResolvedCapability {
        &self.identity.resolved_capability
    }

    pub fn action(&self) -> &CapabilityActionDescriptor {
        &self.identity.action
    }

    pub fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    pub fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }

    pub fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    pub fn input(&self) -> &StrictJsonValue {
        &self.input
    }
}

#[async_trait]
pub trait NomiPluginProductToolInvoker: Send + Sync {
    async fn preflight(&self, _request: NomiPluginProductToolInvocation) -> Result<(), NomiPluginToolError> {
        Err(NomiPluginToolError::Contract("Product Tool does not provide read-only hook admission".into()))
    }
    async fn invoke(
        &self,
        request: NomiPluginProductToolInvocation,
    ) -> Result<StrictJsonValue, NomiPluginToolError>;
}

#[derive(Clone, Debug)]
pub struct NomiHostDynamicToolDescriptor {
    pub capability_id: CapabilityId,
    pub provider_name: String,
    pub description: String,
    pub input_schema: StrictJsonValue,
    pub effect_class: EffectClass,
    pub deferred: bool,
}

#[derive(Clone, Debug)]
pub struct NomiHostDynamicToolInvocation {
    pub capability_id: CapabilityId,
    pub provider_name: String,
    pub operation_id: OperationId,
    pub idempotency_key: IdempotencyKey,
    pub correlation_id: CorrelationId,
    pub arguments: StrictJsonValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NomiHostDynamicToolError {
    pub code: CanonicalErrorCode,
    pub internal_message: String,
    pub retry_safe: bool,
}

impl NomiHostDynamicToolError {
    pub fn new(
        code: impl Into<CanonicalErrorCode>,
        message: impl Into<String>,
        retry_safe: bool,
    ) -> Self {
        Self {
            code: code.into(),
            internal_message: message.into(),
            retry_safe,
        }
    }
}

fn model_safe_dynamic_tool_error(error: &NomiHostDynamicToolError) -> ToolResult {
    let (code, message, retry_safe) = match error.code.as_ref() {
        "HOSTED_EFFECT_UNPROVEN" => (
            "HOSTED_EFFECT_UNPROVEN",
            "The hosted effect outcome is unproven. This Session is fenced; do not retry or infer that the effect was undone.",
            false,
        ),
        "INVALID_PAYLOAD" => (
            "INVALID_PAYLOAD",
            "The device tool arguments are invalid.",
            false,
        ),
        "PRESET_RESOURCE_NOT_BOUND" => (
            "PRESET_RESOURCE_NOT_BOUND",
            "The required device resource is not bound.",
            false,
        ),
        "RESOURCE_OWNER_MISMATCH" => (
            "RESOURCE_OWNER_MISMATCH",
            "The bound device resource is no longer authorized.",
            false,
        ),
        "ROBOT_NOT_FOUND" => (
            "ROBOT_NOT_FOUND",
            "The bound robot is no longer available.",
            false,
        ),
        "ROBOT_NOT_PAIRED" => (
            "ROBOT_NOT_PAIRED",
            "The bound robot is no longer paired.",
            false,
        ),
        "ROBOT_OFFLINE" => (
            "ROBOT_OFFLINE",
            "The bound robot is offline.",
            error.retry_safe,
        ),
        "ROBOT_DEVICE_REJECTED" => (
            "ROBOT_DEVICE_REJECTED",
            "The robot rejected the device command.",
            false,
        ),
        "ROBOT_EFFECT_FAILED" => (
            "ROBOT_EFFECT_FAILED",
            "The robot command failed.",
            false,
        ),
        "ROBOT_EFFECT_OUTCOME_UNKNOWN" => (
            "ROBOT_EFFECT_OUTCOME_UNKNOWN",
            "The robot command outcome is unknown; do not retry automatically.",
            false,
        ),
        "ROBOT_EFFECT_RECEIPT_FAILED" => (
            "ROBOT_EFFECT_RECEIPT_FAILED",
            "The robot command receipt is unavailable; do not retry automatically.",
            false,
        ),
        _ => (
            "CAPABILITY_EXECUTION_FAILED",
            "The host capability request failed.",
            false,
        ),
    };
    ToolResult::error(
        serde_json::to_string(&serde_json::json!({
            "code": code,
            "message": message,
            "retry_safe": retry_safe,
        }))
        .unwrap_or_else(|_| {
            "{\"code\":\"CAPABILITY_EXECUTION_FAILED\",\"message\":\"The host capability request failed.\",\"retry_safe\":false}"
                .to_owned()
        }),
    )
}

#[async_trait]
pub trait NomiHostDynamicToolInvoker: Send + Sync {
    async fn invoke(
        &self,
        request: NomiHostDynamicToolInvocation,
    ) -> Result<StrictJsonValue, NomiHostDynamicToolError>;
}

#[derive(Clone, Debug)]
struct NomiHostDynamicToolAction {
    descriptor: NomiHostDynamicToolDescriptor,
    activation_identity: String,
}

/// One structured ContextContributor result assembled from the exact initial
/// capability set of a frozen Nomi Session.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NomiInitialContextContribution {
    capability_id: CapabilityId,
    value: StrictJsonValue,
}

impl NomiInitialContextContribution {
    pub fn capability_id(&self) -> &CapabilityId {
        &self.capability_id
    }

    pub fn value(&self) -> &StrictJsonValue {
        &self.value
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct NomiLifecycleIdentity {
    resolved_snapshot_ref: ResolvedSnapshotRef,
    resolved_capability: ResolvedCapability,
    schema_ref: Option<CanonicalSchemaRef>,
}

#[path = "model_middleware.rs"]
pub mod model_middleware;
#[path = "tool_middleware.rs"]
pub mod tool_middleware;

/// Host-owned inputs collected before installing any execution consumer.
/// Product tools, hidden hooks and dynamic tools receive one finalized scope.
/// This is assembly data, not another registry or executor.
pub struct NomiHostedSessionBindings {
    pub effect_scope: Arc<crate::engine_effect_scope::EngineEffectScope>,
    pub product: Option<(Vec<NomiPluginProductToolAction>, Arc<dyn NomiPluginProductToolInvoker>)>,
    pub dynamic: Option<(Vec<NomiHostDynamicToolDescriptor>, Arc<dyn NomiHostDynamicToolInvoker>)>,
    pub context: Vec<Arc<dyn ContextContributor>>,
    pub session_control: Option<Arc<dyn crate::SessionControlSink>>,
    pub mcp_resources: Option<crate::nomi_resources::NomiMcpResources>,
}

impl NomiHostedSessionBindings {
    pub fn new(effect_scope: Arc<crate::engine_effect_scope::EngineEffectScope>) -> Self {
        Self { effect_scope, product: None, dynamic: None, context: Vec::new(),
            session_control: None, mcp_resources: None }
    }
}

/// A complete set of Plugin action tools for one frozen Nomi session.
#[derive(Clone)]
pub struct NomiPluginToolSession {
    #[cfg(feature = "browser-use")]
    local_search_binding: Option<crate::local_web_search::LocalSearchBinding>,
    #[cfg(feature = "browser-use")]
    system_browser_binding: Option<nomifun_browser_platform::system_browser::SystemBrowserBinding>,
    browser_provider: Option<nomifun_agent_contracts::ExactRoleProviderRef>,
    execution_constraints: nomifun_api_types::ExecutionConstraints,
    selected_skills: Option<crate::nomi_skills::NomiSelectedSkills>,
    mcp_resources: Option<crate::nomi_resources::NomiMcpResources>,
    effect_scope: Option<Arc<crate::engine_effect_scope::EngineEffectScope>>,
    discovery_policy: Option<crate::tool_discovery::DiscoveryBinding>,
    model_middleware: Vec<model_middleware::Binding>,
    tool_middleware: Vec<tool_middleware::Binding>,
    resolved_snapshot_ref: ResolvedSnapshotRef,
    /// Exact server-compiled resource bindings for this frozen Session.
    /// Runtime factories may inspect these bindings to lazily connect
    /// host-owned resources, but callers cannot replace or augment them.
    target_resource_bindings:
        Arc<[nomifun_agent_contracts::TypedResourceBinding]>,
    actions: Arc<[NomiPluginToolAction]>,
    invoker: Arc<dyn NomiPluginToolInvoker>,
    plugin_product_actions: Arc<[NomiPluginProductToolAction]>,
    plugin_product_invoker: Option<Arc<dyn NomiPluginProductToolInvoker>>,
    initial_context_contributions: Arc<[NomiInitialContextContribution]>,
    host_dynamic_actions: Arc<[NomiHostDynamicToolAction]>,
    host_dynamic_invoker: Option<Arc<dyn NomiHostDynamicToolInvoker>>,
    capability_state: Option<Arc<SessionCapabilityState>>,
    pub(crate) host_skills: Arc<[Arc<nomi_agent::host_skills::HostSkill>]>,
    /// Host-owned, Session-scoped dynamic context sources. These are attached
    /// only after the provider has resolved the exact persisted Session; a
    /// Plugin manifest or model payload cannot construct one.
    context_contributors: Arc<[Arc<dyn ContextContributor>]>,
    /// Native current-session control owner supplied by the same host lookup
    /// that authenticated and materialized this exact AgentSession.
    session_control_sink: Option<Arc<dyn crate::SessionControlSink>>,
}

impl fmt::Debug for NomiPluginToolSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NomiPluginToolSession")
            .field("resolved_snapshot_ref", &self.resolved_snapshot_ref)
            .field("actions", &self.actions)
            .field("plugin_product_actions", &self.plugin_product_actions)
            .field(
                "initial_context_contributions",
                &self.initial_context_contributions,
            )
            .field("host_dynamic_tool_count", &self.host_dynamic_actions.len())
            .field("has_capability_state", &self.capability_state.is_some())
            .field("context_contributor_count", &self.context_contributors.len())
            .field(
                "has_session_control_sink",
                &self.session_control_sink.is_some(),
            )
            .finish_non_exhaustive()
    }
}

impl NomiPluginToolSession {
    #[cfg(feature = "browser-use")]
    pub fn local_search_binding(&self)->Option<&crate::local_web_search::LocalSearchBinding> {self.local_search_binding.as_ref()}
    #[cfg(feature = "browser-use")]
    pub fn system_browser_binding(&self) -> Option<&nomifun_browser_platform::system_browser::SystemBrowserBinding> { self.system_browser_binding.as_ref() }
    pub fn browser_provider(&self) -> Result<Option<&nomifun_agent_contracts::ExactRoleProviderRef>, NomiPluginToolError> {
        if let Some(state) = &self.capability_state {
            let snapshot = state.snapshot()?;
            if snapshot.active.iter().any(|id| matches!(id.as_ref(),
                "browser.observe" | "browser.navigate" | "browser.act"
                | "browser.render_content" | "browser.download"
                | "browser.upload" | "browser.evaluate"
            ))
                && self.browser_provider.is_none()
            {
                return Err(NomiPluginToolError::Contract(
                    "Browser capability requires an exact Browser Role Provider in the verified Snapshot".into(),
                ));
            }
        }
        Ok(self.browser_provider.as_ref())
    }
    pub(crate) fn media_creation_catalog_tools(&self) -> Vec<(String, String)> {
        self.actions.iter().filter(|action| is_builtin_creation(&action.identity)
            && action.capability_id().as_ref() != "creation.text")
            .map(|action| (action.capability_id().as_ref().to_owned(), action.provider_name.clone())).collect()
    }

    pub(crate) fn media_creation_provider_names(&self) -> std::collections::HashSet<String> {
        self.actions.iter().filter(|action| is_builtin_creation(&action.identity)
            && action.capability_id().as_ref() != "creation.text")
            .map(|action| action.provider_name.clone()).collect()
    }

    pub(crate) fn with_creation_receipt_sink(mut self, sink: Arc<crate::capability::backend_output_sink::BackendOutputSink>, conversation_id: String) -> Self {
        self.invoker = Arc::new(NomiCreationReceiptInvoker { delegate: self.invoker.clone(), sink, conversation_id });
        self
    }

    pub fn execution_constraints(&self) -> nomifun_api_types::ExecutionConstraints {
        self.execution_constraints
    }

    /// Called after every dynamic extension, before registry installation.
    /// Native aliases are intersected; exact host-materialized action names
    /// are admitted only if their canonical capability passed materialization.
    pub fn constrain_tool_policy(&self, allowed: &mut Vec<String>, deferred: &mut Vec<String>) {
        let ceiling = self.execution_constraints;
        let allowed_name = |name: &str| {
            if ceiling.exclude_delegation && name == crate::subagent_gateway::gateway_delegate_provider_name() {
                return false;
            }
            ceiling.allows_nomi_tool(name)
                || (ceiling.restricted() && (self.actions.iter().any(|action| action.provider_name == name)
                    || (name == crate::nomi_skills::RESOURCE_TOOL && self.selected_skills.is_some())))
        };
        allowed.retain(|name| allowed_name(name));
        deferred.retain(|name| allowed.contains(name));
    }

    pub(crate) fn has_frozen_mcp_tools(&self) -> bool {
        self.actions.iter().any(|action| action.identity.resolved_capability.contribution_lock.source_kind == ContributionSourceKind::McpBinding)
    }

    pub(crate) fn has_hosted_mcp_resources(&self) -> bool { self.mcp_resources.is_some() }

    fn with_mcp_resources(mut self, resources: crate::nomi_resources::NomiMcpResources) -> Result<Self, NomiPluginToolError> {
        let bindings = self.target_resource_bindings.iter()
            .filter(|binding| binding.resource_kind.as_ref() == "mcp_server")
            .collect::<Vec<_>>();
        let unique_bindings = bindings.iter().map(|binding| &binding.binding_id).collect::<BTreeSet<_>>();
        let unique_servers = bindings.iter().map(|binding| &binding.resource_id).collect::<BTreeSet<_>>();
        if self.mcp_resources.is_some()
            || self.execution_constraints.restricted()
            || bindings.is_empty()
            || unique_bindings.len() != bindings.len()
            || unique_servers.len() != bindings.len()
            || bindings.iter().any(|binding| {
                binding.resource_id.as_ref().is_empty()
                    || !binding.operations.contains("connect")
                    || !binding.operations.contains("read")
                    || binding.connection_config_ref.is_none()
                    || !binding.typed_parameters.is_empty()
            })
        {
            return Err(NomiPluginToolError::Contract("MCP resource adapter requires exact frozen server bindings and an unrestricted Session".into()));
        }
        self.mcp_resources = Some(resources);
        Ok(self)
    }
    pub(crate) fn new(
        resolved_snapshot_ref: ResolvedSnapshotRef,
        mut actions: Vec<NomiPluginToolAction>,
        invoker: Arc<dyn NomiPluginToolInvoker>,
    ) -> Result<Self, NomiPluginToolError> {
        actions.sort_by(|left, right| {
            (
                left.capability_id(),
                left.action_id(),
                left.provider_name(),
            )
                .cmp(&(
                    right.capability_id(),
                    right.action_id(),
                    right.provider_name(),
                ))
        });
        let mut names = BTreeSet::new();
        let mut identities = BTreeSet::new();
        for action in &actions {
            if !names.insert(action.provider_name.clone()) {
                return Err(NomiPluginToolError::Contract(format!(
                    "duplicate provider tool name {}",
                    action.provider_name
                )));
            }
            if !identities.insert(action.activation_identity.clone()) {
                return Err(NomiPluginToolError::Contract(format!(
                    "duplicate activation identity for {}/{}",
                    action.capability_id().as_ref(),
                    action.action_id().as_ref()
                )));
            }
        }
        Ok(Self {
            resolved_snapshot_ref,
            #[cfg(feature = "browser-use")]
            local_search_binding: None,
            #[cfg(feature = "browser-use")]
            system_browser_binding: None,
            browser_provider: None,
            execution_constraints: Default::default(),
            effect_scope: None,
            discovery_policy: None,
            model_middleware: Vec::new(),
            tool_middleware: Vec::new(),
            target_resource_bindings: Arc::from(
                Vec::<nomifun_agent_contracts::TypedResourceBinding>::new(),
            ),
            actions: Arc::from(actions),
            invoker,
            plugin_product_actions: Arc::from(Vec::<NomiPluginProductToolAction>::new()),
            plugin_product_invoker: None,
            initial_context_contributions: Arc::from(
                Vec::<NomiInitialContextContribution>::new(),
            ),
            selected_skills: None,
            mcp_resources: None,
            host_dynamic_actions: Arc::from(Vec::<NomiHostDynamicToolAction>::new()),
            host_dynamic_invoker: None,
            capability_state: None,
            host_skills: Arc::from(Vec::new()),
            context_contributors: Arc::from(
                Vec::<Arc<dyn ContextContributor>>::new(),
            ),
            session_control_sink: None,
        })
    }

    /// Install host execution bindings once, after collecting all dependencies.
    /// No consumer is published with an unscoped invoker and later rebound.
    pub fn bind_hosted_execution(
        mut self,
        bindings: NomiHostedSessionBindings,
    ) -> Result<Self, NomiPluginToolError> {
        self = self.install_effect_scope(bindings.effect_scope)?;
        if let Some((actions, invoker)) = bindings.product {
            self = self.install_plugin_product_actions(actions, invoker)?;
        }
        if let Some((actions, invoker)) = bindings.dynamic {
            self = self.install_host_dynamic_tools(actions, invoker)?;
        }
        for contributor in bindings.context {
            self = self.with_context_contributor(contributor)?;
        }
        if let Some(resources) = bindings.mcp_resources {
            self = self.with_mcp_resources(resources)?;
        }
        if let Some(sink) = bindings.session_control {
            self = self.with_session_control_sink(sink);
        }
        self.model_middleware()?;
        self.tool_middleware()?;
        if matches!(self.discovery_policy, Some(crate::tool_discovery::DiscoveryBinding::Product(_))) {
            return Err(NomiPluginToolError::Contract("Selected Product discovery policy is missing its exact action adapter".into()));
        }
        Ok(self)
    }

    fn install_effect_scope(
        mut self,
        scope: Arc<crate::engine_effect_scope::EngineEffectScope>,
    ) -> Result<Self, NomiPluginToolError> {
        if self.effect_scope.is_some() {
            return Err(NomiPluginToolError::Contract("effect scope already installed".into()));
        }
        if self.context_contributors.len() >= 64 {
            return Err(NomiPluginToolError::Contract("too many host context contributors".into()));
        }
        // Check before lifecycle/context contributors and every model round,
        // including when a cancelled call has already produced a late receipt.
        let mut contributors: Vec<Arc<dyn ContextContributor>> = vec![Arc::new(NomiEffectContextFence {
            scope: scope.clone(),
        })];
        contributors.extend(self.context_contributors.iter().cloned());
        self.context_contributors = Arc::from(contributors);
        self.invoker = Arc::new(OwnedNomiPluginToolInvoker {
            delegate: self.invoker.clone(), scope: scope.clone(),
        });
        self.effect_scope = Some(scope);
        Ok(self)
    }

    pub fn effect_scope(&self) -> Option<Arc<crate::engine_effect_scope::EngineEffectScope>> {
        self.effect_scope.clone()
    }

    pub fn context_contributors(
        &self,
    ) -> &[Arc<dyn ContextContributor>] {
        &self.context_contributors
    }

    pub fn with_selected_skills(mut self, skills: crate::nomi_skills::NomiSelectedSkills) -> Result<Self, NomiPluginToolError> {
        if self.selected_skills.is_some() {
            return Err(NomiPluginToolError::Contract("selected Skills already installed".into()));
        }
        self.selected_skills = Some(skills);
        Ok(self)
    }

    pub(crate) fn with_context_image_policy(mut self, supports_image: bool) -> Result<Self, nomifun_common::AppError> {
        // The host has already intersected exact model support with the frozen
        // enabled llm.vision selection. There is no runtime activation grant.
        if let Some(resources) = &self.mcp_resources { resources.bind_image_policy(supports_image)?; }
        if let Some(skills) = &mut self.selected_skills { skills.image_policy(supports_image); }
        Ok(self)
    }

    /// Append a mandatory host context source without replacing lifecycle or
    /// Robot contributors already materialized for this exact Session.
    fn with_context_contributor(mut self, contributor: Arc<dyn ContextContributor>) -> Result<Self, NomiPluginToolError> {
        if self.context_contributors.len() >= 64 {
            return Err(NomiPluginToolError::Contract("too many host context contributors".into()));
        }
        let mut contributors = self.context_contributors.to_vec();
        contributors.push(contributor);
        self.context_contributors = Arc::from(contributors);
        Ok(self)
    }
    pub fn model_middleware(&self) -> Result<Vec<Arc<dyn nomi_agent::model_middleware::ModelRequestMiddleware>>, NomiPluginToolError> {
        self.model_middleware.iter().map(model_middleware::Binding::consumer).collect()
    }
    pub fn tool_middleware(&self) -> Result<Vec<Arc<dyn nomi_agent::tool_middleware::ToolCallMiddleware>>, NomiPluginToolError> {
        self.tool_middleware.iter().map(tool_middleware::Binding::consumer).collect()
    }

    /// Attach the native control owner for this exact host-authenticated
    /// AgentSession. It is intentionally not accepted by Plugin manifests or
    /// model input.
    fn with_session_control_sink(
        mut self,
        sink: Arc<dyn crate::SessionControlSink>,
    ) -> Self {
        self.session_control_sink = Some(sink);
        self
    }

    pub fn session_control_sink(
        &self,
    ) -> Option<Arc<dyn crate::SessionControlSink>> {
        self.session_control_sink.clone()
    }

    /// Add exact Plugin Product Active Release actions to this same Nomi Tool
    /// session. Plugin and Plugin Product actions share one registry/policy
    /// surface, while their invokers remain separate execution adapters.
    fn install_plugin_product_actions(
        mut self,
        mut actions: Vec<NomiPluginProductToolAction>,
        invoker: Arc<dyn NomiPluginProductToolInvoker>,
    ) -> Result<Self, NomiPluginToolError> {
        if self.plugin_product_invoker.is_some() {
            return Err(NomiPluginToolError::Contract("Plugin Product adapter already installed".into()));
        }
        if self.execution_constraints.restricted() && !actions.is_empty() {
            return Err(NomiPluginToolError::Contract("Plugin Product tools exceed the Session execution ceiling".into()));
        }
        // Bind every consumer to the same retained invoker, including Hidden
        // consumers assembled below, not just the model-visible tool list.
        let scope = self.effect_scope.clone().ok_or_else(|| NomiPluginToolError::Contract("host execution scope is missing".into()))?;
        let invoker: Arc<dyn NomiPluginProductToolInvoker> =
            Arc::new(OwnedNomiPluginProductToolInvoker { delegate: invoker, scope });
        let middleware = actions.iter().filter(|a| a.identity.action == model_middleware::action()).collect::<Vec<_>>();
        model_middleware::bind(&mut self.model_middleware, &middleware, &self.resolved_snapshot_ref, invoker.clone())?;
        let tool_checks = actions.iter().filter(|a| a.identity.action == tool_middleware::before_action()).collect::<Vec<_>>();
        tool_middleware::bind(&mut self.tool_middleware, &tool_checks, &self.resolved_snapshot_ref, invoker.clone())?;
        let hidden = actions.iter().filter(|action| action.identity.action.presentation == ToolPresentationKind::Hidden
            && action.identity.action != model_middleware::action()
            && action.identity.action != tool_middleware::before_action()).collect::<Vec<_>>();
        match (&self.discovery_policy, hidden.as_slice()) {
            (Some(crate::tool_discovery::DiscoveryBinding::Product(expected)), [action])
                if &action.identity.resolved_capability == expected
                    && action.identity.resolved_snapshot_ref == self.resolved_snapshot_ref
                    && action.identity.action == crate::tool_discovery::action() => {
                self.discovery_policy = Some(crate::tool_discovery::DiscoveryBinding::Ready(
                    expected.capability.id.clone(), Arc::new(NomiPluginProductDiscoveryPolicy {
                        action: (*action).clone(), invoker: invoker.clone(),
                    }),
                ));
            }
            (Some(crate::tool_discovery::DiscoveryBinding::Product(_)), _) => return Err(NomiPluginToolError::Contract(
                "Selected Product discovery policy is missing its exact action adapter".into(),
            )),
            (_, []) => {}
            _ => return Err(NomiPluginToolError::Contract("Unexpected or conflicting Product discovery action".into())),
        }
        actions.retain(|action| action.identity.action.presentation == ToolPresentationKind::FunctionTool);
        actions.sort_by(|left, right| {
            (
                left.capability_id(),
                left.action_id(),
                left.provider_name(),
            )
                .cmp(&(
                    right.capability_id(),
                    right.action_id(),
                    right.provider_name(),
                ))
        });
        let mut names = self
            .actions
            .iter()
            .map(|action| action.provider_name.clone())
            .collect::<BTreeSet<_>>();
        let mut identities = self
            .actions
            .iter()
            .map(|action| action.activation_identity.clone())
            .collect::<BTreeSet<_>>();
        for action in &actions {
            if !names.insert(action.provider_name.clone()) {
                return Err(NomiPluginToolError::Contract(format!(
                    "duplicate provider tool name {}",
                    action.provider_name
                )));
            }
            if !identities.insert(action.activation_identity.clone()) {
                return Err(NomiPluginToolError::Contract(format!(
                    "duplicate activation identity for {}/{}",
                    action.capability_id().as_ref(),
                    action.action_id().as_ref()
                )));
            }
        }
        self.plugin_product_actions = Arc::from(actions);
        self.plugin_product_invoker = Some(invoker);
        Ok(self)
    }

    pub fn resolved_snapshot_ref(&self) -> &ResolvedSnapshotRef {
        &self.resolved_snapshot_ref
    }

    pub fn target_resource_bindings(
        &self,
    ) -> &[nomifun_agent_contracts::TypedResourceBinding] {
        &self.target_resource_bindings
    }

    pub fn actions(&self) -> &[NomiPluginToolAction] {
        &self.actions
    }

    pub fn plugin_product_actions(&self) -> &[NomiPluginProductToolAction] {
        &self.plugin_product_actions
    }

    pub fn initial_context_contributions(
        &self,
    ) -> &[NomiInitialContextContribution] {
        &self.initial_context_contributions
    }

    pub fn capability_state(&self) -> Option<Arc<SessionCapabilityState>> {
        self.capability_state.clone()
    }

    fn install_host_dynamic_tools(
        mut self,
        mut descriptors: Vec<NomiHostDynamicToolDescriptor>,
        invoker: Arc<dyn NomiHostDynamicToolInvoker>,
    ) -> Result<Self, NomiPluginToolError> {
        if self.host_dynamic_invoker.is_some() {
            return Err(NomiPluginToolError::Contract("dynamic Tool adapter already installed".into()));
        }
        descriptors.sort_by(|left, right| {
            (&left.capability_id, &left.provider_name)
                .cmp(&(&right.capability_id, &right.provider_name))
        });
        let mut names = self
            .actions
            .iter()
            .map(|action| action.provider_name.clone())
            .chain(self.plugin_product_actions.iter().map(|action| action.provider_name.clone()))
            .collect::<BTreeSet<_>>();
        let mut actions = Vec::with_capacity(descriptors.len());
        for descriptor in descriptors {
            if self.execution_constraints.restricted()
                || !self.execution_constraints.allows_capability(descriptor.capability_id.as_ref())
            {
                return Err(NomiPluginToolError::Contract("Dynamic tool exceeds the Session execution ceiling".into()));
            }
            if descriptor.provider_name.trim().is_empty()
                || !descriptor.input_schema.0.is_object()
                || !names.insert(descriptor.provider_name.clone())
            {
                return Err(NomiPluginToolError::Contract(
                    "host dynamic Tool descriptor is invalid or duplicates a Session route"
                        .to_owned(),
                ));
            }
            let identity = canonical_json_bytes(&serde_json::json!({
                "snapshot": self.resolved_snapshot_ref.clone(),
                "capability_id": descriptor.capability_id.clone(),
                "provider_name": descriptor.provider_name.clone(),
            }))
            .map_err(|error| NomiPluginToolError::Contract(error.to_string()))?;
            actions.push(NomiHostDynamicToolAction {
                descriptor,
                activation_identity: String::from_utf8(identity).map_err(|error| {
                    NomiPluginToolError::Contract(error.to_string())
                })?,
            });
        }
        self.host_dynamic_actions = Arc::from(actions);
        let scope = self.effect_scope.clone().ok_or_else(|| NomiPluginToolError::Contract("host execution scope is missing".into()))?;
        self.host_dynamic_invoker = Some(Arc::new(OwnedNomiDynamicToolInvoker { delegate: invoker, scope }));
        Ok(self)
    }

    /// Append the frozen initial capability context to Nomi's system prompt as
    /// canonical structured data.
    ///
    /// The section is absent when no approved ContextContributor returned a
    /// value. It is assembled once while the runtime is built from the exact
    /// Snapshot. Every enabled context contribution is projected here.
    pub fn system_prompt_with_initial_context(
        &self,
        base: Option<&str>,
    ) -> Result<Option<String>, NomiPluginToolError> {
        if self.initial_context_contributions.is_empty() && self.selected_skills.is_none() {
            return Ok(base.map(str::to_owned));
        }
        let bytes = canonical_json_bytes(
            &self.initial_context_contributions.to_vec(),
        )
        .map_err(|error| {
            NomiPluginToolError::Contract(format!(
                "initial capability context could not be encoded: {error}"
            ))
        })?;
        if bytes.len().saturating_add(self.selected_skills.as_ref().map_or(0, |skills| skills.prompt().len())) > MAX_INITIAL_CAPABILITY_CONTEXT_BYTES {
            return Err(NomiPluginToolError::Contract(format!(
                "initial capability and Skill context exceeds the {MAX_INITIAL_CAPABILITY_CONTEXT_BYTES}-byte Nomi prompt limit"
            )));
        }
        let context = String::from_utf8(bytes).map_err(|error| {
            NomiPluginToolError::Contract(format!(
                "initial capability context is not UTF-8: {error}"
            ))
        })?;
        let mut prompt = base.unwrap_or_default().to_owned();
        if !prompt.is_empty() {
            prompt.push_str("\n\n");
        }
        prompt.push_str(
            "<nomifun_initial_capability_context format=\"canonical-json\">\n",
        );
        prompt.push_str(&context);
        prompt.push_str("\n</nomifun_initial_capability_context>");
        if let Some(skills) = &self.selected_skills {
            prompt.push_str("\n\n");
            prompt.push_str(skills.prompt());
        }
        Ok(Some(prompt))
    }

    pub fn tool_count(&self) -> usize {
        self.actions.len()
            + self.plugin_product_actions.len()
            + self.host_dynamic_actions.len()
            + usize::from(self.selected_skills.as_ref().is_some_and(|skills| skills.has_resources()))
            + if self.mcp_resources.is_some() { crate::nomi_resources::NAMES.len() } else { 0 }
    }

    pub fn provider_names_for(
        &self,
        capability_id: &str,
        deferred: bool,
    ) -> Vec<String> {
        if self.discovery_policy.as_ref().is_some_and(|binding| binding.id().as_ref() == capability_id) {
            return vec!["ToolSearch".into()];
        }
        self.actions
            .iter()
            .filter(|action| {
                action.capability_id().as_ref() == capability_id
            })
            .map(|action| action.provider_name.clone())
            .chain(
                self.plugin_product_actions
                    .iter()
                    .filter(|action| {
                        action.capability_id().as_ref() == capability_id
                            })
                    .map(|action| action.provider_name.clone()),
            )
            .chain(
                self.host_dynamic_actions
                    .iter()
                    .filter(|action| {
                        action.descriptor.capability_id.as_ref() == capability_id
                            && action.descriptor.deferred == deferred
                    })
                    .map(|action| action.descriptor.provider_name.clone()),
            )
            .collect()
    }

    /// Merge the exact dynamic routes into Nomi's persistent registration
    /// policy before the bootstrap builds its registry.
    pub fn extend_tool_policy(
        &self,
        allowed_tools: &mut Vec<String>,
        deferred_tools: &mut Vec<String>,
    ) {
        if let Some(resources) = &self.mcp_resources {
            for name in crate::nomi_resources::NAMES {
                push_unique(allowed_tools, name);
                if resources.deferred { push_unique(deferred_tools, name); }
            }
        }
        if self.selected_skills.as_ref().is_some_and(|skills| skills.has_resources()) {
            push_unique(allowed_tools, crate::nomi_skills::RESOURCE_TOOL);
        }
        if self.discovery_policy.is_some() {
            push_unique(allowed_tools, "ToolSearch");
            deferred_tools.retain(|name| name != "ToolSearch");
        }
        if !self.host_skills.is_empty() {
            push_unique(allowed_tools, "Skill");
            deferred_tools.retain(|name| name != "Skill");
        }
        for action in self.actions.iter() {
            push_unique(allowed_tools, &action.provider_name);
        }
        for action in self.plugin_product_actions.iter() {
            push_unique(allowed_tools, &action.provider_name);
        }
        for action in self.host_dynamic_actions.iter() {
            push_unique(allowed_tools, &action.descriptor.provider_name);
            if action.descriptor.deferred {
                push_unique(deferred_tools, &action.descriptor.provider_name);
            }
        }
        if !deferred_tools.is_empty() {
            push_unique(allowed_tools, "ToolSearch");
        }
    }

    /// Register all actions atomically against the already-installed Nomi
    /// allowlist and deferred state.
    pub fn register_into(
        &self,
        registry: &mut ToolRegistry,
    ) -> Result<(), NomiPluginToolError> {
        // A selected Product policy cannot silently become native discovery
        // when a caller forgets the second existing source-adapter phase.
        let discovery_policy = self.discovery_policy.as_ref().map(|binding| binding.policy()).transpose()?;
        self.model_middleware()?;
        self.tool_middleware()?;
        if self.actions.is_empty()
            && self.plugin_product_actions.is_empty()
            && self.host_dynamic_actions.is_empty()
            && !self.selected_skills.as_ref().is_some_and(|skills| skills.has_resources())
            && self.mcp_resources.is_none()
        {
            if let Some(policy) = &discovery_policy {
                registry.install_discovery_policy(policy.clone())
                    .map_err(|message| NomiPluginToolError::Contract(message.into()))?;
            }
            return Ok(());
        }
        let deferred_state = registry.deferred_state();
        let mut tools: Vec<Box<dyn Tool>> = self
            .actions
            .iter()
            .cloned()
            .map(|action| {
                Box::new(NomiPluginTool {
                    action,
                    invoker: Arc::clone(&self.invoker),
                }) as Box<dyn Tool>
            })
            .collect();
        if let Some(invoker) = &self.plugin_product_invoker {
            tools.extend(self.plugin_product_actions.iter().cloned().map(|action| {
                Box::new(NomiPluginProductTool {
                    action,
                    invoker: Arc::clone(invoker),
                }) as Box<dyn Tool>
            }));
        } else if !self.plugin_product_actions.is_empty() {
            return Err(NomiPluginToolError::Contract(
                "Plugin Product actions are present without an execution adapter".to_owned(),
            ));
        }
        if let Some(invoker) = &self.host_dynamic_invoker {
            tools.extend(self.host_dynamic_actions.iter().cloned().map(|action| {
                Box::new(NomiHostDynamicTool {
                    action,
                    invoker: Arc::clone(invoker),
                    deferred_state: deferred_state.clone(),
                }) as Box<dyn Tool>
            }));
        } else if !self.host_dynamic_actions.is_empty() {
            return Err(NomiPluginToolError::Contract(
                "host dynamic Tools are present without an execution adapter".to_owned(),
            ));
        }
        if let Some(skills) = &self.selected_skills {
            if skills.has_resources() { tools.push(Box::new(skills.clone())); }
        }
        if let Some(resources) = &self.mcp_resources {
            let scope = self.effect_scope.clone().ok_or_else(|| NomiPluginToolError::Contract("MCP resources require a retained effect scope".into()))?;
            tools.extend(resources.tools(deferred_state.clone(), scope));
        }
        let inserted = registry.register_batch(tools);
        let mut expected = self
            .actions
            .iter()
            .map(|action| action.provider_name.clone())
            .chain(
                self.plugin_product_actions
                    .iter()
                    .map(|action| action.provider_name.clone()),
            )
            .chain(
                self.host_dynamic_actions
                    .iter()
                    .map(|action| action.descriptor.provider_name.clone()),
            )
            .collect::<BTreeSet<_>>();
        let inserted = inserted.into_iter().collect::<BTreeSet<_>>();
        if self.selected_skills.as_ref().is_some_and(|skills| skills.has_resources()) {
            expected.insert(crate::nomi_skills::RESOURCE_TOOL.into());
        }
        if self.mcp_resources.is_some() { expected.extend(crate::nomi_resources::NAMES.map(str::to_owned)); }
        if inserted != expected {
            return Err(NomiPluginToolError::Contract(
                "Nomi registry rejected one or more exact hosted Tool routes"
                    .to_owned(),
            ));
        }
        if let Some(policy) = &discovery_policy {
            registry.install_discovery_policy(policy.clone())
                .map_err(|message| NomiPluginToolError::Contract(message.into()))?;
        }
        Ok(())
    }
}

/// Builder for a Kernel-backed Nomi Plugin Tool session.
pub struct KernelNomiPluginToolSession;

#[derive(Clone, Copy)]
enum NomiKernelToolSchemaSource {
    ManagedPlugin,
    PlatformBuiltin,
    FrozenMcp,
}

impl KernelNomiPluginToolSession {
    #[allow(clippy::too_many_arguments)]
    pub async fn materialize(
        kernel: Arc<KernelRegistry>,
        compiled: Arc<CompiledSnapshot>,
        owner: PrincipalRef,
        agent_session_id: AgentSessionId,
        state_scope_key: ScopeKey,
        schema_resolver: Arc<dyn NomiPluginToolSchemaResolver>,
    ) -> Result<NomiPluginToolSession, NomiPluginToolError> {
        Self::materialize_internal(
            kernel,
            compiled,
            owner,
            agent_session_id,
            state_scope_key,
            schema_resolver,
            None,
            None,
            None,
            Default::default(),
        )
        .await
    }

    /// Materialize ManagedLocal Plugin Tools plus the exact bundled
    /// PlatformBuiltin Tool set approved by the Nomi host composition.
    ///
    /// This is the only Bundled admission path. Callers must build the
    /// admission from their already-materialized real wave registrations and
    /// supply the IDs still owned by Nomi's native registry. Unapproved
    /// builtins remain absent rather than falling back to a declarative or
    /// native route.
    #[allow(clippy::too_many_arguments)]
    pub async fn materialize_with_platform_builtins(
        kernel: Arc<KernelRegistry>,
        compiled: Arc<CompiledSnapshot>,
        owner: PrincipalRef,
        agent_session_id: AgentSessionId,
        state_scope_key: ScopeKey,
        plugin_schema_resolver: Arc<dyn NomiPluginToolSchemaResolver>,
        platform_builtin_admission: Arc<NomiPlatformBuiltinToolAdmission>,
    ) -> Result<NomiPluginToolSession, NomiPluginToolError> {
        Self::materialize_internal(
            kernel,
            compiled,
            owner,
            agent_session_id,
            state_scope_key,
            plugin_schema_resolver,
            Some(platform_builtin_admission),
            None,
            None,
            Default::default(),
        )
        .await
    }

    /// Materialize the exact bundled Tool set and project approved bundled
    /// ContextContributors through the same compiled Snapshot and active
    /// capability set.
    ///
    /// Keeping both projections in one Session object prevents catalog,
    /// context, and Tool authority from being resolved along separate paths.
    /// Enabled ContextContributors are assembled into the system prompt.
    #[allow(clippy::too_many_arguments)]
    pub async fn materialize_with_platform_builtins_and_context(
        kernel: Arc<KernelRegistry>,
        compiled: Arc<CompiledSnapshot>,
        owner: PrincipalRef,
        agent_session_id: AgentSessionId,
        state_scope_key: ScopeKey,
        plugin_schema_resolver: Arc<dyn NomiPluginToolSchemaResolver>,
        platform_builtin_tool_admission: Arc<
            NomiPlatformBuiltinToolAdmission,
        >,
        platform_builtin_context_admission: Arc<
            NomiPlatformBuiltinContextAdmission,
        >,
    ) -> Result<NomiPluginToolSession, NomiPluginToolError> {
        Self::materialize_internal(
            kernel,
            compiled,
            owner,
            agent_session_id,
            state_scope_key,
            plugin_schema_resolver,
            Some(platform_builtin_tool_admission),
            Some(platform_builtin_context_admission),
            None,
            Default::default(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn materialize_with_platform_builtins_context_and_lifecycle(
        kernel: Arc<KernelRegistry>,
        compiled: Arc<CompiledSnapshot>,
        owner: PrincipalRef,
        agent_session_id: AgentSessionId,
        state_scope_key: ScopeKey,
        plugin_schema_resolver: Arc<dyn NomiPluginToolSchemaResolver>,
        platform_builtin_tool_admission: Arc<
            NomiPlatformBuiltinToolAdmission,
        >,
        platform_builtin_context_admission: Arc<
            NomiPlatformBuiltinContextAdmission,
        >,
        platform_builtin_lifecycle_admission: Arc<
            NomiPlatformBuiltinLifecycleAdmission,
        >,
    ) -> Result<NomiPluginToolSession, NomiPluginToolError> {
        Self::materialize_for_execution(
            kernel, compiled, owner, agent_session_id, state_scope_key,
            plugin_schema_resolver, platform_builtin_tool_admission,
            platform_builtin_context_admission, platform_builtin_lifecycle_admission,
            Default::default(),
        ).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn materialize_for_execution(
        kernel: Arc<KernelRegistry>,
        compiled: Arc<CompiledSnapshot>,
        owner: PrincipalRef,
        agent_session_id: AgentSessionId,
        state_scope_key: ScopeKey,
        plugin_schema_resolver: Arc<dyn NomiPluginToolSchemaResolver>,
        platform_builtin_tool_admission: Arc<NomiPlatformBuiltinToolAdmission>,
        platform_builtin_context_admission: Arc<NomiPlatformBuiltinContextAdmission>,
        platform_builtin_lifecycle_admission: Arc<NomiPlatformBuiltinLifecycleAdmission>,
        constraints: nomifun_api_types::ExecutionConstraints,
    ) -> Result<NomiPluginToolSession, NomiPluginToolError> {
        Self::materialize_internal(
            kernel,
            compiled,
            owner,
            agent_session_id,
            state_scope_key,
            plugin_schema_resolver,
            Some(platform_builtin_tool_admission),
            (!constraints.restricted()).then_some(platform_builtin_context_admission),
            (!constraints.restricted()).then_some(platform_builtin_lifecycle_admission),
            constraints,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn materialize_internal(
        kernel: Arc<KernelRegistry>,
        compiled: Arc<CompiledSnapshot>,
        owner: PrincipalRef,
        agent_session_id: AgentSessionId,
        state_scope_key: ScopeKey,
        plugin_schema_resolver: Arc<dyn NomiPluginToolSchemaResolver>,
        platform_builtin_admission: Option<
            Arc<NomiPlatformBuiltinToolAdmission>,
        >,
        platform_builtin_context_admission: Option<
            Arc<NomiPlatformBuiltinContextAdmission>,
        >,
        platform_builtin_lifecycle_admission: Option<
            Arc<NomiPlatformBuiltinLifecycleAdmission>,
        >,
        constraints: nomifun_api_types::ExecutionConstraints,
    ) -> Result<NomiPluginToolSession, NomiPluginToolError> {
        validate_session_identity(
            &compiled,
            &owner,
            &agent_session_id,
            &state_scope_key,
        )?;
        if compiled.content().enabled_capabilities.iter().any(|capability| {
            nomifun_mcp::is_retired_mcp_authoring_capability(
                capability.capability.id.as_ref(),
            )
        }) {
            return Err(NomiPluginToolError::Contract(
                "retired broad MCP capabilities cannot enter a Runtime Session".into(),
            ));
        }
        let registry = kernel.snapshot()?;

        let active = Arc::new(SessionCapabilityState::new(&compiled));
        let discovery_policy = if constraints.restricted() { None } else {
            crate::tool_discovery::KernelDiscoveryPolicy::materialize(
            kernel.clone(), compiled.clone(), active.clone(), owner.clone(), agent_session_id.clone(), state_scope_key.clone(),
            )?
        };
        let active_snapshot = active.snapshot()?;
        let (mut initial_context_contributions, turn_context_ids) = if constraints.restricted() {
            (Vec::new(), Vec::new())
        } else {
            assemble_initial_capability_context(
                &kernel,
                &compiled,
                &active_snapshot,
                registry.as_ref(),
                &owner,
                &agent_session_id,
                &state_scope_key,
                platform_builtin_context_admission.as_deref(),
            )
            .await?
        };
        if let Some(admission) =
            platform_builtin_lifecycle_admission.as_ref()
        {
            initial_context_contributions.extend(
                assemble_initial_platform_builtin_lifecycle(
                    &compiled,
                    registry.as_ref(),
                    &owner,
                    &agent_session_id,
                    &state_scope_key,
                    admission,
                )
                .await?,
            );
        }
        let middleware_identities = match
            platform_builtin_lifecycle_admission.as_ref()
        {
            Some(admission) => turn_middleware_identities(
                &compiled,
                registry.as_ref(),
                admission,
            )?,
            None => Vec::new(),
        };
        let middleware_context_contributor = match (
            middleware_identities.is_empty(),
            platform_builtin_lifecycle_admission.as_ref(),
        ) {
            (false, Some(admission)) => Some(
                Arc::new(NomiLifecycleContextContributor {
                    compiled: Arc::clone(&compiled),
                    active: Arc::clone(&active),
                    owner: owner.clone(),
                    agent_session_id: agent_session_id.clone(),
                    state_scope_key: state_scope_key.clone(),
                    admission: Arc::clone(admission),
                    identities: Arc::from(middleware_identities),
                    turn_sequence: AtomicU64::new(0),
                }) as Arc<dyn ContextContributor>,
            ),
            _ => None,
        };
        let mut lifecycle_context_contributors = match
            platform_builtin_lifecycle_admission.as_ref()
        {
            Some(admission) => lifecycle_context_contributors(
                &compiled,
                registry.as_ref(),
                &owner,
                &agent_session_id,
                &state_scope_key,
                admission,
            )
            .await?,
            None => Vec::new(),
        };
        if let Some(contributor) = middleware_context_contributor {
            lifecycle_context_contributors.push(contributor);
        }
        if !turn_context_ids.is_empty() {
            lifecycle_context_contributors.push(Arc::new(context::NomiTurnContextContributor::new(
                Arc::clone(&kernel), Arc::clone(&compiled), Arc::clone(&active),
                owner.clone(), agent_session_id.clone(), state_scope_key.clone(), turn_context_ids,
            )));
        }

        let mut pending = Vec::new();
        for resolved in compiled
            .content()
            .contributions()
        {
            if !constraints.allows_capability(resolved.capability.id.as_ref())
                || (constraints.restricted()
                    && (resolved.contribution_lock.source_kind != ContributionSourceKind::PlatformBuiltin
                        || resolved.resolved_source.source_kind != PluginSourceKind::Bundled
                        || !matches!(resolved.capability.id.as_ref(), "fs.read" | "fs.search" | "process.exec")))
            { continue; }
            let schema_source = match resolved.contribution_lock.source_kind {
                ContributionSourceKind::PluginMount => {
                    NomiKernelToolSchemaSource::ManagedPlugin
                }
                ContributionSourceKind::PlatformBuiltin
                    if resolved.resolved_source.source_kind
                        == PluginSourceKind::Bundled =>
                {
                    let Some(admission) = platform_builtin_admission.as_ref()
                    else {
                        continue;
                    };
                    if admission.target_for(resolved)?.is_none() {
                        continue;
                    }
                    NomiKernelToolSchemaSource::PlatformBuiltin
                }
                ContributionSourceKind::McpBinding => {
                    let (target, _) = platform_builtin_admission.as_ref()
                        .and_then(|admission| admission.mcp_targets.get(&resolved.capability.id))
                        .ok_or_else(|| NomiPluginToolError::Contract("selected MCP tool has no exact host admission".into()))?;
                    validate_exact_target(resolved, target)?;
                    NomiKernelToolSchemaSource::FrozenMcp
                }
                _ => continue,
            };
            let current = registry
                .capability(&resolved.capability.id)
                .ok_or_else(|| KernelError::CapabilityNotMaterialized {
                    capability_id: resolved.capability.id.clone(),
                    version: resolved.capability.version.clone(),
                })?;
            validate_exact_target(resolved, current)?;
            let manifest = &current.manifest;
            if manifest.kind != CapabilityKind::Tool
                || !manifest.supports_consumer(CapabilityConsumer::Agent)
            {
                continue;
            }
            let policy = compiled.policy(&manifest.id).ok_or_else(|| {
                NomiPluginToolError::Contract(format!(
                    "compiled Snapshot has no authority policy for {}",
                    manifest.id.as_ref()
                ))
            })?;
            for action in &manifest.contributions.actions {
                if !policy.allowed_actions.contains(&action.action_id)
                    || action.presentation != ToolPresentationKind::FunctionTool
                {
                    continue;
                }
                pending.push((
                    resolved.clone(),
                    action.clone(),
                    manifest.display.name.clone(),
                    manifest.display.description.clone(),
                    schema_source,
                ));
            }
        }

        let actions = futures_util::future::try_join_all(
            pending.into_iter().map(
                |(
                    resolved,
                    action,
                    display_name,
                    description,
                    schema_source,
                )| {
                    let plugin_schema_resolver =
                        Arc::clone(&plugin_schema_resolver);
                    let platform_builtin_admission =
                        platform_builtin_admission.clone();
                    let snapshot_ref = compiled.snapshot_ref().clone();
                    async move {
                        let input_schema = match schema_source {
                            NomiKernelToolSchemaSource::FrozenMcp => {
                                platform_builtin_admission.as_ref()
                                    .and_then(|admission| admission.mcp_targets.get(&resolved.capability.id))
                                    .map(|(_, schema)| schema.clone())
                                    .ok_or_else(|| "frozen MCP schema is absent".to_owned())
                            }
                            NomiKernelToolSchemaSource::ManagedPlugin => {
                                plugin_schema_resolver
                                    .resolve(
                                        &resolved,
                                        &action.input_schema,
                                    )
                                    .await
                            }
                            NomiKernelToolSchemaSource::PlatformBuiltin => {
                                platform_builtin_admission
                                    .as_ref()
                                    .expect(
                                        "PlatformBuiltin pending entries require admission",
                                    )
                                    .schema_resolver
                                    .resolve(
                                        &resolved,
                                        &action.input_schema,
                                    )
                                    .await
                            }
                        }
                        .map_err(|reason| NomiPluginToolError::Schema {
                            reference: action.input_schema.clone(),
                            reason,
                        })?;
                        build_action(
                            snapshot_ref,
                            resolved,
                            action,
                            display_name,
                            description,
                            input_schema,
                                )
                    }
                },
            ),
        )
        .await?;

        let identities = actions
            .iter()
            .map(|action| {
                (
                    (
                        action.capability_id().clone(),
                        action.action_id().clone(),
                    ),
                    action.identity.clone(),
                )
            })
            .collect();
        let creation_turn = Arc::new(NomiCreationTurnContext::default());
        if actions.iter().any(|action| is_builtin_creation(&action.identity)) {
            lifecycle_context_contributors.push(creation_turn.clone());
        }
        let invoker = Arc::new(KernelNomiPluginToolInvoker {
            creation_turn,
            kernel: Arc::clone(&kernel),
            compiled: Arc::clone(&compiled),
            active: Arc::clone(&active),
            owner: owner.clone(),
            agent_session_id: agent_session_id.clone(),
            state_scope_key: state_scope_key.clone(),
            identities,
        });
        let mut session = NomiPluginToolSession::new(
            compiled.snapshot_ref().clone(),
            actions,
            invoker,
        )?;
        session.execution_constraints = constraints;
        session.initial_context_contributions =
            Arc::from(initial_context_contributions);
        session.capability_state = Some(active);
        session.browser_provider = compiled.content().resolved_role_providers
            .get(&nomifun_agent_contracts::ExecutionRoleId::from("system.browser_use"))
            .map(|lock| lock.provider.clone());
        #[cfg(feature = "browser-use")]
        if let Some(resolved)=compiled.content().enabled_capabilities.iter().find(|capability|capability.capability.id.as_ref()==crate::local_web_search::TOOL_NAME) {
            let live=registry.capability(&resolved.capability.id).ok_or_else(||NomiPluginToolError::Contract("Local search capability disappeared".into()))?;
            if resolved.schema_digest!=live.schema_digest || resolved.contribution_lock!=live.contribution_lock || resolved.target_artifact_digest!=live.target_artifact_digest {
                return Err(NomiPluginToolError::Contract("Local search binding differs from the frozen Snapshot".into()));
            }
            session.local_search_binding=crate::local_web_search::binding_from_manifest(&live.manifest)
                .map_err(|error|NomiPluginToolError::Contract(error.to_string()))?;
        }
        #[cfg(feature = "browser-use")]
        if let Some(resolved) = compiled.content().enabled_capabilities.iter().find(|capability| capability.capability.id.as_ref() == nomifun_browser_platform::system_browser::TOOL_NAME) {
            let live = registry.capability(&resolved.capability.id).ok_or_else(|| NomiPluginToolError::Contract("System browser capability disappeared".into()))?;
            if resolved.schema_digest != live.schema_digest || resolved.contribution_lock != live.contribution_lock || resolved.target_artifact_digest != live.target_artifact_digest {
                return Err(NomiPluginToolError::Contract("System browser binding differs from the frozen Snapshot".into()));
            }
            session.system_browser_binding = Some(crate::system_browser::binding_from_manifest(&live.manifest)
                .map_err(|error| NomiPluginToolError::Contract(error.to_string()))?);
        }
        session.discovery_policy = discovery_policy;
        session.model_middleware = if constraints.restricted() { Vec::new() } else {
            model_middleware::selected(compiled.content())?
        };
        session.tool_middleware = tool_middleware::selected(compiled.content())?;
        if constraints.restricted() && !session.tool_middleware.is_empty() {
            return Err(NomiPluginToolError::Contract("Selected tool checks exceed the Session execution ceiling".into()));
        }
        session.target_resource_bindings =
            Arc::from(compiled.target_resource_bindings.clone());
        for contributor in lifecycle_context_contributors {
            session = session.with_context_contributor(contributor)?;
        }
        Ok(session)
    }

    /// Materialize the Plugin Product portion of the same frozen Nomi Tool
    /// session.
    ///
    /// Plugin Product capabilities are intentionally not looked up in the
    /// Kernel Plugin Registry. Their exact release/provenance projection is
    /// already frozen in the Snapshot and schema bytes come from the
    /// owner-scoped Plugin Product release resolver.
    #[allow(clippy::too_many_arguments)]
    pub async fn materialize_plugin_product_actions(
        compiled: &CompiledSnapshot,
        owner: &PrincipalRef,
        agent_session_id: &AgentSessionId,
        state_scope_key: &ScopeKey,
        schema_resolver: Arc<dyn NomiPluginProductToolSchemaResolver>,
    ) -> Result<Vec<NomiPluginProductToolAction>, NomiPluginToolError> {
        validate_session_identity(
            compiled,
            owner,
            agent_session_id,
            state_scope_key,
        )?;
        let mut actions = Vec::new();
        for resolved in compiled
            .content()
            .contributions()
            .filter(|capability| {
                capability.contribution_lock.source_kind
                    == ContributionSourceKind::PluginProductActiveRelease
            })
        {
            resolved
                .validate()
                .map_err(|error| NomiPluginToolError::Contract(error.message))?;
            let display_name = resolved
                .display_name
                .clone()
                .unwrap_or_else(|| resolved.capability.id.as_ref().to_owned());
            let description = resolved.description.clone().unwrap_or_default();
            for action in &resolved.actions {
                let hidden_consumer = resolved.actions == [crate::tool_discovery::action()]
                    || resolved.actions == [model_middleware::action()]
                    || resolved.actions == [tool_middleware::before_action()];
                if (!resolved.action_allowlist.is_empty()
                    && !resolved.action_allowlist.contains(&action.action_id))
                    || (action.presentation != ToolPresentationKind::FunctionTool && !hidden_consumer)
                {
                    continue;
                }
                let input_schema = schema_resolver
                    .resolve(owner, resolved, &action.input_schema)
                    .await
                    .map_err(|reason| NomiPluginToolError::Schema {
                        reference: action.input_schema.clone(),
                        reason,
                    })?;
                if hidden_consumer {
                    let output_schema = schema_resolver.resolve(owner, resolved, &action.output_schema)
                        .await.map_err(|reason| NomiPluginToolError::Schema {
                            reference: action.output_schema.clone(), reason,
                        })?;
                    validate_canonical_input_schema(&action.output_schema, &output_schema)?;
                }
                actions.push(build_plugin_product_action(
                    compiled.snapshot_ref().clone(),
                    resolved.clone(),
                    action.clone(),
                    display_name.clone(),
                    description.clone(),
                    input_schema,
                )?);
            }
        }
        Ok(actions)
    }
}

#[allow(clippy::too_many_arguments)]
async fn assemble_initial_capability_context(
    kernel: &Arc<KernelRegistry>,
    compiled: &CompiledSnapshot,
    active: &nomifun_agent_kernel::ActiveCapabilitySetSnapshot,
    registry: &MaterializedRegistry,
    owner: &PrincipalRef,
    agent_session_id: &AgentSessionId,
    state_scope_key: &ScopeKey,
    admission: Option<&NomiPlatformBuiltinContextAdmission>,
) -> Result<(Vec<NomiInitialContextContribution>, Vec<CapabilityId>), NomiPluginToolError> {
    let mut contributions = Vec::new();
    let mut turn_context_ids = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let positions = compiled.content().context_order.iter().enumerate()
        .map(|(position, id)| (id, position)).collect::<BTreeMap<_, _>>();
    let mut ordered = compiled.content().contributions().collect::<Vec<_>>();
    ordered.sort_by_key(|resolved| (
        positions.get(&resolved.capability.id).copied().unwrap_or(usize::MAX),
        &resolved.capability.id,
    ));
    for resolved in ordered {
        let managed_plugin = resolved.contribution_lock.source_kind
            == ContributionSourceKind::PluginMount
            && resolved.resolved_source.source_kind
                == PluginSourceKind::ManagedLocal;
        let approved_builtin = resolved.contribution_lock.source_kind
            == ContributionSourceKind::PlatformBuiltin
            && resolved.resolved_source.source_kind == PluginSourceKind::Bundled
            && admission
                .map(|value| value.target_for(resolved))
                .transpose()?
                .flatten()
                .is_some();
        if !managed_plugin && !approved_builtin {
            if positions.contains_key(&resolved.capability.id) {
                return Err(NomiPluginToolError::Contract(format!(
                    "ordered Context {} is not admitted by this runtime",
                    resolved.capability.id.as_ref()
                )));
            }
            continue;
        }
        let current = registry
            .capability(&resolved.capability.id)
            .ok_or_else(|| KernelError::CapabilityNotMaterialized {
                capability_id: resolved.capability.id.clone(),
                version: resolved.capability.version.clone(),
            })?;
        validate_exact_target(resolved, current)?;
        if current.manifest.kind != CapabilityKind::ContextContributor {
            if managed_plugin {
                // Tools use the Tool consumer, never the prompt path.
                continue;
            }
            return Err(NomiPluginToolError::Contract(format!(
                "approved initial context {} is no longer a ContextContributor",
                resolved.capability.id.as_ref()
            )));
        }
        if !current.manifest.supports_consumer(CapabilityConsumer::Agent) {
            continue;
        }
        if !supports_nomi_plugin_capability(&current.manifest) {
            return Err(NomiPluginToolError::Contract(format!(
                "initial ContextContributor {} must declare one canonical context schema",
                resolved.capability.id.as_ref()
            )));
        }
        let policy = compiled.policy(&resolved.capability.id).ok_or_else(|| {
            NomiPluginToolError::Contract(format!(
                "compiled Snapshot has no authority policy for initial context {}",
                resolved.capability.id.as_ref()
            ))
        })?;
        let capability_id = resolved.capability.id.clone();
        if current.manifest.contributions.context_phase
            == nomifun_agent_contracts::ContextContributionPhase::BeforeTurn
        {
            turn_context_ids.push(capability_id);
            continue;
        }
        let operation_id = OperationId::from(format!(
            "nomi-context:{}:{}:{}:{}",
            agent_session_id.as_ref(),
            compiled.snapshot_ref().snapshot_id.as_ref(),
            uuid::Uuid::now_v7(),
            capability_id.as_ref()
        ));
        let result = tokio::time::timeout_at(
            deadline,
            kernel.contribute_context(
                compiled,
                active,
                CapabilityAccessRequest {
                    principal: owner.clone(),
                    session_owner: owner.clone(),
                    agent_session_id: agent_session_id.clone(),
                    operation_id: operation_id.clone(),
                    correlation_id: CorrelationId::from(format!(
                        "{}:context",
                        operation_id.as_ref()
                    )),
                    capability_id: capability_id.clone(),
                    resource_binding_ids: policy.resource_binding_ids.clone(),
                    state_scope_key: state_scope_key.clone(),
                    resolved_snapshot_ref: compiled.snapshot_ref().clone(),
                    active_set_generation: active.generation,
                },
            ),
        )
        .await
        .map_err(|_| {
            NomiPluginToolError::Contract(format!(
                "initial ContextContributor {} exceeded the shared 5 second context deadline",
                capability_id.as_ref()
            ))
        })??;
        if let Some(value) = result.value {
            contributions.push(NomiInitialContextContribution {
                capability_id,
                value,
            });
        }
    }
    let bytes = canonical_json_bytes(&contributions).map_err(|error| {
        NomiPluginToolError::Contract(format!(
            "initial capability context could not be encoded: {error}"
        ))
    })?;
    if bytes.len() > MAX_INITIAL_CAPABILITY_CONTEXT_BYTES {
        return Err(NomiPluginToolError::Contract(format!(
            "initial capability context exceeds the {MAX_INITIAL_CAPABILITY_CONTEXT_BYTES}-byte Nomi prompt limit"
        )));
    }
    Ok((contributions, turn_context_ids))
}

#[allow(clippy::too_many_arguments)]
async fn assemble_initial_platform_builtin_lifecycle(
    compiled: &CompiledSnapshot,
    registry: &MaterializedRegistry,
    owner: &PrincipalRef,
    agent_session_id: &AgentSessionId,
    state_scope_key: &ScopeKey,
    admission: &NomiPlatformBuiltinLifecycleAdmission,
) -> Result<Vec<NomiInitialContextContribution>, NomiPluginToolError> {
    let mut results = Vec::new();
    for resolved in compiled.content().contributions() {
        if resolved.contribution_lock.source_kind
            != ContributionSourceKind::PlatformBuiltin
            || resolved.resolved_source.source_kind
                != PluginSourceKind::Bundled
            || admission.target_for(resolved)?.is_none()
        {
            continue;
        }
        let current = registry
            .capability(&resolved.capability.id)
            .ok_or_else(|| KernelError::CapabilityNotMaterialized {
                capability_id: resolved.capability.id.clone(),
                version: resolved.capability.version.clone(),
            })?;
        validate_exact_target(resolved, current)?;
        if current.manifest.kind == CapabilityKind::TurnMiddleware {
            // TurnMiddleware is evaluated by the engine's per-turn context
            // contributor below, not frozen into the runtime-build prompt.
            continue;
        }
        let operation_id = OperationId::from(format!(
            "nomi-lifecycle:{}:{}:{}",
            agent_session_id.as_ref(),
            compiled.snapshot_ref().snapshot_id.as_ref(),
            resolved.capability.id.as_ref()
        ));
        let invocation = lifecycle_invocation(
            compiled,
            owner,
            agent_session_id,
            state_scope_key,
            resolved,
            lifecycle_schema_ref(&current.manifest)?,
            operation_id,
        )?;
        let output = tokio::time::timeout(
            Duration::from_secs(5),
            admission.invoker.activate(invocation),
        )
        .await
        .map_err(|_| {
            NomiPluginToolError::Contract(format!(
                "initial lifecycle capability {} exceeded its 5 second deadline",
                resolved.capability.id.as_ref()
            ))
        })?
        .map_err(NomiPluginToolError::Contract)?;
        results.push(NomiInitialContextContribution {
            capability_id: resolved.capability.id.clone(),
            value: output,
        });
    }
    Ok(results)
}

fn turn_middleware_identities(
    compiled: &CompiledSnapshot,
    registry: &MaterializedRegistry,
    admission: &NomiPlatformBuiltinLifecycleAdmission,
) -> Result<Vec<NomiLifecycleIdentity>, NomiPluginToolError> {
    let mut identities = Vec::new();
    for resolved in compiled
        .content()
        .contributions()
    {
        if admission.target_for(resolved)?.is_none() {
            continue;
        }
        let current = registry
            .capability(&resolved.capability.id)
            .ok_or_else(|| KernelError::CapabilityNotMaterialized {
                capability_id: resolved.capability.id.clone(),
                version: resolved.capability.version.clone(),
            })?;
        if current.manifest.kind != CapabilityKind::TurnMiddleware {
            continue;
        }
        validate_exact_target(resolved, current)?;
        identities.push(NomiLifecycleIdentity {
            resolved_snapshot_ref: compiled.snapshot_ref().clone(),
            resolved_capability: resolved.clone(),
            schema_ref: lifecycle_schema_ref(&current.manifest)?,
        });
    }
    identities.sort_by(|left, right| {
        left.resolved_capability
            .capability
            .id
            .cmp(&right.resolved_capability.capability.id)
    });
    Ok(identities)
}

async fn lifecycle_context_contributors(
    compiled: &CompiledSnapshot,
    registry: &MaterializedRegistry,
    owner: &PrincipalRef,
    agent_session_id: &AgentSessionId,
    state_scope_key: &ScopeKey,
    admission: &Arc<NomiPlatformBuiltinLifecycleAdmission>,
) -> Result<Vec<Arc<dyn ContextContributor>>, NomiPluginToolError> {
    let mut contributors = Vec::new();
    for resolved in compiled.content().contributions() {
        if admission.target_for(resolved)?.is_none() { continue; }
        let current = registry.capability(&resolved.capability.id)
            .ok_or_else(|| KernelError::CapabilityNotMaterialized {
                capability_id: resolved.capability.id.clone(),
                version: resolved.capability.version.clone(),
            })?;
        let invocation = lifecycle_invocation(
            compiled, owner, agent_session_id, state_scope_key, resolved,
            lifecycle_schema_ref(&current.manifest)?,
            OperationId::from(format!("nomi-lifecycle-prepare:{}:{}", agent_session_id.as_ref(), resolved.capability.id.as_ref())),
        )?;
        if let Some(contributor) = admission.invoker.context_contributor(invocation).await.map_err(NomiPluginToolError::Contract)? {
            contributors.push(contributor);
        }
    }
    Ok(contributors)
}

fn lifecycle_invocation(
    compiled: &CompiledSnapshot,
    owner: &PrincipalRef,
    agent_session_id: &AgentSessionId,
    state_scope_key: &ScopeKey,
    resolved: &ResolvedCapability,
    schema_ref: Option<CanonicalSchemaRef>,
    operation_id: OperationId,
) -> Result<NomiPlatformBuiltinLifecycleInvocation, NomiPluginToolError> {
    let policy = compiled.policy(&resolved.capability.id).ok_or_else(|| {
        NomiPluginToolError::Contract(format!(
            "compiled Snapshot has no authority policy for lifecycle capability {}",
            resolved.capability.id.as_ref()
        ))
    })?;
    let mut resource_bindings = policy
        .resource_binding_ids
        .iter()
        .filter_map(|binding_id| compiled.binding(binding_id).cloned())
        .collect::<Vec<_>>();
    resource_bindings.sort_by(|left, right| left.binding_id.cmp(&right.binding_id));
    Ok(NomiPlatformBuiltinLifecycleInvocation {
        principal: owner.clone(),
        agent_session_id: agent_session_id.clone(),
        correlation_id: CorrelationId::from(format!(
            "{}:lifecycle",
            operation_id.as_ref()
        )),
        operation_id,
        resolved_snapshot_ref: compiled.snapshot_ref().clone(),
        registry_generation: compiled.registry_generation,
        registry_digest: compiled.registry_digest.clone(),
        capability: resolved.clone(),
        state_scope_key: state_scope_key.clone(),
        resource_bindings,
        schema_ref,
        turn_input: StrictJsonValue(serde_json::json!({})),
    })
}

struct NomiLifecycleContextContributor {
    compiled: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    owner: PrincipalRef,
    agent_session_id: AgentSessionId,
    state_scope_key: ScopeKey,
    admission: Arc<NomiPlatformBuiltinLifecycleAdmission>,
    identities: Arc<[NomiLifecycleIdentity]>,
    turn_sequence: AtomicU64,
}

impl NomiLifecycleContextContributor {
    async fn contribute_for_turn(
        &self,
        turn: Option<&TurnContext>,
    ) -> Result<Option<String>, String> {
        let sequence = self.turn_sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let active = self
            .active
            .snapshot()
            .map_err(|_| "TURN_MIDDLEWARE_CAPABILITY_STATE_UNAVAILABLE".to_owned())?;
        let mut contributions = Vec::new();
        for identity in self.identities.iter() {
            let capability_id = identity.resolved_capability.capability.id.clone();
            if !active.active.contains(&capability_id) {
                continue;
            }
            let operation_id = OperationId::from(format!(
                "nomi-middleware:{}:{sequence}:{}",
                self.agent_session_id.as_ref(),
                capability_id.as_ref()
            ));
            let mut invocation = lifecycle_invocation(
                &self.compiled,
                &self.owner,
                &self.agent_session_id,
                &self.state_scope_key,
                &identity.resolved_capability,
                identity.schema_ref.clone(),
                operation_id,
            )
            .map_err(|_| "TURN_MIDDLEWARE_CONTRACT_INVALID".to_owned())?;
            if let Some(turn) = turn {
                invocation.turn_input = StrictJsonValue(serde_json::json!({
                    "source_message_id": turn.source_message_id,
                    "text": turn.text,
                    "image_media_types": turn.image_media_types,
                    "cs_dialogue_id": turn.cs_dialogue_id,
                }));
            }
            match tokio::time::timeout(
                Duration::from_secs(5),
                self.admission.invoker.activate(invocation),
            )
            .await
            {
                Ok(Ok(value)) => {
                    contributions.push(NomiInitialContextContribution {
                        capability_id,
                        value,
                    });
                }
                Ok(Err(_)) => {
                    tracing::warn!(
                        capability_id = capability_id.as_ref(),
                        "Nomi TurnMiddleware contribution failed"
                    );
                    return Err("TURN_MIDDLEWARE_REJECTED".to_owned());
                }
                Err(_) => {
                    tracing::warn!(
                        capability_id = capability_id.as_ref(),
                        "Nomi TurnMiddleware contribution timed out"
                    );
                    return Err("TURN_MIDDLEWARE_TIMEOUT".to_owned());
                }
            }
        }
        if contributions.is_empty() {
            return Ok(None);
        }
        let bytes = canonical_json_bytes(&contributions)
            .map_err(|_| "TURN_MIDDLEWARE_CONTEXT_INVALID".to_owned())?;
        if bytes.len() > MAX_INITIAL_CAPABILITY_CONTEXT_BYTES {
            tracing::warn!(
                bytes = bytes.len(),
                "Nomi TurnMiddleware context exceeded the prompt limit"
            );
            return Err("TURN_MIDDLEWARE_CONTEXT_TOO_LARGE".to_owned());
        }
        let payload = String::from_utf8(bytes)
            .map_err(|_| "TURN_MIDDLEWARE_CONTEXT_INVALID".to_owned())?;
        Ok(Some(format!(
            "<nomifun_turn_middleware_context format=\"canonical-json\">\n{payload}\n</nomifun_turn_middleware_context>"
        )))
    }
}

#[async_trait]
impl ContextContributor for NomiLifecycleContextContributor {
    async fn pre_turn_context(&self) -> Option<String> {
        self.contribute_for_turn(None).await.ok().flatten()
    }

    async fn pre_turn_context_for_turn(
        &self,
        turn: &TurnContext,
    ) -> Option<String> {
        self.contribute_for_turn(Some(turn)).await.ok().flatten()
    }

    async fn pre_turn_context_for_turn_result(
        &self,
        turn: &TurnContext,
    ) -> Result<Option<String>, String> {
        self.contribute_for_turn(Some(turn)).await
    }

    fn label(&self) -> &str {
        "nomifun_turn_middleware"
    }
}

struct OwnedNomiPluginToolInvoker {
    delegate: Arc<dyn NomiPluginToolInvoker>,
    scope: Arc<crate::engine_effect_scope::EngineEffectScope>,
}

struct NomiEffectContextFence {
    scope: Arc<crate::engine_effect_scope::EngineEffectScope>,
}

#[async_trait]
impl ContextContributor for NomiEffectContextFence {
    async fn pre_turn_context(&self) -> Option<String> { None }

    async fn pre_turn_context_for_turn_result(&self, _: &TurnContext) -> Result<Option<String>, String> {
        self.scope.ensure_turn_open().map_err(|_| "HOSTED_EFFECT_TURN_CLOSED".to_owned())?;
        Ok(None)
    }

    fn label(&self) -> &str { "platform_hosted_effect_fence" }
}

struct OwnedNomiPluginProductToolInvoker {
    delegate: Arc<dyn NomiPluginProductToolInvoker>,
    scope: Arc<crate::engine_effect_scope::EngineEffectScope>,
}

/// A timed-out/cancelled waiter cannot leave dispatch open while its retained
/// task is still running. The result task and durable owner receipt survive.
struct NomiEffectWaitGuard {
    scope: Arc<crate::engine_effect_scope::EngineEffectScope>,
    received: bool,
    cancellation: Option<NomiPluginProductCallCancellation>,
}
impl Drop for NomiEffectWaitGuard {
    fn drop(&mut self) {
        if !self.received {
            if let Some(cancellation) = &self.cancellation { cancellation.cancel(); }
            let _ = self.scope.close_turn();
        }
    }
}
pub(crate) async fn await_owned_effect<T>(scope: Arc<crate::engine_effect_scope::EngineEffectScope>, task: crate::engine_tasks::EngineOwnedTask<T>) -> Result<T, AppError> {
    await_owned_effect_with_cancellation(scope, task, None).await
}
async fn await_owned_effect_with_cancellation<T>(
    scope: Arc<crate::engine_effect_scope::EngineEffectScope>,
    task: crate::engine_tasks::EngineOwnedTask<T>,
    cancellation: Option<NomiPluginProductCallCancellation>,
) -> Result<T, AppError> {
    let mut guard = NomiEffectWaitGuard { scope, received: false, cancellation };
    let result = task.result().await;
    guard.received = result.is_ok();
    result
}

#[async_trait]
impl NomiPluginProductToolInvoker for OwnedNomiPluginProductToolInvoker {
    async fn preflight(&self, request: NomiPluginProductToolInvocation) -> Result<(), NomiPluginToolError> {
        self.delegate.preflight(request).await
    }
    async fn invoke(&self, request: NomiPluginProductToolInvocation) -> Result<StrictJsonValue, NomiPluginToolError> {
        let cancellation = request.cancellation.clone();
        let delegate = self.delegate.clone();
        let scope = self.scope.clone();
        let task = self.scope.spawn(async move {
            let result = delegate.invoke(request).await;
            if matches!(&result, Err(NomiPluginToolError::OutcomeUnknown(_))) {
                let _ = scope.close_session();
            }
            result
        }).map_err(|error| NomiPluginToolError::OutcomeUnknown(error.to_string()))?;
        await_owned_effect_with_cancellation(self.scope.clone(), task, Some(cancellation)).await
            .map_err(|error| NomiPluginToolError::OutcomeUnknown(error.to_string()))?
    }
}

struct OwnedNomiDynamicToolInvoker {
    delegate: Arc<dyn NomiHostDynamicToolInvoker>,
    scope: Arc<crate::engine_effect_scope::EngineEffectScope>,
}

#[async_trait]
impl NomiHostDynamicToolInvoker for OwnedNomiDynamicToolInvoker {
    async fn invoke(&self, request: NomiHostDynamicToolInvocation) -> Result<StrictJsonValue, NomiHostDynamicToolError> {
        let delegate = self.delegate.clone();
        let scope = self.scope.clone();
        let failed = |error: AppError| NomiHostDynamicToolError::new("HOSTED_EFFECT_UNPROVEN", error.to_string(), false);
        let task = self.scope.spawn(async move {
            let result = delegate.invoke(request).await;
            if result.as_ref().is_err_and(|error| error.code.as_ref() == "HOSTED_EFFECT_UNPROVEN") {
                let _ = scope.close_session();
            }
            result
        }).map_err(failed)?;
        await_owned_effect(self.scope.clone(), task).await.map_err(failed)?
    }
}

#[async_trait]
impl NomiPluginToolInvoker for OwnedNomiPluginToolInvoker {
    async fn preflight(&self, request: NomiPluginToolInvocation) -> Result<(), NomiPluginToolError> {
        self.delegate.preflight(request).await
    }
    async fn invoke(&self, request: NomiPluginToolInvocation) -> Result<StrictJsonValue, NomiPluginToolError> {
        let delegate = self.delegate.clone();
        let task = self.scope.spawn(async move { delegate.invoke(request).await })
            .map_err(|error| NomiPluginToolError::Contract(error.to_string()))?;
        await_owned_effect(self.scope.clone(), task).await
            .map_err(|error| NomiPluginToolError::Contract(error.to_string()))?
    }
}

/// Captures server-owned turn identity before the provider sees any tools.
/// A model is never asked to choose a conversation or message owner.
#[derive(Default)]
struct NomiCreationTurnContext {
    source_message_id: RwLock<Option<String>>,
}

#[async_trait]
impl ContextContributor for NomiCreationTurnContext {
    async fn pre_turn_context(&self) -> Option<String> { None }
    async fn pre_turn_context_for_turn_result(&self, turn: &TurnContext) -> Result<Option<String>, String> {
        let mut current = self.source_message_id.write().map_err(|_| "creation turn identity lock poisoned".to_owned())?;
        *current = None;
        let parsed = uuid::Uuid::parse_str(&turn.source_message_id).map_err(|_| "creation requires an admitted message UUID".to_owned())?;
        if parsed.get_version_num() != 7 || parsed.to_string() != turn.source_message_id {
            return Err("creation requires an admitted UUIDv7 message".into());
        }
        *current = Some(turn.source_message_id.clone());
        Ok(None)
    }
    fn label(&self) -> &str { "nomifun_creation_turn_owner" }
}

fn is_builtin_creation(identity: &NomiPluginToolActionIdentity) -> bool {
    identity.resolved_capability.contribution_lock.source_kind == ContributionSourceKind::PlatformBuiltin
        && matches!(identity.resolved_capability.capability.id.as_ref(), "creation.text" | "creation.image" | "creation.image_edit" | "creation.video" | "creation.music" | "creation.audio")
}

fn conversation_creation_schema(mut schema: Value) -> Value {
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) { properties.remove("target"); }
    if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) { required.retain(|key| key.as_str() != Some("target")); }
    schema
}

struct NomiCreationReceiptInvoker {
    delegate: Arc<dyn NomiPluginToolInvoker>,
    sink: Arc<crate::capability::backend_output_sink::BackendOutputSink>,
    conversation_id: String,
}

#[async_trait]
impl NomiPluginToolInvoker for NomiCreationReceiptInvoker {
    async fn preflight(&self, request: NomiPluginToolInvocation) -> Result<(), NomiPluginToolError> {
        self.delegate.preflight(request).await
    }

    async fn invoke(&self, request: NomiPluginToolInvocation) -> Result<StrictJsonValue, NomiPluginToolError> {
        let scope = if is_builtin_creation(&request.identity) {
            Some(self.sink.native_creation_task_scope(&self.conversation_id).map_err(NomiPluginToolError::Contract)?)
        } else { None };
        let output = self.delegate.invoke(request).await?;
        if let Some(scope) = scope {
            let task_id = output.0.get("creation_task_id").and_then(Value::as_str)
                .ok_or_else(|| NomiPluginToolError::Contract("creation host returned no durable task identity".into()))?;
            self.sink.register_native_creation_task(&scope, task_id).map_err(NomiPluginToolError::Contract)?;
        }
        Ok(output)
    }
}

struct KernelNomiPluginToolInvoker {
    creation_turn: Arc<NomiCreationTurnContext>,
    kernel: Arc<KernelRegistry>,
    compiled: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    owner: PrincipalRef,
    agent_session_id: AgentSessionId,
    state_scope_key: ScopeKey,
    identities:
        BTreeMap<(CapabilityId, ActionId), NomiPluginToolActionIdentity>,
}

impl KernelNomiPluginToolInvoker {
    fn prepare(&self, mut request: NomiPluginToolInvocation)
        -> Result<(nomifun_agent_kernel::ActiveCapabilitySetSnapshot, CapabilityInvocationRequest), NomiPluginToolError> {
        let key = (
            request.identity.resolved_capability.capability.id.clone(),
            request.identity.action.action_id.clone(),
        );
        if self.identities.get(&key) != Some(&request.identity) {
            return Err(NomiPluginToolError::Contract(
                "Plugin Tool invocation identity differs from the materialized session"
                    .to_owned(),
            ));
        }
        if is_builtin_creation(&request.identity) {
            let message_id = self.creation_turn.source_message_id.read()
                .map_err(|_| NomiPluginToolError::Contract("creation turn identity lock poisoned".into()))?
                .clone().ok_or_else(|| NomiPluginToolError::Contract("creation requires the active admitted turn".into()))?;
            let object = request.input.0.as_object_mut().ok_or_else(|| NomiPluginToolError::Contract("creation input must be an object".into()))?;
            object.insert("target".into(), serde_json::json!({"kind":"conversation_turn", "conversation_id": self.agent_session_id.as_ref(), "message_id": message_id}));
        }
        let active = self.active.snapshot()?;
        let policy = self.compiled.policy(&key.0).ok_or_else(|| {
            NomiPluginToolError::Contract(format!(
                "compiled Snapshot lost authority policy for {}",
                key.0.as_ref()
            ))
        })?;
        let invocation = CapabilityInvocationRequest {
                    principal: self.owner.clone(),
                    session_owner: self.owner.clone(),
                    agent_session_id: self.agent_session_id.clone(),
                    operation_id: request.operation_id,
                    idempotency_key: request.idempotency_key,
                    correlation_id: request.correlation_id,
                    resolved_snapshot_ref: self.compiled.snapshot_ref().clone(),
                    active_set_generation: active.generation,
                    capability_id: key.0,
                    action_id: key.1,
                    resource_binding_ids: policy.resource_binding_ids.clone(),
                    state_scope_key: self.state_scope_key.clone(),
                    input: request.input,
                };
        Ok((active, invocation))
    }
}

#[async_trait]
impl NomiPluginToolInvoker for KernelNomiPluginToolInvoker {
    async fn preflight(&self, request: NomiPluginToolInvocation) -> Result<(), NomiPluginToolError> {
        let (active, invocation) = self.prepare(request)?;
        self.kernel.preflight_invocation(&self.compiled, &active, &invocation).map_err(Into::into)
    }
    async fn invoke(&self, request: NomiPluginToolInvocation) -> Result<StrictJsonValue, NomiPluginToolError> {
        let (active, invocation) = self.prepare(request)?;
        self.kernel.invoke_shared(Arc::clone(&self.compiled), &active, invocation).await.map_err(Into::into)
    }
}

struct NomiPluginTool {
    action: NomiPluginToolAction,
    invoker: Arc<dyn NomiPluginToolInvoker>,
}

struct NomiPluginProductTool {
    action: NomiPluginProductToolAction,
    invoker: Arc<dyn NomiPluginProductToolInvoker>,
}

impl NomiPluginProductTool {
    fn invocation(&self, input: Value, context: &ToolExecutionContext) -> NomiPluginProductToolInvocation {
        let key = format!("nomi-plugin-product:{}", context.operation_id());
        NomiPluginProductToolInvocation { cancellation: Default::default(), identity: self.action.identity.clone(), operation_id: key.clone().into(),
            idempotency_key: key.clone().into(), correlation_id: key.into(), input: StrictJsonValue(input) }
    }
}

impl NomiPluginTool {
    fn invocation(&self, input: Value, context: &ToolExecutionContext) -> NomiPluginToolInvocation {
        let key = format!("nomi-plugin:{}", context.operation_id());
        NomiPluginToolInvocation { identity: self.action.identity.clone(), operation_id: key.clone().into(),
            idempotency_key: key.clone().into(), correlation_id: key.into(), input: StrictJsonValue(input) }
    }
}

struct NomiPluginProductDiscoveryPolicy {
    action: NomiPluginProductToolAction,
    invoker: Arc<dyn NomiPluginProductToolInvoker>,
}

#[async_trait]
impl nomi_tools::tool_search::ToolDiscoveryPolicy for NomiPluginProductDiscoveryPolicy {
    async fn select(&self, input: nomi_tools::tool_search::ToolDiscoveryInput) -> Result<Vec<String>, String> {
        let key = format!("nomi-discovery:{}", uuid::Uuid::now_v7());
        let result = self.invoker.invoke(NomiPluginProductToolInvocation {
            cancellation: Default::default(),
            identity: self.action.identity.clone(),
            operation_id: key.clone().into(), idempotency_key: key.clone().into(), correlation_id: key.into(),
            input: StrictJsonValue(serde_json::to_value(input).map_err(|e| e.to_string())?),
        }).await.map_err(|e| e.to_string())?;
        crate::tool_discovery::decode_selection(result)
    }
}

struct NomiHostDynamicTool {
    action: NomiHostDynamicToolAction,
    invoker: Arc<dyn NomiHostDynamicToolInvoker>,
    deferred_state: DeferredToolState,
}

#[async_trait]
impl Tool for NomiHostDynamicTool {
    fn name(&self) -> &str {
        &self.action.descriptor.provider_name
    }

    fn activation_identity(&self) -> &str {
        &self.action.activation_identity
    }

    fn artifact_identity(&self) -> &str {
        self.action.descriptor.capability_id.as_ref()
    }

    fn deferred_search_aliases(&self) -> Vec<String> {
        vec![self.action.descriptor.capability_id.as_ref().to_owned()]
    }

    fn description(&self) -> &str {
        &self.action.descriptor.description
    }

    fn input_schema(&self) -> JsonSchema {
        self.action.descriptor.input_schema.0.clone()
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        // The hosted attribution lane permits one unresolved call per Session.
        // A device's read-only declaration does not grant parallel dispatch.
        false
    }

    fn is_deferred(&self) -> bool {
        self.action.descriptor.deferred
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        ToolResult::error("host dynamic Tool requires an engine-owned execution context")
    }

    async fn execute_with_context(
        &self,
        input: Value,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        if self.action.descriptor.deferred
            && !self
                .deferred_state
                .is_activated(&self.action.activation_identity)
        {
            return ToolResult::error(format!(
                "host dynamic Tool '{}' is deferred; activate it through ToolSearch before invoking it",
                self.action.descriptor.provider_name
            ));
        }
        let operation_identity = context.operation_id();
        let request = NomiHostDynamicToolInvocation {
            capability_id: self.action.descriptor.capability_id.clone(),
            provider_name: self.action.descriptor.provider_name.clone(),
            operation_id: OperationId::from(format!(
                "nomi-host-tool:{operation_identity}"
            )),
            idempotency_key: IdempotencyKey::from(format!(
                "nomi-host-tool:{operation_identity}"
            )),
            correlation_id: CorrelationId::from(format!(
                "nomi-host-tool:{operation_identity}"
            )),
            arguments: StrictJsonValue(input),
        };
        match self.invoker.invoke(request).await {
            Ok(output) => match serde_json::to_string_pretty(&output.0) {
                Ok(content) => ToolResult::text(content),
                Err(_) => ToolResult::error(
                    "{\"code\":\"CAPABILITY_OUTPUT_INVALID\",\"message\":\"The capability result could not be encoded.\"}",
                ),
            },
            Err(error) => model_safe_dynamic_tool_error(&error),
        }
    }

    fn category(&self) -> ToolCategory {
        effect_category(self.action.descriptor.effect_class)
    }
}

#[async_trait]
impl Tool for NomiPluginProductTool {
    async fn preflight_hook(&self, input: &Value, context: &ToolExecutionContext) -> Result<(), String> {
        self.invoker.preflight(self.invocation(input.clone(), context)).await.map_err(|e| e.to_string())
    }
    fn name(&self) -> &str {
        &self.action.provider_name
    }

    fn activation_identity(&self) -> &str {
        &self.action.activation_identity
    }

    fn artifact_identity(&self) -> &str {
        self.action.artifact_identity()
    }

    fn deferred_search_aliases(&self) -> Vec<String> {
        vec![
            self.action.capability_id().as_ref().to_owned(),
            self.action.action_id().as_ref().to_owned(),
        ]
    }

    fn description(&self) -> &str {
        &self.action.description
    }

    fn input_schema(&self) -> JsonSchema {
        self.action.input_schema.0.clone()
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        // The Session attribution owner admits one unresolved hosted call at
        // a time, even if the service manifest describes it as a pure read.
        false
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        ToolResult::error(
            "Plugin Product Tool invocation requires an engine-owned execution context",
        )
    }

    async fn execute_with_context(
        &self,
        input: Value,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        let request = self.invocation(input, context);
        match self.invoker.invoke(request).await {
            Ok(output) => match serde_json::to_string_pretty(&output.0) {
                Ok(content) => ToolResult::text(content),
                Err(error) => ToolResult::error(format!(
                    "Plugin Product Tool output could not be serialized: {error}"
                )),
            },
            Err(error) => model_safe_tool_error(&error),
        }
    }

    fn category(&self) -> ToolCategory {
        effect_category(self.action.identity.action.effect_class)
    }
}

#[async_trait]
impl Tool for NomiPluginTool {
    async fn preflight_hook(&self, input: &Value, context: &ToolExecutionContext) -> Result<(), String> {
        self.invoker.preflight(self.invocation(input.clone(), context)).await.map_err(|e| e.to_string())
    }
    fn name(&self) -> &str {
        &self.action.provider_name
    }

    fn activation_identity(&self) -> &str {
        &self.action.activation_identity
    }

    fn artifact_identity(&self) -> &str {
        if is_builtin_creation(&self.action.identity) { "nomifun_creation_task" } else { self.action.artifact_identity() }
    }

    fn deferred_search_aliases(&self) -> Vec<String> {
        vec![
            self.action.capability_id().as_ref().to_owned(),
            self.action.action_id().as_ref().to_owned(),
        ]
    }

    fn description(&self) -> &str {
        &self.action.description
    }

    fn input_schema(&self) -> JsonSchema {
        if is_builtin_creation(&self.action.identity) { conversation_creation_schema(self.action.input_schema.0.clone()) } else { self.action.input_schema.0.clone() }
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        matches!(
            self.action.identity.action.effect_class,
            EffectClass::Pure
                | EffectClass::ReadLocal
                | EffectClass::ReadSensitive
        )
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        ToolResult::error(
            "Plugin Tool invocation requires an engine-owned execution context",
        )
    }

    async fn execute_with_context(
        &self,
        input: Value,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        let request = self.invocation(input, context);
        match self.invoker.invoke(request).await {
            Ok(output) => match serde_json::to_string_pretty(&output.0) {
                Ok(content) => ToolResult::text(content),
                Err(error) => ToolResult::error(format!(
                    "Plugin Tool output could not be serialized: {error}"
                )),
            },
            Err(error) => model_safe_tool_error(&error),
        }
    }

    fn category(&self) -> ToolCategory {
        effect_category(self.action.identity.action.effect_class)
    }
}

fn build_action(
    resolved_snapshot_ref: ResolvedSnapshotRef,
    resolved_capability: ResolvedCapability,
    action: CapabilityActionDescriptor,
    display_name: String,
    description: String,
    input_schema: StrictJsonValue,
) -> Result<NomiPluginToolAction, NomiPluginToolError> {
    let input_schema_digest =
        validate_canonical_input_schema(&action.input_schema, &input_schema)?;
    let identity = NomiPluginToolActionIdentity {
        resolved_snapshot_ref,
        resolved_capability,
        action,
        input_schema_digest,
    };
    let canonical_identity = canonical_json_bytes(&identity).map_err(|error| {
        NomiPluginToolError::Contract(format!(
            "Plugin Tool activation identity could not be encoded: {error}"
        ))
    })?;
    let activation_identity =
        String::from_utf8(canonical_identity.clone()).map_err(|error| {
            NomiPluginToolError::Contract(format!(
                "Plugin Tool activation identity is not UTF-8: {error}"
            ))
        })?;
    let provider_name = provider_tool_name(
        identity.resolved_capability.capability.id.as_ref(),
        identity.action.action_id.as_ref(),
        &canonical_identity,
    );
    let artifact_identity = format!(
        "{} {}",
        identity.resolved_capability.capability.id.as_ref(),
        identity.action.action_id.as_ref(),
    );
    let description = if description.trim().is_empty() {
        format!("{display_name} action {}", identity.action.action_id.as_ref())
    } else {
        format!(
            "{display_name}: {description} Action: {}.",
            identity.action.action_id.as_ref()
        )
    };
    Ok(NomiPluginToolAction {
        provider_name,
        activation_identity,
        artifact_identity,
        description,
        input_schema,
        identity,
    })
}

fn build_plugin_product_action(
    resolved_snapshot_ref: ResolvedSnapshotRef,
    resolved_capability: ResolvedCapability,
    action: CapabilityActionDescriptor,
    display_name: String,
    description: String,
    input_schema: StrictJsonValue,
) -> Result<NomiPluginProductToolAction, NomiPluginToolError> {
    let input_schema_digest =
        validate_canonical_input_schema(&action.input_schema, &input_schema)?;
    let identity = NomiPluginProductToolActionIdentity {
        resolved_snapshot_ref,
        resolved_capability,
        action,
        input_schema_digest,
    };
    let canonical_identity = canonical_json_bytes(&identity).map_err(|error| {
        NomiPluginToolError::Contract(format!(
            "Plugin Product Tool activation identity could not be encoded: {error}"
        ))
    })?;
    let activation_identity =
        String::from_utf8(canonical_identity.clone()).map_err(|error| {
            NomiPluginToolError::Contract(format!(
                "Plugin Product Tool activation identity is not UTF-8: {error}"
            ))
        })?;
    let provider_name = provider_tool_name_with_prefix(
        "plugin_product__",
        identity.resolved_capability.capability.id.as_ref(),
        identity.action.action_id.as_ref(),
        &canonical_identity,
    );
    let artifact_identity = format!(
        "{} {}",
        identity.resolved_capability.capability.id.as_ref(),
        identity.action.action_id.as_ref(),
    );
    let description = if description.trim().is_empty() {
        format!("{display_name} action {}", identity.action.action_id.as_ref())
    } else {
        format!(
            "{display_name}: {description} Action: {}.",
            identity.action.action_id.as_ref()
        )
    };
    Ok(NomiPluginProductToolAction {
        provider_name,
        activation_identity,
        artifact_identity,
        description,
        input_schema,
        identity,
    })
}

fn validate_session_identity(
    compiled: &CompiledSnapshot,
    owner: &PrincipalRef,
    agent_session_id: &AgentSessionId,
    state_scope_key: &ScopeKey,
) -> Result<(), NomiPluginToolError> {
    compiled
        .envelope
        .validate()
        .map_err(|error| NomiPluginToolError::Contract(error.message))?;
    if &compiled.envelope.actor != owner {
        return Err(NomiPluginToolError::Contract(
            "compiled Snapshot actor differs from the Nomi session owner"
                .to_owned(),
        ));
    }
    if agent_session_id.as_ref().trim().is_empty()
        || state_scope_key.as_ref().trim().is_empty()
    {
        return Err(NomiPluginToolError::Contract(
            "Nomi Plugin Tool session identity is incomplete".to_owned(),
        ));
    }
    if compiled
        .resource_bindings()
        .iter()
        .any(|binding| binding.owner_id != owner.principal_id)
    {
        return Err(NomiPluginToolError::Contract(
            "compiled Snapshot contains a resource owned by another principal"
                .to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_exact_target(
    resolved: &ResolvedCapability,
    current: &MaterializedCapability,
) -> Result<(), NomiPluginToolError> {
    resolved
        .validate()
        .map_err(|error| NomiPluginToolError::Contract(error.message))?;
    let manifest_digest = digest_payload(&current.manifest).map_err(|error| {
        NomiPluginToolError::Contract(format!(
            "materialized Capability digest failed: {error}"
        ))
    })?;
    if current.manifest.id != resolved.capability.id
        || current.manifest.version != resolved.capability.version
        || current.manifest.package != resolved.source_package
        || current.contribution_id != resolved.contribution_id
        || current.contribution_lock != resolved.contribution_lock
        || resolved.resolved_mount_id.as_ref() != Some(&current.mount_id)
        || current.source != resolved.resolved_source
        || current.target_artifact_digest != resolved.target_artifact_digest
        || current.schema_digest != resolved.schema_digest
        || manifest_digest != resolved.schema_digest
    {
        return Err(KernelError::CapabilityProvenanceDrift {
            capability_id: resolved.capability.id.clone(),
            reason:
                "current materialization differs from the frozen Nomi Snapshot"
                    .to_owned(),
        }
        .into());
    }
    Ok(())
}

fn validate_canonical_input_schema(
    reference: &CanonicalSchemaRef,
    schema: &StrictJsonValue,
) -> Result<DigestHex, NomiPluginToolError> {
    if !schema.0.is_object() {
        return Err(NomiPluginToolError::Schema {
            reference: reference.clone(),
            reason: "tool input schema must be a JSON object".to_owned(),
        });
    }
    let (_, expected_digest) =
        reference
            .as_ref()
            .rsplit_once('#')
            .ok_or_else(|| NomiPluginToolError::Schema {
                reference: reference.clone(),
                reason:
                    "canonical schema ref must end in a content digest fragment"
                        .to_owned(),
            })?;
    if expected_digest.len() != 64
        || !expected_digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(NomiPluginToolError::Schema {
            reference: reference.clone(),
            reason:
                "canonical schema digest must be 64 lowercase hexadecimal characters"
                    .to_owned(),
        });
    }
    let observed = digest_payload(&schema.0).map_err(|error| {
        NomiPluginToolError::Schema {
            reference: reference.clone(),
            reason: error.to_string(),
        }
    })?;
    if observed.as_ref() != expected_digest {
        return Err(NomiPluginToolError::Schema {
            reference: reference.clone(),
            reason: "resolved schema content does not match its canonical ref"
                .to_owned(),
        });
    }
    Ok(observed)
}

fn provider_tool_name(
    capability_id: &str,
    action_id: &str,
    canonical_identity: &[u8],
) -> String {
    provider_tool_name_with_prefix(
        PROVIDER_NAME_PREFIX,
        capability_id,
        action_id,
        canonical_identity,
    )
}

fn provider_tool_name_with_prefix(
    prefix: &str,
    capability_id: &str,
    action_id: &str,
    canonical_identity: &[u8],
) -> String {
    let mut slug = format!("{capability_id}_{action_id}")
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() {
                byte.to_ascii_lowercase()
            } else {
                b'_'
            }
        })
        .collect::<Vec<_>>();
    while slug.last() == Some(&b'_') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.extend_from_slice(b"tool");
    }
    let slug_bytes = PROVIDER_NAME_MAX_BYTES
        .saturating_sub(prefix.len())
        .saturating_sub(PROVIDER_NAME_SEPARATOR.len())
        .saturating_sub(PROVIDER_NAME_HASH_HEX_BYTES);
    slug.truncate(slug_bytes);
    while slug.last() == Some(&b'_') {
        slug.pop();
    }
    let hash = hex::encode(Sha256::digest(canonical_identity));
    let slug = String::from_utf8(slug).expect("ASCII slug");
    let name = format!(
        "{prefix}{slug}{PROVIDER_NAME_SEPARATOR}{}",
        &hash[..PROVIDER_NAME_HASH_HEX_BYTES]
    );
    debug_assert!(name.len() <= PROVIDER_NAME_MAX_BYTES);
    debug_assert!(
        name.bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    );
    name
}

fn effect_category(effect: EffectClass) -> ToolCategory {
    match effect {
        EffectClass::Pure | EffectClass::ReadLocal => ToolCategory::Info,
        EffectClass::ReadSensitive
        | EffectClass::WriteReversible
        | EffectClass::WriteDurable
        | EffectClass::ExecuteLocal
        | EffectClass::ExternalTransmit => ToolCategory::Exec,
        EffectClass::Destructive
        | EffectClass::Irreversible
        | EffectClass::Physical => ToolCategory::Irreversible,
    }
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_owned());
    }
}

#[cfg(test)]
mod dynamic_error_tests {
    use super::*;

    #[tokio::test]
    async fn creation_receipt_preflight_preserves_authorization_without_executing() {
        use std::sync::atomic::{AtomicBool, AtomicUsize};

        struct AdmissionOnly {
            expected: NomiPluginToolInvocation,
            denied: AtomicBool,
            calls: AtomicUsize,
        }

        #[async_trait]
        impl NomiPluginToolInvoker for AdmissionOnly {
            async fn preflight(&self, request: NomiPluginToolInvocation) -> Result<(), NomiPluginToolError> {
                assert_eq!(request, self.expected, "preflight must retain exact invocation authority and input");
                self.calls.fetch_add(1, Ordering::SeqCst);
                if self.denied.load(Ordering::SeqCst) {
                    Err(NomiPluginToolError::Contract("admission denied".into()))
                } else {
                    Ok(())
                }
            }

            async fn invoke(&self, _request: NomiPluginToolInvocation) -> Result<StrictJsonValue, NomiPluginToolError> {
                panic!("preflight must never execute a tool");
            }
        }

        let digest = "a".repeat(64);
        let request = NomiPluginToolInvocation {
            identity: NomiPluginToolActionIdentity {
                resolved_snapshot_ref: ResolvedSnapshotRef {
                    snapshot_id: uuid::Uuid::now_v7().to_string().into(),
                    snapshot_digest: digest.clone().into(),
                },
                resolved_capability: serde_json::from_value(serde_json::json!({
                    "capability": {"id": "creation.image", "version": "1.0.0"},
                    "source_package": {"id": "nomifun.creation", "version": "1.0.0"},
                    "contribution_id": "capability:creation.image",
                    "contribution_lock": {
                        "source_kind": ContributionSourceKind::PlatformBuiltin,
                        "source_identity": "platform-builtin:creation.image",
                        "contribution_id": "capability:creation.image",
                        "contract_digest": digest,
                    },
                    "resolved_source": {
                        "source_kind": PluginSourceKind::ManagedLocal,
                        "source_identity": "platform-builtin:creation.image",
                        "source_digest": digest,
                    },
                    "target_artifact_digest": digest,
                    "schema_digest": digest,
                    "dependency_path": ["creation.image"],
                    "required_runtime_features": [],
                })).unwrap(),
                action: CapabilityActionDescriptor {
                    action_id: "creation.image.invoke".into(),
                    input_schema: format!("schema://creation.image/input@1#{digest}").into(),
                    output_schema: format!("schema://creation.image/output@1#{digest}").into(),
                    effect_class: EffectClass::Pure,
                    presentation: ToolPresentationKind::FunctionTool,
                },
                input_schema_digest: digest.into(),
            },
            operation_id: "receipt-preflight-operation".into(),
            idempotency_key: "receipt-preflight-key".into(),
            correlation_id: "receipt-preflight-correlation".into(),
            input: StrictJsonValue(serde_json::json!({"prompt": "a cat"})),
        };
        assert!(is_builtin_creation(&request.identity));
        let delegate = Arc::new(AdmissionOnly {
            expected: request.clone(),
            denied: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        });
        let (events, _receiver) = tokio::sync::broadcast::channel(1);
        // No admitted output turn: attempting to reserve a creation receipt
        // during preflight would fail even when the delegate allows the call.
        let invoker = NomiCreationReceiptInvoker {
            delegate: delegate.clone(),
            sink: Arc::new(crate::capability::backend_output_sink::BackendOutputSink::new(events)),
            conversation_id: uuid::Uuid::now_v7().to_string(),
        };
        invoker.preflight(request.clone()).await.unwrap();
        delegate.denied.store(true, Ordering::SeqCst);
        assert!(matches!(invoker.preflight(request).await,
            Err(NomiPluginToolError::Contract(message)) if message == "admission denied"));
        assert_eq!(delegate.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn creation_turn_context_uses_only_admitted_uuid_and_schema_hides_owner() {
        let context = NomiCreationTurnContext::default();
        assert!(context.source_message_id.read().unwrap().is_none());
        assert!(context.pre_turn_context_for_turn_result(&TurnContext::default()).await.is_err());
        let message_id = uuid::Uuid::now_v7().to_string();
        context.pre_turn_context_for_turn_result(&TurnContext { source_message_id: message_id.clone(), ..Default::default() }).await.unwrap();
        assert_eq!(context.source_message_id.read().unwrap().as_deref(), Some(message_id.as_str()));
        assert!(context.pre_turn_context_for_turn_result(&TurnContext::default()).await.is_err());
        assert!(context.source_message_id.read().unwrap().is_none());
        let schema = conversation_creation_schema(serde_json::json!({"type":"object","additionalProperties":false,"properties":{"target":{},"prompt":{"type":"string"}},"required":["target","prompt"]}));
        assert!(schema["properties"].get("target").is_none());
        assert_eq!(schema["required"], serde_json::json!(["prompt"]));
        assert_eq!(schema["additionalProperties"], false);
    }

    fn payload(code: &str, retry_safe: bool) -> Value {
        let result = model_safe_dynamic_tool_error(
            &NomiHostDynamicToolError::new(
                code,
                "database at https://secret.example?api_key=token failed",
                retry_safe,
            ),
        );
        assert!(result.is_error);
        assert!(!result.content.contains("secret.example"));
        assert!(!result.content.contains("api_key"));
        assert!(!result.content.contains("token"));
        serde_json::from_str(&result.content).unwrap()
    }

    #[test]
    fn outcome_unknown_is_never_retry_safe_and_never_leaks_internal_text() {
        let value = payload("ROBOT_EFFECT_OUTCOME_UNKNOWN", true);
        assert_eq!(value["code"], "ROBOT_EFFECT_OUTCOME_UNKNOWN");
        assert_eq!(value["retry_safe"], false);
    }

    #[test]
    fn unknown_dynamic_error_is_generic_and_not_retry_safe() {
        let value = payload("DATABASE_DRIVER_FAILURE", true);
        assert_eq!(value["code"], "CAPABILITY_EXECUTION_FAILED");
        assert_eq!(value["retry_safe"], false);
    }

    #[test]
    fn invalid_payload_is_not_recommended_for_retry() {
        let value = payload("INVALID_PAYLOAD", true);
        assert_eq!(value["code"], "INVALID_PAYLOAD");
        assert_eq!(value["retry_safe"], false);
    }
}

#[cfg(test)]
mod product_waiter_cancellation_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn dropped_product_waiter_signals_cancel_but_retains_the_owner_task() {
        let scope = Arc::new(crate::engine_effect_scope::EngineEffectScope::new(Vec::new()).unwrap());
        scope.begin_turn().unwrap();
        let cancellation = NomiPluginProductCallCancellation::default();
        let completed = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(tokio::sync::Notify::new());
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let task = scope.spawn({
            let release = release.clone();
            let cancellation = cancellation.clone();
            let completed = completed.clone();
            async move {
                entered_tx.send(()).unwrap();
                release.notified().await;
                assert!(cancellation.is_canceled(), "owner must see the same signal");
                completed.fetch_add(1, Ordering::SeqCst);
                7
            }
        }).unwrap();
        entered_rx.await.unwrap();
        let mut waiter = Box::pin(await_owned_effect_with_cancellation(
            scope.clone(), task, Some(cancellation.clone()),
        ));
        assert!(futures_util::poll!(&mut waiter).is_pending());
        drop(waiter);
        assert!(cancellation.is_canceled());
        assert!(scope.ensure_turn_open().is_err());
        assert_eq!(completed.load(Ordering::SeqCst), 0);
        assert!(scope.begin_turn().is_err(), "cancel signal is not a settlement proof");
        release.notify_one();
        scope.settle_turn().await.unwrap();
        assert_eq!(completed.load(Ordering::SeqCst), 1, "original owner work must complete exactly once");
    }

    #[tokio::test]
    async fn received_product_result_does_not_signal_cancellation() {
        let scope = Arc::new(crate::engine_effect_scope::EngineEffectScope::new(Vec::new()).unwrap());
        scope.begin_turn().unwrap();
        let cancellation = NomiPluginProductCallCancellation::default();
        let task = scope.spawn(async { 7 }).unwrap();
        assert_eq!(await_owned_effect_with_cancellation(
            scope.clone(), task, Some(cancellation.clone()),
        ).await.unwrap(), 7);
        assert!(!cancellation.is_canceled());
        scope.ensure_turn_open().unwrap();
        scope.settle_turn().await.unwrap();
    }
}
