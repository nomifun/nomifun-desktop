//! Exact Product contracts consumed at the Nomi tool boundary.
use crate::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const BEFORE_ACTION_ID: &str = "agent.before_tool";

fn input_schema() -> Value {
    json!({"type":"object","additionalProperties":false,
        "required":["phase","invocation_id","tool_call_id","tool_name","arguments","redacted"],
        "properties":{
            "phase":{"const":"before_tool"},
            "invocation_id":{"type":"string","minLength":1},
            "tool_call_id":{"type":"string","minLength":1},
            "tool_name":{"type":"string","minLength":1},
            "arguments":{"type":"object"},
            "redacted":{"type":"boolean"}
        }})
}

fn output_schema() -> Value {
    json!({"oneOf":[
        {"type":"object","additionalProperties":false,"required":["decision"],
            "properties":{"decision":{"const":"allow"}}},
        {"type":"object","additionalProperties":false,"required":["decision","reason"],
            "properties":{"decision":{"const":"deny"},"reason":{"type":"string","minLength":1,"maxLength":2048}}}
    ]})
}

fn reference(name: &str, value: &Value) -> CanonicalSchemaRef {
    format!("schema://nomifun/before-tool/{name}@1#{}", digest_payload(value).expect("static tool hook schema").as_ref()).into()
}

pub fn schemas() -> BTreeMap<CanonicalSchemaRef, StrictJsonValue> {
    [("input", input_schema()), ("output", output_schema())].into_iter()
        .map(|(name, value)| (reference(name, &value), StrictJsonValue(value))).collect()
}

pub fn before_action() -> CapabilityActionDescriptor {
    CapabilityActionDescriptor {
        action_id: BEFORE_ACTION_ID.into(),
        input_schema: reference("input", &input_schema()),
        output_schema: reference("output", &output_schema()),
        effect_class: EffectClass::Pure,
        presentation: ToolPresentationKind::Hidden,
    }
}

pub fn is_tool_hook(action: &ActionId) -> bool {
    action.as_ref() == BEFORE_ACTION_ID
}

/// User-facing stage comes from the exact host contract, never a plugin label.
pub fn phase_for_actions(actions: &[CapabilityActionDescriptor]) -> Option<&'static str> {
    if actions == [before_action()] { Some("before_tool") }
    else if actions == [crate::model_middleware::action()] { Some("before_model") }
    else { None }
}

pub fn validate_manifest(manifest: &CapabilityManifest) -> Result<(), String> {
    for action in &manifest.contributions.actions {
        let id = action.action_id.as_ref();
        if (id.starts_with("agent.before_") || id.starts_with("agent.after_"))
            && id != BEFORE_ACTION_ID && id != crate::model_middleware::ACTION_ID {
            return Err("This Agent hook phase has no supported execution consumer".into());
        }
    }
    if !manifest.contributions.actions.iter().any(|a| is_tool_hook(&a.action_id)) { return Ok(()); }
    if manifest.kind != CapabilityKind::TurnMiddleware
        || !manifest.supports_consumer(CapabilityConsumer::Agent)
        || !manifest.supports_consumer(CapabilityConsumer::PluginService)
        || manifest.contributions.actions != [before_action()]
        || !manifest.contributions.resource_kinds.is_empty()
        || !manifest.contributions.context_schema_refs.is_empty()
        || !manifest.contributions.event_schema_refs.is_empty()
        || !manifest.contributions.host_ports.is_empty()
        || manifest.contributions.ui_slot.is_some() {
        return Err("agent.before_tool requires the exact Agent/PluginService TurnMiddleware contract without host resources".into());
    }
    Ok(())
}
