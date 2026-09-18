//! Standard model-facing Nomi Tool definitions.
//!
//! These definitions are a presentation layer only. Every Tool is compiled
//! against the immutable Snapshot by `compile_agent_tool_plan`, and execution
//! still goes through the NomiFun Capability Kernel.

use nomifun_agent_contracts::{ActionId, CapabilityId, StrictJsonValue};
use nomifun_chat_model_broker::ChatToolDefinition;
use serde_json::{Value, json};

use crate::kernel::AgentToolExposure;

/// Candidate platform actions known by this adapter. The exact Snapshot and
/// Action grants filter them during compilation; there is no runtime/profile
/// level that can widen or narrow authority.
pub fn standard_agent_tool_exposures() -> Vec<AgentToolExposure> {
    STANDARD_TOOLS
        .iter()
        .map(StandardTool::exposure)
        .collect()
}

struct StandardTool {
    model_name: &'static str,
    capability_id: &'static str,
    action_id: &'static str,
    description: &'static str,
    schema: fn() -> Value,
}

impl StandardTool {
    fn exposure(&self) -> AgentToolExposure {
        AgentToolExposure {
            definition: ChatToolDefinition {
                name: self.model_name.to_owned(),
                description: self.description.to_owned(),
                input_schema: StrictJsonValue((self.schema)()),
                deferred: false,
            },
            capability_id: CapabilityId::from(self.capability_id),
            action_id: ActionId::from(self.action_id),
        }
    }
}

