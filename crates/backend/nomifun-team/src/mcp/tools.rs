use nomifun_db::models::AgentMetadataRow;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::scheduler::SchedulerAction;
use crate::types::TeammateRole;

// ---------------------------------------------------------------------------
// Tool description constants (原样复用 Nomi `toolDescriptions.ts`)
// ---------------------------------------------------------------------------

/// `team_spawn_agent` 工具描述 — 原样复制自 Nomi `toolDescriptions.ts`
/// 对应 team-prompts.md §5.2 `team_spawn_agent` Description 原文。
/// 禁止翻译、改写；nomifun-audit §8 #5 硬约束。
pub const TEAM_SPAWN_AGENT_DESCRIPTION: &str = r#"Create a new teammate agent to join the team.

Use this only when one of the following is true:
- The user explicitly approved the proposed teammate lineup in a previous message
- The user explicitly instructed you to create a specific teammate immediately

Before calling this tool in the normal planning flow:
- Start with one short sentence explaining why additional teammates would help
- Tell the user which teammate(s) you recommend
- Present the proposal as a table with: name, responsibility, recommended agent type/backend, and recommended model
- Include each teammate's responsibility, recommended agent type/backend, and model
- Ask whether to create them as proposed or change any names, responsibilities, or agent types
- In that approval question, remind the user that they can later ask you to replace or adjust any teammate if the lineup is not working well
- Do NOT call this tool in that same turn; wait for explicit approval in a later user message

When calling this tool, provide the model parameter if a specific model was recommended and approved.

The new agent will be created and added to the team. You can then assign tasks and send messages to it."#;

/// Description for `team_list_models` — verbatim from team-prompts.md §5.2.
pub const TEAM_LIST_MODELS_DESCRIPTION: &str = "Query available models for team agent types. Returns the real-time model list that matches the frontend model selector.

Use this to:
- Check what models are available before spawning an agent with a specific model
- See all available agent types and their models at once
- Verify a model ID is valid for a given agent type

Pass agent_type to query a specific backend, or omit it to see all.";

/// Description for `team_describe_assistant` — verbatim from team-prompts.md §5.2.
pub const TEAM_DESCRIBE_ASSISTANT_DESCRIPTION: &str =
    "Get detailed information about a preset assistant before spawning it as a teammate.

Returns the preset's full description, enabled skills, and example tasks so you can
judge whether it fits the user's request. Use this when two or more presets look
relevant from the one-line catalog in your system prompt.

Only works on preset assistants listed in \"Available Preset Assistants for Spawning\".
After confirming a match, call team_spawn_agent with the same custom_agent_id.";

