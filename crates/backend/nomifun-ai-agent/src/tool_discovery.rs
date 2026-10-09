//! Exact ToolSearch policy contract and Kernel adapter. This is a hidden
//! capability action, not another ToolSearch route or a second registry.
use std::collections::BTreeMap;

use async_trait::async_trait;
use nomifun_agent_contracts::*;
use nomifun_agent_kernel::{
    CapabilityHandler, CapabilityInvocationContext,
    KernelError, MaterializedRegistry,
};
use serde::Deserialize;
use serde_json::json;

pub const CAPABILITY_ID: &str = "agent.tool-discovery";
pub const ROLE_ID: &str = "system.tool-discovery";
pub const PACKAGE_ID: &str = "nomifun.tool-discovery";
pub const ACTION_ID: &str = "tool.discovery.rank";

/// Schema-free metadata supplied by the host's authorized tool catalog.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolDiscoveryCandidate {
    name: String,
    description: String,
    aliases: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolDiscoveryInput {
    query: String,
    candidates: Vec<ToolDiscoveryCandidate>,
    limit: usize,
}

fn rank_tool_discovery(input: &ToolDiscoveryInput) -> Vec<String> {
    let query = input.query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut ranked: Vec<(u8, &str)> = input
        .candidates
        .iter()
        .filter_map(|candidate| {
            let name = candidate.name.to_lowercase();
            let rank = if name == query {
                0
            } else if candidate.aliases.iter().any(|alias| alias == &query) {
                1
            } else if name.starts_with(&query) {
                2
            } else if candidate.aliases.iter().any(|alias| alias.starts_with(&query)) {
                3
            } else if name.contains(&query) {
                4
            } else if candidate.aliases.iter().any(|alias| alias.contains(&query)) {
                5
            } else if candidate.description.to_lowercase().contains(&query) {
                6
            } else {
                return None;
            };
            Some((rank, candidate.name.as_str()))
        })
        .collect();
    ranked.sort_unstable();
    let exact = ranked.first().map(|(rank, _)| *rank).filter(|rank| *rank <= 1);
    ranked
        .into_iter()
        .take_while(|(rank, _)| exact.is_none_or(|exact| *rank == exact))
        .take(input.limit.min(nomifun_agent_runtime::MAX_TOOL_DISCOVERY_MATCHES))
        .map(|(_, name)| name.to_owned())
        .collect()
}

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

/// The bundled pure ranking implementation for the canonical discovery Role.
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
        let current = registry
            .capability(&resolved.capability.id)
            .ok_or_else(|| KernelError::CapabilityNotMaterialized {
                capability_id: resolved.capability.id.clone(),
            })?;
        crate::plugin_tools::validate_exact_target(resolved, current)?;
        if current.manifest.contributions.actions == [action()] && !supports(&current.manifest)
        {
            return Err(crate::NomiPluginToolError::Contract(
                "Discovery capability must publish the exact Agent discovery Action".into(),
            ));
        }
        let actions = &current.manifest.contributions.actions;
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
        if selected.replace(resolved).is_some() {
            return Err(crate::NomiPluginToolError::Contract("Choose one discovery capability or one Role Provider, not multiple discovery policies".into()));
        }
    }
    Ok(selected)
}
