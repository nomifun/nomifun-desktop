//! Host-owned Plugin schemas and canonical Context consumption.
//!
//! The adapter consumes one exact compiled Agent Snapshot and the shared
//! Kernel registry. It does not resolve the latest Catalog, accept model-owned
//! capability identity, or use the non-Agent operation API.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use crate::context_contributor::{ContextContributor, TurnContext};
use nomifun_agent_contracts::{
    AgentSessionId, CapabilityConsumer,
    CapabilityId, CapabilityManifest, CanonicalErrorCode,
    CanonicalSchemaRef, ContributionSourceKind,
    CorrelationId, IdempotencyKey, OperationId,
    PluginSourceKind, PrincipalRef, ResolvedCapability,
    ScopeKey,
    StrictJsonValue, ToolPresentationKind, canonical_json_bytes,
    digest_payload,
};
use nomifun_agent_kernel::{
    CapabilityAccessRequest, CompiledSnapshot,
    KernelError, KernelRegistry,
    MaterializedCapability, MaterializedRegistry, SessionCapabilityState,
};
use serde::Serialize;
use thiserror::Error;

#[path = "plugin_context.rs"]
mod context;
pub use context::NomiTurnContextContributor;

const MAX_INITIAL_CAPABILITY_CONTEXT_BYTES: usize = 64 * 1024;

/// Capability shapes actually consumed by the Nomi managed-Plugin adapter.
/// Catalog availability uses the same predicate as runtime materialization;
/// declaring a kind in a Package alone does not make it executable by Nomi.
/// Source, exact artifact, runtime readiness and authorization are checked by
/// their existing owners, not granted by this shape check.
pub fn supports_nomi_plugin_capability(manifest: &CapabilityManifest) -> bool {
    manifest.supports_consumer(CapabilityConsumer::Agent)
        && (has_function_tool_action(manifest)
            || manifest.contributions.actions == [crate::tool_discovery::action()]
            || manifest.contributions.context_schema_refs.len() == 1)
}

fn has_function_tool_action(manifest: &CapabilityManifest) -> bool {
    manifest
        .contributions
        .actions
        .iter()
        .any(|action| action.presentation == ToolPresentationKind::FunctionTool)
}

#[cfg(test)]
mod kind_invariance_tests {
    use super::*;
    use nomifun_agent_contracts::{CapabilityActionDescriptor, EffectClass};

    fn schema_ref(name: &str) -> CanonicalSchemaRef {
        format!("schema://kind-invariance/{name}@1#{}", "a".repeat(64)).into()
    }

