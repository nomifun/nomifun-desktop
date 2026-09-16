//! Exact ToolSearch policy contract and Kernel adapter. This is a hidden
//! capability action, not another ToolSearch route or a second registry.
use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use nomi_tools::tool_search::{ToolDiscoveryInput, ToolDiscoveryPolicy, rank_tool_discovery};
use nomifun_agent_contracts::*;
use nomifun_agent_kernel::{
    CapabilityHandler, CapabilityInvocationContext, CapabilityInvocationRequest, CompiledSnapshot,
    KernelError, KernelRegistry, MaterializedRegistry, SessionCapabilityState,
};
use serde::Deserialize;
use serde_json::json;

pub const CAPABILITY_ID: &str = "agent.tool-discovery";
pub const ROLE_ID: &str = "system.tool-discovery";
pub const PACKAGE_ID: &str = "nomifun.tool-discovery";
pub const ACTION_ID: &str = "tool.discovery.rank";

pub fn schemas() -> BTreeMap<CanonicalSchemaRef, StrictJsonValue> {
    [("input", input_schema()), ("output", output_schema())]
        .into_iter()
        .map(|(name, value)| (schema_ref(name, &value), StrictJsonValue(value)))
        .collect()
}

fn input_schema() -> serde_json::Value {
    json!({"type":"object","additionalProperties":false,
    "required":["query","candidates","limit"],"properties":{
        "query":{"type":"string","minLength":1,"maxLength":4096},
        "limit":{"type":"integer","minimum":1,"maximum":5},
        "candidates":{"type":"array","items":{"type":"object","additionalProperties":false,
            "required":["name","description","aliases"],"properties":{
                "name":{"type":"string","minLength":1},"description":{"type":"string"},
                "aliases":{"type":"array","items":{"type":"string"}}
            }}}
    }})
}

fn output_schema() -> serde_json::Value {
    json!({"type":"object","additionalProperties":false,"required":["names"],"properties":{
        "names":{"type":"array","maxItems":5,"uniqueItems":true,"items":{"type":"string","minLength":1}}
    }})
}

fn schema_ref(name: &str, schema: &serde_json::Value) -> CanonicalSchemaRef {
    format!(
        "schema://nomifun/tool-discovery/{name}@1#{}",
        digest_payload(schema)
            .expect("static discovery schema")
            .as_ref()
    )
    .into()
}

pub fn action() -> CapabilityActionDescriptor {
    CapabilityActionDescriptor {
        action_id: ACTION_ID.into(),
        input_schema: schema_ref("input", &input_schema()),
        output_schema: schema_ref("output", &output_schema()),
        effect_class: EffectClass::Pure,
        presentation: ToolPresentationKind::Hidden,
    }
}

pub fn supports(manifest: &CapabilityManifest) -> bool {
    manifest.supports_consumer(CapabilityConsumer::Agent)
        && manifest.contributions.actions == [action()]
}

/// The bundled implementation shares the algorithm with standalone Nomi.
pub struct BuiltinDiscovery;

#[async_trait]
impl CapabilityHandler for BuiltinDiscovery {
    async fn invoke(
        &self,
        _: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError> {
        let input: ToolDiscoveryInput =
            serde_json::from_value(input.0).map_err(|error| KernelError::CapabilityExecution {
                reason: error.to_string(),
            })?;
        let query = input.query.trim();
        let low_information = query.chars().filter(|ch| ch.is_alphanumeric()).count() < 3;
        let exact = input.candidates.iter().any(|candidate| {
            candidate.name.eq_ignore_ascii_case(query)
                || candidate
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(query))
        });
        if !(1..=5).contains(&input.limit) || query.is_empty() || (low_information && !exact) {
            return Err(KernelError::CapabilityExecution {
                reason: "invalid discovery input".into(),
            });
        }
        Ok(StrictJsonValue(
            json!({"names":rank_tool_discovery(&input)}),
        ))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    names: Vec<String>,
}

pub(crate) fn decode_selection(value: StrictJsonValue) -> Result<Vec<String>, String> {
    let output: Selection = serde_json::from_value(value.0).map_err(|e| e.to_string())?;
    Ok(output.names)
}

/// Assembly may still need the existing Product adapter. This is not an
/// executable fallback: register_into rejects an unbound Product selection.
#[derive(Clone)]
pub(crate) enum DiscoveryBinding {
    Ready(CapabilityId, Arc<dyn ToolDiscoveryPolicy>),
    Product(ResolvedCapability),
}

impl DiscoveryBinding {
    pub(crate) fn id(&self) -> &CapabilityId {
        match self {
            Self::Ready(id, _) => id,
            Self::Product(resolved) => &resolved.capability.id,
        }
    }

