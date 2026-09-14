//! Canonical process action contract shared by registered platform hosts.
use nomifun_agent_contracts::StrictJsonValue;
use serde_json::json;

pub fn process_exec_input_schema() -> StrictJsonValue {
    let mut launch = json!({
        "type":"object", "additionalProperties":false, "required":["command"],
        "properties":{
            "operation":{"type":"string","enum":["exec","start"],"default":"exec"},
            "command":{"type":"string","minLength":1,"maxLength":32768},
            "args":{"type":"array","maxItems":256,"items":{"type":"string","maxLength":65536}},
            "cwd":{"type":"string","maxLength":4096},
            "env":{"type":"object","maxProperties":128,"additionalProperties":{"type":"string","maxLength":65536}},
            "timeout_ms":{"type":"integer","minimum":1,"maximum":600000},
            "wait_ms":{"type":"integer","minimum":0,"maximum":30000},
            "tty":{"type":"boolean","default":false},
            "cols":{"type":"integer","minimum":1,"maximum":65535},
            "rows":{"type":"integer","minimum":1,"maximum":65535}
        }
    });
    launch["description"] = "exec waits for terminal; start returns a turn-owned process_id for subsequent controls. No process survives the owning turn.".into();
    let mut variants = vec![launch];
    for operation in ["poll", "stdin", "close_stdin", "resize", "cancel"] {
        let mut properties = json!({
            "operation":{"const":operation},
            "process_id":{"type":"string","minLength":1,"maxLength":128},
            "wait_ms":{"type":"integer","minimum":0,"maximum":30000}
        });
        let mut required = vec!["operation", "process_id"];
        if operation == "stdin" {
            properties["input"] = json!({"type":"string","maxLength":1048576});
            required.push("input");
        }
        if operation == "resize" {
            for field in ["cols", "rows"] {
                properties[field] = json!({"type":"integer","minimum":1,"maximum":65535});
                required.push(field);
            }
        }
        variants.push(json!({"type":"object","additionalProperties":false,"required":required,"properties":properties}));
    }
    // Surface the fields at the root for function-calling clients that discover
    // parameters through `properties`. The original closed variants remain the
    // authority for required fields and operation-specific combinations.
    let mut properties = serde_json::Map::new();
    for variant in &variants {
        for (name, schema) in variant["properties"].as_object().expect("process variant properties") {
            if name == "operation" { continue; }
            if let Some(previous) = properties.insert(name.clone(), schema.clone()) {
                assert_eq!(previous, *schema, "shared process field constraints must agree");
            }
        }
    }
    properties.insert("operation".into(), json!({"type":"string",
        "enum":["exec","start","poll","stdin","close_stdin","resize","cancel"],"default":"exec"}));
    StrictJsonValue(json!({"type":"object","properties":properties,"oneOf":variants}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_process_fields_preserve_closed_operation_validation() {
        let schema = process_exec_input_schema().0;
        let mut legacy = schema.clone();
        legacy.as_object_mut().unwrap().remove("properties");
        let before = jsonschema::validator_for(&legacy).unwrap();
        let after = jsonschema::validator_for(&schema).unwrap();
        for field in ["command", "args", "cwd", "env", "timeout_ms", "operation", "process_id", "input", "cols", "rows", "wait_ms", "tty"] {
            assert!(schema["properties"].get(field).is_some(), "model must see {field}");
        }
        let mut cases = vec![
            (json!({"command":"git","args":["--version"]}), true),
            (json!({"operation":"start","command":"git","env":{"LANG":"C"},"cwd":".","tty":true,"cols":120,"rows":40,"wait_ms":1,"timeout_ms":10000}), true),
            (json!({"operation":"exec","command":"git","process_id":"old"}), false),
            (json!({"operation":"exec","command":"git","input":"x"}), false),
            (json!({"command":"git","env":null}), false),
            (json!({"command":"git","args":null}), false),
            (json!({"command":"git","unknown":true}), false),
            (json!({"command":"git","timeout_ms":600001}), false),
            (json!({"command":""}), false),
            (json!({"operation":"unknown","command":"git"}), false),
            (json!({"operation":"stdin","process_id":"p","input":"x"}), true),
            (json!({"operation":"stdin","process_id":"p"}), false),
            (json!({"operation":"resize","process_id":"p","cols":120,"rows":40}), true),
            (json!({"operation":"resize","process_id":"p","cols":0,"rows":40}), false),
        ];
        for op in ["poll", "close_stdin", "cancel"] {
            cases.push((json!({"operation":op,"process_id":"p","wait_ms":0}), true));
            cases.push((json!({"operation":op}), false));
            cases.push((json!({"operation":op,"process_id":"p","command":"git"}), false));
        }
        for (input, expected) in cases {
            assert_eq!(before.is_valid(&input), expected, "original contract: {input}");
            assert_eq!(after.is_valid(&input), expected, "visible contract: {input}");
        }
    }
}
