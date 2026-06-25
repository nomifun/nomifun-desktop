//! Tool handlers for the Guide MCP server (`nomi_*` tools) and argument
//! parsing for lead-facing create-team flows.
//!
//! Guide tools run in the agent's own MCP client and expose "meta" operations
//! that help the agent reason about team composition (list available models,
//! create a team, describe a preset). They are distinct from the in-team
//! `team_*` tools exposed by `mcp::server`.

use serde_json::{Value, json};

use crate::mcp::tools::handle_team_list_models;

#[derive(Debug, Clone)]
pub struct CreateTeamParams {
    pub summary: String,
    pub name: String,
    pub workspace: String,
}

/// Parse `nomi_create_team` tool arguments into structured params.
///
/// Defaults:
/// - `name` falls back to the first 5 whitespace-separated tokens of `summary`.
/// - `workspace` falls back to the caller's workspace, then to `"."`.
pub fn parse_create_team_args(args: &Value, caller_workspace: Option<&str>) -> Result<CreateTeamParams, String> {
    let summary = args
        .get("summary")
        .and_then(Value::as_str)
        .ok_or("missing required field: summary")?
        .to_owned();

    let name = args
        .get("name")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| summary.split_whitespace().take(5).collect::<Vec<_>>().join(" "));

    let workspace = args
        .get("workspace")
        .and_then(Value::as_str)
        .map(String::from)
        .or_else(|| caller_workspace.map(String::from))
        .unwrap_or_else(|| ".".to_owned());

    Ok(CreateTeamParams {
        summary,
        name,
        workspace,
    })
}

/// Handle the `nomi_list_models` tool call.
///
/// Returns available backend × model combinations the agent can pick from when
/// planning a team. The Guide advertises everything the user could reasonably
/// select; the per-backend security whitelist enforced by
/// `nomi_create_team` / `team_spawn_agent` is a separate concern.
///
/// Claude + Codex entries are sourced verbatim from the project-wide
/// `handle_team_list_models` so Guide and in-team tools stay in lockstep.
/// Gemini is appended here because Guide surfaces every frontend-visible
/// option regardless of the spawn whitelist.
pub fn handle_nomi_list_models() -> Value {
    let mut base = handle_team_list_models(&Value::Null);
    if let Some(agent_types) = base.get_mut("agent_types").and_then(Value::as_array_mut) {
        agent_types.push(json!({
            "type": "gemini",
            "models": ["gemini-2.5-pro", "gemini-2.5-flash"]
        }));
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_when_summary_missing() {
        let args = json!({ "name": "alpha", "workspace": "/tmp" });
        let err = parse_create_team_args(&args, None).unwrap_err();
        assert!(err.contains("summary"), "unexpected error: {err}");
    }

    #[test]
    fn errors_when_summary_not_string() {
        let args = json!({ "summary": 42 });
        let err = parse_create_team_args(&args, None).unwrap_err();
        assert!(err.contains("summary"), "unexpected error: {err}");
    }

    #[test]
    fn name_defaults_to_first_five_summary_words() {
        let args = json!({
            "summary": "implement login flow and add OAuth provider support end-to-end",
        });
        let params = parse_create_team_args(&args, None).unwrap();
        assert_eq!(params.name, "implement login flow and add");
        assert_eq!(
            params.summary,
            "implement login flow and add OAuth provider support end-to-end"
        );
    }

    #[test]
    fn name_defaults_use_all_summary_when_shorter_than_five_words() {
        let args = json!({ "summary": "hello world" });
        let params = parse_create_team_args(&args, None).unwrap();
        assert_eq!(params.name, "hello world");
    }

    #[test]
    fn workspace_inherits_from_caller_when_missing() {
        let args = json!({ "summary": "do work" });
        let params = parse_create_team_args(&args, Some("/caller/ws")).unwrap();
        assert_eq!(params.workspace, "/caller/ws");
    }

    #[test]
    fn workspace_defaults_to_dot_when_caller_absent() {
        let args = json!({ "summary": "do work" });
        let params = parse_create_team_args(&args, None).unwrap();
        assert_eq!(params.workspace, ".");
    }

    #[test]
    fn custom_fields_take_precedence_over_defaults() {
        let args = json!({
            "summary": "refactor the scheduler end-to-end",
            "name": "scheduler-refactor",
            "workspace": "/repo/path",
        });
        let params = parse_create_team_args(&args, Some("/caller/ws")).unwrap();
        assert_eq!(params.summary, "refactor the scheduler end-to-end");
        assert_eq!(params.name, "scheduler-refactor");
        assert_eq!(params.workspace, "/repo/path");
    }

    #[test]
    fn non_string_name_falls_back_to_summary_prefix() {
        let args = json!({
            "summary": "one two three four five six",
            "name": 123,
        });
        let params = parse_create_team_args(&args, None).unwrap();
        assert_eq!(params.name, "one two three four five");
    }

    #[test]
    fn non_string_workspace_falls_back_to_caller() {
        let args = json!({
            "summary": "do work",
            "workspace": 42,
        });
        let params = parse_create_team_args(&args, Some("/caller/ws")).unwrap();
        assert_eq!(params.workspace, "/caller/ws");
    }

    #[test]
    fn returns_agent_types_array() {
        let value = handle_nomi_list_models();
        let types = value
            .get("agent_types")
            .and_then(Value::as_array)
            .expect("agent_types must be an array");
        let names: Vec<&str> = types.iter().filter_map(|t| t.get("type")?.as_str()).collect();
        assert!(names.contains(&"claude"));
        assert!(names.contains(&"codex"));
        assert!(names.contains(&"gemini"));
    }

    #[test]
    fn every_entry_has_models_list() {
        let value = handle_nomi_list_models();
        for entry in value["agent_types"].as_array().unwrap() {
            let models = entry["models"].as_array().expect("models must be array");
            assert!(!models.is_empty(), "models list must not be empty");
            assert!(entry["type"].as_str().map(|s| !s.is_empty()).unwrap_or(false));
        }
    }

    #[test]
    fn reuses_team_list_models_for_claude_and_codex() {
        let guide = handle_nomi_list_models();
        let team = handle_team_list_models(&Value::Null);
        for backend in ["claude", "codex"] {
            let guide_entry = find_entry(&guide, backend).expect("guide entry present");
            let team_entry = find_entry(&team, backend).expect("team entry present");
            assert_eq!(guide_entry["models"], team_entry["models"]);
        }
    }

    fn find_entry<'a>(value: &'a Value, backend: &str) -> Option<&'a Value> {
        value["agent_types"]
            .as_array()?
            .iter()
            .find(|entry| entry["type"].as_str() == Some(backend))
    }
}