    pub(crate) fn policy(
        &self,
    ) -> Result<Arc<dyn ToolDiscoveryPolicy>, crate::NomiPluginToolError> {
        match self {
            Self::Ready(_, policy) => Ok(policy.clone()),
            Self::Product(_) => Err(crate::NomiPluginToolError::Contract(
                "Selected Product discovery policy has no execution adapter".into(),
            )),
        }
    }
}

/// Shared by runtime assembly and the host's preview/save validation. It only
/// validates the canonical selection; it never resolves or rewrites a plan.
pub fn validate_selection(
    registry: &MaterializedRegistry,
    content: &ResolvedSnapshotContent,
) -> Result<(), crate::NomiPluginToolError> {
    selected_capability(registry, content).map(|_| ())
}

fn selected_capability<'a>(
    registry: &MaterializedRegistry,
    content: &'a ResolvedSnapshotContent,
) -> Result<Option<&'a ResolvedCapability>, crate::NomiPluginToolError> {
    let mut selected = None;
    for resolved in content.contributions() {
        let actions = if resolved.contribution_lock.source_kind
            == ContributionSourceKind::PluginProductActiveRelease
        {
            resolved
                .validate()
                .map_err(|e| crate::NomiPluginToolError::Contract(e.message))?;
            &resolved.actions
        } else {
            let current = registry
                .capability(&resolved.capability.id)
                .ok_or_else(|| KernelError::CapabilityNotMaterialized {
                    capability_id: resolved.capability.id.clone(),
                    version: resolved.capability.version.clone(),
                })?;
            crate::plugin_tools::validate_exact_target(resolved, current)?;
            if current.manifest.contributions.actions == [action()] && !supports(&current.manifest)
            {
                return Err(crate::NomiPluginToolError::Contract(
                    "Discovery capability must publish the exact Agent discovery Action".into(),
                ));
            }
            &current.manifest.contributions.actions
        };
        if !actions.iter().any(|a| {
            a.action_id.as_ref() == ACTION_ID && a.presentation == ToolPresentationKind::Hidden
        }) {
            continue;
        }
        if actions != &[action()]
            || (!resolved.action_allowlist.is_empty()
                && !resolved
                    .action_allowlist
                    .contains(&ActionId::from(ACTION_ID)))
        {
            return Err(crate::NomiPluginToolError::Contract(
                "Discovery capability requires the exact hidden rank contract and action authority"
                    .into(),
            ));
        }
        if resolved.contribution_lock.source_kind
            == ContributionSourceKind::PluginProductActiveRelease
            && !resolved.required_resource_kinds.is_empty()
        {
            return Err(crate::NomiPluginToolError::Contract(
                "Product discovery resources require an unavailable binding adapter".into(),
            ));
        }
        if selected.replace(resolved).is_some() {
            return Err(crate::NomiPluginToolError::Contract("Choose one discovery capability or one Role Provider, not multiple discovery policies".into()));
        }
    }
    Ok(selected)
}

pub(crate) struct KernelDiscoveryPolicy {
    kernel: Arc<KernelRegistry>,
    compiled: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    owner: PrincipalRef,
    session: AgentSessionId,
    scope: ScopeKey,
    capability: CapabilityId,
}

impl KernelDiscoveryPolicy {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn materialize(
        kernel: Arc<KernelRegistry>,
        compiled: Arc<CompiledSnapshot>,
        active: Arc<SessionCapabilityState>,
        owner: PrincipalRef,
        session: AgentSessionId,
        scope: ScopeKey,
    ) -> Result<Option<DiscoveryBinding>, crate::NomiPluginToolError> {
        let registry = kernel.snapshot()?;
        let Some(resolved) = selected_capability(&registry, compiled.content())? else {
            return Ok(None);
        };
        if resolved.contribution_lock.source_kind
            == ContributionSourceKind::PluginProductActiveRelease
        {
            return Ok(Some(DiscoveryBinding::Product(resolved.clone())));
        }
        let policy = compiled.policy(&resolved.capability.id).ok_or_else(|| {
            crate::NomiPluginToolError::Contract(
                "Discovery policy has no compiled authority".into(),
            )
        })?;
        if !policy.allowed_actions.contains(&ActionId::from(ACTION_ID)) {
            return Err(crate::NomiPluginToolError::Contract(
                "Selected discovery capability must allow its rank action".into(),
            ));
        }
        let capability = resolved.capability.id.clone();
        Ok(Some(DiscoveryBinding::Ready(
            capability.clone(),
            Arc::new(Self {
                kernel,
                compiled,
                active,
                owner,
                session,
                scope,
                capability,
            }) as Arc<dyn ToolDiscoveryPolicy>,
        )))
    }
}

#[async_trait]
impl ToolDiscoveryPolicy for KernelDiscoveryPolicy {
    async fn select(&self, input: ToolDiscoveryInput) -> Result<Vec<String>, String> {
        let active = self.active.snapshot().map_err(|e| e.to_string())?;
        let policy = self
            .compiled
            .policy(&self.capability)
            .ok_or("Discovery policy lost compiled authority")?;
        let key = format!(
            "nomi-discovery:{}:{}",
            self.session.as_ref(),
            uuid::Uuid::now_v7()
        );
        let result = self
            .kernel
            .invoke_shared(
                self.compiled.clone(),
                &active,
                CapabilityInvocationRequest {
                    principal: self.owner.clone(),
                    session_owner: self.owner.clone(),
                    agent_session_id: self.session.clone(),
                    operation_id: key.clone().into(),
                    idempotency_key: key.clone().into(),
                    correlation_id: key.into(),
                    resolved_snapshot_ref: self.compiled.snapshot_ref().clone(),
                    active_set_generation: active.generation,
                    capability_id: self.capability.clone(),
                    action_id: ACTION_ID.into(),
                    resource_binding_ids: policy.resource_binding_ids.clone(),
                    state_scope_key: self.scope.clone(),
                    input: StrictJsonValue(serde_json::to_value(input).map_err(|e| e.to_string())?),
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        decode_selection(result)
    }
}
