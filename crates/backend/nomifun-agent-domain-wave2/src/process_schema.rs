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
            "description":"Explicit shell command/script. Choose exactly one of cmd, command or component; never inferred from their contents."},
        "command":{"type":"string","minLength":1,"maxLength":32768,
            "description":"User executable or native command launcher name/path only, such as git or npm. Choose exactly one of command, cmd or component. Do not put arguments, pipes or a whole shell command here; use args for each argument."},
        "component":{"type":"string","enum":["officecli","bun","node"],
            "description":"Approved host component ID: officecli, bun or node. The host resolves its verified executable and required environment. Choose exactly one of component, command or cmd; pass each literal argument in args, without shell quoting or expansion."},
        "args":{"type":"array","maxItems":256,"items":{"type":"string","maxLength":65536},
            "description":"Separate literal argument tokens, for example [\"test\",\"tests/unit.test.js\"]. No shell quoting or expansion. Omit when empty; never send null."},
        "cwd":{"type":"string","maxLength":4096,"pattern":"^[^\\u0000]*$",
            "description":"Optional native absolute directory or path relative to the Session default working directory. Omit to use that default; parent traversal, native separators, legal links and mounted directories are supported."},
        "execution_identity":{"type":"string","enum":["current_user","administrator"],"default":"current_user",
            "description":"Execution identity required for this operation. Ordinary operations use the real login user. Administrator operations use the platform native authentication and system execution path; independent privacy and desktop permissions still apply."},
        "env":{"type":"object","maxProperties":128,"additionalProperties":{"type":"string","maxLength":65536},
            "description":"Optional string-valued environment overrides. Omit when empty; never send null."},
        "timeout_ms":{"type":"integer","minimum":1,"maximum":600000},
        "tty":{"type":"boolean","default":false},
        "cols":{"type":"integer","minimum":1,"maximum":32767},
        "rows":{"type":"integer","minimum":1,"maximum":32767}
    });
    if include_wait {
        properties["wait_ms"] = json!({"type":"integer","minimum":0,"maximum":30000,"default":0});
    }
    StrictJsonValue(json!({"type":"object","additionalProperties":false,"properties":properties,
        "oneOf":[{"required":["cmd"],"not":{"anyOf":[{"required":["command"]},{"required":["component"]}]},"properties":{"args":{"maxItems":0}}},
                 {"required":["command"],"not":{"anyOf":[{"required":["cmd"]},{"required":["component"]}]}},
                 {"required":["component"],"not":{"anyOf":[{"required":["cmd"]},{"required":["command"]}]}}]}))
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
                "cursor":{"type":"integer","minimum":0,"default":0},
                "wait_ms":{"type":"integer","minimum":0,"maximum":30000,"default":0}
            }),
            &["process_id"],
        ),
        "workspace.process/input" => object(
            json!({
                "process_id":{"type":"string","minLength":1,"maxLength":128},
                "input":{"type":"string","maxLength":1048576,
                    "description":"Exact UTF-8 text to write without trimming or normalization."},
                "append_newline":{"type":"boolean","default":false,
                    "description":"When true, append exactly one LF byte (0x0A) after input. Use this for a requested trailing line feed."}
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
                "cols":{"type":"integer","minimum":1,"maximum":32767},
                "rows":{"type":"integer","minimum":1,"maximum":32767}
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
    fn local_directory_and_execution_identity_are_operation_context() {
        for action in ["workspace.process/exec", "workspace.process/start"] {
            let schema = process_action_input_schema(action).unwrap();
            let validator = jsonschema::validator_for(&schema.0).unwrap();
            for cwd in ["../outside", "/Volumes/Test", r"C:\Users\Test User", r"\\server\share"] {
                for identity in ["current_user", "administrator"] {
                    assert!(validator.is_valid(&json!({"command":"program","cwd":cwd,"execution_identity":identity})));
                }
            }
            assert_eq!(schema.0["properties"]["execution_identity"]["default"], "current_user");
            assert!(!validator.is_valid(&json!({"command":"program","execution_identity":"trusted"})));
            assert!(!validator.is_valid(&json!({"command":"program","permission_mode":"administrator"})));
            assert!(!validator.is_valid(&json!({"command":"program","cwd":"bad\0directory"})));
        }
    }

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
                json!({"process_id":"p","cursor":0,"wait_ms":1}),
                json!({"process_id":"p","cursor":-1}),
            ),
            (
                "workspace.process/input",
                json!({"process_id":"p","input":"hello","append_newline":true}),
                json!({"process_id":"p"}),
            ),
            (
                "workspace.process/resize",
                json!({"process_id":"p","cols":120,"rows":40}),
                json!({"process_id":"p","cols":32768,"rows":40}),
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

    #[test]
    fn launch_targets_are_exclusive_and_component_arguments_remain_literal() {
        for action in ["workspace.process/exec", "workspace.process/start"] {
            let schema = process_action_input_schema(action).unwrap();
            let validator = jsonschema::validator_for(&schema.0).unwrap();
            for component in ["officecli", "bun", "node"] {
                let valid = json!({"component":component,"args":["a path with spaces","$HOME","$(command)","'quoted'","*.xlsx","line\nbreak"]});
                assert!(validator.is_valid(&valid), "valid {action}: {valid}");
            }
            for invalid in [
                json!({"component":"officecli","command":"officecli"}),
                json!({"component":"officecli","cmd":"officecli help"}),
                json!({"component":"officecli","command":"officecli","cmd":"officecli help"}),
                json!({"component":"unknown"}),
                json!({"component":"/usr/local/bin/officecli"}),
                json!({"component":null}),
                json!({"component":"officecli","args":null}),
                json!({"component":"officecli","executable_path":"/user/tool"}),
                json!({"component":"officecli","operation":"start"}),
                json!({"component":"officecli","unexpected":true}),
            ] {
                assert!(!validator.is_valid(&invalid), "invalid {action}: {invalid}");
            }
        }
    }

    #[test]
    fn process_controls_reject_component_launch_fields() {
        for (action, mut input) in [
            ("workspace.process/poll", json!({"process_id":"p"})),
            ("workspace.process/input", json!({"process_id":"p","input":"text"})),
            ("workspace.process/close_stdin", json!({"process_id":"p"})),
            ("workspace.process/resize", json!({"process_id":"p","cols":120,"rows":40})),
            ("workspace.process/cancel", json!({"process_id":"p"})),
        ] {
            let schema = process_action_input_schema(action).unwrap();
            let validator = jsonschema::validator_for(&schema.0).unwrap();
            assert!(validator.is_valid(&input), "valid {action}: {input}");
            input["component"] = json!("officecli");
            assert!(!validator.is_valid(&input), "invalid {action}: {input}");
        }
    }

    #[test]
    fn stdin_input_contract_preserves_requested_line_endings() {
        let schema = process_action_input_schema("workspace.process/input")
            .unwrap()
            .0;
        let description = schema["properties"]["input"]["description"]
            .as_str()
            .unwrap();
        assert!(description.contains("without trimming or normalization"));
        assert_eq!(schema["properties"]["append_newline"]["default"], false);
        assert!(schema["properties"]["append_newline"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("0x0A")));
    }
}
