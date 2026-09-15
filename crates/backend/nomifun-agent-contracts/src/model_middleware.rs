//! The consumed before-model contract, shared by publication and the host.
use crate::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const ACTION_ID: &str = "agent.before_model";

fn input_schema() -> Value {
    json!({"type":"object","additionalProperties":false,
        "required":["phase","turn","system","tools"],"properties":{
        "phase":{"const":"before_model"},"system":{"type":"string"},
        "turn":{"type":"object","additionalProperties":false,
            "required":["source_message_id","text","image_media_types","cs_dialogue_id"],"properties":{
                "source_message_id":{"type":"string"},"text":{"type":"string"},
                "image_media_types":{"type":"array","items":{"type":"string"}},
                "cs_dialogue_id":{"type":["string","null"]}}},
        "tools":{"type":"array","items":{"type":"object","additionalProperties":false,
            "required":["name","description"],"properties":{
                "name":{"type":"string","minLength":1},"description":{"type":"string"}}}}
    }})
}

fn output_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "system":{"type":["string","null"]},
        "tool_names":{"type":["array","null"],"uniqueItems":true,
            "items":{"type":"string","minLength":1}}
    }})
}

fn reference(name: &str, value: &Value) -> CanonicalSchemaRef {
    format!(
        "schema://nomifun/before-model/{name}@1#{}",
        digest_payload(value)
            .expect("static before-model schema")
            .as_ref()
    )
    .into()
}

pub fn schemas() -> BTreeMap<CanonicalSchemaRef, StrictJsonValue> {
    [("input", input_schema()), ("output", output_schema())]
        .into_iter()
        .map(|(name, value)| (reference(name, &value), StrictJsonValue(value)))
        .collect()
}

pub fn action() -> CapabilityActionDescriptor {
    CapabilityActionDescriptor {
        action_id: ACTION_ID.into(),
        input_schema: reference("input", &input_schema()),
        output_schema: reference("output", &output_schema()),
        effect_class: EffectClass::Pure,
        presentation: ToolPresentationKind::Hidden,
    }
}

/// Reserved action cannot masquerade as an ordinary Tool or acquire resources
/// for which the current Product consumer has no binding adapter.
pub fn validate_manifest(manifest: &CapabilityManifest) -> Result<(), String> {
    if !manifest
        .contributions
        .actions
        .iter()
        .any(|a| a.action_id.as_ref() == ACTION_ID)
    {
        return Ok(());
    }
    if manifest.kind != CapabilityKind::TurnMiddleware
        || !manifest.supports_consumer(CapabilityConsumer::Agent)
        || !manifest.supports_consumer(CapabilityConsumer::PluginService)
        || manifest.contributions.actions != [action()]
        || !manifest.contributions.resource_kinds.is_empty()
        || !manifest.contributions.context_schema_refs.is_empty()
        || !manifest.contributions.event_schema_refs.is_empty()
        || !manifest.contributions.host_ports.is_empty()
        || manifest.contributions.ui_slot.is_some()
    {
        return Err("agent.before_model requires the exact Agent/PluginService TurnMiddleware contract without host resources".into());
    }
    Ok(())
}
