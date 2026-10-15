//! Owner-edited device metadata. This never creates hardware tools or grants authority.

use std::collections::{BTreeMap, BTreeSet};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use crate::{capability::RobotAction, mcp_bridge::RobotToolDescriptor};

pub const MAX_PROFILE_BYTES: usize = 64 * 1024;
const MAX_DESCRIPTION_BYTES: usize = 4096;

fn enabled_default() -> bool { true }

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RobotCapabilityProfile {
    #[serde(default)]
    pub tool_overrides: BTreeMap<String, RobotToolOverride>,
    #[serde(default)]
    pub presets: Vec<RobotCapabilityPreset>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RobotToolOverride {
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    #[serde(default)]
    pub action: Option<RobotAction>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default_arguments: Map<String, Value>,
}

impl Default for RobotToolOverride {
    fn default() -> Self { Self { enabled: true, action: None, description: None, default_arguments: Map::new() } }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RobotCapabilityPreset {
    pub id: String,
    pub name: String,
    pub tool_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub arguments: Map<String, Value>,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedProfileTool {
    pub provider_name: String,
    pub description: String,
    pub input_schema: Value,
    pub action: RobotAction,
    pub device_name: String,
    pub device_exposed_name: String,
    pub raw_input_schema: Value,
    pub default_arguments: Map<String, Value>,
    pub fixed_arguments: Option<Map<String, Value>>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ProfileError(pub String);

fn invalid(message: impl Into<String>) -> ProfileError { ProfileError(message.into()) }

pub fn tool_enabled(profile: &RobotCapabilityProfile, name: &str) -> bool {
    profile.tool_overrides.get(name).is_none_or(|value| value.enabled)
}

pub fn resolve_tool_action(profile: &RobotCapabilityProfile, name: &str) -> RobotAction {
    profile.tool_overrides.get(name).and_then(|value| value.action)
        .unwrap_or_else(|| crate::tool_registry::tool_action(name))
}

/// Host-owned ceilings cannot be reduced by editable metadata. In particular,
/// physical and camera tools keep their device permission requirements.
pub fn allowed_tool_actions(name: &str) -> Vec<RobotAction> {
    match crate::tool_registry::tool_action(name) {
        RobotAction::Motion => vec![RobotAction::Motion],
        RobotAction::Vision => vec![RobotAction::Vision],
        RobotAction::Display => vec![RobotAction::Display, RobotAction::Motion],
        RobotAction::Device => RobotAction::ALL.to_vec(),
    }
}

pub fn effective_description(profile: &RobotCapabilityProfile, tool: &RobotToolDescriptor) -> String {
    profile.tool_overrides.get(&tool.device_name).and_then(|value| value.description.as_ref())
        .unwrap_or(&tool.description).clone()
}

fn valid_text(value: &str, max_bytes: usize, multiline: bool) -> bool {
    value.len() <= max_bytes && !value.chars().any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\t' | '\r')))
}

pub fn validate_profile_shape(profile: &RobotCapabilityProfile) -> Result<(), ProfileError> {
    if serde_json::to_vec(profile).map_err(|error| invalid(error.to_string()))?.len() > MAX_PROFILE_BYTES {
        return Err(invalid("Robot capability configuration exceeds 64 KiB"));
    }
    if profile.tool_overrides.len() > 128 || profile.presets.len() > 64 {
        return Err(invalid("Robot capability configuration exceeds 128 tools or 64 presets"));
    }
    for (name, value) in &profile.tool_overrides {
        if name.trim().is_empty() || !valid_text(name, 256, false) {
            return Err(invalid("Tool names must be nonempty, at most 256 bytes, and contain no control characters"));
        }
        if value.description.as_ref().is_some_and(|description| !valid_text(description, MAX_DESCRIPTION_BYTES, true)) {
            return Err(invalid(format!("Description for {name} exceeds 4096 bytes or contains control characters")));
        }
        if value.action.is_some_and(|action| !allowed_tool_actions(name).contains(&action)) {
            return Err(invalid(format!("Tool {name} cannot be reclassified below its hardware permission ceiling")));
        }
    }
    let mut ids = BTreeSet::new();
    for preset in &profile.presets {
        let bytes = preset.id.as_bytes();
        if bytes.is_empty() || bytes.len() > 32 || !bytes[0].is_ascii_lowercase()
            || !bytes.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
            || !ids.insert(&preset.id) {
            return Err(invalid("Preset IDs must be unique and match [a-z][a-z0-9_]{0,31}"));
        }
        if preset.name.trim().is_empty() || !valid_text(&preset.name, 160, false) {
            return Err(invalid("Preset names must be nonempty and at most 160 bytes"));
        }
        if preset.tool_name.trim().is_empty() || !valid_text(&preset.tool_name, 256, false)
            || !valid_text(&preset.description, MAX_DESCRIPTION_BYTES, true) {
            return Err(invalid("Preset tool name or description is invalid"));
        }
    }
    Ok(())
}

struct NoExternalSchema;
impl jsonschema::Retrieve for NoExternalSchema {
    fn retrieve(&self, _: &jsonschema::Uri<String>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err("Robot schemas cannot retrieve external resources".into())
    }
}

fn validator(schema: &Value) -> Result<jsonschema::Validator, ProfileError> {
    if serde_json::to_vec(schema).map_err(|e| invalid(e.to_string()))?.len() > 64 * 1024 {
        return Err(invalid("Device tool schema exceeds 64 KiB"));
    }
    if !schema.is_object() || schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err(invalid("Device tool schema root must be an object"));
    }
    jsonschema::options().with_retriever(NoExternalSchema).build(schema)
        .map_err(|error| invalid(format!("Invalid device tool schema: {error}")))
}

fn validate_defaults(tool: &RobotToolDescriptor, defaults: &Map<String, Value>) -> Result<(), ProfileError> {
    if defaults.is_empty() { return Ok(()); }
    let mut partial = tool.input_schema.clone();
    let object = partial.as_object_mut().ok_or_else(|| invalid("Device tool schema root must be an object"))?;
    object.remove("required");
    object.remove("minProperties");
    // A default may only supply a real advertised parameter, even if a lax
    // firmware schema accepts unknown properties.
    let properties = object.get("properties").and_then(Value::as_object);
    if defaults.keys().any(|name| !properties.is_some_and(|properties| properties.contains_key(name))) {
        return Err(invalid(format!("Defaults for {} contain an unadvertised parameter", tool.device_name)));
    }
    if !validator(&partial)?.is_valid(&Value::Object(defaults.clone())) {
        return Err(invalid(format!("Defaults for {} do not satisfy its device schema", tool.device_name)));
    }
    Ok(())
}

/// Validate owner updates against the current authenticated device catalog.
/// Missing old targets may be retained for later reconnects; changing them
/// requires the device to advertise that exact tool again.
pub fn validate_profile_update(profile: &RobotCapabilityProfile, previous: &RobotCapabilityProfile, tools: &[RobotToolDescriptor]) -> Result<(), ProfileError> {
    validate_profile_shape(profile)?;
    let by_name: BTreeMap<_, _> = tools.iter().map(|tool| (tool.device_name.as_str(), tool)).collect();
    for (name, value) in &profile.tool_overrides {
        match by_name.get(name.as_str()) {
            Some(tool) => validate_defaults(tool, &value.default_arguments)?,
            None if previous.tool_overrides.get(name) == Some(value) => {},
            None => return Err(invalid(format!("The connected device does not advertise tool {name}"))),
        }
    }
    for preset in &profile.presets {
        match by_name.get(preset.tool_name.as_str()) {
            Some(tool) => {
                if !validator(&tool.input_schema)?.is_valid(&Value::Object(preset.arguments.clone())) {
                    return Err(invalid(format!("Preset {} arguments do not satisfy {}", preset.id, preset.tool_name)));
                }
            },
            None if previous.presets.iter().any(|old| old == preset) => {},
            None => return Err(invalid(format!("The connected device does not advertise preset target {}", preset.tool_name))),
        }
    }
    resolve_profile_tools(profile, tools)?;
    Ok(())
}

/// Resolve only enabled, currently discovered tools. Stale saved targets are
/// retained as editable metadata, never synthesized into executable tools.
pub fn resolve_profile_tools(profile: &RobotCapabilityProfile, tools: &[RobotToolDescriptor]) -> Result<Vec<ResolvedProfileTool>, ProfileError> {
    validate_profile_shape(profile)?;
    let mut names = BTreeSet::new();
    for tool in tools {
        if !names.insert(tool.exposed_name.clone()) { return Err(invalid("Device tool aliases collide")); }
    }
    for preset in &profile.presets {
        if !names.insert(format!("robot_preset_{}", preset.id)) { return Err(invalid(format!("Preset {} alias collides with another tool", preset.id))); }
    }
    let mut resolved = Vec::new();
    for tool in tools {
        if !tool_enabled(profile, &tool.device_name) { continue; }
        let defaults = profile.tool_overrides.get(&tool.device_name).map(|value| value.default_arguments.clone()).unwrap_or_default();
        validate_defaults(tool, &defaults)?;
        let mut schema = tool.input_schema.clone();
        if let Some(object) = schema.as_object_mut() {
            if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
                required.retain(|value| value.as_str().is_none_or(|name| !defaults.contains_key(name)));
            }
            if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
                for (name, value) in &defaults {
                    if let Some(property) = properties.get_mut(name).and_then(Value::as_object_mut) {
                        property.insert("default".to_owned(), value.clone());
                    }
                }
            }
        }
        resolved.push(ResolvedProfileTool {
            provider_name: tool.exposed_name.clone(), description: effective_description(profile, tool), input_schema: schema,
            action: resolve_tool_action(profile, &tool.device_name), device_name: tool.device_name.clone(),
            device_exposed_name: tool.exposed_name.clone(), raw_input_schema: tool.input_schema.clone(),
            default_arguments: defaults, fixed_arguments: None,
        });
    }
    for preset in &profile.presets {
        if !preset.enabled || !tool_enabled(profile, &preset.tool_name) { continue; }
        let Some(tool) = tools.iter().find(|tool| tool.device_name == preset.tool_name) else { continue; };
        if !validator(&tool.input_schema)?.is_valid(&Value::Object(preset.arguments.clone())) {
            return Err(invalid(format!("Preset {} arguments no longer satisfy the device schema", preset.id)));
        }
        resolved.push(ResolvedProfileTool {
            provider_name: format!("robot_preset_{}", preset.id), description: if preset.description.trim().is_empty() { preset.name.clone() } else { format!("{}: {}", preset.name, preset.description) },
            input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
            action: resolve_tool_action(profile, &tool.device_name), device_name: tool.device_name.clone(), device_exposed_name: tool.exposed_name.clone(),
            raw_input_schema: tool.input_schema.clone(), default_arguments: Map::new(), fixed_arguments: Some(preset.arguments.clone()),
        });
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tool(name: &str) -> RobotToolDescriptor { RobotToolDescriptor { device_name:name.into(), exposed_name:crate::mcp_bridge::exposed_tool_name(name), description:"Device gesture".into(), input_schema:json!({"type":"object","properties":{"angle":{"type":"integer","minimum":0,"maximum":90}},"required":["angle"],"additionalProperties":false}) } }
    #[test]
    fn arbitrary_names_defaults_and_presets_resolve_without_board_knowledge() {
        let tools = vec![tool("vendor.neck.turn")];
        let mut profile = RobotCapabilityProfile::default();
        profile.tool_overrides.insert(tools[0].device_name.clone(), RobotToolOverride { action:Some(RobotAction::Motion), default_arguments:json!({"angle":30}).as_object().unwrap().clone(), ..Default::default() });
        profile.presets.push(RobotCapabilityPreset { id:"greeting".into(), name:"Greeting".into(), tool_name:tools[0].device_name.clone(), description:"A brief greeting".into(), arguments:json!({"angle":45}).as_object().unwrap().clone(), enabled:true });
        validate_profile_update(&profile, &Default::default(), &tools).unwrap();
        let resolved = resolve_profile_tools(&profile, &tools).unwrap();
        assert_eq!(resolved.len(),2);
        assert_eq!(resolved[0].action,RobotAction::Motion);
        assert_eq!(resolved[0].input_schema["required"],json!([]));
        assert_eq!(resolved[0].raw_input_schema["required"],json!(["angle"]));
        assert_eq!(resolved[1].provider_name,"robot_preset_greeting");
        assert!(resolved[1].fixed_arguments.is_some());
        profile.tool_overrides.get_mut(&tools[0].device_name).unwrap().enabled=false;
        assert!(resolve_profile_tools(&profile,&tools).unwrap().is_empty());
    }
    #[test]
    fn invalid_arguments_unknown_targets_and_alias_collisions_are_rejected() {
        let tools=vec![tool("self.arm.turn")];
        let mut profile=RobotCapabilityProfile::default();
        profile.tool_overrides.insert(tools[0].device_name.clone(),RobotToolOverride { default_arguments:json!({"angle":91}).as_object().unwrap().clone(), ..Default::default() });
        assert!(validate_profile_update(&profile,&Default::default(),&tools).is_err());
        profile.tool_overrides.clear();
        profile.presets.push(RobotCapabilityPreset { id:"greeting".into(),name:"Greeting".into(),tool_name:tools[0].device_name.clone(),description:String::new(),arguments:Map::new(),enabled:true });
        assert!(validate_profile_update(&profile,&Default::default(),&tools).is_err());
        profile.presets[0].arguments=json!({"angle":20}).as_object().unwrap().clone();
        assert!(validate_profile_update(&profile,&Default::default(),&[]).is_err());
        let colliding=vec![tools[0].clone(),tool("self.preset.greeting")];
        assert!(resolve_profile_tools(&profile,&colliding).is_err());
    }
    #[test]
    fn stale_targets_remain_metadata_and_cannot_create_tools() {
        let mut profile=RobotCapabilityProfile::default();
        profile.tool_overrides.insert("vendor.missing".into(),RobotToolOverride::default());
        assert!(resolve_profile_tools(&profile,&[]).unwrap().is_empty());
        assert!(validate_profile_update(&profile,&profile,&[]).is_ok());
        assert!(validate_profile_update(&profile,&Default::default(),&[]).is_err());
    }
    #[test]
    fn arbitrary_continuous_vision_tools_keep_the_additional_permission() {
        let mut profile=RobotCapabilityProfile::default();
        profile.tool_overrides.insert("vendor.optics.start_stream".into(), RobotToolOverride {
            action:Some(RobotAction::Vision), ..Default::default()
        });
        let mut permissions=crate::registry::RobotPermissions { vision:true, ..Default::default() };
        assert!(!permissions.allows_configured_tool(&profile,"vendor.optics.start_stream"));
        permissions.continuous_vision=true;
        assert!(permissions.allows_configured_tool(&profile,"vendor.optics.start_stream"));
    }

    #[test]
    fn editable_mapping_cannot_reduce_physical_or_camera_permission() {
        for name in ["self.head.express", "self.camera.start_stream"] {
            let mut profile=RobotCapabilityProfile::default();
            profile.tool_overrides.insert(name.into(), RobotToolOverride { action:Some(RobotAction::Display), ..Default::default() });
            assert!(validate_profile_shape(&profile).is_err(),"{name}");
        }
        let mut profile=RobotCapabilityProfile::default();
        profile.tool_overrides.insert("self.display.express".into(),RobotToolOverride { action:Some(RobotAction::Motion), ..Default::default() });
        assert!(validate_profile_shape(&profile).is_ok());
        let permissions=crate::registry::RobotPermissions::default();
        assert!(!permissions.allows_configured_tool(&profile,"self.display.express"));
    }
}
