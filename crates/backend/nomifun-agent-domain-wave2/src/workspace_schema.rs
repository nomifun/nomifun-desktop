//! Canonical parameter contracts for existing File/VCS owners. Production
//! engines consume these through exact schema refs, never an empty object
//! substituted for a richer model-facing schema.
use nomifun_agent_contracts::StrictJsonValue;
use serde_json::{Value, json};

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object", "additionalProperties":false, "properties":properties, "required":required})
}

fn path() -> Value {
    json!({"type":"string", "minLength":1, "maxLength":4096, "pattern":"^[^\\u0000]+$",
        "description":"Local native absolute path or a path relative to the Session default working directory. Parent traversal, legal links and mounted volumes are supported; the default directory is not an access boundary."})
}

fn project_path() -> Value {
    json!({"type":"string", "minLength":1, "maxLength":4096, "pattern":"\\S",
        "description":"Path inside the bound project; project/VCS/artifact identity remains distinct from global local file access."})
}

pub(super) fn input(action: &str) -> Option<StrictJsonValue> {
    Some(StrictJsonValue(match action {
        "workspace.files/read" => json!({
            "type":"object", "additionalProperties":false, "required":["path"],
            "properties":{
                "format":{"type":"string", "enum":["text", "image", "instruction_scope"], "default":"text"},
                "execution_identity":{"type":"string", "enum":["current_user", "administrator"], "default":"current_user",
                    "description":"Identity required for this read-only operation, for every read format. Administrator reads use the existing native system execution path; independent privacy permissions still apply. This does not authorize writes or change the Session's frozen tool selection."},
                "recursive":{"type":"boolean", "default":false},
                "missing_ok":{"type":"boolean", "default":false,
                    "description":"Use true when checking an optional file such as AGENTS.md so absence is reported as a normal observation."},
                "path":path(),
                "start_line":{"type":"integer", "minimum":1, "maximum":8388609},
                "line_count":{"type":"integer", "minimum":1, "maximum":2000, "default":200},
                "offset":{"type":"integer", "minimum":0, "maximum":8388608, "default":0},
                "limit":{"type":"integer", "minimum":4, "maximum":16384, "default":16384},
                "expected_sha256":{"type":"string", "pattern":"^[0-9a-f]{64}$"}
            },
            "allOf":[{"if":{"properties":{"format":{"enum":["image","instruction_scope"]}},"required":["format"]},"then":{"properties":{"missing_ok":{"const":false}}}},
                {"if":{"anyOf":[{"required":["start_line"]},{"required":["line_count"]}]},
                 "then":{"not":{"anyOf":[{"required":["offset"]},{"required":["limit"]}]}}},
                {"if":{"properties":{"format":{"const":"image"}},"required":["format"]},
                 "then":{"not":{"anyOf":[{"required":["offset"]},{"required":["limit"]},{"required":["start_line"]},{"required":["line_count"]}]}}},
                {"if":{"properties":{"format":{"const":"instruction_scope"}},"required":["format"]},
                 "then":{"not":{"anyOf":[{"required":["offset"]},{"required":["limit"]},{"required":["expected_sha256"]},{"required":["start_line"]},{"required":["line_count"]}]}},
                 "else":{"properties":{"recursive":{"const":false}}}}
            ]
        }),
        "workspace.files/search" => object(
            json!({
                "query":{"type":"string", "minLength":1, "maxLength":1024, "pattern":"\\S"},
                "path":{"type":"string", "maxLength":4096, "pattern":"^[^\\u0000]*$",
                    "description":"Local native absolute file/directory or Session-default-directory relative path; omission searches the default directory."},
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
        "workspace.files/rename" => object(json!({
            "path":path(),
            "destination":path(),
        }), &["path", "destination"]),
        "workspace.vcs/status" => object(json!({}), &[]),
        "workspace.vcs/diff" => object(json!({"path":project_path()}), &[]),
        "workspace.vcs/stage" => object(json!({"path":project_path()}), &["path"]),
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
                "path":project_path(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_identity_applies_to_every_format_without_granting_file_writes() {
        let schema = input("workspace.files/read").unwrap();
        let validator = jsonschema::validator_for(&schema.0).unwrap();
        assert_eq!(schema.0["properties"]["execution_identity"]["default"], "current_user");
        for identity in ["current_user", "administrator"] {
            for mut arguments in [
                json!({"path":"/private/var/root/AGENTS.md","start_line":1,"line_count":8}),
                json!({"path":"protected/report.txt","format":"text","offset":0,"limit":128,"expected_sha256":"a".repeat(64)}),
                json!({"path":"protected/optional.txt","missing_ok":true}),
                json!({"path":"protected/image.png","format":"image","expected_sha256":"a".repeat(64)}),
                json!({"path":"/private/var/root","format":"instruction_scope","recursive":true}),
            ] {
                arguments["execution_identity"] = json!(identity);
                assert!(validator.is_valid(&arguments), "{arguments}");
            }
        }
        for invalid in [
            json!({"path":"file","execution_identity":"root"}),
            json!({"path":"file","execution_identity":null}),
            json!({"path":"file","permission_mode":"administrator"}),
            json!({"path":"protected","format":"instruction_scope","execution_identity":"administrator","missing_ok":true}),
            json!({"path":"protected","format":"instruction_scope","execution_identity":"administrator","offset":0}),
            json!({"path":"protected/image.png","format":"image","execution_identity":"administrator","line_count":1}),
        ] { assert!(!validator.is_valid(&invalid), "{invalid}"); }
        for (action, arguments) in [
            ("workspace.files/write", json!({"path":"file","content":"text","execution_identity":"administrator"})),
            ("workspace.files/delete", json!({"path":"file","execution_identity":"administrator"})),
            ("workspace.files/rename", json!({"path":"file","destination":"other","execution_identity":"administrator"})),
            ("workspace.files/search", json!({"query":"text","execution_identity":"administrator"})),
        ] {
            assert!(!jsonschema::validator_for(&input(action).unwrap().0).unwrap().is_valid(&arguments), "{action}");
        }
    }

    #[test]
    fn canonical_file_paths_allow_native_absolute_parent_and_volume_paths() {
        for action in ["workspace.files/read", "workspace.files/write", "workspace.files/delete", "workspace.files/rename"] {
            let schema = input(action).unwrap();
            let validator = jsonschema::validator_for(&schema.0).unwrap();
            for path in ["../outside/report.txt", "/Volumes/Test/report.txt", r"C:\Users\Test User\报告.txt", r"\\server\share\report.txt", " "] {
                let mut arguments = json!({"path":path});
                if action == "workspace.files/write" { arguments["content"] = json!("exact text"); }
                if action == "workspace.files/rename" { arguments["destination"] = json!(format!("{path}.renamed")); }
                assert!(validator.is_valid(&arguments), "{action}: {arguments}");
            }
            let mut invalid = json!({"path":"bad\0path"});
            if action == "workspace.files/write" { invalid["content"] = json!("text"); }
            if action == "workspace.files/rename" { invalid["destination"] = json!("valid"); }
            assert!(!validator.is_valid(&invalid));
        }
        let schema = input("workspace.files/patch").unwrap();
        assert!(jsonschema::validator_for(&schema.0).unwrap().is_valid(&json!({"files":[{
            "path":"../outside/report.txt", "expected_source":{"kind":"absent"},
            "hunks":[{"old_start":0,"old_lines":0,"new_start":1,"new_lines":1,"lines":[{"kind":"add","text":"created"}]}]
        }]})));
    }
}
