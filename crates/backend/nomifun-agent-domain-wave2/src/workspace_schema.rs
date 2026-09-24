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

pub(super) fn input(action: &str) -> Option<StrictJsonValue> {
    Some(StrictJsonValue(match action {
        "workspace.files/read" => json!({
            "type":"object", "additionalProperties":false, "required":["path"],
            "properties":{
                "format":{"type":"string", "enum":["text", "image", "instruction_scope"], "default":"text"},
                "recursive":{"type":"boolean", "default":false},
                "missing_ok":{"type":"boolean", "default":false,
                    "description":"Use true when checking an optional file such as AGENTS.md so absence is reported as a normal observation."},
                "path":path(),
                "offset":{"type":"integer", "minimum":0, "maximum":8388608, "default":0},
                "limit":{"type":"integer", "minimum":4, "maximum":16384, "default":16384},
                "expected_sha256":{"type":"string", "pattern":"^[0-9a-f]{64}$"}
            },
            "allOf":[
                {"if":{"properties":{"offset":{"minimum":1}},"required":["offset"]},
                 "then":{"required":["expected_sha256"]}},
                {"if":{"properties":{"format":{"const":"image"}},"required":["format"]},
                 "then":{"not":{"anyOf":[{"required":["offset"]},{"required":["limit"]},{"required":["missing_ok"]}]}}},
                {"if":{"properties":{"format":{"const":"instruction_scope"}},"required":["format"]},
                 "then":{"not":{"anyOf":[{"required":["offset"]},{"required":["limit"]},{"required":["expected_sha256"]},{"required":["missing_ok"]}]}},
                 "else":{"not":{"required":["recursive"]}}}
            ]
        }),
        "workspace.files/search" => object(
            json!({
                "query":{"type":"string", "minLength":1, "maxLength":1024, "pattern":"\\S"},
                "path":{"type":"string", "maxLength":4096},
                "limit":{"type":"integer", "minimum":1, "maximum":200, "default":100}
            }),
            &["query"],
        ),
        "workspace.files/write" => object(
            json!({
                "path":path(),
                "content":{"type":"string", "maxLength":8388608,
                    "description":"Complete UTF-8 text. Host also enforces an 8 MiB byte limit. Prefer workspace.files/patch for focused edits."}
            }),
            &["path", "content"],
        ),
        "workspace.files/patch" => patch(),
        "workspace.files/delete" => object(json!({"path":path()}), &["path"]),
        "workspace.vcs/status" => object(json!({}), &[]),
        "workspace.vcs/diff" => object(json!({"path":path()}), &[]),
        "workspace.vcs/stage" => object(json!({"path":path()}), &["path"]),
        "workspace.vcs/commit" => object(
            json!({
                "message":{"type":"string", "minLength":1, "maxLength":512, "pattern":"\\S",
                    "description":"Commit message, at most 512 characters; requires the selected action's existing authority."}
            }),
            &["message"],
        ),
        "workspace.vcs/push" => object(
            json!({
                "remote":{"type":"string", "minLength":1, "maxLength":256},
                "refspec":{"type":"string", "minLength":1, "maxLength":1024,
                    "pattern":"^(HEAD|refs/heads/.+):refs/heads/.+$"},
                "force":{"const":false, "default":false}
            }),
            &["remote", "refspec"],
        ),
        "workspace.artifacts/read" => object(
            json!({
                "artifact_id":{"type":"string", "pattern":"^[0-9a-f]{64}$"},
                "offset":{"type":"integer", "minimum":0, "maximum":536870912, "default":0},
                "limit":{"type":"integer", "minimum":1, "maximum":1048576, "default":16384}
            }),
            &["artifact_id"],
        ),
        "workspace.artifacts/publish" => object(
            json!({
                "path":path(),
                "expected_sha256":{"type":"string", "pattern":"^[0-9a-f]{64}$"}
            }),
            &["path"],
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