const STANDARD_TOOLS: &[StandardTool] = &[
    StandardTool {
        model_name: "read_file",
        capability_id: "workspace.files",
        action_id: "workspace.files/read",
        description: "Read a workspace file or inspect instruction scope. Default format=text: bounded UTF-8 pages, source at most 8 MiB; start at byte offset 0, follow next_offset with prior expected_sha256 until eof. Offsets/limit are bytes, not lines. FILE_CONTENT_CHANGED means discard prior pages and restart. Text-only missing_ok=true returns workspace_file_absent for genuine absence, never for denied access. format=image: PNG/JPEG/WebP at most 4 MiB, omit offset/limit/missing_ok and submit alone; requires an image-capable exact model route. Returns prepared pixels, not base64 text; images may be resized and must be re-read after history/compaction omitted pixels. Optional expected_sha256 guards text/image source versions. format=instruction_scope: metadata for a file or directory (path=. for workspace root); omit text/image options. Optional recursive=true discovers descendant instruction directories, including hidden/ignored entries, within bounded limits. Check complete/incomplete_reasons and use canonical_path; incomplete is not absence. This is not a filesystem snapshot or proof of shell access scope.",
        schema: read_schema,
    },
    StandardTool {
        model_name: "search_files",
        capability_id: "workspace.files",
        action_id: "workspace.files/search",
        description: "Fresh bounded literal single-line UTF-8 workspace search. Directory walks respect hidden/ignore rules and skip symlink entries. One match per line includes a snippet around the match, byte_offset and whole-source sha256 for read_file. Empty matches are not proof of absence; inspect truncated/incomplete_reasons/files_skipped and narrow path/query when needed. Not a filesystem snapshot.",
        schema: search_schema,
    },
    StandardTool {
        model_name: "git_status",
        capability_id: "workspace.vcs",
        action_id: "workspace.vcs/status",
        description: "Return repository status for the bound workspace.",
        schema: empty_schema,
    },
    StandardTool {
        model_name: "git_diff",
        capability_id: "workspace.vcs",
        action_id: "workspace.vcs/diff",
        description: "Return a repository diff, optionally scoped to one workspace path.",
        schema: optional_path_schema,
    },
    StandardTool {
        model_name: "write_file",
        capability_id: "workspace.files",
        action_id: "workspace.files/write",
        description: "Write a complete UTF-8 text file (at most 8 MiB) through the workspace owner. This replaces the whole file; inspect existing content and prefer apply_patch for focused edits. A prior read is not a write lock.",
        schema: write_schema,
    },
    StandardTool {
        model_name: "apply_patch",
        capability_id: "workspace.files",
        action_id: "workspace.files/patch",
        description: "Apply bounded, ordered, exact line hunks through the workspace owner. Prefer each file's expected_source={kind:existing,sha256:<full read_file digest>} or {kind:absent} after observing absence; this detects changes outside the hunk too. Omission/any preserves legacy line-only matching, not a source-version check. Never remove a rejected guard just to retry; re-read and replan. Supply logical text without CR/LF or a file-leading UTF-8 BOM; source BOM, unchanged line endings and EOF-newline policy are preserved, added lines use the first source ending (LF if none). Nonempty ranges are 1-based; old_lines=0 inserts after old_start source lines (0=BOF), new_lines=0 names the surviving output prefix. Context/remove text must match exactly; re-read on mismatch. All targets and source guards are prepared before writes, with per-file atomic publication and best-effort restoration of existing files, not a multi-file transaction. Failed patches may retain newly created files; inspect zero-based request.files indices in the failure observation and re-read every target before replanning/retrying. A restored result is historical, not a current-state lock. Concurrent native edits remain possible. Each file's written_sha256 identifies published bytes for a later guarded read_file or patch, not task completion.",
        schema: patch_schema,
    },
    StandardTool {
        model_name: "delete_path",
        capability_id: "workspace.files",
        action_id: "workspace.files/delete",
        description: "Delete one workspace-relative file or directory through the owner.",
        schema: path_schema,
    },
    StandardTool {
        model_name: "git_stage",
        capability_id: "workspace.vcs",
        action_id: "workspace.vcs/stage",
        description: "Stage one workspace-relative path.",
        schema: path_schema,
    },
    StandardTool {
        model_name: "exec_command",
        capability_id: "workspace.process",
        action_id: "workspace.process/exec",
        description: "Run one bounded command to a terminal owner result. A zero exit is an observation, not proof that requested verification passed.",
        schema: process_launch_schema,
    },
    StandardTool {
        model_name: "start_process",
        capability_id: "workspace.process",
        action_id: "workspace.process/start",
        description: "Start a turn-owned background or interactive process. Use the returned process_id with the exact process actions; no process survives turn cleanup.",
        schema: process_start_schema,
    },
    StandardTool {
        model_name: "poll_process",
        capability_id: "workspace.process",
        action_id: "workspace.process/poll",
        description: "Poll a turn-owned process from a bounded output cursor and observe its current state/cleanup evidence.",
        schema: process_poll_schema,
    },
    StandardTool {
        model_name: "write_process_stdin",
        capability_id: "workspace.process",
        action_id: "workspace.process/input",
        description: "Write bounded input to a turn-owned process. This is an effect and invalidates older workspace evidence.",
        schema: process_input_schema,
    },
    StandardTool {
        model_name: "close_process_stdin",
        capability_id: "workspace.process",
        action_id: "workspace.process/close_stdin",
        description: "Close stdin for a turn-owned process and then poll it for a terminal observation.",
        schema: process_id_schema,
    },
    StandardTool {
        model_name: "resize_process",
        capability_id: "workspace.process",
        action_id: "workspace.process/resize",
        description: "Resize the PTY of a turn-owned interactive process.",
        schema: process_resize_schema,
    },
    StandardTool {
        model_name: "cancel_process",
        capability_id: "workspace.process",
        action_id: "workspace.process/cancel",
        description: "Cancel a turn-owned process and obtain cleanup evidence; cancellation is not rollback.",
        schema: process_id_schema,
    },
    StandardTool {
        model_name: "read_artifact",
        capability_id: "workspace.artifacts",
        action_id: "workspace.artifacts/read",
        description: "Read one bounded page of an exact Session artifact by content-addressed identity.",
        schema: artifact_read_schema,
    },
    StandardTool {
        model_name: "publish_artifact",
        capability_id: "workspace.artifacts",
        action_id: "workspace.artifacts/publish",
        description: "Publish a workspace file to the Session artifact owner with an optional exact source digest.",
        schema: artifact_publish_schema,
    },
    StandardTool {
        model_name: "git_commit",
        capability_id: "workspace.vcs",
        action_id: "workspace.vcs/commit",
        description: "Create a commit from the staged workspace changes.",
        schema: commit_schema,
    },
    StandardTool {
        model_name: "git_push",
        capability_id: "workspace.vcs",
        action_id: "workspace.vcs/push",
        description: "Publish an explicit refspec to an already configured local/file Git remote. Requires the Agent's workspace.vcs/push grant. Network SSH/HTTPS credentials, force push and ref deletion are unavailable. A timeout or unknown outcome is not permission to retry; inspect platform effect history. Do not push unless the user's task authorizes publication.",
        schema: push_schema,
    },
];

fn empty_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

fn path_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "path": {
                "type": "string",
                "minLength": 1,
                "maxLength": 4096
            }
        },
        "required": ["path"]
    })
}

