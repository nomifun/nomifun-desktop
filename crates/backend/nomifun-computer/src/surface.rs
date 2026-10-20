//! Frozen definitions and model projection. Authorization always remains exact.
pub use nomifun_agent_contracts::computer_arguments_use_foreground as foreground;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
    time::Duration,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FrozenPlatform {
    pub input_schema: Value,
    #[serde(default)]
    pub annotations: Value,
    #[serde(default)]
    pub capabilities: Vec<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FrozenTool {
    pub action_id: String,
    pub driver_tool: String,
    pub model_tool: String,
    pub effect_class: String,
    pub resource_operation: String,
    pub loading: String,
    pub no_image: String,
    pub group: String,
    pub titles: BTreeMap<String, String>,
    pub descriptions: BTreeMap<String, String>,
    pub platforms: BTreeMap<String, FrozenPlatform>,
    pub system_permissions: SystemPermissions,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SystemPermissions {
    pub accessibility: bool,
    pub screen_recording: ScreenRecordingRequirement,
    #[serde(default)]
    pub screenshot_default: bool,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenRecordingRequirement {
    Never,
    Always,
    WindowScreenshot,
    BatchScreenshot,
}
#[derive(Debug, Deserialize)]
pub struct FrozenManifest {
    pub tools: Vec<FrozenTool>,
    pub instructions: BTreeMap<String, BTreeMap<String, String>>,
    pub schema_profile: SchemaProfile,
}
#[derive(Debug, Deserialize)]
pub struct SchemaProfile {
    pub trusted_parameters: Vec<String>,
    pub strip_parameter_prefixes: Vec<String>,
}
pub fn manifest() -> &'static FrozenManifest {
    static DATA: OnceLock<FrozenManifest> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../nomifun-agent-contracts/contracts/computer-tools.v1.json"
        ))
        .expect("validated frozen Computer manifest")
    })
}
pub fn tool(name: &str) -> Option<&'static FrozenTool> {
    manifest()
        .tools
        .iter()
        .find(|tool| name == tool.action_id || name == tool.driver_tool || name == tool.model_tool)
}
pub const fn platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unsupported"
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteTier {
    Unavailable,
    NoImage,
    Full,
}
impl RouteTier {
    pub fn from_capabilities(tool_calls: bool, image_input: bool) -> Self {
        if !tool_calls {
            Self::Unavailable
        } else if image_input {
            Self::Full
        } else {
            Self::NoImage
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct ModelTool {
    pub action_id: String,
    pub name: String,
    pub title: String,
    pub description: String,
    pub input_schema: Value,
    pub deferred: bool,
}
pub fn tools_for_tier(tier: RouteTier, locale: &str) -> Vec<ModelTool> {
    if tier == RouteTier::Unavailable {
        return Vec::new();
    }
    manifest()
        .tools
        .iter()
        .filter_map(|tool| {
            if tier == RouteTier::NoImage && tool.no_image == "full_only" {
                return None;
            }
            let definition = tool.platforms.get(platform())?;
            let mut input_schema = definition.input_schema.clone();
            if tier == RouteTier::NoImage {
                // Frame x/y place a native window in list_windows units;
                // they do not address an input point in a screenshot.
                project_no_image_schema(&mut input_schema, tool.driver_tool == "set_window_frame");
            }
            if tier == RouteTier::NoImage && tool.no_image == "element_token" {
                let required = input_schema
                    .as_object_mut()
                    .unwrap()
                    .entry("required")
                    .or_insert_with(|| json!([]));
                if let Some(required) = required.as_array_mut() {
                    if !required.iter().any(|value| value == "element_token") {
                        required.push(json!("element_token"));
                    }
                }
            }
            Some(ModelTool {
                action_id: tool.action_id.clone(),
                name: tool.model_tool.clone(),
                title: tool
                    .titles
                    .get(locale)
                    .or_else(|| tool.titles.get("en-US"))
                    .cloned()
                    .unwrap_or_default(),
                description: tool
                    .descriptions
                    .get(locale)
                    .or_else(|| tool.descriptions.get("en-US"))
                    .cloned()
                    .unwrap_or_default(),
                input_schema,
                deferred: tool.loading == "deferred",
            })
        })
        .collect()
}
pub fn instructions(tier: RouteTier, locale: &str) -> String {
    if tier == RouteTier::Unavailable {
        return String::new();
    }
    let get = |key: &str| {
        manifest()
            .instructions
            .get(key)
            .and_then(|text| text.get(locale).or_else(|| text.get("en-US")))
            .cloned()
            .unwrap_or_default()
    };
    let mut text = get("common");
    if tier == RouteTier::NoImage {
        text.push('\n');
        text.push_str(&get("no_image"));
    }
    text
}
fn project_no_image_schema(schema: &mut Value, window_frame_coordinates: bool) {
    if let Some(values) = schema.get_mut("enum").and_then(Value::as_array_mut) {
        values.retain(|value| {
            value != "desktop"
                && value
                    .as_str()
                    .and_then(tool)
                    .is_none_or(|tool| tool.no_image != "full_only")
        });
    }
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        for key in [
            "x",
            "y",
            "from_x",
            "from_y",
            "to_x",
            "to_y",
            "capture_id",
            "from_zoom",
        ] {
            if window_frame_coordinates && matches!(key, "x" | "y") {
                continue;
            }
            properties.remove(key);
        }
        if properties.contains_key("include_screenshot") {
            properties.insert(
                "include_screenshot".into(),
                json!({"type":"boolean","const":false}),
            );
        }
        for value in properties.values_mut() {
            project_no_image_schema(value, false);
        }
    }
    if let Some(items) = schema.get_mut("items") {
        project_no_image_schema(items, false);
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{code}: {message}")]
pub struct SurfaceError {
    pub code: &'static str,
    pub message: String,
}
fn denied(message: impl Into<String>) -> SurfaceError {
    SurfaceError {
        code: "COMPUTER_CALL_REJECTED",
        message: message.into(),
    }
}
#[derive(Debug, Clone)]
pub struct PreparedCall {
    pub tool: &'static FrozenTool,
    pub arguments: Value,
    pub foreground: bool,
    pub budget: Duration,
}

pub fn prepare(
    action: &str,
    arguments: Value,
    authorized: &BTreeSet<String>,
    tier: RouteTier,
) -> Result<PreparedCall, SurfaceError> {
    let selected = tool(action).ok_or_else(|| denied("Unknown Computer action"))?;
    if !authorized.contains(&selected.action_id) {
        return Err(denied("Computer action is not authorized"));
    }
    if tier == RouteTier::Unavailable {
        return Err(denied("The model does not support tool calls"));
    }
    if !selected.platforms.contains_key(platform()) {
        return Err(denied(
            "Computer action has no frozen schema for this platform",
        ));
    }
    let mut arguments = arguments;
    if !arguments.is_object() {
        return Err(denied("Computer arguments must be an object"));
    }
    strip_trusted(&mut arguments);
    prepare_inner(selected, &mut arguments, authorized, tier, 0)?;
    if matches!(
        selected.driver_tool.as_str(),
        "get_window_state" | "get_desktop_state" | "zoom"
    ) {
        arguments
            .as_object_mut()
            .unwrap()
            .insert("max_image_dimension".into(), json!(1568));
    }
    if let Some(observe) = arguments.get_mut("observe").and_then(Value::as_object_mut) {
        observe.insert("max_image_dimension".into(), json!(1568));
    }
    let foreground =
        nomifun_agent_contracts::computer_action_uses_foreground(&selected.action_id, &arguments);
    // The upstream maximum request budget is 120s; batches cap at 60s.
    // Transport overhead leaves legitimate capture/AX work time to finish.
    let budget = if selected.driver_tool == "run_steps" {
        90_000
    } else {
        let schema = &selected.platforms[platform()].input_schema;
        schema
            .pointer("/properties/timeout_ms/maximum")
            .and_then(Value::as_u64)
            .unwrap_or(120_000)
            .saturating_add(30_000)
    };
    Ok(PreparedCall {
        tool: selected,
        arguments,
        foreground,
        budget: Duration::from_millis(budget),
    })
}
pub fn strip_trusted(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.retain(|key, _| {
                !manifest().schema_profile.trusted_parameters.contains(key)
                    && !manifest()
                        .schema_profile
                        .strip_parameter_prefixes
                        .iter()
                        .any(|prefix| key.starts_with(prefix))
            });
            for child in object.values_mut() {
                strip_trusted(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                strip_trusted(child);
            }
        }
        _ => {}
    }
}
fn prepare_inner(
    selected: &'static FrozenTool,
    args: &mut Value,
    authorized: &BTreeSet<String>,
    tier: RouteTier,
    depth: usize,
) -> Result<(), SurfaceError> {
    if depth > 1 {
        return Err(denied("Nested Computer batches are not supported"));
    }
    if tier == RouteTier::NoImage {
        if selected.no_image == "full_only" {
            return Err(denied("This Computer action requires image input"));
        }
        if selected.no_image == "element_token"
            && args
                .get("element_token")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        {
            return Err(denied("Use a fresh element_token without image input"));
        }
        if contains_pixel_address(args, selected.driver_tool == "set_window_frame") {
            return Err(denied(
                "Pixel addressing is unavailable without image input",
            ));
        }
        force_no_screenshot(args);
        if matches!(
            selected.system_permissions.screen_recording,
            ScreenRecordingRequirement::WindowScreenshot
        ) {
            args.as_object_mut()
                .unwrap()
                .insert("include_screenshot".into(), json!(false));
        }
    }
    if selected.driver_tool == "run_steps" {
        if tier == RouteTier::NoImage {
            if let Some(observe) = args.get_mut("observe").and_then(Value::as_object_mut) {
                observe.insert("include_screenshot".into(), json!(false));
            }
        }
        let steps = args
            .get_mut("steps")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| denied("run_steps requires steps"))?;
        for step in steps {
            let object = step
                .as_object_mut()
                .ok_or_else(|| denied("Each Computer step must be an object"))?;
            let name = object
                .get("tool")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let name = if let Some(name) = name {
                name
            } else {
                let candidates: Vec<_> = object
                    .keys()
                    .filter(|key| tool(key).is_some())
                    .cloned()
                    .collect();
                if candidates.len() != 1 {
                    return Err(denied(
                        "Each Computer step must name exactly one authorized tool",
                    ));
                }
                let name = candidates[0].clone();
                let value = object.remove(&name).unwrap();
                object.insert("args".into(), value);
                name
            };
            let nested = tool(&name).ok_or_else(|| denied("Unknown Computer step tool"))?;
            if nested.driver_tool == "run_steps" || !authorized.contains(&nested.action_id) {
                return Err(denied("Computer step is not authorized"));
            }
            object.insert("tool".into(), json!(nested.model_tool));
            let params = object.entry("args").or_insert_with(|| json!({}));
            if !params.is_object() {
                return Err(denied("Computer step args must be an object"));
            }
            prepare_inner(nested, params, authorized, tier, depth + 1)?;
            // Step checks/implicit observation must also remain authorized.
            if (object.contains_key("wait_for") || object.contains_key("expect"))
                && !authorized.contains("computer/verify_state")
            {
                return Err(denied("Computer step verification is not authorized"));
            }
        }
        if args
            .get("observe")
            .is_some_and(|value| value != &Value::Bool(false))
            && !authorized.contains("computer/get_window_state")
        {
            return Err(denied("Computer batch observation is not authorized"));
        }
    }
    Ok(())
}
fn contains_pixel_address(value: &Value, window_frame_coordinates: bool) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (matches!(
                key.as_str(),
                "x" | "y" | "from_x" | "from_y" | "to_x" | "to_y" | "capture_id" | "from_zoom"
            ) && !(window_frame_coordinates && matches!(key.as_str(), "x" | "y")))
                || (key == "kind" && value == "desktop")
                || contains_pixel_address(value, false)
        }),
        Value::Array(items) => items.iter().any(|value| contains_pixel_address(value, false)),
        _ => false,
    }
}
fn force_no_screenshot(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if object.contains_key("include_screenshot") {
                object.insert("include_screenshot".into(), json!(false));
            }
            for value in object.values_mut() {
                force_no_screenshot(value);
            }
        }
        Value::Array(items) => {
            for value in items {
                force_no_screenshot(value);
            }
        }
        _ => {}
    }
}
/// Keep the canonical/model schema's names until the private stdio boundary.
pub fn driver_arguments(call: &PreparedCall) -> Value {
    let mut args = call.arguments.clone();
    if call.tool.driver_tool == "run_steps" {
        if let Some(steps) = args.get_mut("steps").and_then(Value::as_array_mut) {
            for step in steps {
                if let Some(object) = step.as_object_mut() {
                    if let Some(name) = object
                        .get("tool")
                        .and_then(Value::as_str)
                        .and_then(tool)
                        .map(|tool| tool.driver_tool.clone())
                    {
                        object.insert("tool".into(), json!(name));
                    }
                }
            }
        }
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    fn all() -> BTreeSet<String> {
        manifest()
            .tools
            .iter()
            .map(|tool| tool.action_id.clone())
            .collect()
    }
    #[test]
    fn canonical_frozen_surface_has_all_current_actions_and_permissions() {
        assert_eq!(manifest().tools.len(), 22);
        for tool in &manifest().tools {
            assert_eq!(
                crate::permissions::required_permissions(&tool.action_id, &json!({})).len(),
                usize::from(tool.system_permissions.accessibility)
                    + usize::from(
                        matches!(
                            tool.system_permissions.screen_recording,
                            ScreenRecordingRequirement::Always
                        ) | tool.system_permissions.screenshot_default
                    )
            );
        }
        assert!(tools_for_tier(RouteTier::Unavailable, "en-US").is_empty());
        assert!(!instructions(RouteTier::NoImage, "zh-CN").is_empty());
    }
    #[test]
    fn trusted_fields_are_removed_at_every_parameter_level() {
        let prepared=prepare("computer/get_window_state",json!({"session":"spoof","max_dimension":9000,"max_image_dimension":0,"screenshot_out_file":"outside","_internal":true,"target":{"kind":"window","pid":1,"window_id":2,"_private":4}}),&all(),RouteTier::Full).unwrap();
        assert_eq!(prepared.arguments["max_image_dimension"], 1568);
        assert!(prepared.arguments.get("session").is_none());
        assert!(prepared.arguments.get("max_dimension").is_none());
        assert!(prepared.arguments.get("screenshot_out_file").is_none());
        assert!(prepared.arguments["target"].get("_private").is_none());
    }
    #[test]
    fn no_image_accessibility_inputs_remain_valid_canonical_arguments() {
        for (action, args) in [
            ("computer/click", json!({"element_token":"s0000002a:11"})),
            ("computer/press_key", json!({"key":"enter"})),
        ] {
            let prepared = prepare(action, args, &all(), RouteTier::NoImage).unwrap();
            let mut args = prepared.arguments;
            strip_trusted(&mut args);
            assert!(
                args.get("include_screenshot").is_none(),
                "input must not acquire observation fields"
            );
            let schema = &tool(action).unwrap().platforms[platform()].input_schema;
            assert!(jsonschema::is_valid(schema, &args), "{action}: {args}");
        }
        assert!(
            prepare(
                "computer/click",
                json!({"x":5,"y":3}),
                &all(),
                RouteTier::NoImage
            )
            .is_err()
        );
        assert!(
            prepare(
                "computer/get_desktop_state",
                json!({}),
                &all(),
                RouteTier::NoImage
            )
            .is_err()
        );
        let read = prepare(
            "computer/get_window_state",
            json!({}),
            &all(),
            RouteTier::NoImage,
        )
        .unwrap();
        assert_eq!(read.arguments["include_screenshot"], false);
    }
    #[test]
    fn no_image_window_observations_disable_optional_screenshots_from_frozen_metadata() {
        let selected = tool("computer/verify_state").unwrap();
        assert!(matches!(
            selected.system_permissions.screen_recording,
            ScreenRecordingRequirement::WindowScreenshot
        ));
        let prepared = prepare(
            &selected.action_id,
            json!({"pid":1,"window_id":2,"expect":[{"window":{"exists":true}}]}),
            &all(),
            RouteTier::NoImage,
        )
        .unwrap();
        assert_eq!(prepared.arguments["include_screenshot"], false);
        assert!(jsonschema::is_valid(
            &selected.platforms[platform()].input_schema,
            &prepared.arguments
        ));
    }

    #[test]
    fn no_image_window_frame_coordinates_remain_valid_native_arguments() {
        let selected = tool("computer/set_window_frame").unwrap();
        let definition = tools_for_tier(RouteTier::NoImage, "en-US").into_iter()
            .find(|definition| definition.action_id == selected.action_id).unwrap();
        for required in definition.input_schema["required"].as_array().unwrap() {
            assert!(definition.input_schema["properties"].get(required.as_str().unwrap()).is_some(),
                "the projected strict object must retain each required native frame field");
        }
        let args = json!({"pid":1,"window_id":2,"x":-20,"y":12,"width":400,"height":300});
        assert!(jsonschema::is_valid(&definition.input_schema, &args));
        let prepared = prepare(&selected.action_id, args.clone(), &all(), RouteTier::NoImage).unwrap();
        assert_eq!(prepared.arguments, args);
        assert!(jsonschema::is_valid(&selected.platforms[platform()].input_schema, &prepared.arguments));
    }

    #[test]
    fn no_image_pixels_remain_rejected_for_input_actions_and_batches() {
        for (action, args) in [
            ("computer/click", json!({"element_token":"s0000002a:11","x":5,"y":3})),
            ("computer/type_text", json!({"element_token":"s0000002a:11","text":"owned text","x":5,"y":3})),
            ("computer/drag", json!({"from_x":5,"from_y":3,"to_x":10,"to_y":8})),
            ("computer/run_steps", json!({"steps":[{"tool":"computer_click","args":{"element_token":"s0000002a:11","x":5,"y":3}}]})),
        ] {
            assert!(prepare(action, args, &all(), RouteTier::NoImage).is_err(), "{action}");
        }
        let click = tools_for_tier(RouteTier::NoImage, "en-US").into_iter()
            .find(|definition| definition.action_id == tool("computer/click").unwrap().action_id).unwrap();
        assert!(click.input_schema["properties"].get("x").is_none());
        assert!(click.input_schema["properties"].get("y").is_none());
        assert!(!tools_for_tier(RouteTier::NoImage, "en-US").iter()
            .any(|definition| definition.action_id == tool("computer/drag").unwrap().action_id));
        let batch = tool("computer/run_steps").unwrap();
        let frame = tool("computer/set_window_frame").unwrap();
        assert!(!batch.platforms[platform()].input_schema["properties"]["steps"]["items"]["properties"]["tool"]["enum"]
            .as_array().unwrap().iter().any(|name| name == &frame.model_tool),
            "the upstream batchable-tool contract is unchanged");
    }

    #[test]
    fn batch_steps_are_authorized_before_dispatch_and_names_change_only_at_wire() {
        let input = json!({"steps":[{"tool":"computer_click","args":{"element_token":"s0000002a:11"}}],"observe":{}});
        let prepared = prepare("computer/run_steps", input, &all(), RouteTier::NoImage).unwrap();
        assert_eq!(prepared.arguments["steps"][0]["tool"], "computer_click");
        assert_eq!(prepared.arguments["observe"]["include_screenshot"], false);
        assert_eq!(driver_arguments(&prepared)["steps"][0]["tool"], "click");
        let mut subset = all();
        subset.remove("computer/click");
        assert!(
            prepare(
                "computer/run_steps",
                prepared.arguments.clone(),
                &subset,
                RouteTier::Full
            )
            .is_err()
        );
        assert!(
            prepare(
                "computer/run_steps",
                json!({"steps":[{"tool":"run_script","args":{}}]}),
                &all(),
                RouteTier::Full
            )
            .is_err()
        );
    }
}