// ---------------------------------------------------------------------------
// Tool descriptors (returned by tools/list)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub fn all_tool_descriptors() -> Vec<ToolDescriptor> {
    vec![
        ToolDescriptor {
            name: "team_send_message".into(),
            description: "Send a message to a teammate or broadcast to all (to=\"*\").".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "to": { "type": "string", "description": "Target agent slot_id or \"*\" for broadcast" },
                    "message": { "type": "string", "description": "Message content" }
                },
                "required": ["to", "message"]
            }),
        },
        ToolDescriptor {
            name: "team_spawn_agent".into(),
            description: TEAM_SPAWN_AGENT_DESCRIPTION.into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Agent display name" },
                    "agent_type": { "type": "string", "description": "Agent type/backend to use (e.g. \"claude\", \"codex\", \"codebuddy\", \"gemini\"). Query team_list_models first to see available options." },
                    "model": { "type": "string", "description": "Specific model ID to use (e.g. \"claude-sonnet-4\"). Must be a valid model for the chosen agent_type. Query team_list_models to see available models." },
                    "custom_agent_id": { "type": "string", "description": "Preset assistant ID to spawn (from the Available Preset Assistants catalog). When set, agent_type is derived from the preset's backend." },
                    "backend": { "type": "string", "description": "Legacy alias for agent_type. Prefer agent_type." },
                    "role": { "type": "string", "description": "Agent role (default: 'teammate')" }
                },
                "required": ["name"]
            }),
        },
        ToolDescriptor {
            name: "team_task_create".into(),
            description: "Create a new task on the team task board.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "subject": { "type": "string", "description": "Task subject" },
                    "description": { "type": "string", "description": "Task description" },
                    "owner": { "type": "string", "description": "Owning agent slotId" },
                    "blocked_by": { "type": "array", "items": { "type": "string" }, "description": "Task IDs this task depends on" }
                },
                "required": ["subject"]
            }),
        },
        ToolDescriptor {
            name: "team_task_update".into(),
            description: "Update an existing task on the team task board.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task ID to update" },
                    "status": { "type": "string", "description": "New status: pending, in_progress, completed, deleted" },
                    "description": { "type": "string", "description": "New description" },
                    "owner": { "type": "string", "description": "New owning agent slotId" },
                    "blocked_by": { "type": "array", "items": { "type": "string" }, "description": "New dependency list" }
                },
                "required": ["task_id"]
            }),
        },
        ToolDescriptor {
            name: "team_task_list".into(),
            description: "List all tasks on the team task board.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDescriptor {
            name: "team_members".into(),
            description: "List all team members with their roles and current status.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDescriptor {
            name: "team_rename_agent".into(),
            description: "Rename a team member.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "slot_id": { "type": "string", "description": "Agent slot_id to rename" },
                    "new_name": { "type": "string", "description": "New display name" }
                },
                "required": ["slot_id", "new_name"]
            }),
        },
        ToolDescriptor {
            name: "team_shutdown_agent".into(),
            description: "Initiate shutdown of a teammate (Lead only). Sends a shutdown_request to the target agent."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "slot_id": { "type": "string", "description": "Agent slot_id to shut down" },
                    "reason": { "type": "string", "description": "Reason for shutdown" }
                },
                "required": ["slot_id"]
            }),
        },
        ToolDescriptor {
            name: "team_describe_assistant".into(),
            description: TEAM_DESCRIBE_ASSISTANT_DESCRIPTION.into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "custom_agent_id": { "type": "string", "description": "The preset assistant ID from the \"Available Preset Assistants\" catalog (e.g., \"word-creator\")." },
                    "locale": { "type": "string", "description": "Locale like \"zh-CN\" or \"en-US\". Defaults to the user's current UI language when omitted." }
                },
                "required": ["custom_agent_id"]
            }),
        },
        ToolDescriptor {
            name: "team_list_models".into(),
            description: TEAM_LIST_MODELS_DESCRIPTION.into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_type": { "type": "string", "description": "Agent type/backend to query (e.g. \"gemini\", \"claude\", \"codex\"). Shows all when omitted." }
                }
            }),
        },
    ]
}

// ---------------------------------------------------------------------------
// Tool call input types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SendMessageInput {
    pub to: String,
    pub message: String,
}