    fn mixed_manifest(kind: nomifun_agent_contracts::CapabilityKind) -> CapabilityManifest {
        CapabilityManifest {
            id: "example.mixed".into(),
            contribution_id: "capability:example.mixed".into(),
            kind,
            package: nomifun_agent_contracts::PackageRef {
                id: "example.package".into(),
                version: "1.0.0".into(),
            },
            display: nomifun_agent_contracts::LocalizedMetadata {
                name: "Mixed module".into(),
                description: "Action, Context, and Event contributions".into(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            requires: Vec::new(),
            conflicts: Vec::new(),
            supported_surfaces:
                nomifun_agent_contracts::capability_module_surface_declarations(
                    ["desktop"],
                    [CapabilityConsumer::Agent],
                    nomifun_agent_contracts::CapabilityAuthoringPolicy::Direct,
                ),
            requires_runtime_features: Vec::new(),
            supported_platforms: vec![nomifun_agent_contracts::PlatformConstraint::Any],
            config_schema: StrictJsonValue(serde_json::json!({
                "type": "object",
                "additionalProperties": false
            })),
            contributions: nomifun_agent_contracts::CapabilityContributions {
                actions: vec![CapabilityActionDescriptor {
                    action_id: "example.mixed/read".into(),
                    input_schema: schema_ref("action-input"),
                    output_schema: schema_ref("action-output"),
                    effect_class: EffectClass::ReadLocal,
                    presentation: ToolPresentationKind::FunctionTool,
                }],
                context_schema_refs: vec![schema_ref("context")],
                event_schema_refs: vec![schema_ref("event")],
                resource_kinds: BTreeSet::from(["example_resource".into()]),
                host_ports: vec![nomifun_agent_contracts::HostPortRef {
                    id: "host.example.mixed".into(),
                    version: "1.0.0".into(),
                }],
                ..Default::default()
            },
        }
    }

    fn presentation_kinds() -> [nomifun_agent_contracts::CapabilityKind; 10] {
        use nomifun_agent_contracts::CapabilityKind as Kind;
        [
            Kind::Tool,
            Kind::ContextContributor,
            Kind::ResourceProvider,
            Kind::EventSource,
            Kind::EventConsumer,
            Kind::TurnMiddleware,
            Kind::Transport,
            Kind::Scheduler,
            Kind::BackgroundService,
            Kind::UiContribution,
        ]
    }

    #[test]
    fn mixed_module_consumers_are_invariant_to_presentation_kind() {
        for kind in presentation_kinds() {
            let manifest = mixed_manifest(kind);
            assert!(supports_nomi_plugin_capability(&manifest));
            assert!(has_function_tool_action(&manifest));
        }
    }

    #[test]
    fn discovery_consumer_ignores_presentation_kind() {
        for kind in presentation_kinds() {
            let mut discovery = mixed_manifest(kind);
            discovery.contributions.actions = vec![crate::tool_discovery::action()];
            assert!(crate::tool_discovery::supports(&discovery));
            assert!(supports_nomi_plugin_capability(&discovery));
        }
    }
}

#[derive(Debug, Error)]
pub enum NomiPluginToolError {
    #[error("Nomi Plugin Tool contract error: {0}")]
    Contract(String),
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
    if !manifest.supports_consumer(CapabilityConsumer::Agent)
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

#[async_trait]
pub trait NomiHostDynamicToolInvoker: Send + Sync {
    async fn invoke(
        &self,
        request: NomiHostDynamicToolInvocation,
    ) -> Result<StrictJsonValue, NomiHostDynamicToolError>;
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

#[allow(clippy::too_many_arguments)]
pub async fn assemble_initial_capability_context(
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
        if !compiled.capability_resources_bound(&resolved.capability.id)? {
            continue;
        }
        let agent_module = resolved.contribution_lock.source_kind
            == ContributionSourceKind::AgentModule
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
        if !agent_module && !approved_builtin {
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
            })?;
        validate_exact_target(resolved, current)?;
        if current.manifest.contributions.context_schema_refs.is_empty() {
            if agent_module {
                // A mixed Module without Context contributions still uses its
                // Action consumers; it never enters the prompt path.
                continue;
            }
            return Err(NomiPluginToolError::Contract(format!(
                "approved initial context {} no longer publishes Context",
                resolved.capability.id.as_ref()
            )));
        }
        if !current.manifest.supports_consumer(CapabilityConsumer::Agent) {
            continue;
        }
        if current.manifest.contributions.context_schema_refs.len() != 1 {
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
                    turn_id: None,
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

/// Encode one already-authorized initial Context set in the exact system-prompt
/// envelope shared by every Agent execution host.
pub fn render_initial_capability_context_section(
    contributions: &[NomiInitialContextContribution],
) -> Result<Option<String>, NomiPluginToolError> {
    if contributions.is_empty() {
        return Ok(None);
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
    let context = String::from_utf8(bytes).map_err(|error| {
        NomiPluginToolError::Contract(format!(
            "initial capability context is not UTF-8: {error}"
        ))
    })?;
    Ok(Some(format!(
        "<nomifun_initial_capability_context format=\"canonical-json\">\n{context}\n</nomifun_initial_capability_context>"
    )))
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
