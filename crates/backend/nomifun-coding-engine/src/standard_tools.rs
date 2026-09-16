//! Standard model-facing Coding Tool definitions.
//!
//! These definitions are a presentation layer only. Every Tool is compiled
//! against the immutable Snapshot by `compile_coding_tool_plan`, and execution
//! still goes through the NomiFun Capability Kernel.

use nomifun_agent_contracts::{ActionId, CapabilityId, StrictJsonValue};
use nomifun_chat_model_broker::ChatToolDefinition;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::kernel::CodingToolExposure;

/// Explicit standard Coding surface levels.
///
/// Levels are convenience filters for the Agent Workbench. Snapshot admission
/// remains authoritative: selecting `Full` cannot expose a capability that is
/// absent from the frozen enabled set in the compiled AgentPreset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardCodingToolLevel {
    Inspect,
    Edit,
    Execute,
    Full,
}

pub fn standard_coding_tool_exposures(level: StandardCodingToolLevel) -> Vec<CodingToolExposure> {
    STANDARD_TOOLS
        .iter()
        .filter(|tool| tool.minimum_level <= level)
        .map(StandardTool::exposure)
        .collect()
}

struct StandardTool {
    model_name: &'static str,
    capability_id: &'static str,
    description: &'static str,
    minimum_level: StandardCodingToolLevel,
    schema: fn() -> Value,
}

impl StandardTool {
    fn exposure(&self) -> CodingToolExposure {
        CodingToolExposure {
            definition: ChatToolDefinition {
                name: self.model_name.to_owned(),
                description: self.description.to_owned(),
                input_schema: StrictJsonValue((self.schema)()),
                deferred: false,
            },
            capability_id: CapabilityId::from(self.capability_id),
            action_id: ActionId::from(format!("{}.invoke", self.capability_id)),
        }
    }
}

