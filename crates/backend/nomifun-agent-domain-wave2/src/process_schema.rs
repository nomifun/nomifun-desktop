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
    StrictJsonValue(json!({"type":"object","oneOf":variants}))
}