/// Arguments for the `team_spawn_agent` MCP tool call.
///
/// The Nomi contract (`docs/teams/phase1/nomifun-audit.md` §2.1) names the
/// agent-type field `agent_type` and adds `custom_agent_id` + `model`. The
/// phase-1 Rust dispatch originally exposed `backend` (and `role`); those are
/// preserved for back-compat and used as fallbacks when the modern fields
/// are not provided — `backend` is treated as an alias for `agent_type`.
#[derive(Debug, Default, Deserialize)]
pub struct SpawnAgentInput {
    pub name: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub custom_agent_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TaskCreateInput {
    pub subject: String,
    pub description: Option<String>,
    pub owner: Option<String>,
    pub blocked_by: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct TaskUpdateInput {
    pub task_id: String,
    pub status: Option<String>,
    pub description: Option<String>,
    pub owner: Option<String>,
    pub blocked_by: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct RenameAgentInput {
    pub slot_id: String,
    pub new_name: String,
}

#[derive(Debug, Deserialize)]
pub struct ShutdownAgentInput {
    pub slot_id: String,
    pub reason: Option<String>,
}

// ---------------------------------------------------------------------------
// Backend whitelist for spawn_agent (hard whitelist only — synchronous fast-path).
// Dynamic capability check (MCP-based) happens in TeamSession::spawn_agent.
// ---------------------------------------------------------------------------

pub fn is_whitelisted_backend(backend: &str) -> bool {
    nomifun_common::constants::TEAM_CAPABLE_BACKENDS.contains(&backend)
}

// ---------------------------------------------------------------------------
// Parse tool call into SchedulerAction
// ---------------------------------------------------------------------------

pub fn parse_tool_call(
    tool_name: &str,
    arguments: &Value,
    caller_role: TeammateRole,
) -> Result<SchedulerAction, String> {
    match tool_name {
        "team_send_message" => {
            let input: SendMessageInput = serde_json::from_value(arguments.clone())
                .map_err(|e| format!("Invalid arguments for team_send_message: {e}"))?;
            Ok(SchedulerAction::SendMessage {
                to: input.to,
                message: input.message,
            })
        }
        "team_spawn_agent" => {
            if caller_role != TeammateRole::Lead {
                return Err("Only Lead can spawn agents".into());
            }
            let input: SpawnAgentInput = serde_json::from_value(arguments.clone())
                .map_err(|e| format!("Invalid arguments for team_spawn_agent: {e}"))?;
            let backend = input
                .agent_type
                .clone()
                .or(input.backend.clone())
                .ok_or_else(|| "Missing 'agent_type' (or legacy 'backend') for team_spawn_agent".to_string())?;
            if !is_whitelisted_backend(&backend) {
                return Err(format!(
                    "Backend '{}' not in hard whitelist. Whitelist: {}",
                    backend,
                    nomifun_common::constants::TEAM_CAPABLE_BACKENDS.join(", ")
                ));
            }
            Ok(SchedulerAction::SpawnAgent {
                name: input.name,
                role: input.role.unwrap_or_else(|| "teammate".into()),
                backend,
            })
        }
        "team_task_create" => {
            let input: TaskCreateInput = serde_json::from_value(arguments.clone())
                .map_err(|e| format!("Invalid arguments for team_task_create: {e}"))?;
            Ok(SchedulerAction::TaskCreate {
                subject: input.subject,
                description: input.description,
                owner: input.owner,
                blocked_by: input.blocked_by.unwrap_or_default(),
            })
        }
        "team_task_update" => {
            let input: TaskUpdateInput = serde_json::from_value(arguments.clone())
                .map_err(|e| format!("Invalid arguments for team_task_update: {e}"))?;
            Ok(SchedulerAction::TaskUpdate {
                task_id: input.task_id,
                status: input.status,
                description: input.description,
                owner: input.owner,
                blocked_by: input.blocked_by,
            })
        }
        "team_task_list"
        | "team_members"
        | "team_rename_agent"
        | "team_shutdown_agent"
        | "team_list_models"
        | "team_describe_assistant" => Err("handled directly by server".into()),
        _ => Err(format!("Unknown tool: {tool_name}")),
    }
}

// ---------------------------------------------------------------------------
// Phase-1 minimal handlers for `team_list_models` and `team_describe_assistant`
// ---------------------------------------------------------------------------

/// Phase-1 minimal `team_list_models` handler. Returns a hard-coded
/// agent-type → models mapping. Used as fallback when DB is unavailable.
pub fn handle_team_list_models(_args: &Value) -> Value {
    json!({
        "agent_types": [
            {
                "type": "claude",
                "models": ["claude-sonnet-4", "claude-opus-4"]
            },
            {
                "type": "codex",
                "models": ["codex-mini-latest"]
            }
        ]
    })
}

/// Build `team_list_models` response from DB rows. Reads each enabled,
/// team-capable backend's `available_models` column. Filters by
/// `agent_type` if provided. For internal agents (backend=NULL),
/// `provider_models` supplies the aggregated models from the providers table.
pub fn build_list_models_from_rows(
    rows: &[AgentMetadataRow],
    agent_type_filter: Option<&str>,
    provider_models: &[String],
) -> Value {
    use nomifun_api_types::BehaviorPolicy;
    use nomifun_common::constants::is_team_capable;

    let mut agent_types: Vec<Value> = Vec::new();

    for row in rows {
        if !row.enabled {
            continue;
        }
        // Use backend if present, otherwise agent_type as identifier (handles nomi with backend=NULL)
        let key = match row.backend.as_deref() {
            Some(b) => b.to_owned(),
            None => row.agent_type.clone(),
        };
        let is_internal = row.backend.is_none();

        // Check team capability: behavior_policy.supports_team OR legacy whitelist+MCP detection
        let bp_supports = row
            .behavior_policy
            .as_deref()
            .and_then(|s| serde_json::from_str::<BehaviorPolicy>(s).ok())
            .is_some_and(|bp| bp.supports_team);
        if !bp_supports {
            let caps = row
                .agent_capabilities
                .as_deref()
                .and_then(|s| serde_json::from_str::<Value>(s).ok());
            if !is_team_capable(&key, caps.as_ref()) {
                continue;
            }
        }

        // Apply agent_type filter
        if let Some(filter) = agent_type_filter
            && key != filter
        {
            continue;
        }

        // For internal agents (nomi), use provider models
        if is_internal && !provider_models.is_empty() {
            agent_types.push(json!({
                "type": key,
                "models": provider_models,
            }));
            continue;
        }

        // Parse available_models from DB.
        // Format is either:
        //   {"current_model_id":"...", "available_models": [{"id":"...", "label":"..."}]}
        // or legacy array:
        //   [{"id":"...", "name":"..."}]
        let models: Vec<String> = row
            .available_models
            .as_deref()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .and_then(|v| {
                // Try object with "available_models" key first (ModelInfoPayload format)
                if let Some(arr) = v.get("available_models").and_then(Value::as_array) {
                    let ids: Vec<String> = arr
                        .iter()
                        .filter_map(|e| e.get("id").and_then(Value::as_str).map(String::from))
                        .collect();
                    if !ids.is_empty() {
                        return Some(ids);
                    }
                }
                // Fallback: try parsing as direct array
                if let Some(arr) = v.as_array() {
                    let ids: Vec<String> = arr
                        .iter()
                        .filter_map(|e| e.get("id").and_then(Value::as_str).map(String::from))
                        .collect();
                    if !ids.is_empty() {
                        return Some(ids);
                    }
                }
                None
            })
            .unwrap_or_default();

        agent_types.push(json!({
            "type": key,
            "models": models,
        }));
    }

    json!({ "agent_types": agent_types })
}

/// Phase-1 minimal `team_describe_assistant` handler. Backend has no preset
/// assistants wired yet, so every call returns the not-found text.
pub fn handle_team_describe_assistant(_args: &Value) -> String {
    "Preset assistant not found".to_owned()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_descriptors_count() {
        assert_eq!(all_tool_descriptors().len(), 10);
    }

    #[test]
    fn descriptor_names_are_unique() {
        let descs = all_tool_descriptors();
        let mut names: Vec<&str> = descs.iter().map(|d| d.name.as_str()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 10);
    }

    #[test]
    fn descriptors_have_required_fields() {
        for d in all_tool_descriptors() {
            assert!(!d.name.is_empty());
            assert!(!d.description.is_empty());
            assert_eq!(d.input_schema["type"], "object");
        }
    }

    #[test]
    fn team_spawn_agent_description_is_nomifun_original() {
        let desc = all_tool_descriptors()
            .into_iter()
            .find(|d| d.name == "team_spawn_agent")
            .expect("team_spawn_agent descriptor must exist")
            .description;
        assert_eq!(desc, TEAM_SPAWN_AGENT_DESCRIPTION);
        assert!(
            desc.contains("Before calling this tool"),
            "description must be the full Nomi original, not the legacy one-liner"
        );
        assert!(
            desc.contains("explicitly approved"),
            "description must retain the explicit-approval precondition clause"
        );
    }

    #[test]
    fn team_spawn_agent_schema_exposes_model_and_agent_type() {
        let desc = all_tool_descriptors()
            .into_iter()
            .find(|d| d.name == "team_spawn_agent")
            .unwrap();
        let props = desc.input_schema["properties"].as_object().unwrap();
        assert!(props.contains_key("model"), "schema must expose 'model' field");
        assert!(
            props.contains_key("agent_type"),
            "schema must expose 'agent_type' field"
        );
        assert!(
            props.contains_key("custom_agent_id"),
            "schema must expose 'custom_agent_id' field"
        );
    }

    #[test]
    fn team_spawn_agent_schema_required_is_only_name() {
        let desc = all_tool_descriptors()
            .into_iter()
            .find(|d| d.name == "team_spawn_agent")
            .unwrap();
        let required = desc.input_schema["required"].as_array().unwrap();
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"name"), "name must be required");
        assert!(
            !names.contains(&"backend"),
            "backend should not be required (agent_type is preferred, backend is legacy alias)"
        );
    }

    #[test]
    fn parse_send_message() {
        let args = json!({"to": "slot-1", "message": "hello"});
        let action = parse_tool_call("team_send_message", &args, TeammateRole::Teammate).unwrap();
        assert!(matches!(
            action,
            SchedulerAction::SendMessage { to, message }
            if to == "slot-1" && message == "hello"
        ));
    }

    #[test]
    fn parse_spawn_agent_lead_ok() {
        let args = json!({"name": "Helper", "backend": "claude"});
        let action = parse_tool_call("team_spawn_agent", &args, TeammateRole::Lead).unwrap();
        assert!(matches!(
            action,
            SchedulerAction::SpawnAgent { name, backend, role }
            if name == "Helper" && backend == "claude" && role == "teammate"
        ));
    }

    #[test]
    fn parse_spawn_agent_teammate_rejected() {
        let args = json!({"name": "X", "backend": "claude"});
        let result = parse_tool_call("team_spawn_agent", &args, TeammateRole::Teammate);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Only Lead"));
    }

    #[test]
    fn parse_spawn_agent_bad_backend() {
        let args = json!({"name": "X", "backend": "malicious"});
        let result = parse_tool_call("team_spawn_agent", &args, TeammateRole::Lead);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not in hard whitelist"));
    }

    #[test]
    fn parse_task_create() {
        let args = json!({"subject": "Implement X", "owner": "slot-a"});
        let action = parse_tool_call("team_task_create", &args, TeammateRole::Teammate).unwrap();
        assert!(matches!(
            action,
            SchedulerAction::TaskCreate { subject, owner, .. }
            if subject == "Implement X" && owner == Some("slot-a".into())
        ));
    }

    #[test]
    fn parse_task_update() {
        let args = json!({"task_id": "tk-1", "status": "completed"});
        let action = parse_tool_call("team_task_update", &args, TeammateRole::Teammate).unwrap();
        assert!(matches!(
            action,
            SchedulerAction::TaskUpdate { task_id, status, .. }
            if task_id == "tk-1" && status == Some("completed".into())
        ));
    }

    #[test]
    fn unknown_tool_errors() {
        let result = parse_tool_call("unknown_tool", &json!({}), TeammateRole::Lead);
        assert!(result.is_err());
    }

    #[test]
    fn whitelist_check() {
        assert!(is_whitelisted_backend("claude"));
        assert!(is_whitelisted_backend("codex"));
        assert!(!is_whitelisted_backend("gpt"));
        assert!(!is_whitelisted_backend(""));
    }

    #[test]
    fn parse_send_message_missing_field() {
        let args = json!({"to": "slot-1"});
        let result = parse_tool_call("team_send_message", &args, TeammateRole::Teammate);
        assert!(result.is_err());
    }

    #[test]
    fn parse_spawn_with_explicit_role() {
        let args = json!({"name": "W", "role": "worker", "backend": "codex"});
        let action = parse_tool_call("team_spawn_agent", &args, TeammateRole::Lead).unwrap();
        assert!(matches!(
            action,
            SchedulerAction::SpawnAgent { role, .. }
            if role == "worker"
        ));
    }

    #[test]
    fn task_create_with_blocked_by() {
        let args = json!({"subject": "Test", "blocked_by": ["tk-a", "tk-b"]});
        let action = parse_tool_call("team_task_create", &args, TeammateRole::Lead).unwrap();
        assert!(matches!(
            action,
            SchedulerAction::TaskCreate { blocked_by, .. }
            if blocked_by == vec!["tk-a", "tk-b"]
        ));
    }

    #[test]
    fn parse_task_list_handled_by_server() {
        let result = parse_tool_call("team_task_list", &json!({}), TeammateRole::Teammate);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("handled directly by server"));
    }

