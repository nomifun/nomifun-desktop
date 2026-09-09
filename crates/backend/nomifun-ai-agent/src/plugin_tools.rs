//! Nomi adapter for ordinary Plugin Tool capabilities.
//!
//! The adapter consumes one exact compiled Agent Snapshot and the shared
//! Kernel registry. It does not resolve the latest Catalog, accept model-owned
//! capability identity, or use the non-Agent operation API.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use nomi_protocol::events::ToolCategory;
use nomi_tools::{
    Tool, ToolExecutionContext,
    registry::{DeferredToolState, ToolRegistry},
};
use nomi_types::tool::{JsonSchema, ToolResult};
use nomifun_agent_contracts::{
    ActionId, AgentSessionId, CapabilityActionDescriptor, CapabilityConsumer,
    CapabilityId, CapabilityKind, CanonicalSchemaRef, CorrelationId,
    DigestHex, EffectClass, IdempotencyKey, OperationId,
    PrincipalRef, ResolvedCapability, ResolvedMiniAppCapability,
    ResolvedSnapshotRef, ScopeKey, StrictJsonValue, ToolPresentationKind,
    canonical_json_bytes, digest_payload,
};
use nomifun_agent_kernel::{
    CapabilityInvocationRequest, CompiledSnapshot, CompletedTurnBoundary,
    KernelError, KernelRegistry, MaterializedCapability,
    SessionCapabilityState,
};
use nomifun_common::AppError;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

const PROVIDER_NAME_PREFIX: &str = "plugin__";
const PROVIDER_NAME_SEPARATOR: &str = "__";
const PROVIDER_NAME_MAX_BYTES: usize = 64;
const PROVIDER_NAME_HASH_HEX_BYTES: usize = 20;

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

/// A complete set of Plugin action tools for one frozen Nomi session.
#[derive(Clone)]
pub struct NomiPluginToolSession {
    resolved_snapshot_ref: ResolvedSnapshotRef,
    actions: Arc<[NomiPluginToolAction]>,
    invoker: Arc<dyn NomiPluginToolInvoker>,
    miniapp_actions: Arc<[NomiMiniAppToolAction]>,
    miniapp_invoker: Option<Arc<dyn NomiMiniAppToolInvoker>>,
}

impl fmt::Debug for NomiPluginToolSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NomiPluginToolSession")
            .field("resolved_snapshot_ref", &self.resolved_snapshot_ref)
            .field("actions", &self.actions)
            .field("miniapp_actions", &self.miniapp_actions)
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
            actions: Arc::from(actions),
            invoker,
            miniapp_actions: Arc::from(Vec::<NomiMiniAppToolAction>::new()),
            miniapp_invoker: None,
        })
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

    pub fn actions(&self) -> &[NomiPluginToolAction] {
        &self.actions
    }

    pub fn miniapp_actions(&self) -> &[NomiMiniAppToolAction] {
        &self.miniapp_actions
    }

    pub fn tool_count(&self) -> usize {
        self.actions.len() + self.miniapp_actions.len()
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
        if self.actions.is_empty() && self.miniapp_actions.is_empty() {
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

        let mut pending = Vec::new();
        for resolved in compiled
            .content()
            .initial_capabilities
            .iter()
            .chain(&compiled.content().on_demand_capabilities)
        {
            if resolved.contribution_lock.source_kind
                != nomifun_agent_contracts::ContributionSourceKind::PluginMount
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
                ));
            }
        }

        let actions = futures_util::future::try_join_all(
            pending.into_iter().map(
                |(resolved, action, display_name, description, deferred)| {
                    let schema_resolver = Arc::clone(&schema_resolver);
                    let snapshot_ref = compiled.snapshot_ref().clone();
                    async move {
                        let input_schema = schema_resolver
                            .resolve(&resolved, &action.input_schema)
                            .await
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
        let invoker = Arc::new(KernelNomiPluginToolInvoker {
            kernel,
            compiled: Arc::clone(&compiled),
            active: SessionCapabilityState::new(&compiled),
            activation_gate: tokio::sync::Mutex::new(()),
            owner,
            agent_session_id,
            state_scope_key,
            identities,
        });
        NomiPluginToolSession::new(
            compiled.snapshot_ref().clone(),
            actions,
            invoker,
        )
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

struct KernelNomiPluginToolInvoker {
    kernel: Arc<KernelRegistry>,
    compiled: Arc<CompiledSnapshot>,
    active: SessionCapabilityState,
    activation_gate: tokio::sync::Mutex<()>,
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

#[async_trait]
impl Tool for NomiMiniAppTool {
    fn name(&self) -> &str {
        &self.action.provider_name
    }

    fn activation_identity(&self) -> &str {
        &self.action.activation_identity
    }

    fn artifact_identity(&self) -> &str {
        &self.action.activation_identity
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
            Err(error) => ToolResult::error(error.to_string()),
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
        &self.action.activation_identity
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
            Err(error) => ToolResult::error(error.to_string()),
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