fn read_schema() -> Value {
    json!({
        "type":"object", "additionalProperties":false, "required":["path"],
        "properties": {
            "format":{"type":"string", "enum":["text", "image", "instruction_scope"], "default":"text"},
            "recursive":{"type":"boolean", "default":false},
            "missing_ok":{"type":"boolean", "default":false},
            "path":{"type":"string", "minLength":1, "maxLength":4096},
            "offset":{"type":"integer", "minimum":0, "maximum":8388608, "default":0},
            "limit":{"type":"integer", "minimum":4, "maximum":16384, "default":16384},
            "expected_sha256":{"type":"string", "pattern":"^[0-9a-f]{64}$"}
        },
        "allOf":[{"if":{"properties":{"offset":{"minimum":1}},"required":["offset"]},
                  "then":{"required":["expected_sha256"]}},
                 {"if":{"properties":{"format":{"const":"image"}},"required":["format"]},
                  "then":{"not":{"anyOf":[{"required":["offset"]},{"required":["limit"]},{"required":["missing_ok"]}]}}},
                 {"if":{"properties":{"format":{"const":"instruction_scope"}},"required":["format"]},
                  "then":{"not":{"anyOf":[{"required":["offset"]},{"required":["limit"]},{"required":["expected_sha256"]},{"required":["missing_ok"]}]}},
                  "else":{"not":{"required":["recursive"]}}}]
    })
}

fn optional_path_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "path": {
                "type": "string",
                "minLength": 1,
                "maxLength": 4096
            }
        }
    })
}

fn search_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "query": {
                "type": "string",
                "minLength": 1,
                "maxLength": 1024
            },
            "path": {
                "type": "string",
                "minLength": 1,
                "maxLength": 4096
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": 200
            }
        },
        "required": ["query"]
    })
}

fn write_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "path": {
                "type": "string",
                "minLength": 1,
                "maxLength": 4096
            },
            "content": {
                "type": "string",
                "maxLength": 8_388_608
            }
        },
        "required": ["path", "content"]
    })
}

fn patch_schema() -> Value {
    let line = json!({
        "oneOf": [
            {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "kind": {"const": "context"},
                    "text": {"type": "string", "maxLength": 1_048_576, "pattern":"^[^\\r\\n\\u0000]*$"}
                },
                "required": ["kind", "text"]
            },
            {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "kind": {"const": "add"},
                    "text": {"type": "string", "maxLength": 1_048_576, "pattern":"^[^\\r\\n\\u0000]*$"}
                },
                "required": ["kind", "text"]
            },
            {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "kind": {"const": "remove"},
                    "text": {"type": "string", "maxLength": 1_048_576, "pattern":"^[^\\r\\n\\u0000]*$"}
                },
                "required": ["kind", "text"]
            }
        ]
    });
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "files": {
                "type": "array",
                "minItems": 1,
                "maxItems": 64,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "path": {
                            "type": "string",
                            "minLength": 1,
                            "maxLength": 4096
                        },
                        "expected_source": {
                            "description":"Bind to the full sha256 from read_file, or to observed absence. Omission/any uses legacy line-only matching. A conflict requires re-reading and replanning, not removing the guard.",
                            "oneOf":[
                                {"type":"object","additionalProperties":false,"properties":{"kind":{"const":"any"}},"required":["kind"]},
                                {"type":"object","additionalProperties":false,"properties":{"kind":{"const":"absent"}},"required":["kind"]},
                                {"type":"object","additionalProperties":false,"properties":{"kind":{"const":"existing"},"sha256":{"type":"string","pattern":"^[0-9a-f]{64}$"}},"required":["kind","sha256"]}
                            ]
                        },
                        "hunks": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 256,
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "old_start": {"type": "integer", "minimum": 0, "maximum":131072},
                                    "old_lines": {"type": "integer", "minimum": 0, "maximum":131072},
                                    "new_start": {"type": "integer", "minimum": 0, "maximum":131072},
                                    "new_lines": {"type": "integer", "minimum": 0, "maximum":131072},
                                    "lines": {
                                        "type": "array",
                                        "minItems":1,
                                        "maxItems": 16384,
                                        "items": line
                                    }
                                },
                                "required": [
                                    "old_start",
                                    "old_lines",
                                    "new_start",
                                    "new_lines",
                                    "lines"
                                ]
                            }
                        }
                    },
                    "required": ["path", "hunks"]
                }
            }
        },
        "required": ["files"]
    })
}