    #[test]
    fn parse_members_handled_by_server() {
        let result = parse_tool_call("team_members", &json!({}), TeammateRole::Lead);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("handled directly by server"));
    }

    #[test]
    fn parse_rename_agent_handled_by_server() {
        let args = json!({"slot_id": "s1", "new_name": "X"});
        let result = parse_tool_call("team_rename_agent", &args, TeammateRole::Lead);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("handled directly by server"));
    }

    #[test]
    fn parse_shutdown_agent_handled_by_server() {
        let args = json!({"slot_id": "s1"});
        let result = parse_tool_call("team_shutdown_agent", &args, TeammateRole::Lead);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("handled directly by server"));
    }

    // ---- D4 descriptor text matches team-prompts.md §5.2 verbatim ----

    #[test]
    fn team_list_models_descriptor_text_matches() {
        let desc = all_tool_descriptors()
            .into_iter()
            .find(|d| d.name == "team_list_models")
            .expect("team_list_models descriptor missing");
        assert_eq!(desc.description, TEAM_LIST_MODELS_DESCRIPTION);
        assert!(
            desc.description
                .starts_with("Query available models for team agent types.")
        );
        assert!(
            desc.description
                .contains("Pass agent_type to query a specific backend, or omit it to see all.")
        );
    }

    #[test]
    fn team_describe_assistant_descriptor_text_matches() {
        let desc = all_tool_descriptors()
            .into_iter()
            .find(|d| d.name == "team_describe_assistant")
            .expect("team_describe_assistant descriptor missing");
        assert_eq!(desc.description, TEAM_DESCRIBE_ASSISTANT_DESCRIPTION);
        assert!(
            desc.description
                .starts_with("Get detailed information about a preset assistant")
        );
        assert!(
            desc.description
                .contains("After confirming a match, call team_spawn_agent with the same custom_agent_id.")
        );
    }

    // ---- D4 handlers return non-error payloads ----

    #[test]
    fn team_list_models_handler_returns_non_error() {
        let value = handle_team_list_models(&json!({}));
        let agent_types = value
            .get("agent_types")
            .and_then(|v| v.as_array())
            .expect("agent_types array missing");
        assert!(!agent_types.is_empty());
        let types: Vec<&str> = agent_types
            .iter()
            .filter_map(|e| e.get("type").and_then(|v| v.as_str()))
            .collect();
        assert!(types.contains(&"claude"));
        assert!(types.contains(&"codex"));
    }

    #[test]
    fn build_list_models_from_rows_includes_enabled_team_capable_backends() {
        let rows = vec![
            make_agent_row("claude", true, r#"[{"id":"claude-sonnet-4","name":"Sonnet 4"}]"#),
            make_agent_row("codebuddy", true, r#"[{"id":"codebuddy-pro","name":"CodeBuddy Pro"}]"#),
            make_agent_row("disabled-one", false, r#"[{"id":"m1","name":"M1"}]"#),
        ];
        let value = build_list_models_from_rows(&rows, None, &[]);
        let types: Vec<&str> = value["agent_types"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["type"].as_str())
            .collect();
        assert!(types.contains(&"claude"));
        assert!(types.contains(&"codebuddy"));
        assert!(!types.contains(&"disabled-one"), "disabled backends must be excluded");
    }

    #[test]
    fn build_list_models_from_rows_uses_db_models_not_hardcoded() {
        let rows = vec![make_agent_row(
            "claude",
            true,
            r#"[{"id":"claude-opus-4","name":"Opus 4"},{"id":"claude-sonnet-4","name":"Sonnet 4"}]"#,
        )];
        let value = build_list_models_from_rows(&rows, None, &[]);
        let claude_entry = value["agent_types"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["type"].as_str() == Some("claude"))
            .expect("claude entry");
        let models: Vec<&str> = claude_entry["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(models, vec!["claude-opus-4", "claude-sonnet-4"]);
    }

    #[test]
    fn build_list_models_from_rows_filters_by_agent_type() {
        let rows = vec![
            make_agent_row("claude", true, r#"[{"id":"claude-sonnet-4","name":"Sonnet 4"}]"#),
            make_agent_row("codebuddy", true, r#"[{"id":"cb-pro","name":"Pro"}]"#),
        ];
        let value = build_list_models_from_rows(&rows, Some("codebuddy"), &[]);
        let types: Vec<&str> = value["agent_types"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["type"].as_str())
            .collect();
        assert_eq!(types, vec!["codebuddy"]);
    }

    #[test]
    fn build_list_models_from_rows_skips_null_available_models() {
        let rows = vec![
            make_agent_row("claude", true, r#"[{"id":"claude-sonnet-4","name":"Sonnet 4"}]"#),
            make_agent_row_no_models("gemini", true),
        ];
        let value = build_list_models_from_rows(&rows, None, &[]);
        let types: Vec<&str> = value["agent_types"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["type"].as_str())
            .collect();
        // gemini has no available_models in DB → should still appear but with empty models
        assert!(types.contains(&"gemini"));
    }

    fn make_agent_row(backend: &str, enabled: bool, available_models: &str) -> AgentMetadataRow {
        AgentMetadataRow {
            id: format!("id-{backend}"),
            icon: None,
            name: capitalize_first(backend),
            name_i18n: None,
            description: None,
            description_i18n: None,
            backend: Some(backend.to_owned()),
            agent_type: "acp".to_owned(),
            agent_source: "builtin".to_owned(),
            agent_source_info: None,
            enabled,
            command: None,
            args: None,
            env: None,
            native_skills_dirs: None,
            behavior_policy: None,
            yolo_id: None,
            agent_capabilities: Some(r#"{"mcp":true}"#.to_owned()),
            auth_methods: None,
            config_options: None,
            available_modes: None,
            available_models: Some(available_models.to_owned()),
            available_commands: None,
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn make_agent_row_no_models(backend: &str, enabled: bool) -> AgentMetadataRow {
        let mut row = make_agent_row(backend, enabled, "[]");
        row.available_models = None;
        row
    }

    fn capitalize_first(s: &str) -> String {
        let mut c = s.chars();
        match c.next() {
            None => String::new(),
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        }
    }

    #[test]
    fn build_list_models_from_rows_includes_null_backend_with_supports_team() {
        let mut nomi_row = make_agent_row("nomi", true, r#"[{"id":"nomi-default","name":"Nomi"}]"#);
        nomi_row.backend = None;
        nomi_row.agent_type = "nomi".to_owned();
        nomi_row.agent_source = "internal".to_owned();
        nomi_row.agent_capabilities = None;
        nomi_row.behavior_policy = Some(r#"{"supports_team":true}"#.to_owned());

        let rows = vec![
            make_agent_row("claude", true, r#"[{"id":"claude-sonnet-4","name":"Sonnet 4"}]"#),
            nomi_row,
        ];
        let value = build_list_models_from_rows(&rows, None, &[]);
        let types: Vec<&str> = value["agent_types"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["type"].as_str())
            .collect();
        assert!(types.contains(&"claude"));
        assert!(
            types.contains(&"nomi"),
            "nomi with backend=NULL but supports_team=true must be included"
        );
    }

    #[test]
    fn build_list_models_from_rows_filters_null_backend_by_agent_type() {
        let mut nomi_row = make_agent_row("nomi", true, r#"[{"id":"nomi-default","name":"Nomi"}]"#);
        nomi_row.backend = None;
        nomi_row.agent_type = "nomi".to_owned();
        nomi_row.agent_capabilities = None;
        nomi_row.behavior_policy = Some(r#"{"supports_team":true}"#.to_owned());

        let rows = vec![
            make_agent_row("claude", true, r#"[{"id":"claude-sonnet-4","name":"Sonnet 4"}]"#),
            nomi_row,
        ];
        // Filter by "nomi" should only return nomi
        let value = build_list_models_from_rows(&rows, Some("nomi"), &[]);
        let types: Vec<&str> = value["agent_types"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["type"].as_str())
            .collect();
        assert_eq!(types, vec!["nomi"]);
    }

    #[test]
    fn build_list_models_from_rows_parses_model_info_payload_format() {
        let model_info_json = r#"{"current_model_id":"DeepSeek-V3.2","current_model_label":"DeepSeek-V3.2","available_models":[{"id":"GLM-5.0","label":"GLM-5.0"},{"id":"GLM-5.0-Turbo","label":"GLM-5.0-Turbo"},{"id":"DeepSeek-V3.2","label":"DeepSeek-V3.2"}]}"#;
        let rows = vec![make_agent_row("codebuddy", true, model_info_json)];
        let value = build_list_models_from_rows(&rows, None, &[]);
        let cb_entry = value["agent_types"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["type"].as_str() == Some("codebuddy"))
            .expect("codebuddy entry");
        let models: Vec<&str> = cb_entry["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(models, vec!["GLM-5.0", "GLM-5.0-Turbo", "DeepSeek-V3.2"]);
    }

    #[test]
    fn build_list_models_from_rows_uses_provider_models_for_internal_agents() {
        let mut nomi_row = make_agent_row("nomi", true, "[]");
        nomi_row.backend = None;
        nomi_row.agent_type = "nomi".to_owned();
        nomi_row.agent_source = "internal".to_owned();
        nomi_row.agent_capabilities = None;
        nomi_row.available_models = None;
        nomi_row.behavior_policy = Some(r#"{"supports_team":true}"#.to_owned());

        let provider_models = vec![
            "gemini-3.1-pro-preview".to_owned(),
            "gpt-5.4".to_owned(),
            "gpt-5.2".to_owned(),
        ];
        let rows = vec![
            make_agent_row(
                "claude",
                true,
                r#"{"available_models":[{"id":"claude-sonnet-4","label":"Sonnet 4"}]}"#,
            ),
            nomi_row,
        ];
        let value = build_list_models_from_rows(&rows, None, &provider_models);
        let nomi_entry = value["agent_types"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["type"].as_str() == Some("nomi"))
            .expect("nomi entry");
        let models: Vec<&str> = nomi_entry["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(models, vec!["gemini-3.1-pro-preview", "gpt-5.4", "gpt-5.2"]);
    }

    #[test]
    fn team_describe_assistant_handler_returns_non_error() {
        let text = handle_team_describe_assistant(&json!({"custom_agent_id": "unknown"}));
        assert_eq!(text, "Preset assistant not found");
    }
}
