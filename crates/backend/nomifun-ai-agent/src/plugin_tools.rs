//! Nomi adapter for ordinary Plugin Tool capabilities.
//!
//! The adapter consumes one exact compiled Agent Snapshot and the shared
//! Kernel registry. It does not resolve the latest Catalog, accept model-owned
//! capability identity, or use the non-Agent operation API.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::sync::Arc;
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
    ResolvedMiniAppCapability, ResolvedSnapshotRef, ScopeKey,
    StrictJsonValue, ToolPresentationKind, canonical_json_bytes,
    digest_payload,
};
use nomifun_agent_kernel::{
    CapabilityAccessRequest, CapabilityInvocationRequest, CompiledSnapshot,
    CompletedTurnBoundary, KernelError, KernelRegistry,
    MaterializedCapability, MaterializedRegistry, SessionCapabilityState,
};
use nomifun_common::AppError;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::plugin_tool_error_projection::model_safe_tool_error;

const PROVIDER_NAME_PREFIX: &str = "plugin__";
const PROVIDER_NAME_SEPARATOR: &str = "__";
const PROVIDER_NAME_MAX_BYTES: usize = 64;
const PROVIDER_NAME_HASH_HEX_BYTES: usize = 20;
const MAX_INITIAL_CAPABILITY_CONTEXT_BYTES: usize = 64 * 1024;

tokio::task_local! {
    static CURRENT_NOMI_PLUGIN_TOOL_SESSION: Option<NomiPluginToolSession>;
}

