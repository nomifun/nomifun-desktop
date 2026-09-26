//! Canonical process action contract shared by registered platform hosts.
use nomifun_agent_contracts::StrictJsonValue;
use serde_json::json;

fn object(properties: serde_json::Value, required: &[&str]) -> StrictJsonValue {
    StrictJsonValue(json!({
        "type":"object",
        "additionalProperties":false,
        "properties":properties,
        "required":required,
    }))
}

fn launch(include_wait: bool) -> StrictJsonValue {
    let mut properties = json!({
        "cmd":{"type":"string","minLength":1,"maxLength":32768,
            "description":"Explicit shell command/script. Alternative to command plus args; never inferred from their contents."},
        "command":{"type":"string","minLength":1,"maxLength":32768,
            "description":"Executable name or path only, such as bun or git. Do not put arguments, pipes or a whole shell command here; use args for each argument."},
        "args":{"type":"array","maxItems":256,"items":{"type":"string","maxLength":65536},
            "description":"Separate argument tokens, for example [\"test\",\"tests/unit.test.js\"]. Omit when empty; never send null."},
        "cwd":{"type":"string","maxLength":4096,
            "description":"Optional workspace-relative directory; omit to use the bound workspace root."},
        "env":{"type":"object","maxProperties":128,"additionalProperties":{"type":"string","maxLength":65536},
            "description":"Optional string-valued environment overrides. Omit when empty; never send null."},
        "timeout_ms":{"type":"integer","minimum":1,"maximum":600000},
        "tty":{"type":"boolean","default":false},
        "cols":{"type":"integer","minimum":1,"maximum":65535},
        "rows":{"type":"integer","minimum":1,"maximum":65535}
    });
    if include_wait {
        properties["wait_ms"] = json!({"type":"integer","minimum":0,"maximum":30000,"default":0});
    }
    StrictJsonValue(json!({"type":"object","additionalProperties":false,"properties":properties,
        "oneOf":[{"required":["cmd"],"not":{"required":["command"]},"properties":{"args":{"maxItems":0}}},
                 {"required":["command"],"not":{"required":["cmd"]}}]}))
}

/// Exact schema for one `workspace.process` Action. The Action ID, rather
/// than an untrusted payload discriminator, selects the operation.
pub fn process_action_input_schema(action_id: &str) -> Option<StrictJsonValue> {
    Some(match action_id {
        "workspace.process/exec" => launch(false),
        "workspace.process/start" => launch(true),
        "workspace.process/poll" => object(
            json!({
                "process_id":{"type":"string","minLength":1,"maxLength":128},
                "wait_ms":{"type":"integer","minimum":0,"maximum":30000,"default":0}
            }),
            &["process_id"],
        ),
        "workspace.process/input" => object(
            json!({
                "process_id":{"type":"string","minLength":1,"maxLength":128},
                "input":{"type":"string","maxLength":1048576}
            }),
            &["process_id", "input"],
        ),
        "workspace.process/close_stdin" | "workspace.process/cancel" => object(
            json!({"process_id":{"type":"string","minLength":1,"maxLength":128}}),
            &["process_id"],
        ),
        "workspace.process/resize" => object(
            json!({
                "process_id":{"type":"string","minLength":1,"maxLength":128},
                "cols":{"type":"integer","minimum":1,"maximum":65535},
                "rows":{"type":"integer","minimum":1,"maximum":65535}
            }),
            &["process_id", "cols", "rows"],
        ),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_identity_selects_each_closed_process_payload() {
        for (action, valid, invalid) in [
            (
                "workspace.process/exec",
                json!({"command":"git","args":["--version"]}),
                json!({"operation":"exec","command":"git"}),
            ),
            (
                "workspace.process/start",
                json!({"command":"git","wait_ms":1,"tty":true,"cols":120,"rows":40}),
                json!({"command":"git","process_id":"old"}),
            ),
            (
                "workspace.process/poll",
                json!({"process_id":"p","wait_ms":1}),
                json!({"process_id":"p","command":"git"}),
            ),
            (
                "workspace.process/input",
                json!({"process_id":"p","input":"hello"}),
                json!({"process_id":"p"}),
            ),
            (
                "workspace.process/resize",
                json!({"process_id":"p","cols":120,"rows":40}),
                json!({"process_id":"p","cols":0,"rows":40}),
            ),
            (
                "workspace.process/cancel",
                json!({"process_id":"p"}),
                json!({"operation":"cancel","process_id":"p"}),
            ),
        ] {
            let schema = process_action_input_schema(action).unwrap().0;
            let validator = jsonschema::validator_for(&schema).unwrap();
            assert!(validator.is_valid(&valid), "valid {action}: {valid}");
            assert!(!validator.is_valid(&invalid), "invalid {action}: {invalid}");
            assert!(schema["properties"].get("operation").is_none());
        }
    }

    #[test]
    fn shell_scripts_are_explicit_and_cannot_be_mixed_with_literal_launch_arguments() {
        let schema=process_action_input_schema("workspace.process/exec").unwrap();
        let validator=jsonschema::validator_for(&schema.0).unwrap();
        for valid in [json!({"cmd":"printf '%s' ok"}),json!({"cmd":"ls -la","args":[]}),
            json!({"command":"program with spaces","args":["literal"]})] { assert!(validator.is_valid(&valid),"{valid}"); }
        for invalid in [json!({"cmd":"ls","command":"ls"}),json!({"cmd":"ls","args":["-la"]}),json!({})] {
            assert!(!validator.is_valid(&invalid),"{invalid}");
        }
    }
}