const STANDARD_TOOLS: &[StandardTool] = &[
    StandardTool {
        model_name: "read_file",
        capability_id: "fs.read",
        description: "Read a workspace file or inspect instruction scope. Default format=text: bounded UTF-8 pages, source at most 8 MiB; start at byte offset 0, follow next_offset with prior expected_sha256 until eof. Offsets/limit are bytes, not lines. FILE_CONTENT_CHANGED means discard prior pages and restart. Text-only missing_ok=true returns workspace_file_absent for genuine absence, never for denied access. format=image: PNG/JPEG/WebP at most 4 MiB, omit offset/limit/missing_ok and submit alone; requires active llm.vision and an image-capable model. Returns prepared pixels, not base64 text; images may be resized and must be re-read after history/compaction omitted pixels. Optional expected_sha256 guards text/image source versions. format=instruction_scope: metadata for a file or directory (path=. for workspace root); omit text/image options. Optional recursive=true discovers descendant instruction directories, including hidden/ignored entries, within bounded limits. Check complete/incomplete_reasons and use canonical_path; incomplete is not absence. This is not a filesystem snapshot or proof of shell access scope.",
        minimum_level: StandardCodingToolLevel::Inspect,
        schema: read_schema,
    },
    StandardTool {
        model_name: "search_files",
        capability_id: "fs.search",
        description: "Fresh bounded literal single-line UTF-8 workspace search. Directory walks respect hidden/ignore rules and skip symlink entries. One match per line includes a snippet around the match, byte_offset and whole-source sha256 for read_file. Empty matches are not proof of absence; inspect truncated/incomplete_reasons/files_skipped and narrow path/query when needed. Not a filesystem snapshot.",
        minimum_level: StandardCodingToolLevel::Inspect,
        schema: search_schema,
    },
    StandardTool {
        model_name: "git_status",
        capability_id: "vcs.status",
        description: "Return repository status for the bound workspace.",
        minimum_level: StandardCodingToolLevel::Inspect,
        schema: empty_schema,
    },
    StandardTool {
        model_name: "git_diff",
        capability_id: "vcs.diff",
        description: "Return a repository diff, optionally scoped to one workspace path.",
        minimum_level: StandardCodingToolLevel::Inspect,
        schema: optional_path_schema,
    },
    StandardTool {
        model_name: "write_file",
        capability_id: "fs.write",
        description: "Write a complete UTF-8 text file (at most 8 MiB) through the workspace owner. This replaces the whole file; inspect existing content and prefer apply_patch for focused edits. A prior read is not a write lock.",
        minimum_level: StandardCodingToolLevel::Edit,
        schema: write_schema,
    },
    StandardTool {
        model_name: "apply_patch",
        capability_id: "fs.patch",
        description: "Apply bounded, ordered, exact line hunks through the workspace owner. Prefer each file's expected_source={kind:existing,sha256:<full read_file digest>} or {kind:absent} after observing absence; this detects changes outside the hunk too. Omission/any preserves legacy line-only matching, not a source-version check. Never remove a rejected guard just to retry; re-read and replan. Supply logical text without CR/LF or a file-leading UTF-8 BOM; source BOM, unchanged line endings and EOF-newline policy are preserved, added lines use the first source ending (LF if none). Nonempty ranges are 1-based; old_lines=0 inserts after old_start source lines (0=BOF), new_lines=0 names the surviving output prefix. Context/remove text must match exactly; re-read on mismatch. All targets and source guards are prepared before writes, with per-file atomic publication and best-effort restoration of existing files, not a multi-file transaction. Failed patches may retain newly created files; inspect zero-based request.files indices in the failure observation and re-read every target before replanning/retrying. A restored result is historical, not a current-state lock. Concurrent native edits remain possible. Each file's written_sha256 identifies published bytes for a later guarded read_file or patch, not task completion.",
        minimum_level: StandardCodingToolLevel::Edit,
        schema: patch_schema,
    },
    StandardTool {
        model_name: "delete_path",
        capability_id: "fs.delete",
        description: "Delete one workspace-relative file or directory through the owner.",
        minimum_level: StandardCodingToolLevel::Edit,
        schema: path_schema,
    },
    StandardTool {
        model_name: "git_stage",
        capability_id: "vcs.stage",
        description: "Stage one workspace-relative path.",
        minimum_level: StandardCodingToolLevel::Edit,
        schema: path_schema,
    },
    StandardTool {
        model_name: "exec_command",
        capability_id: "process.exec",
        description: "Run a bounded command (operation=exec), or start a turn-owned background/interactive process (operation=start). Use the returned process_id with poll, stdin (input), close_stdin, resize (cols/rows), or cancel. A running result is not command success. All processes are cleaned up when this turn ends; none are detached services.",
        minimum_level: StandardCodingToolLevel::Execute,
        schema: process_exec_schema,
    },
    StandardTool {
        model_name: "workspace_snapshot",
        capability_id: "fs.snapshot",
        description: "Initialize a Session-owned workspace baseline, compare changes, read baseline file content, or dispose it. Does not restore files. Git workspaces use Git HEAD; non-Git workspaces use a temporary baseline. The baseline is not preserved after Session runtime teardown or application restart.",
        minimum_level: StandardCodingToolLevel::Execute,
        schema: snapshot_schema,
    },
    StandardTool {
        model_name: "git_commit",
        capability_id: "vcs.commit",
        description: "Create a commit from the staged workspace changes.",
        minimum_level: StandardCodingToolLevel::Full,
        schema: commit_schema,
    },
    StandardTool {
        model_name: "git_push",
        capability_id: "vcs.push",
        description: "Publish an explicit refspec to an already configured local/file Git remote. Requires the Agent's vcs.push grant. Network SSH/HTTPS credentials, force push and ref deletion are unavailable. A timeout or unknown outcome is not permission to retry; inspect platform effect history. Do not push unless the user's task authorizes publication.",
        minimum_level: StandardCodingToolLevel::Full,
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

fn process_exec_schema() -> Value {
    let mut schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "command": {
                "type": "string",
                "minLength": 1,
                "maxLength": 32768
            },
            "args": {
                "type": "array",
                "maxItems": 256,
                "items": {
                    "type": "string",
                    "maxLength": 65536
                }
            },
            "cwd": {
                "type": "string",
                "minLength": 1,
                "maxLength": 4096
            },
            "env": {
                "type": "object",
                "maxProperties": 128,
                "additionalProperties": {
                    "type": "string",
                    "maxLength": 65536
                }
            },
            "timeout_ms": {
                "type": "integer",
                "minimum": 1,
                "maximum": 600000
            }
        },
        "required": ["command"]
    });
    let properties = schema["properties"]
        .as_object_mut()
        .expect("process schema properties");
    properties.insert("operation".into(), json!({"type":"string","enum":["exec","start","poll","stdin","close_stdin","resize","cancel"]}));
    properties.insert(
        "process_id".into(),
        json!({"type":"string","minLength":1,"maxLength":128}),
    );
    properties.insert("input".into(), json!({"type":"string","maxLength":1048576}));
    properties.insert(
        "wait_ms".into(),
        json!({"type":"integer","minimum":0,"maximum":30000}),
    );
    properties.insert("tty".into(), json!({"type":"boolean"}));
    for field in ["cols", "rows"] {
        properties.insert(
            field.into(),
            json!({"type":"integer","minimum":1,"maximum":65535}),
        );
    }
    schema
        .as_object_mut()
        .expect("process schema")
        .remove("required");
    schema["allOf"] = json!([{
        "if":{"properties":{"operation":{"enum":["poll","stdin","close_stdin","resize","cancel"]}},"required":["operation"]},
        "then":{"required":["process_id"]},"else":{"required":["command"]}
    }]);
    schema
}

fn snapshot_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "operation": {
                "type": "string",
                "enum": ["init", "compare", "baseline", "dispose"]
            },
            "path": {
                "type": "string",
                "minLength": 1,
                "maxLength": 4096
            }
        },
        "required": ["operation"]
    })
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
    fn levels_are_monotonic_and_tool_names_are_unique() {
        let inspect = standard_coding_tool_exposures(StandardCodingToolLevel::Inspect);
        let edit = standard_coding_tool_exposures(StandardCodingToolLevel::Edit);
        let execute = standard_coding_tool_exposures(StandardCodingToolLevel::Execute);
        let full = standard_coding_tool_exposures(StandardCodingToolLevel::Full);

        assert!(inspect.len() < edit.len());
        assert!(edit.len() < execute.len());
        assert!(execute.len() < full.len());
        let names = full
            .iter()
            .map(|tool| tool.definition.name.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(names.len(), full.len());
    }

    #[test]
    fn full_surface_maps_only_to_canonical_capability_actions() {
        for exposure in standard_coding_tool_exposures(StandardCodingToolLevel::Full) {
            assert_eq!(
                exposure.action_id.as_ref(),
                format!("{}.invoke", exposure.capability_id.as_ref())
            );
            assert_eq!(exposure.definition.input_schema.0["type"], "object");
            assert_eq!(
                exposure.definition.input_schema.0["additionalProperties"],
                false
            );
        }
    }
}