#[derive(Debug, Error)]
pub enum NomiPluginToolError {
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

/// Exact, composition-time approval for bundled ContextContributors.
///
/// Context is admitted independently from ordinary Tools. Initial capability
/// results enter the system prompt; on-demand ContextContributors are exposed
/// as deferred empty-input activation Tools and contribute only after the
/// normal ToolSearch/completed-boundary transition.
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

/// Host-owned resolver for schemas exported by a MiniApp Active Release.
///
/// The owner is passed separately because MiniApp release storage is
/// owner-scoped. The resolver must verify the exact release/catalog facts in
/// the supplied snapshot projection before returning schema bytes.
#[async_trait]
pub trait NomiMiniAppToolSchemaResolver: Send + Sync {
    async fn resolve(
        &self,
        owner: &PrincipalRef,
        capability: &ResolvedMiniAppCapability,
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
struct NomiMiniAppToolActionIdentity {
    resolved_snapshot_ref: ResolvedSnapshotRef,
    resolved_capability: ResolvedMiniAppCapability,
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
    deferred: bool,
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

    pub fn is_deferred(&self) -> bool {
        self.deferred
    }
}

/// One provider-visible action derived from an exact MiniApp Active Release
/// capability.
#[derive(Clone, Debug, PartialEq)]
pub struct NomiMiniAppToolAction {
    provider_name: String,
    activation_identity: String,
    artifact_identity: String,
    description: String,
    input_schema: StrictJsonValue,
    identity: NomiMiniAppToolActionIdentity,
    deferred: bool,
}

impl NomiMiniAppToolAction {
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

    pub fn is_deferred(&self) -> bool {
        self.deferred
    }
}

/// Invocation identity created by the Nomi engine, never by model arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct NomiPluginToolInvocation {
    identity: NomiPluginToolActionIdentity,
    operation_id: OperationId,
    idempotency_key: IdempotencyKey,
    correlation_id: CorrelationId,
    deferred_activation_proven: bool,
    input: StrictJsonValue,
}

#[async_trait]
pub trait NomiPluginToolInvoker: Send + Sync {
    async fn invoke(
        &self,
        request: NomiPluginToolInvocation,
    ) -> Result<StrictJsonValue, NomiPluginToolError>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct NomiMiniAppToolInvocation {
    identity: NomiMiniAppToolActionIdentity,
    operation_id: OperationId,
    idempotency_key: IdempotencyKey,
    correlation_id: CorrelationId,
    deferred_activation_proven: bool,
    input: StrictJsonValue,
}

impl NomiMiniAppToolInvocation {
    pub fn capability(&self) -> &ResolvedMiniAppCapability {
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

    pub fn deferred_activation_proven(&self) -> bool {
        self.deferred_activation_proven
    }

    pub fn input(&self) -> &StrictJsonValue {
        &self.input
    }
}

#[async_trait]
pub trait NomiMiniAppToolInvoker: Send + Sync {
    async fn invoke(
        &self,
        request: NomiMiniAppToolInvocation,
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
struct NomiDeferredContextIdentity {
    resolved_snapshot_ref: ResolvedSnapshotRef,
    resolved_capability: ResolvedCapability,
    context_schema_ref: CanonicalSchemaRef,
}

/// Provider-visible activation Tool for one on-demand bundled
/// ContextContributor.
///
/// The Tool has an empty input contract. ToolSearch exposes it at the normal
/// deferred boundary; invocation then activates the canonical capability set
/// and returns the contributed structured value to the current model turn.
#[derive(Clone, Debug)]
pub struct NomiDeferredContextAction {
    provider_name: String,
    activation_identity: String,
    description: String,
    identity: NomiDeferredContextIdentity,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct NomiDeferredLifecycleIdentity {
    resolved_snapshot_ref: ResolvedSnapshotRef,
    resolved_capability: ResolvedCapability,
    schema_ref: Option<CanonicalSchemaRef>,
}

#[derive(Clone, Debug)]
pub struct NomiDeferredLifecycleAction {
    provider_name: String,
    activation_identity: String,
    description: String,
    identity: NomiDeferredLifecycleIdentity,
}

impl NomiDeferredLifecycleAction {
    pub fn provider_name(&self) -> &str {
        &self.provider_name
    }

    pub fn capability_id(&self) -> &CapabilityId {
        &self.identity.resolved_capability.capability.id
    }
}

impl NomiDeferredContextAction {
    pub fn provider_name(&self) -> &str {
        &self.provider_name
    }

    pub fn activation_identity(&self) -> &str {
        &self.activation_identity
    }

    pub fn capability_id(&self) -> &CapabilityId {
        &self.identity.resolved_capability.capability.id
    }
}

/// A complete set of Plugin action tools for one frozen Nomi session.
#[derive(Clone)]
pub struct NomiPluginToolSession {
    resolved_snapshot_ref: ResolvedSnapshotRef,
    /// Exact server-compiled resource bindings for this frozen Session.
    /// Runtime factories may inspect these bindings to lazily connect
    /// host-owned resources, but callers cannot replace or augment them.
    target_resource_bindings:
        Arc<[nomifun_agent_contracts::TypedResourceBinding]>,
    actions: Arc<[NomiPluginToolAction]>,
    invoker: Arc<dyn NomiPluginToolInvoker>,
    miniapp_actions: Arc<[NomiMiniAppToolAction]>,
    miniapp_invoker: Option<Arc<dyn NomiMiniAppToolInvoker>>,
    initial_context_contributions: Arc<[NomiInitialContextContribution]>,
    deferred_context_actions: Arc<[NomiDeferredContextAction]>,
    context_invoker: Option<Arc<KernelNomiContextInvoker>>,
    deferred_lifecycle_actions: Arc<[NomiDeferredLifecycleAction]>,
    lifecycle_invoker: Option<Arc<KernelNomiLifecycleInvoker>>,
    host_dynamic_actions: Arc<[NomiHostDynamicToolAction]>,
    host_dynamic_invoker: Option<Arc<dyn NomiHostDynamicToolInvoker>>,
    capability_state: Option<Arc<SessionCapabilityState>>,
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
            .field("miniapp_actions", &self.miniapp_actions)
            .field(
                "initial_context_contributions",
                &self.initial_context_contributions,
            )
            .field("deferred_context_actions", &self.deferred_context_actions)
            .field(
                "deferred_lifecycle_actions",
                &self.deferred_lifecycle_actions,
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
            target_resource_bindings: Arc::from(
                Vec::<nomifun_agent_contracts::TypedResourceBinding>::new(),
            ),
            actions: Arc::from(actions),
            invoker,
            miniapp_actions: Arc::from(Vec::<NomiMiniAppToolAction>::new()),
            miniapp_invoker: None,
            initial_context_contributions: Arc::from(
                Vec::<NomiInitialContextContribution>::new(),
            ),
            deferred_context_actions: Arc::from(
                Vec::<NomiDeferredContextAction>::new(),
            ),
            context_invoker: None,
            deferred_lifecycle_actions: Arc::from(
                Vec::<NomiDeferredLifecycleAction>::new(),
            ),
            lifecycle_invoker: None,
            host_dynamic_actions: Arc::from(Vec::<NomiHostDynamicToolAction>::new()),
            host_dynamic_invoker: None,
            capability_state: None,
            context_contributors: Arc::from(
                Vec::<Arc<dyn ContextContributor>>::new(),
            ),
            session_control_sink: None,
        })
    }

    /// Attach one host-authenticated dynamic context source to this exact
    /// Session. The contributor is retained by the runtime and therefore its
    /// `Drop` lifecycle is the Session disposal boundary.
    pub fn with_context_contributor(
        mut self,
        contributor: Arc<dyn ContextContributor>,
    ) -> Self {
        let mut contributors = self.context_contributors.to_vec();
        contributors.push(contributor);
        self.context_contributors = Arc::from(contributors);
        self
    }

    pub fn context_contributors(
        &self,
    ) -> &[Arc<dyn ContextContributor>] {
        &self.context_contributors
    }

    /// Attach the native control owner for this exact host-authenticated
    /// AgentSession. It is intentionally not accepted by Plugin manifests or
    /// model input.
    pub fn with_session_control_sink(
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

    /// Add exact MiniApp Active Release actions to this same Nomi Tool
    /// session. Plugin and MiniApp actions share one registry/policy surface,
    /// while their invokers remain separate execution adapters.
    pub fn with_miniapp_actions(
        mut self,
        mut actions: Vec<NomiMiniAppToolAction>,
        invoker: Arc<dyn NomiMiniAppToolInvoker>,
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
        self.miniapp_actions = Arc::from(actions);
        self.miniapp_invoker = Some(invoker);
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

    pub fn miniapp_actions(&self) -> &[NomiMiniAppToolAction] {
        &self.miniapp_actions
    }

    pub fn initial_context_contributions(
        &self,
    ) -> &[NomiInitialContextContribution] {
        &self.initial_context_contributions
    }

    pub fn deferred_context_actions(&self) -> &[NomiDeferredContextAction] {
        &self.deferred_context_actions
    }

    pub fn deferred_lifecycle_actions(
        &self,
    ) -> &[NomiDeferredLifecycleAction] {
        &self.deferred_lifecycle_actions
    }

    pub fn capability_state(&self) -> Option<Arc<SessionCapabilityState>> {
        self.capability_state.clone()
    }

    pub fn with_host_dynamic_tools(
        mut self,
        mut descriptors: Vec<NomiHostDynamicToolDescriptor>,
        invoker: Arc<dyn NomiHostDynamicToolInvoker>,
    ) -> Result<Self, NomiPluginToolError> {
        descriptors.sort_by(|left, right| {
            (&left.capability_id, &left.provider_name)
                .cmp(&(&right.capability_id, &right.provider_name))
        });
        let mut names = self
            .actions
            .iter()
            .map(|action| action.provider_name.clone())
            .chain(self.miniapp_actions.iter().map(|action| action.provider_name.clone()))
            .chain(self.deferred_context_actions.iter().map(|action| action.provider_name.clone()))
            .chain(self.deferred_lifecycle_actions.iter().map(|action| action.provider_name.clone()))
            .collect::<BTreeSet<_>>();
        let mut actions = Vec::with_capacity(descriptors.len());
        for descriptor in descriptors {
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
        self.host_dynamic_invoker = Some(invoker);
        Ok(self)
    }

    /// Append the frozen initial capability context to Nomi's system prompt as
    /// canonical structured data.
    ///
    /// The section is absent when no approved ContextContributor returned a
    /// value. It is assembled once while the runtime is built from the exact
    /// Snapshot; on-demand context is intentionally not projected here.
    pub fn system_prompt_with_initial_context(
        &self,
        base: Option<&str>,
    ) -> Result<Option<String>, NomiPluginToolError> {
        if self.initial_context_contributions.is_empty() {
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
        if bytes.len() > MAX_INITIAL_CAPABILITY_CONTEXT_BYTES {
            return Err(NomiPluginToolError::Contract(format!(
                "initial capability context exceeds the {MAX_INITIAL_CAPABILITY_CONTEXT_BYTES}-byte Nomi prompt limit"
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
        Ok(Some(prompt))
    }

    pub fn tool_count(&self) -> usize {
        self.actions.len()
            + self.miniapp_actions.len()
            + self.deferred_context_actions.len()
            + self.deferred_lifecycle_actions.len()
            + self.host_dynamic_actions.len()
    }

    pub fn provider_names_for(
        &self,
        capability_id: &str,
        deferred: bool,
    ) -> Vec<String> {
        self.actions
            .iter()
            .filter(|action| {
                action.capability_id().as_ref() == capability_id
                    && action.deferred == deferred
            })
            .map(|action| action.provider_name.clone())
            .chain(
                self.miniapp_actions
                    .iter()
                    .filter(|action| {
                        action.capability_id().as_ref() == capability_id
                            && action.deferred == deferred
                    })
                    .map(|action| action.provider_name.clone()),
            )
            .chain(
                self.deferred_context_actions
                    .iter()
                    .filter(|action| {
                        action.capability_id().as_ref() == capability_id
                            && deferred
                    })
                    .map(|action| action.provider_name.clone()),
            )
            .chain(
                self.deferred_lifecycle_actions
                    .iter()
                    .filter(|action| {
                        action.capability_id().as_ref() == capability_id
                            && deferred
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
        for action in self.actions.iter() {
            push_unique(allowed_tools, &action.provider_name);
            if action.deferred {
                push_unique(deferred_tools, &action.provider_name);
            }
        }
        for action in self.miniapp_actions.iter() {
            push_unique(allowed_tools, &action.provider_name);
            if action.deferred {
                push_unique(deferred_tools, &action.provider_name);
            }
        }
        for action in self.deferred_context_actions.iter() {
            push_unique(allowed_tools, &action.provider_name);
            push_unique(deferred_tools, &action.provider_name);
        }
        for action in self.deferred_lifecycle_actions.iter() {
            push_unique(allowed_tools, &action.provider_name);
            push_unique(deferred_tools, &action.provider_name);
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
        if self.actions.is_empty()
            && self.miniapp_actions.is_empty()
            && self.deferred_context_actions.is_empty()
            && self.deferred_lifecycle_actions.is_empty()
            && self.host_dynamic_actions.is_empty()
        {
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
                    deferred_state: deferred_state.clone(),
                }) as Box<dyn Tool>
            })
            .collect();
        if let Some(invoker) = &self.miniapp_invoker {
            tools.extend(self.miniapp_actions.iter().cloned().map(|action| {
                Box::new(NomiMiniAppTool {
                    action,
                    invoker: Arc::clone(invoker),
                    deferred_state: deferred_state.clone(),
                }) as Box<dyn Tool>
            }));
        } else if !self.miniapp_actions.is_empty() {
            return Err(NomiPluginToolError::Contract(
                "MiniApp actions are present without an execution adapter".to_owned(),
            ));
        }
        if let Some(invoker) = &self.context_invoker {
            tools.extend(self.deferred_context_actions.iter().cloned().map(
                |action| {
                    Box::new(NomiDeferredContextTool {
                        action,
                        invoker: Arc::clone(invoker),
                        deferred_state: deferred_state.clone(),
                    }) as Box<dyn Tool>
                },
            ));
        } else if !self.deferred_context_actions.is_empty() {
            return Err(NomiPluginToolError::Contract(
                "deferred ContextContributor actions are present without a Kernel execution adapter"
                    .to_owned(),
            ));
        }
        if let Some(invoker) = &self.lifecycle_invoker {
            tools.extend(self.deferred_lifecycle_actions.iter().cloned().map(
                |action| {
                    Box::new(NomiDeferredLifecycleTool {
                        action,
                        invoker: Arc::clone(invoker),
                        deferred_state: deferred_state.clone(),
                    }) as Box<dyn Tool>
                },
            ));
        } else if !self.deferred_lifecycle_actions.is_empty() {
            return Err(NomiPluginToolError::Contract(
                "deferred lifecycle actions are present without a host execution adapter"
                    .to_owned(),
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
        let inserted = registry.register_batch(tools);
        let expected = self
            .actions
            .iter()
            .map(|action| action.provider_name.clone())
            .chain(
                self.miniapp_actions
                    .iter()
                    .map(|action| action.provider_name.clone()),
            )
            .chain(
                self.deferred_context_actions
                    .iter()
                    .map(|action| action.provider_name.clone()),
            )
            .chain(
                self.deferred_lifecycle_actions
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
        if inserted != expected {
            return Err(NomiPluginToolError::Contract(
                "Nomi registry rejected one or more exact hosted Tool routes"
                    .to_owned(),
            ));
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
        )
        .await
    }

    /// Materialize the exact bundled Tool set and project approved bundled
    /// ContextContributors through the same compiled Snapshot and active
    /// capability set.
    ///
    /// Keeping both projections in one Session object prevents catalog,
    /// context, and Tool authority from being resolved along separate paths.
    /// Initial ContextContributors are assembled into the system prompt;
    /// on-demand ContextContributors become deferred empty-input Tools whose
    /// invocation performs the completed-boundary activation and returns the
    /// context to the current turn.
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
        Self::materialize_internal(
            kernel,
            compiled,
            owner,
            agent_session_id,
            state_scope_key,
            plugin_schema_resolver,
            Some(platform_builtin_tool_admission),
            Some(platform_builtin_context_admission),
            Some(platform_builtin_lifecycle_admission),
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
    ) -> Result<NomiPluginToolSession, NomiPluginToolError> {
        validate_session_identity(
            &compiled,
            &owner,
            &agent_session_id,
            &state_scope_key,
        )?;
        let registry = kernel.snapshot()?;
        let initial = compiled
            .content()
            .initial_capabilities
            .iter()
            .map(|capability| capability.capability.id.clone())
            .collect::<BTreeSet<_>>();
        let on_demand = compiled
            .content()
            .on_demand_capabilities
            .iter()
            .map(|capability| capability.capability.id.clone())
            .collect::<BTreeSet<_>>();

        let active = Arc::new(SessionCapabilityState::new(&compiled));
        let active_snapshot = active.snapshot()?;
        let mut initial_context_contributions = match (
            platform_builtin_context_admission.as_ref(),
            compiled.content().initial_capabilities.is_empty(),
        ) {
            (Some(admission), false) => {
                assemble_initial_platform_builtin_context(
                    &kernel,
                    &compiled,
                    &active_snapshot,
                    registry.as_ref(),
                    &owner,
                    &agent_session_id,
                    &state_scope_key,
                    admission,
                )
                .await?
            }
            _ => Vec::new(),
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
        let deferred_context_actions = match
            platform_builtin_context_admission.as_ref()
        {
            Some(admission) => materialize_deferred_platform_builtin_context(
                &compiled,
                registry.as_ref(),
                admission,
            )?,
            None => Vec::new(),
        };
        let deferred_lifecycle_actions = match
            platform_builtin_lifecycle_admission.as_ref()
        {
            Some(admission) => materialize_deferred_platform_builtin_lifecycle(
                &compiled,
                registry.as_ref(),
                admission,
            )?,
            None => Vec::new(),
        };
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
        let (mut lifecycle_context_contributors, lifecycle_context_cells) = match
            platform_builtin_lifecycle_admission.as_ref()
        {
            Some(admission) => lifecycle_context_contributors(
                &compiled,
                registry.as_ref(),
                &owner,
                &agent_session_id,
                &state_scope_key,
                admission,
                &active,
            )
            .await?,
            None => (Vec::new(), BTreeMap::new()),
        };
        if let Some(contributor) = middleware_context_contributor {
            lifecycle_context_contributors.push(contributor);
        }

        let mut pending = Vec::new();
        for resolved in compiled
            .content()
            .initial_capabilities
            .iter()
            .chain(&compiled.content().on_demand_capabilities)
        {
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
            let deferred = if initial.contains(&manifest.id) {
                false
            } else if on_demand.contains(&manifest.id) {
                true
            } else {
                return Err(NomiPluginToolError::Contract(format!(
                    "resolved capability {} is in neither Snapshot set",
                    manifest.id.as_ref()
                )));
            };
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
                    deferred,
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
                    deferred,
                    schema_source,
                )| {
                    let plugin_schema_resolver =
                        Arc::clone(&plugin_schema_resolver);
                    let platform_builtin_admission =
                        platform_builtin_admission.clone();
                    let snapshot_ref = compiled.snapshot_ref().clone();
                    async move {
                        let input_schema = match schema_source {
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
                            deferred,
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
        let activation_gate = Arc::new(tokio::sync::Mutex::new(()));
        let invoker = Arc::new(KernelNomiPluginToolInvoker {
            kernel: Arc::clone(&kernel),
            compiled: Arc::clone(&compiled),
            active: Arc::clone(&active),
            activation_gate: Arc::clone(&activation_gate),
            owner: owner.clone(),
            agent_session_id: agent_session_id.clone(),
            state_scope_key: state_scope_key.clone(),
            identities,
        });
        let context_invoker = if deferred_context_actions.is_empty() {
            None
        } else {
            Some(Arc::new(KernelNomiContextInvoker {
                kernel: Arc::clone(&kernel),
                compiled: Arc::clone(&compiled),
                active: Arc::clone(&active),
                activation_gate: Arc::clone(&activation_gate),
                owner: owner.clone(),
                agent_session_id: agent_session_id.clone(),
                state_scope_key: state_scope_key.clone(),
                identities: deferred_context_actions
                    .iter()
                    .map(|action| {
                        (
                            action.capability_id().clone(),
                            action.identity.clone(),
                        )
                    })
                    .collect(),
            }))
        };
        let lifecycle_invoker = match (
            deferred_lifecycle_actions.is_empty(),
            platform_builtin_lifecycle_admission,
        ) {
            (false, Some(admission)) => Some(Arc::new(KernelNomiLifecycleInvoker {
                compiled: Arc::clone(&compiled),
                active: Arc::clone(&active),
                activation_gate: Arc::clone(&activation_gate),
                owner: owner.clone(),
                agent_session_id: agent_session_id.clone(),
                state_scope_key: state_scope_key.clone(),
                admission,
                identities: deferred_lifecycle_actions
                    .iter()
                    .map(|action| {
                        (
                            action.capability_id().clone(),
                            action.identity.clone(),
                        )
                    })
                    .collect(),
                context_cells: lifecycle_context_cells,
            })),
            _ => None,
        };
        let mut session = NomiPluginToolSession::new(
            compiled.snapshot_ref().clone(),
            actions,
            invoker,
        )?;
        session.initial_context_contributions =
            Arc::from(initial_context_contributions);
        session.deferred_context_actions = Arc::from(deferred_context_actions);
        session.context_invoker = context_invoker;
        session.deferred_lifecycle_actions =
            Arc::from(deferred_lifecycle_actions);
        session.lifecycle_invoker = lifecycle_invoker;
        session.capability_state = Some(active);
        session.target_resource_bindings =
            Arc::from(compiled.target_resource_bindings.clone());
        for contributor in lifecycle_context_contributors {
            session = session.with_context_contributor(contributor);
        }
        Ok(session)
    }

    /// Materialize the MiniApp portion of the same frozen Nomi Tool session.
    ///
    /// MiniApp capabilities are intentionally not looked up in the Kernel
    /// Plugin Registry. Their exact release/provenance projection is already
    /// frozen in the Snapshot and schema bytes come from the owner-scoped
    /// MiniApp release resolver.
    #[allow(clippy::too_many_arguments)]
    pub async fn materialize_miniapp_actions(
        compiled: &CompiledSnapshot,
        owner: &PrincipalRef,
        agent_session_id: &AgentSessionId,
        state_scope_key: &ScopeKey,
        schema_resolver: Arc<dyn NomiMiniAppToolSchemaResolver>,
    ) -> Result<Vec<NomiMiniAppToolAction>, NomiPluginToolError> {
        validate_session_identity(
            compiled,
            owner,
            agent_session_id,
            state_scope_key,
        )?;
        let initial = compiled
            .content()
            .initial_miniapp_capabilities
            .iter()
            .map(|capability| capability.capability.id.clone())
            .collect::<BTreeSet<_>>();
        let on_demand = compiled
            .content()
            .on_demand_miniapp_capabilities
            .iter()
            .map(|capability| capability.capability.id.clone())
            .collect::<BTreeSet<_>>();
        let mut actions = Vec::new();
        for resolved in compiled
            .content()
            .initial_miniapp_capabilities
            .iter()
            .chain(&compiled.content().on_demand_miniapp_capabilities)
        {
            resolved
                .validate()
                .map_err(|error| NomiPluginToolError::Contract(error.message))?;
            let deferred = if initial.contains(&resolved.capability.id) {
                false
            } else if on_demand.contains(&resolved.capability.id) {
                true
            } else {
                return Err(NomiPluginToolError::Contract(format!(
                    "MiniApp capability {} is in neither Snapshot set",
                    resolved.capability.id.as_ref()
                )));
            };
            for action in &resolved.actions {
                if (!resolved.action_allowlist.is_empty()
                    && !resolved.action_allowlist.contains(&action.action_id))
                    || action.presentation != ToolPresentationKind::FunctionTool
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
                actions.push(build_miniapp_action(
                    compiled.snapshot_ref().clone(),
                    resolved.clone(),
                    action.clone(),
                    resolved.display_name.clone(),
                    resolved.description.clone(),
                    input_schema,
                    deferred,
                )?);
            }
        }
        Ok(actions)
    }
}

#[allow(clippy::too_many_arguments)]
async fn assemble_initial_platform_builtin_context(
    kernel: &Arc<KernelRegistry>,
    compiled: &CompiledSnapshot,
    active: &nomifun_agent_kernel::ActiveCapabilitySetSnapshot,
    registry: &MaterializedRegistry,
    owner: &PrincipalRef,
    agent_session_id: &AgentSessionId,
    state_scope_key: &ScopeKey,
    admission: &NomiPlatformBuiltinContextAdmission,
) -> Result<Vec<NomiInitialContextContribution>, NomiPluginToolError> {
    let mut contributions = Vec::new();
    for resolved in &compiled.content().initial_capabilities {
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
        if current.manifest.kind != CapabilityKind::ContextContributor {
            return Err(NomiPluginToolError::Contract(format!(
                "approved initial context {} is no longer a ContextContributor",
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
        let operation_id = OperationId::from(format!(
            "nomi-context:{}:{}:{}",
            agent_session_id.as_ref(),
            compiled.snapshot_ref().snapshot_id.as_ref(),
            capability_id.as_ref()
        ));
        let result = tokio::time::timeout(
            Duration::from_secs(5),
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
                "initial ContextContributor {} exceeded its 5 second deadline",
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
    contributions.sort_by(|left, right| {
        left.capability_id.cmp(&right.capability_id)
    });
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
    Ok(contributions)
}

fn materialize_deferred_platform_builtin_context(
    compiled: &CompiledSnapshot,
    registry: &MaterializedRegistry,
    admission: &NomiPlatformBuiltinContextAdmission,
) -> Result<Vec<NomiDeferredContextAction>, NomiPluginToolError> {
    let mut actions = Vec::new();
    for resolved in &compiled.content().on_demand_capabilities {
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
        let [context_schema_ref] =
            current.manifest.contributions.context_schema_refs.as_slice()
        else {
            return Err(NomiPluginToolError::Contract(format!(
                "on-demand ContextContributor {} must own one canonical context schema",
                resolved.capability.id.as_ref()
            )));
        };
        let identity = NomiDeferredContextIdentity {
            resolved_snapshot_ref: compiled.snapshot_ref().clone(),
            resolved_capability: resolved.clone(),
            context_schema_ref: context_schema_ref.clone(),
        };
        let canonical_identity = canonical_json_bytes(&identity).map_err(
            |error| {
                NomiPluginToolError::Contract(format!(
                    "deferred ContextContributor identity could not be encoded: {error}"
                ))
            },
        )?;
        let activation_identity =
            String::from_utf8(canonical_identity.clone()).map_err(|error| {
                NomiPluginToolError::Contract(format!(
                    "deferred ContextContributor identity is not UTF-8: {error}"
                ))
            })?;
        let provider_name = provider_tool_name(
            resolved.capability.id.as_ref(),
            "context.activate",
            &canonical_identity,
        );
        actions.push(NomiDeferredContextAction {
            provider_name,
            activation_identity,
            description: format!(
                "Activate {} and return its structured context for this turn.",
                current.manifest.display.name
            ),
            identity,
        });
    }
    actions.sort_by(|left, right| {
        (left.capability_id(), left.provider_name()).cmp(&(
            right.capability_id(),
            right.provider_name(),
        ))
    });
    let mut provider_names = BTreeSet::new();
    let mut activation_identities = BTreeSet::new();
    for action in &actions {
        if !provider_names.insert(action.provider_name.clone())
            || !activation_identities.insert(action.activation_identity.clone())
        {
            return Err(NomiPluginToolError::Contract(format!(
                "deferred ContextContributor {} has a duplicate Nomi activation route",
                action.capability_id().as_ref()
            )));
        }
    }
    Ok(actions)
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
    for resolved in &compiled.content().initial_capabilities {
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
) -> Result<Vec<NomiDeferredLifecycleIdentity>, NomiPluginToolError> {
    let mut identities = Vec::new();
    for resolved in compiled
        .content()
        .initial_capabilities
        .iter()
        .chain(&compiled.content().on_demand_capabilities)
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
        identities.push(NomiDeferredLifecycleIdentity {
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

#[allow(clippy::too_many_arguments)]
async fn lifecycle_context_contributors(
    compiled: &CompiledSnapshot,
    registry: &MaterializedRegistry,
    owner: &PrincipalRef,
    agent_session_id: &AgentSessionId,
    state_scope_key: &ScopeKey,
    admission: &Arc<NomiPlatformBuiltinLifecycleAdmission>,
    active: &Arc<SessionCapabilityState>,
) -> Result<
    (
        Vec<Arc<dyn ContextContributor>>,
        BTreeMap<CapabilityId, LifecycleContextCell>,
    ),
    NomiPluginToolError,
> {
    let mut contributors = Vec::new();
    let mut context_cells = BTreeMap::new();
    let initial_ids = compiled
        .content()
        .initial_capabilities
        .iter()
        .map(|capability| capability.capability.id.clone())
        .collect::<BTreeSet<_>>();
    for resolved in compiled
        .content()
        .initial_capabilities
        .iter()
        .chain(&compiled.content().on_demand_capabilities)
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
        let operation_id = OperationId::from(format!(
            "nomi-lifecycle-prepare:{}:{}",
            agent_session_id.as_ref(),
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
        let inner = Arc::new(tokio::sync::OnceCell::new());
        if initial_ids.contains(&resolved.capability.id) {
            let prepared = admission
                .invoker
                .context_contributor(invocation.clone())
                .await
                .map_err(NomiPluginToolError::Contract)?;
            inner.set(prepared).map_err(|_| {
                NomiPluginToolError::Contract(format!(
                    "initial lifecycle context for {} was prepared more than once",
                    resolved.capability.id.as_ref()
                ))
            })?;
        }
        context_cells.insert(resolved.capability.id.clone(), Arc::clone(&inner));
        contributors.push(Arc::new(GatedLifecycleContextContributor {
            capability_id: resolved.capability.id.clone(),
            active: Arc::clone(active),
            inner,
        }) as Arc<dyn ContextContributor>);
    }
    Ok((contributors, context_cells))
}

struct GatedLifecycleContextContributor {
    capability_id: CapabilityId,
    active: Arc<SessionCapabilityState>,
    inner: LifecycleContextCell,
}

#[async_trait]
impl ContextContributor for GatedLifecycleContextContributor {
    async fn pre_turn_context(&self) -> Option<String> {
        self.pre_turn_context_result(None).await.ok().flatten()
    }

    async fn pre_turn_context_for_turn(
        &self,
        turn: &TurnContext,
    ) -> Option<String> {
        self.pre_turn_context_result(Some(turn))
            .await
            .ok()
            .flatten()
    }

    async fn pre_turn_context_for_turn_result(
        &self,
        turn: &TurnContext,
    ) -> Result<Option<String>, String> {
        self.pre_turn_context_result(Some(turn)).await
    }

    fn label(&self) -> &str {
        "nomifun_lifecycle_context"
    }
}

impl GatedLifecycleContextContributor {
    async fn pre_turn_context_result(
        &self,
        turn: Option<&TurnContext>,
    ) -> Result<Option<String>, String> {
        let active = self
            .active
            .snapshot()
            .map_err(|_| "LIFECYCLE_CAPABILITY_STATE_UNAVAILABLE".to_owned())?;
        if !active.active.contains(&self.capability_id) {
            return Ok(None);
        }
        let prepared = self.inner.get().ok_or_else(|| {
            "LIFECYCLE_CONTEXT_NOT_PREPARED".to_owned()
        })?;
        let Some(inner) = prepared.as_ref() else {
            return Ok(None);
        };
        match turn {
            Some(turn) => inner.pre_turn_context_for_turn_result(turn).await,
            None => Ok(inner.pre_turn_context().await),
        }
    }
}

fn materialize_deferred_platform_builtin_lifecycle(
    compiled: &CompiledSnapshot,
    registry: &MaterializedRegistry,
    admission: &NomiPlatformBuiltinLifecycleAdmission,
) -> Result<Vec<NomiDeferredLifecycleAction>, NomiPluginToolError> {
    let mut actions = Vec::new();
    for resolved in &compiled.content().on_demand_capabilities {
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
        let identity = NomiDeferredLifecycleIdentity {
            resolved_snapshot_ref: compiled.snapshot_ref().clone(),
            resolved_capability: resolved.clone(),
            schema_ref: lifecycle_schema_ref(&current.manifest)?,
        };
        let canonical_identity = canonical_json_bytes(&identity).map_err(
            |error| {
                NomiPluginToolError::Contract(format!(
                    "deferred lifecycle identity could not be encoded: {error}"
                ))
            },
        )?;
        let activation_identity =
            String::from_utf8(canonical_identity.clone()).map_err(|error| {
                NomiPluginToolError::Contract(format!(
                    "deferred lifecycle identity is not UTF-8: {error}"
                ))
            })?;
        actions.push(NomiDeferredLifecycleAction {
            provider_name: provider_tool_name(
                resolved.capability.id.as_ref(),
                "lifecycle.activate",
                &canonical_identity,
            ),
            activation_identity,
            description: format!(
                "Activate {} for this Agent Session and return its host receipt.",
                current.manifest.display.name
            ),
            identity,
        });
    }
    actions.sort_by(|left, right| {
        (left.capability_id(), left.provider_name()).cmp(&(
            right.capability_id(),
            right.provider_name(),
        ))
    });
    Ok(actions)
}

#[allow(clippy::too_many_arguments)]
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

struct KernelNomiLifecycleInvoker {
    compiled: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    activation_gate: Arc<tokio::sync::Mutex<()>>,
    owner: PrincipalRef,
    agent_session_id: AgentSessionId,
    state_scope_key: ScopeKey,
    admission: Arc<NomiPlatformBuiltinLifecycleAdmission>,
    identities: BTreeMap<CapabilityId, NomiDeferredLifecycleIdentity>,
    context_cells: BTreeMap<CapabilityId, LifecycleContextCell>,
}

type LifecycleContextCell =
    Arc<tokio::sync::OnceCell<Option<Arc<dyn ContextContributor>>>>;

struct NomiLifecycleContextContributor {
    compiled: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    owner: PrincipalRef,
    agent_session_id: AgentSessionId,
    state_scope_key: ScopeKey,
    admission: Arc<NomiPlatformBuiltinLifecycleAdmission>,
    identities: Arc<[NomiDeferredLifecycleIdentity]>,
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

impl KernelNomiLifecycleInvoker {
    async fn invoke(
        &self,
        identity: &NomiDeferredLifecycleIdentity,
        operation_id: OperationId,
        activation_proven: bool,
    ) -> Result<StrictJsonValue, NomiPluginToolError> {
        let capability_id = identity.resolved_capability.capability.id.clone();
        if self.identities.get(&capability_id) != Some(identity)
            || !activation_proven
        {
            return Err(KernelError::CapabilityNotActive {
                capability_id,
            }
            .into());
        }
        let invocation = lifecycle_invocation(
            &self.compiled,
            &self.owner,
            &self.agent_session_id,
            &self.state_scope_key,
            &identity.resolved_capability,
            identity.schema_ref.clone(),
            operation_id.clone(),
        )?;
        let active = self.active.snapshot()?;
        if !active.active.contains(&capability_id) {
            let _guard = self.activation_gate.lock().await;
            let current = self.active.snapshot()?;
            if !current.active.contains(&capability_id) {
                let prepared = if let Some(cell) = self.context_cells.get(&capability_id) {
                    if cell.get().is_none() {
                        Some((
                            Arc::clone(cell),
                            tokio::time::timeout(
                                Duration::from_secs(5),
                                self.admission
                                    .invoker
                                    .context_contributor(invocation.clone()),
                            )
                            .await
                            .map_err(|_| {
                                NomiPluginToolError::Contract(format!(
                                    "on-demand lifecycle context for {} exceeded its 5 second deadline",
                                    capability_id.as_ref()
                                ))
                            })?
                            .map_err(|_| {
                                NomiPluginToolError::Contract(format!(
                                    "on-demand lifecycle context for {} could not be prepared",
                                    capability_id.as_ref()
                                ))
                            })?,
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let result = tokio::time::timeout(
                    Duration::from_secs(5),
                    self.admission.invoker.activate(invocation),
                )
                .await
                .map_err(|_| {
                    NomiPluginToolError::Contract(format!(
                        "on-demand lifecycle capability {} exceeded its 5 second deadline",
                        capability_id.as_ref()
                    ))
                })?
                .map_err(|_| {
                    NomiPluginToolError::Contract(format!(
                        "on-demand lifecycle capability {} was rejected by its host",
                        capability_id.as_ref()
                    ))
                })?;
                if let Some((cell, prepared)) = prepared {
                    cell.set(prepared).map_err(|_| {
                        NomiPluginToolError::Contract(format!(
                            "on-demand lifecycle context for {} was prepared more than once",
                            capability_id.as_ref()
                        ))
                    })?;
                }
                self.active.activate_at_boundary(
                    current.generation,
                    &capability_id,
                    CompletedTurnBoundary::committed(operation_id.clone()),
                )?;
                return Ok(result);
            }
        }
        tokio::time::timeout(
            Duration::from_secs(5),
            self.admission.invoker.activate(invocation),
        )
        .await
        .map_err(|_| {
            NomiPluginToolError::Contract(format!(
                "on-demand lifecycle capability {} exceeded its 5 second deadline",
                capability_id.as_ref()
            ))
        })?
        .map_err(|_| {
            NomiPluginToolError::Contract(format!(
                "on-demand lifecycle capability {} was rejected by its host",
                capability_id.as_ref()
            ))
        })
    }
}

struct KernelNomiContextInvoker {
    kernel: Arc<KernelRegistry>,
    compiled: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    activation_gate: Arc<tokio::sync::Mutex<()>>,
    owner: PrincipalRef,
    agent_session_id: AgentSessionId,
    state_scope_key: ScopeKey,
    identities: BTreeMap<CapabilityId, NomiDeferredContextIdentity>,
}

impl KernelNomiContextInvoker {
    async fn invoke(
        &self,
        identity: &NomiDeferredContextIdentity,
        operation_id: OperationId,
        deferred_activation_proven: bool,
    ) -> Result<StrictJsonValue, NomiPluginToolError> {
        let capability_id = identity.resolved_capability.capability.id.clone();
        if self.identities.get(&capability_id) != Some(identity) {
            return Err(NomiPluginToolError::Contract(
                "ContextContributor invocation identity differs from the materialized session"
                    .to_owned(),
            ));
        }
        if !deferred_activation_proven {
            return Err(KernelError::CapabilityNotActive {
                capability_id,
            }
            .into());
        }

        let mut active = self.active.snapshot()?;
        if !active.active.contains(&capability_id) {
            let _activation = self.activation_gate.lock().await;
            active = self.active.snapshot()?;
            if !active.active.contains(&capability_id) {
                self.active.activate_at_boundary(
                    active.generation,
                    &capability_id,
                    CompletedTurnBoundary::committed(operation_id.clone()),
                )?;
                active = self.active.snapshot()?;
            }
        }
        let policy = self.compiled.policy(&capability_id).ok_or_else(|| {
            NomiPluginToolError::Contract(format!(
                "compiled Snapshot has no authority policy for ContextContributor {}",
                capability_id.as_ref()
            ))
        })?;
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            self.kernel.contribute_context(
                &self.compiled,
                &active,
                CapabilityAccessRequest {
                    principal: self.owner.clone(),
                    session_owner: self.owner.clone(),
                    agent_session_id: self.agent_session_id.clone(),
                    operation_id: operation_id.clone(),
                    correlation_id: CorrelationId::from(format!(
                        "{}:context",
                        operation_id.as_ref()
                    )),
                    resolved_snapshot_ref: self.compiled.snapshot_ref().clone(),
                    active_set_generation: active.generation,
                    capability_id: capability_id.clone(),
                    resource_binding_ids: policy.resource_binding_ids.clone(),
                    state_scope_key: self.state_scope_key.clone(),
                },
            ),
        )
        .await
        .map_err(|_| {
            NomiPluginToolError::Contract(format!(
                "on-demand ContextContributor {} exceeded its 5 second deadline",
                capability_id.as_ref()
            ))
        })??;
        let output = StrictJsonValue(serde_json::json!({
            "capability_id": capability_id,
            "context": result.value.map(|value| value.0),
        }));
        let bytes = canonical_json_bytes(&output.0).map_err(|error| {
            NomiPluginToolError::Contract(format!(
                "on-demand capability context could not be encoded: {error}"
            ))
        })?;
        if bytes.len() > MAX_INITIAL_CAPABILITY_CONTEXT_BYTES {
            return Err(NomiPluginToolError::Contract(format!(
                "on-demand capability context exceeds the {MAX_INITIAL_CAPABILITY_CONTEXT_BYTES}-byte Nomi Tool result limit"
            )));
        }
        Ok(output)
    }
}

struct KernelNomiPluginToolInvoker {
    kernel: Arc<KernelRegistry>,
    compiled: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    activation_gate: Arc<tokio::sync::Mutex<()>>,
    owner: PrincipalRef,
    agent_session_id: AgentSessionId,
    state_scope_key: ScopeKey,
    identities:
        BTreeMap<(CapabilityId, ActionId), NomiPluginToolActionIdentity>,
}

#[async_trait]
impl NomiPluginToolInvoker for KernelNomiPluginToolInvoker {
    async fn invoke(
        &self,
        request: NomiPluginToolInvocation,
    ) -> Result<StrictJsonValue, NomiPluginToolError> {
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
        let is_deferred = self
            .compiled
            .content()
            .on_demand_capabilities
            .iter()
            .any(|capability| capability.capability.id == key.0);
        if is_deferred && !request.deferred_activation_proven {
            return Err(KernelError::CapabilityNotActive {
                capability_id: key.0,
            }
            .into());
        }

        let mut active = self.active.snapshot()?;
        if !active.active.contains(&key.0) {
            let _activation = self.activation_gate.lock().await;
            active = self.active.snapshot()?;
            if !active.active.contains(&key.0) {
                self.active.activate_at_boundary(
                    active.generation,
                    &key.0,
                    CompletedTurnBoundary::committed(
                        request.operation_id.clone(),
                    ),
                )?;
                active = self.active.snapshot()?;
            }
        }
        let policy = self.compiled.policy(&key.0).ok_or_else(|| {
            NomiPluginToolError::Contract(format!(
                "compiled Snapshot lost authority policy for {}",
                key.0.as_ref()
            ))
        })?;
        self.kernel
            .invoke(
                &self.compiled,
                &active,
                CapabilityInvocationRequest {
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
                },
            )
            .await
            .map_err(Into::into)
    }
}

struct NomiPluginTool {
    action: NomiPluginToolAction,
    invoker: Arc<dyn NomiPluginToolInvoker>,
    deferred_state: DeferredToolState,
}

struct NomiMiniAppTool {
    action: NomiMiniAppToolAction,
    invoker: Arc<dyn NomiMiniAppToolInvoker>,
    deferred_state: DeferredToolState,
}

struct NomiDeferredContextTool {
    action: NomiDeferredContextAction,
    invoker: Arc<KernelNomiContextInvoker>,
    deferred_state: DeferredToolState,
}

struct NomiDeferredLifecycleTool {
    action: NomiDeferredLifecycleAction,
    invoker: Arc<KernelNomiLifecycleInvoker>,
    deferred_state: DeferredToolState,
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
        matches!(
            self.action.descriptor.effect_class,
            EffectClass::Pure | EffectClass::ReadLocal | EffectClass::ReadSensitive
        )
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
impl Tool for NomiDeferredLifecycleTool {
    fn name(&self) -> &str {
        &self.action.provider_name
    }

    fn activation_identity(&self) -> &str {
        &self.action.activation_identity
    }

    fn artifact_identity(&self) -> &str {
        self.action.capability_id().as_ref()
    }

    fn deferred_search_aliases(&self) -> Vec<String> {
        vec![self.action.capability_id().as_ref().to_owned()]
    }

    fn description(&self) -> &str {
        &self.action.description
    }

    fn input_schema(&self) -> JsonSchema {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn is_deferred(&self) -> bool {
        true
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        ToolResult::error(
            "lifecycle activation requires an engine-owned execution context",
        )
    }

    async fn execute_with_context(
        &self,
        _input: Value,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        let activation_proven = self
            .deferred_state
            .is_activated(&self.action.activation_identity);
        if !activation_proven {
            return ToolResult::error(format!(
                "lifecycle capability '{}' is deferred; activate it through ToolSearch before invoking it",
                self.action.provider_name
            ));
        }
        let operation_id = OperationId::from(format!(
            "nomi-lifecycle:{}",
            context.operation_id()
        ));
        match self
            .invoker
            .invoke(&self.action.identity, operation_id, activation_proven)
            .await
        {
            Ok(output) => match serde_json::to_string_pretty(&output.0) {
                Ok(content) => ToolResult::text(content),
                Err(error) => ToolResult::error(format!(
                    "lifecycle receipt could not be serialized: {error}"
                )),
            },
            Err(error) => model_safe_tool_error(&error),
        }
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
}

#[async_trait]
impl Tool for NomiDeferredContextTool {
    fn name(&self) -> &str {
        &self.action.provider_name
    }

    fn activation_identity(&self) -> &str {
        &self.action.activation_identity
    }

    fn artifact_identity(&self) -> &str {
        self.action.capability_id().as_ref()
    }

    fn deferred_search_aliases(&self) -> Vec<String> {
        vec![
            self.action.capability_id().as_ref().to_owned(),
            "context".to_owned(),
        ]
    }

    fn description(&self) -> &str {
        &self.action.description
    }

    fn input_schema(&self) -> JsonSchema {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn is_deferred(&self) -> bool {
        true
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        ToolResult::error(
            "ContextContributor activation requires an engine-owned execution context",
        )
    }

    async fn execute_with_context(
        &self,
        _input: Value,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        let deferred_activation_proven = self
            .deferred_state
            .is_activated(&self.action.activation_identity);
        if !deferred_activation_proven {
            return ToolResult::error(format!(
                "ContextContributor '{}' is deferred; activate it through ToolSearch before invoking it",
                self.action.provider_name
            ));
        }
        let operation_id = OperationId::from(format!(
            "nomi-context:{}",
            context.operation_id()
        ));
        match self
            .invoker
            .invoke(
                &self.action.identity,
                operation_id,
                deferred_activation_proven,
            )
            .await
        {
            Ok(output) => match serde_json::to_string_pretty(&output.0) {
                Ok(content) => ToolResult::text(content),
                Err(error) => ToolResult::error(format!(
                    "ContextContributor output could not be serialized: {error}"
                )),
            },
            Err(error) => model_safe_tool_error(&error),
        }
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
}

#[async_trait]
impl Tool for NomiMiniAppTool {
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
        matches!(
            self.action.identity.action.effect_class,
            EffectClass::Pure
                | EffectClass::ReadLocal
                | EffectClass::ReadSensitive
        )
    }

    fn is_deferred(&self) -> bool {
        self.action.deferred
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        ToolResult::error(
            "MiniApp Tool invocation requires an engine-owned execution context",
        )
    }

    async fn execute_with_context(
        &self,
        input: Value,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        let deferred_activation_proven = !self.action.deferred
            || self
                .deferred_state
                .is_activated(&self.action.activation_identity);
        if !deferred_activation_proven {
            return ToolResult::error(format!(
                "MiniApp Tool '{}' is deferred; activate it through ToolSearch before invoking it",
                self.action.provider_name
            ));
        }
        let operation_identity = context.operation_id();
        let operation_id =
            OperationId::from(format!("nomi-miniapp:{operation_identity}"));
        let request = NomiMiniAppToolInvocation {
            identity: self.action.identity.clone(),
            idempotency_key: IdempotencyKey::from(format!(
                "nomi-miniapp:{operation_identity}"
            )),
            correlation_id: CorrelationId::from(format!(
                "nomi-miniapp:{operation_identity}"
            )),
            operation_id,
            deferred_activation_proven,
            input: StrictJsonValue(input),
        };
        match self.invoker.invoke(request).await {
            Ok(output) => match serde_json::to_string_pretty(&output.0) {
                Ok(content) => ToolResult::text(content),
                Err(error) => ToolResult::error(format!(
                    "MiniApp Tool output could not be serialized: {error}"
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
        matches!(
            self.action.identity.action.effect_class,
            EffectClass::Pure
                | EffectClass::ReadLocal
                | EffectClass::ReadSensitive
        )
    }

    fn is_deferred(&self) -> bool {
        self.action.deferred
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
        let deferred_activation_proven = !self.action.deferred
            || self
                .deferred_state
                .is_activated(&self.action.activation_identity);
        if !deferred_activation_proven {
            return ToolResult::error(format!(
                "Plugin Tool '{}' is deferred; activate it through ToolSearch before invoking it",
                self.action.provider_name
            ));
        }
        let operation_identity = context.operation_id();
        let request = NomiPluginToolInvocation {
            identity: self.action.identity.clone(),
            operation_id: OperationId::from(format!(
                "nomi-plugin:{operation_identity}"
            )),
            idempotency_key: IdempotencyKey::from(format!(
                "nomi-plugin:{operation_identity}"
            )),
            correlation_id: CorrelationId::from(format!(
                "nomi-plugin:{operation_identity}"
            )),
            deferred_activation_proven,
            input: StrictJsonValue(input),
        };
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
    deferred: bool,
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
        deferred,
    })
}

fn build_miniapp_action(
    resolved_snapshot_ref: ResolvedSnapshotRef,
    resolved_capability: ResolvedMiniAppCapability,
    action: CapabilityActionDescriptor,
    display_name: String,
    description: String,
    input_schema: StrictJsonValue,
    deferred: bool,
) -> Result<NomiMiniAppToolAction, NomiPluginToolError> {
    let input_schema_digest =
        validate_canonical_input_schema(&action.input_schema, &input_schema)?;
    let identity = NomiMiniAppToolActionIdentity {
        resolved_snapshot_ref,
        resolved_capability,
        action,
        input_schema_digest,
    };
    let canonical_identity = canonical_json_bytes(&identity).map_err(|error| {
        NomiPluginToolError::Contract(format!(
            "MiniApp Tool activation identity could not be encoded: {error}"
        ))
    })?;
    let activation_identity =
        String::from_utf8(canonical_identity.clone()).map_err(|error| {
            NomiPluginToolError::Contract(format!(
                "MiniApp Tool activation identity is not UTF-8: {error}"
            ))
        })?;
    let provider_name = provider_tool_name_with_prefix(
        "miniapp__",
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
    Ok(NomiMiniAppToolAction {
        provider_name,
        activation_identity,
        artifact_identity,
        description,
        input_schema,
        identity,
        deferred,
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

fn validate_exact_target(
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
        || current.mount_id != resolved.resolved_mount_id
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
