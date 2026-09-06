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
/// absent or inactive in the compiled AgentPreset.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum StandardCodingToolLevel {
    Inspect,
    Edit,
    Execute,
    Full,
}

pub fn standard_coding_tool_exposures(
    level: StandardCodingToolLevel,
) -> Vec<CodingToolExposure> {
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
        description: "Read one UTF-8 text file from the bound workspace.",
        minimum_level: StandardCodingToolLevel::Inspect,
        schema: path_schema,
    },
    StandardTool {
        model_name: "search_files",
        capability_id: "fs.search",
        description: "Search UTF-8 workspace files for an exact text fragment.",
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
        description: "Write a complete UTF-8 text file through the workspace owner.",
        minimum_level: StandardCodingToolLevel::Edit,
        schema: write_schema,
    },
    StandardTool {
        model_name: "apply_patch",
        capability_id: "fs.patch",
        description: "Apply a bounded, typed, atomic text patch through the workspace owner.",
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
        description: "Run one managed command in the bound workspace with a bounded timeout.",
        minimum_level: StandardCodingToolLevel::Execute,
        schema: process_exec_schema,
    },
    StandardTool {
        model_name: "workspace_snapshot",
        capability_id: "fs.snapshot",
        description: "Create, compare, restore the baseline, or dispose a workspace snapshot.",
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
        description: "Push an explicit refspec to an explicit configured remote.",
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
                "maxLength": 4096
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
                    "text": {"type": "string", "maxLength": 1_048_576}
                },
                "required": ["kind", "text"]
            },
            {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "kind": {"const": "add"},
                    "text": {"type": "string", "maxLength": 1_048_576}
                },
                "required": ["kind", "text"]
            },
            {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "kind": {"const": "remove"},
                    "text": {"type": "string", "maxLength": 1_048_576}
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
                        "hunks": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 256,
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "old_start": {"type": "integer", "minimum": 0},
                                    "old_lines": {"type": "integer", "minimum": 0},
                                    "new_start": {"type": "integer", "minimum": 0},
                                    "new_lines": {"type": "integer", "minimum": 0},
                                    "lines": {
                                        "type": "array",
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
    json!({
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
    })
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
                "maxLength": 65536
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
