//! Model guidance annotates an already resolved canonical Action schema.
//! It cannot introduce arguments, constraints, tools or permission.
use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::{StrictJsonValue, tool_presentation::tool_presentation};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostGuidance {
    pub windows: Option<String>,
    pub macos: Option<String>,
    pub linux: Option<String>,
}

impl HostGuidance {
    fn text(&self, host_os: &str) -> Option<&str> {
        match host_os {
            "windows" => self.windows.as_deref(),
            "macos" => self.macos.as_deref(),
            "linux" => self.linux.as_deref(),
            _ => None,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.windows.is_none() && self.macos.is_none() && self.linux.is_none() {
            return Err("Host guidance needs an explicit supported platform".into());
        }
        for text in [&self.windows, &self.macos, &self.linux].into_iter().flatten() {
            validate_text(text)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterGuide {
    pub pointer: String,
    pub description: String,
    pub host_os: Option<HostGuidance>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolGuideExample {
    pub label: String,
    pub arguments: StrictJsonValue,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolModelGuide {
    pub capability_id: String,
    pub action_id: String,
    pub description: String,
    /// Request the existing process-host fact prefix, generated from the Host
    /// argument. Profile prose is never parsed to discover a platform.
    #[serde(default)]
    pub host_os_fact: bool,
    pub host_os: Option<HostGuidance>,
    pub parameters: Vec<ParameterGuide>,
    pub examples: Vec<ToolGuideExample>,
}

#[derive(Debug)]
pub struct RenderedToolGuide {
    pub description: String,
    pub input_schema: StrictJsonValue,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolModelGuideCatalogue {
    managed_launch_notice: String,
    guides: Vec<ToolModelGuide>,
}

fn validate_text(text: &str) -> Result<(), String> {
    if text.trim().is_empty() || text.chars().any(|character| {
        character.is_control() && character != '\n' && character != '\t'
    }) {
        return Err("Model guidance needs nonempty text without control characters".into());
    }
    Ok(())
}

// Restrict pointers to schema positions belonging to existing properties.
// A path may descend through array items and composed property schemas; it
// cannot target required/default/enum or turn an annotation into a constraint.
fn property_pointer(pointer: &str) -> Result<(), String> {
    let mut tokens = Vec::new();
    if !pointer.starts_with('/') {
        return Err("Parameter guidance needs an absolute property JSON pointer".into());
    }
    for token in pointer[1..].split('/') {
        let mut decoded = String::new();
        let mut characters = token.chars();
        while let Some(character) = characters.next() {
            if character == '~' {
                decoded.push(match characters.next() {
                    Some('0') => '~',
                    Some('1') => '/',
                    _ => return Err("Parameter guidance has an invalid JSON pointer escape".into()),
                });
            } else {
                decoded.push(character);
            }
        }
        if decoded.is_empty() || decoded.chars().any(char::is_control) {
            return Err("Parameter guidance has an invalid property path".into());
        }
        tokens.push(decoded);
    }
    let mut index = 0;
    let mut ends_in_property = false;
    while index < tokens.len() {
        match tokens[index].as_str() {
            "properties" if index + 1 < tokens.len() => {
                index += 2;
                ends_in_property = true;
            }
            "items" if index > 0 => {
                index += 1;
                ends_in_property = false;
            }
            "oneOf" | "anyOf" | "allOf" if index > 0 && index + 1 < tokens.len()
                && tokens[index + 1].bytes().all(|byte| byte.is_ascii_digit())
                && (tokens[index + 1] == "0" || !tokens[index + 1].starts_with('0'))
                && tokens[index + 1].parse::<usize>().is_ok() => {
                index += 2;
                ends_in_property = false;
            }
            _ => return Err("Parameter guidance must address an existing schema property".into()),
        }
    }
    if !ends_in_property {
        return Err("Parameter guidance must end at a schema property".into());
    }
    Ok(())
}

impl ToolModelGuide {
    fn validate(&self) -> Result<(), String> {
        if tool_presentation(&self.capability_id, &self.action_id).is_none() {
            return Err("Model guidance needs an exact registered presentation identity".into());
        }
        validate_text(&self.description)?;
        let process_launch = self.capability_id == "workspace.process"
            && matches!(self.action_id.as_str(), "workspace.process/exec" | "workspace.process/start");
        if self.host_os_fact != process_launch {
            return Err("Process launch guidance must declare its exact Host OS fact".into());
        }
        if let Some(profile) = &self.host_os {
            profile.validate()?;
        }
        if process_launch && self.host_os.as_ref().is_none_or(|profile| {
            profile.windows.is_none() || profile.macos.is_none() || profile.linux.is_none()
        }) {
            return Err("Process launch guidance needs all three supported Host profiles".into());
        }
        let mut pointers = BTreeSet::new();
        for parameter in &self.parameters {
            property_pointer(&parameter.pointer)?;
            if !pointers.insert(&parameter.pointer) {
                return Err("Duplicate parameter guidance pointer".into());
            }
            validate_text(&parameter.description)?;
            if let Some(profile) = &parameter.host_os {
                profile.validate()?;
            }
        }
        let mut labels = BTreeSet::new();
        for example in &self.examples {
            validate_text(&example.label)?;
            if !labels.insert(&example.label) || !example.arguments.0.is_object() {
                return Err("Guidance examples need unique labels and literal argument objects".into());
            }
        }
        Ok(())
    }

    pub fn render(&self, canonical: &StrictJsonValue, host_os: &str) -> Result<RenderedToolGuide, String> {
        self.validate()?;
        if !matches!(host_os, "windows" | "macos" | "linux") {
            return Err("Model guidance needs the actual supported Host OS".into());
        }
        let mut input_schema = canonical.clone();
        if !input_schema.0.is_object() {
            return Err("Model guidance requires a resolved canonical object schema".into());
        }
        for parameter in &self.parameters {
            let property = input_schema.0.pointer_mut(&parameter.pointer)
                .and_then(serde_json::Value::as_object_mut)
                .ok_or_else(|| format!("Guidance property is absent from the canonical schema: {}", parameter.pointer))?;
            let mut text = parameter.description.clone();
            if let Some(profile) = parameter.host_os.as_ref().and_then(|profile| profile.text(host_os)) {
                text.push(' ');
                text.push_str(profile);
            }
            property.insert("description".into(), serde_json::Value::String(text));
        }
        let mut description = if self.host_os_fact {
            format!("Workspace process host OS: {host_os}. {}", self.description)
        } else {
            self.description.clone()
        };
        if let Some(profile) = self.host_os.as_ref().and_then(|profile| profile.text(host_os)) {
            description.push(' ');
            description.push_str(profile);
        }
        Ok(RenderedToolGuide { description, input_schema })
    }
}

fn parse_catalogue(source: &str) -> Result<ToolModelGuideCatalogue, String> {
    let catalogue: ToolModelGuideCatalogue = serde_json::from_str(source)
        .map_err(|error| format!("Invalid model guidance data: {error}"))?;
    validate_text(&catalogue.managed_launch_notice)?;
    if catalogue.guides.is_empty() {
        return Err("Model guidance catalogue is empty".into());
    }
    let mut identities = BTreeSet::new();
    for guide in &catalogue.guides {
        guide.validate()?;
        if !identities.insert((&guide.capability_id, &guide.action_id)) {
            return Err("Duplicate model guidance Action identity".into());
        }
    }
    Ok(catalogue)
}

fn catalogue() -> &'static ToolModelGuideCatalogue {
    static CATALOGUE: OnceLock<ToolModelGuideCatalogue> = OnceLock::new();
    CATALOGUE.get_or_init(|| parse_catalogue(include_str!("../contracts/tool-model-guidance.json"))
        .expect("checked model guidance catalogue"))
}

pub fn tool_model_guides() -> &'static [ToolModelGuide] {
    &catalogue().guides
}

pub fn tool_model_guide(capability_id: &str, action_id: &str) -> Option<&'static ToolModelGuide> {
    tool_model_guides().iter().find(|guide| guide.capability_id == capability_id && guide.action_id == action_id)
}

/// Early visible guidance for actually authorized component launch tools,
/// including those whose full schemas are deferred by the existing ToolPlan.
pub fn managed_launch_notice() -> &'static str {
    &catalogue().managed_launch_notice
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> serde_json::Value {
        json!({"managed_launch_notice":"Only actual authorized tools are available.","guides":[{
            "capability_id":"workspace.files","action_id":"workspace.files/read","description":"Read a file.",
            "parameters":[{"pointer":"/properties/path","description":"Copy the exact relative path."}],
            "examples":[{"label":"Read source","arguments":{"path":"source.rs"}}]
        }]})
    }

    #[test]
    fn data_is_complete_and_preserves_registered_names_without_a_second_schema() {
        assert_eq!(tool_model_guides().len(), 19);
        for guide in tool_model_guides() {
            assert!(tool_presentation(&guide.capability_id, &guide.action_id).is_some());
            assert!(guide.examples.iter().all(|example| example.label.starts_with("Syntax only;")));
        }
        assert!(tool_model_guide("workspace.files", "workspace.process/exec").is_none());
        for phrase in ["do not copy paths or environment from Skill", "takes precedence over Skill",
            "one-shot", "resident session", "start watch", "report its actual failure", "do not install",
            "same-named ambient program", "grants no tools or permissions"] {
            assert!(managed_launch_notice().contains(phrase), "missing managed launch guidance: {phrase}");
        }
    }

    #[test]
    fn malformed_data_cannot_add_constraints_unknown_fields_or_duplicate_identity() {
        let mut variants = Vec::new();
        let mut value = fixture(); value["schema"] = json!({"type":"object"}); variants.push(value);
        let mut value = fixture(); value["guides"][0]["required"] = json!(["path"]); variants.push(value);
        let mut value = fixture(); value["guides"][0]["parameters"][0]["maximum"] = json!(0); variants.push(value);
        let mut value = fixture(); value["guides"][0]["host_os"] = json!({"android":"wrong host"}); variants.push(value);
        let mut value = fixture(); value["guides"][0]["examples"][0]["schema"] = json!({}); variants.push(value);
        let mut value = fixture(); let duplicate = value["guides"][0].clone(); value["guides"].as_array_mut().unwrap().push(duplicate); variants.push(value);
        let mut value = fixture(); let duplicate = value["guides"][0]["parameters"][0].clone(); value["guides"][0]["parameters"].as_array_mut().unwrap().push(duplicate); variants.push(value);
        let mut value = fixture(); value["guides"][0]["host_os_fact"] = json!(true); variants.push(value);
        for value in variants {
            assert!(parse_catalogue(&value.to_string()).is_err());
        }
    }

    #[test]
    fn parameter_paths_only_annotate_existing_properties_and_preserve_input_values() {
        let guide = parse_catalogue(&fixture().to_string()).unwrap().guides.remove(0);
        let original = StrictJsonValue(json!({"type":"object","additionalProperties":false,"required":["path"],
            "properties":{"path":{"type":"string","minLength":1},"description":{"type":"string"}}}));
        let rendered = guide.render(&original, "macos").unwrap();
        let mut restored = rendered.input_schema.0.clone();
        restored["properties"]["path"].as_object_mut().unwrap().remove("description");
        assert_eq!(restored, original.0, "only the selected schema annotation can differ");
        assert_eq!(original.0["properties"]["description"], json!({"type":"string"}));
        assert!(guide.render(&StrictJsonValue(json!({"type":"object","properties":{}})), "macos").is_err());
        assert!(guide.render(&original, "android").is_err());
        for pointer in ["", "/required", "/properties/path/default", "/properties/path/description",
            "/properties/path/items", "/properties/path~2invalid", "/$defs/property",
            "/properties/path/oneOf/+1/properties/name", "/properties/path/oneOf/01/properties/name"] {
            let mut value = fixture(); value["guides"][0]["parameters"][0]["pointer"] = json!(pointer);
            assert!(parse_catalogue(&value.to_string()).is_err(), "{pointer}");
        }
    }

    #[test]
    fn nested_and_escaped_property_paths_remain_annotations() {
        let mut value = fixture();
        value["guides"][0]["parameters"] = json!([
            {"pointer":"/properties/files/items/properties/guard/oneOf/0/properties/digest","description":"Use observed digest."},
            {"pointer":"/properties/a~1b~0c","description":"Exact escaped property."}
        ]);
        let guide = parse_catalogue(&value.to_string()).unwrap().guides.remove(0);
        let schema = StrictJsonValue(json!({"properties":{
            "files":{"type":"array","items":{"properties":{"guard":{"oneOf":[{"properties":{"digest":{"type":"string"}}}]}}}},
            "a/b~c":{"type":"string"}
        }}));
        let rendered = guide.render(&schema, "linux").unwrap();
        assert_eq!(rendered.input_schema.0.pointer("/properties/a~1b~0c/description"), Some(&json!("Exact escaped property.")));
        assert_eq!(rendered.input_schema.0.pointer("/properties/files/items/properties/guard/oneOf/0/properties/digest/type"), Some(&json!("string")));
    }

    #[test]
    fn process_host_fact_uses_the_host_argument_and_does_not_restore_two_target_rules() {
        for action in ["workspace.process/exec", "workspace.process/start"] {
            let guide = tool_model_guide("workspace.process", action).unwrap();
            let schema = StrictJsonValue(json!({"type":"object","properties":guide.parameters.iter()
                .map(|parameter| (parameter.pointer.trim_start_matches("/properties/").to_owned(), json!({"type":"string"})))
                .collect::<serde_json::Map<_,_>>()}));
            for host in ["windows", "macos", "linux"] {
                let rendered = guide.render(&schema, host).unwrap();
                assert!(rendered.description.starts_with(&format!("Workspace process host OS: {host}.")));
                assert!(rendered.input_schema.0["properties"]["args"]["description"].as_str().unwrap().contains("command or component"));
                assert!(!rendered.description.contains("The workspace root is already the selected project directory"));
                if host == "windows" {
                    assert!(rendered.description.contains("PowerShell 5.1"));
                    assert!(rendered.description.contains("-Encoding UTF8"));
                    assert!(rendered.description.contains("Hidden=[bool]"));
                } else {
                    assert!(rendered.description.contains("/bin/sh -c"));
                }
            }
            if action.ends_with("/start") {
                assert!(guide.parameters.iter().find(|parameter| parameter.pointer.ends_with("/wait_ms"))
                    .unwrap().description.contains("nonzero initial wait is permitted"));
            }
        }
        let write = tool_model_guide("workspace.files", "workspace.files/write").unwrap();
        assert!(write.examples[0].arguments.0["content"].as_str().unwrap().ends_with('\n'));
    }
}