fn process_launch(include_wait: bool) -> Value {
    let mut properties = json!({
        "command":{"type":"string","minLength":1,"maxLength":32768},
        "args":{"type":"array","maxItems":256,"items":{"type":"string","maxLength":65536}},
        "cwd":{"type":"string","maxLength":4096},
        "env":{"type":"object","maxProperties":128,"additionalProperties":{"type":"string","maxLength":65536}},
        "timeout_ms":{"type":"integer","minimum":1,"maximum":600000},
        "tty":{"type":"boolean","default":false},
        "cols":{"type":"integer","minimum":1,"maximum":65535},
        "rows":{"type":"integer","minimum":1,"maximum":65535}
    });
    if include_wait {
        properties["wait_ms"] = json!({"type":"integer","minimum":0,"maximum":30000,"default":0});
    }
    json!({"type":"object","additionalProperties":false,"properties":properties,"required":["command"]})
}

fn process_launch_schema() -> Value {
    process_launch(false)
}

fn process_start_schema() -> Value {
    process_launch(true)
}

fn process_poll_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "process_id":{"type":"string","minLength":1,"maxLength":128},
        "cursor":{"type":"integer","minimum":0,"default":0},
        "wait_ms":{"type":"integer","minimum":0,"maximum":30000,"default":0}
    },"required":["process_id"]})
}

fn process_input_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "process_id":{"type":"string","minLength":1,"maxLength":128},
        "input":{"type":"string","maxLength":1048576}
    },"required":["process_id","input"]})
}

fn process_id_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "process_id":{"type":"string","minLength":1,"maxLength":128}
    },"required":["process_id"]})
}

fn process_resize_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "process_id":{"type":"string","minLength":1,"maxLength":128},
        "cols":{"type":"integer","minimum":1,"maximum":65535},
        "rows":{"type":"integer","minimum":1,"maximum":65535}
    },"required":["process_id","cols","rows"]})
}

fn artifact_read_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "artifact_id":{"type":"string","pattern":"^[0-9a-f]{64}$"},
        "offset":{"type":"integer","minimum":0,"maximum":536870912,"default":0},
        "limit":{"type":"integer","minimum":1,"maximum":1048576,"default":65536}
    },"required":["artifact_id"]})
}

fn artifact_publish_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "path":{"type":"string","minLength":1,"maxLength":4096},
        "expected_sha256":{"type":"string","pattern":"^[0-9a-f]{64}$"}
    },"required":["path"]})
}

fn commit_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "message": {
                "type": "string",
                "minLength": 1,
                "maxLength": 512,
                "pattern":"\\S"
            }
        },
        "required": ["message"]
    })
}

fn push_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "remote": {
                "type": "string",
                "minLength": 1,
                "maxLength": 256
            },
            "refspec": {
                "type": "string",
                "minLength": 1,
                "maxLength": 4096
            },
            "force": {
                "type": "boolean",
                "default": false
            }
        },
        "required": ["remote", "refspec"]
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn tool_names_are_unique_before_exact_snapshot_filtering() {
        let tools = standard_agent_tool_exposures();
        let names = tools
            .iter()
            .map(|tool| tool.definition.name.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(names.len(), tools.len());
    }

    #[test]
    fn full_surface_maps_only_to_canonical_capability_actions() {
        let mut actions = BTreeSet::new();
        for exposure in standard_agent_tool_exposures() {
            assert!(exposure.action_id.as_ref().starts_with(&format!(
                "{}/",
                exposure.capability_id.as_ref()
            )));
            assert!(actions.insert(exposure.action_id.clone()));
            assert_eq!(exposure.definition.input_schema.0["type"], "object");
            assert_eq!(
                exposure.definition.input_schema.0["additionalProperties"],
                false
            );
        }
        let expected = BTreeSet::from([
            "workspace.files/read",
            "workspace.files/search",
            "workspace.files/write",
            "workspace.files/patch",
            "workspace.files/delete",
            "workspace.vcs/status",
            "workspace.vcs/diff",
            "workspace.vcs/stage",
            "workspace.vcs/commit",
            "workspace.vcs/push",
            "workspace.process/exec",
            "workspace.process/start",
            "workspace.process/poll",
            "workspace.process/input",
            "workspace.process/close_stdin",
            "workspace.process/resize",
            "workspace.process/cancel",
            "workspace.artifacts/read",
            "workspace.artifacts/publish",
        ]);
        assert_eq!(
            actions
                .iter()
                .map(|action| action.as_ref())
                .collect::<BTreeSet<_>>(),
            expected
        );
        assert_eq!(actions.len(), 19);
    }
}
