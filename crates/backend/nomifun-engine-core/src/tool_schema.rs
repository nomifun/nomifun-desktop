//! Lossless model-facing presentation for closed object unions. Some tool
//! interfaces inspect root `properties` to encode argument types. Keep every
//! original oneOf constraint and add only constraints implied by that union.
use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::StrictJsonValue;
use serde_json::{Map, Value, json};

pub fn model_tool_schema(schema: &StrictJsonValue) -> StrictJsonValue {
    project(schema).unwrap_or_else(|| schema.clone())
}

fn project(schema: &StrictJsonValue) -> Option<StrictJsonValue> {
    let root = schema.0.as_object()?;
    if root.get("type")?.as_str()? != "object" || root.keys().any(|key| !matches!(key.as_str(),
        "type" | "oneOf" | "title" | "description" | "$comment" | "$schema" | "examples" | "default"
    )) { return None; }
    let variants = root.get("oneOf")?.as_array()?;
    if variants.is_empty() || variants.len() > 16 { return None; }
    let mut properties: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut common_required: Option<BTreeSet<String>> = None;
    for variant in variants {
        let object = variant.as_object()?;
        if object.get("type")?.as_str()? != "object"
            || object.get("additionalProperties") != Some(&Value::Bool(false))
            || object.keys().any(|key| !matches!(key.as_str(),
                "type" | "properties" | "required" | "additionalProperties" | "title" | "description"
            )) { return None; }
        let fields = object.get("properties")?.as_object()?;
        if fields.len() > 64 || fields.values().any(has_reference_scope) { return None; }
        let required = match object.get("required") {
            Some(value) => value.as_array()?.iter().map(|field| {
                let name = field.as_str()?;
                fields.contains_key(name).then(|| name.to_owned())
            }).collect::<Option<BTreeSet<_>>>()?,
            None => BTreeSet::new(),
        };
        common_required = Some(match common_required {
            Some(previous) => previous.intersection(&required).cloned().collect(),
            None => required,
        });
        for (name, field) in fields {
            let choices = properties.entry(name.clone()).or_default();
            if !choices.contains(field) { choices.push(field.clone()); }
        }
    }
    if properties.len() > 64 { return None; }
    let mut projected: Map<String, Value> = root.clone();
    projected.insert("properties".into(), Value::Object(properties.into_iter().map(|(name, choices)| {
        let field = if choices.len() == 1 { choices[0].clone() } else {
            let same_type = choices[0].get("type").filter(|kind| choices.iter().all(|choice| choice.get("type") == Some(*kind)));
            let constants = choices.iter().map(|choice| choice.get("const").cloned()).collect::<Option<Vec<_>>>();
            let mut field = match constants {
                Some(values) => json!({"enum":values}),
                None => json!({"anyOf":choices}),
            };
            if let Some(kind) = same_type { field["type"] = kind.clone(); }
            field
        };
        (name, field)
    }).collect()));
    projected.insert("required".into(), json!(common_required.unwrap_or_default()));
    projected.insert("additionalProperties".into(), Value::Bool(false));
    Some(StrictJsonValue(Value::Object(projected)))
}

fn has_reference_scope(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            matches!(key.as_str(), "$ref" | "$dynamicRef" | "$recursiveRef" | "$id" | "$anchor" | "$dynamicAnchor")
                || has_reference_scope(value)
        }),
        Value::Array(values) => values.iter().any(has_reference_scope),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn union() -> StrictJsonValue {
        StrictJsonValue(json!({"type":"object","oneOf":[
            {"type":"object","additionalProperties":false,"required":["strategy","goal"],
                "properties":{"strategy":{"type":"string","const":"planned"},"goal":{"type":"string","minLength":1}}},
            {"type":"object","additionalProperties":false,"required":["strategy","tasks"],
                "properties":{"strategy":{"type":"string","const":"parallel"},
                    "tasks":{"type":"array","minItems":1,"items":{"type":"object","additionalProperties":false,
                        "properties":{"name":{"type":"string"},"prompt":{"type":"string"}},"required":["name","prompt"]}},
                    "synthesize":{"type":"boolean"}}}
        ]}))
    }

    #[test]
    fn union_fields_are_explicit_without_replacing_the_original_contract() {
        let original = union();
        let projected = model_tool_schema(&original);
        assert_eq!(projected.0["oneOf"], original.0["oneOf"]);
        assert_eq!(projected.0["properties"]["tasks"]["type"], "array");
        assert_eq!(projected.0["properties"]["synthesize"]["type"], "boolean");
        assert_eq!(projected.0["properties"]["strategy"]["enum"], json!(["planned","parallel"]));
        assert_eq!(projected.0["required"], json!(["strategy"]));
        assert_eq!(model_tool_schema(&projected), projected, "projection is idempotent");
    }

    #[test]
    fn projection_accepts_exactly_the_same_boundary_inputs() {
        let original = union();
        let projected = model_tool_schema(&original);
        let before = jsonschema::validator_for(&original.0).unwrap();
        let after = jsonschema::validator_for(&projected.0).unwrap();
        let choices = [Value::Null, json!(false), json!("false"), json!(""), json!("goal"), json!([]), json!([{"name":"marker","prompt":"Reply OK"}])];
        for strategy in [json!("planned"), json!("parallel"), json!("unknown"), Value::Null] {
            for goal in &choices {
                for tasks in &choices {
                    for synthesize in &choices {
                        for mask in 0..8 {
                            let mut input = json!({"strategy":strategy});
                            if mask & 1 != 0 { input["goal"] = goal.clone(); }
                            if mask & 2 != 0 { input["tasks"] = tasks.clone(); }
                            if mask & 4 != 0 { input["synthesize"] = synthesize.clone(); }
                            assert_eq!(before.is_valid(&input), after.is_valid(&input), "{input}");
                        }
                    }
                }
            }
        }
        for input in [json!({}), json!([]), json!("text"), json!({"strategy":"parallel","tasks":[{}],"extra":true})] {
            assert_eq!(before.is_valid(&input), after.is_valid(&input));
        }
    }

    #[test]
    fn restrictive_roots_patterns_and_reference_scopes_are_not_rewritten() {
        for schema in [
            { let mut value = union(); value.0["additionalProperties"] = json!(false); value },
            { let mut value = union(); value.0["properties"] = json!({}); value },
            { let mut value = union(); value.0["oneOf"][0]["patternProperties"] = json!({"^x":{}}); value },
            { let mut value = union(); value.0["oneOf"][0]["properties"]["goal"] = json!({"$ref":"#/$defs/goal"}); value },
        ] {
            assert_eq!(model_tool_schema(&schema), schema);
        }
    }
}
