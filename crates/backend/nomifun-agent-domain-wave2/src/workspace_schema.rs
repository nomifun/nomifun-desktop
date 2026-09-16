//! Canonical parameter contracts for existing File/VCS owners. Production
//! engines consume these through exact schema refs, never an empty object
//! substituted for a richer model-facing schema.
use nomifun_agent_contracts::StrictJsonValue;
use serde_json::{Value, json};

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object", "additionalProperties":false, "properties":properties, "required":required})
}

fn path() -> Value {
    json!({"type":"string", "minLength":1, "maxLength":4096, "pattern":"\\S",
        "description":"Workspace-relative path; no native root, traversal or authority fields."})
}

pub(super) fn input(capability: &str) -> Option<StrictJsonValue> {
    Some(StrictJsonValue(match capability {
        "fs.write" => object(
            json!({
                "path":path(),
                "content":{"type":"string", "maxLength":8388608,
                    "description":"Complete UTF-8 text. Host also enforces an 8 MiB byte limit. Prefer fs.patch for focused edits."}
            }),
            &["path", "content"],
        ),
        "fs.patch" => patch(),
        "vcs.status" => object(json!({}), &[]),
        "vcs.diff" => object(json!({"path":path()}), &[]),
        "vcs.stage" => object(json!({"path":path()}), &["path"]),
        "vcs.commit" => object(
            json!({
                "message":{"type":"string", "minLength":1, "maxLength":512, "pattern":"\\S",
                    "description":"Commit message, at most 512 characters; requires the selected action's existing authority."}
            }),
            &["message"],
        ),
        _ => return None,
    }))
}

fn patch() -> Value {
    let line = object(
        json!({
            "kind":{"type":"string", "enum":["context", "add", "remove"]},
            "text":{"type":"string", "maxLength":1048576, "pattern":"^[^\\r\\n\\u0000]*$",
                "description":"Exact logical line text without CR/LF terminators or a file-leading UTF-8 BOM. No fuzzy matching."}
        }),
        &["kind", "text"],
    );
    let hunk = object(
        json!({
            "old_start":{"type":"integer", "minimum":0, "maximum":131072,
                "description":"1-based first source line; for old_lines=0, insert after this many source lines (0=BOF)."},
            "old_lines":{"type":"integer", "minimum":0, "maximum":131072},
            "new_start":{"type":"integer", "minimum":0, "maximum":131072,
                "description":"1-based first output line; for new_lines=0, number of surviving output lines before the deletion."},
            "new_lines":{"type":"integer", "minimum":0, "maximum":131072},
            "lines":{"type":"array", "minItems":1, "maxItems":16384, "items":line}
        }),
        &["old_start", "old_lines", "new_start", "new_lines", "lines"],
    );
    object(
        json!({
            "files":{"type":"array", "minItems":1, "maxItems":64,
                "items":object(json!({
                    "path":path(),
                    "expected_source":{"description":"Optional full-source precondition checked before any publication. Prefer existing with sha256 from read_file, or absent after a missing_ok read. Omission/any preserves legacy line-only matching, not a source-version check. On conflict re-read/replan; never remove the guard just to retry.",
                        "oneOf":[
                            object(json!({"kind":{"const":"any"}}), &["kind"]),
                            object(json!({"kind":{"const":"absent"}}), &["kind"]),
                            object(json!({"kind":{"const":"existing"}, "sha256":{"type":"string","pattern":"^[0-9a-f]{64}$"}}), &["kind","sha256"])
                        ]},
                    "hunks":{"type":"array", "minItems":1, "maxItems":256, "items":hunk}
                }), &["path", "hunks"])}
        }),
        &["files"],
    )
}
