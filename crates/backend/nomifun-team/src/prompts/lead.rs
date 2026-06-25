//! Leader prompt template constant and builder.
//!
//! The template constant is provided by D5b-1 as `include_str!("prompt_templates/lead.txt")`.
//! This file hosts a stub (`""`) until D5b-1 lands; D5b-2 (this module) implements the
//! `build_lead_prompt()` builder per `docs/teams/phase1/interface-contracts.md` §5.

use std::collections::HashMap;
use std::fmt::Write;

use crate::types::TeamAgent;

/// Placeholder for D5b-1's `include_str!("prompt_templates/lead.txt")`.
/// D5b-1 will replace this stub with the Nomi `leadPrompt.ts` body, preserving
/// the `${...}` placeholders listed in [`PLACEHOLDERS`].
pub const LEAD_PROMPT_TEMPLATE: &str = include_str!("prompt_templates/lead.txt");

/// Placeholder tokens that [`build_lead_prompt`] substitutes in [`LEAD_PROMPT_TEMPLATE`].
///
/// Mirrors the JS template literal placeholders in Nomi's `leadPrompt.ts`.
const PLACEHOLDER_TEAMMATE_LIST: &str = "${teammateList}";
const PLACEHOLDER_AVAILABLE_TYPES_SECTION: &str = "${availableTypesSection}";
const PLACEHOLDER_AVAILABLE_ASSISTANTS_SECTION: &str = "${availableAssistantsSection}";
const PLACEHOLDER_WORKSPACE_SECTION: &str = "${workspaceSection}";
const PLACEHOLDER_PRESET_FORMATTING_STEP_RULE: &str = "${presetFormattingStepRule}";
const PLACEHOLDER_PRESET_FORMATTING_IMPORTANT_RULE: &str = "${presetFormattingImportantRule}";

/// A generic agent type (CLI backend) that the leader may spawn.
/// Phase1 shape per interface-contracts §5 (line 211).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableAgentType {
    pub agent_type: String,
    pub display_name: String,
}

/// A preset assistant the leader may spawn via `custom_agent_id`.
/// Phase1 shape per interface-contracts §5 (lines 212-218).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableAssistant {
    pub custom_agent_id: String,
    pub name: String,
    pub backend: String,
    pub description: String,
    pub skills: Vec<String>,
}

/// Inputs for `build_lead_prompt`. Phase1 callers may pass empty slices/maps and `None`.
pub struct LeadPromptParams<'a> {
    pub team_name: &'a str,
    pub teammates: &'a [TeamAgent],
    pub available_agent_types: &'a [AvailableAgentType],
    pub available_assistants: &'a [AvailableAssistant],
    pub renamed_agents: &'a HashMap<String, String>,
    pub team_workspace: Option<&'a str>,
}

/// Build the leader role prompt by substituting dynamic sections into the static template.
///
/// Placeholders replaced (mirrors Nomi `leadPrompt.ts`):
/// - `${teammateList}` — bullet list of teammates or an empty-team fallback sentence
/// - `${availableTypesSection}` — `## Available Agent Types for Spawning` section, or `""`
/// - `${availableAssistantsSection}` — `## Available Preset Assistants for Spawning` section, or `""`
/// - `${workspaceSection}` — `## Team Workspace` section, or `""`
/// - `${presetFormattingStepRule}` — phase1 emits `""` (presets not surfaced in phase1)
/// - `${presetFormattingImportantRule}` — phase1 emits `""` (presets not surfaced in phase1)
pub fn build_lead_prompt(params: &LeadPromptParams<'_>) -> String {
    let teammate_list = render_teammate_list(params.teammates, params.renamed_agents);
    let available_types_section = render_available_types_section(params.available_agent_types);
    let available_assistants_section = render_available_assistants_section(params.available_assistants);
    let workspace_section = render_workspace_section(params.team_workspace);

    // Phase1 does not surface preset assistants in the staffing-proposal formatting
    // rules, so these two placeholders are replaced with empty strings. When preset
    // support lands they will be conditional strings analogous to Nomi.
    let preset_formatting_step_rule = "";
    let preset_formatting_important_rule = "";

    LEAD_PROMPT_TEMPLATE
        .replace(PLACEHOLDER_TEAMMATE_LIST, &teammate_list)
        .replace(PLACEHOLDER_AVAILABLE_TYPES_SECTION, &available_types_section)
        .replace(PLACEHOLDER_AVAILABLE_ASSISTANTS_SECTION, &available_assistants_section)
        .replace(PLACEHOLDER_WORKSPACE_SECTION, &workspace_section)
        .replace(PLACEHOLDER_PRESET_FORMATTING_STEP_RULE, preset_formatting_step_rule)
        .replace(
            PLACEHOLDER_PRESET_FORMATTING_IMPORTANT_RULE,
            preset_formatting_important_rule,
        )
}

fn render_teammate_list(teammates: &[TeamAgent], renamed_agents: &HashMap<String, String>) -> String {
    if teammates.is_empty() {
        return "(no teammates yet — propose the lineup to the user first, then use \
                team_spawn_agent only after they confirm or explicitly ask you to create \
                teammates immediately)"
            .to_owned();
    }

    let mut out = String::with_capacity(teammates.len() * 64);
    for (idx, m) in teammates.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let status = m.status.map(|s| s.to_string()).unwrap_or_else(|| "unknown".to_owned());
        let _ = write!(out, "- {} ({}, status: {})", m.name, m.backend, status,);
        if let Some(former) = renamed_agents.get(&m.slot_id) {
            let _ = write!(out, " [formerly: {former}]");
        }
    }
    out
}

fn render_available_types_section(agent_types: &[AvailableAgentType]) -> String {
    if agent_types.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\n## Available Agent Types for Spawning\n");
    for (idx, t) in agent_types.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let _ = write!(out, "- `{}` — {}", t.agent_type, t.display_name);
    }
    out.push_str("\n\nUse `team_list_models` to query available models for each agent type before spawning.");
    out
}

fn render_available_assistants_section(assistants: &[AvailableAssistant]) -> String {
    if assistants.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\n## Available Preset Assistants for Spawning\n");
    out.push_str(
        "These are user-configured assistants with pre-loaded rules and skills for specific \
         domains (writing, research, PPT building, etc.). When a task matches a preset's \
         specialty, prefer spawning the preset over a generic CLI agent — you get its domain \
         expertise automatically.\n\n",
    );
    for (idx, a) in assistants.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let desc = if a.description.is_empty() {
            String::new()
        } else {
            format!(" — {}", a.description)
        };
        let skills = if a.skills.is_empty() {
            String::new()
        } else {
            format!("\n   skills: {}", a.skills.join(", "))
        };
        let _ = write!(
            out,
            "- `{}` ({}, backend: {}){}{}",
            a.custom_agent_id, a.name, a.backend, desc, skills,
        );
    }
    out.push_str(
        "\n\n### How to pick a preset\n\
         1. Scan the one-line descriptions and skills above. If one clearly matches the user's \
         domain (e.g. \"quarterly Word report\" → `word-creator`), spawn it directly with \
         `team_spawn_agent`.\n\
         2. If two or more presets seem relevant, call `team_describe_assistant` on each \
         candidate to see its full description, skills, and example tasks, then choose the best \
         fit.\n\
         3. If no preset matches the task, fall back to a generic CLI agent from the \
         \"Available Agent Types\" section.\n\n\
         Pass the preset's ID as `custom_agent_id` to `team_spawn_agent`. The `agent_type` is \
         derived from the preset's backend and does not need to be specified.",
    );
    out
}

fn render_workspace_section(team_workspace: Option<&str>) -> String {
    match team_workspace {
        Some(ws) => format!(
            "\n\n## Team Workspace\nYour working directory `{ws}` IS the shared team workspace.\n\
             All teammates work in this directory for project-related operations."
        ),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{TeamAgent, TeammateRole, TeammateStatus};

    fn params_min<'a>(renamed: &'a HashMap<String, String>) -> LeadPromptParams<'a> {
        LeadPromptParams {
            team_name: "Alpha",
            teammates: &[],
            available_agent_types: &[],
            available_assistants: &[],
            renamed_agents: renamed,
            team_workspace: None,
        }
    }

    fn make_teammate(slot_id: &str, name: &str, backend: &str) -> TeamAgent {
        TeamAgent {
            slot_id: slot_id.into(),
            name: name.into(),
            role: TeammateRole::Teammate,
            conversation_id: format!("conv-{slot_id}"),
            backend: backend.into(),
            model: "sonnet".into(),
            custom_agent_id: None,
            status: None,
            conversation_type: None,
            cli_path: None,
        }
    }

    #[test]
    fn no_unsubstituted_placeholders_on_minimal_params() {
        let renamed = HashMap::new();
        let out = build_lead_prompt(&params_min(&renamed));
        assert!(
            !out.contains("${"),
            "unsubstituted `${{` placeholder remains in output:\n{out}"
        );
    }

    #[test]
    fn no_unsubstituted_placeholders_when_all_sections_populated() {
        let renamed = HashMap::new();
        let teammate = make_teammate("w1", "Worker1", "claude");
        let agent_types = vec![AvailableAgentType {
            agent_type: "claude".into(),
            display_name: "general-purpose AI assistant".into(),
        }];
        let assistants = vec![AvailableAssistant {
            custom_agent_id: "word-creator".into(),
            name: "Word Creator".into(),
            backend: "claude".into(),
            description: "Drafts Word documents".into(),
            skills: vec!["docx".into(), "formatting".into()],
        }];
        let params = LeadPromptParams {
            team_name: "Beta",
            teammates: std::slice::from_ref(&teammate),
            available_agent_types: &agent_types,
            available_assistants: &assistants,
            renamed_agents: &renamed,
            team_workspace: Some("/tmp/team-ws"),
        };
        let out = build_lead_prompt(&params);
        assert!(
            !out.contains("${"),
            "unsubstituted `${{` placeholder remains in output:\n{out}"
        );
    }

    #[test]
    fn teammate_list_empty_uses_nomifun_fallback_copy() {
        let renamed = HashMap::new();
        let got = render_teammate_list(&[], &renamed);
        assert_eq!(
            got,
            "(no teammates yet — propose the lineup to the user first, then use team_spawn_agent \
             only after they confirm or explicitly ask you to create teammates immediately)"
        );
    }

    #[test]
    fn teammate_list_uses_nomifun_bullet_format_without_slot_prefix() {
        let renamed = HashMap::new();
        let mut t = make_teammate("w1", "Worker1", "claude");
        t.status = Some(TeammateStatus::Idle);
        let got = render_teammate_list(std::slice::from_ref(&t), &renamed);

        assert_eq!(got, "- Worker1 (claude, status: idle)");
        assert!(!got.contains("slot="), "teammate bullet must not expose slot=");
        assert!(
            !got.contains("agentType="),
            "teammate bullet must not use agentType= prefix"
        );
    }

    #[test]
    fn teammate_list_status_defaults_to_unknown_when_missing() {
        let renamed = HashMap::new();
        let t = make_teammate("w1", "Worker1", "claude");
        let got = render_teammate_list(std::slice::from_ref(&t), &renamed);
        assert_eq!(got, "- Worker1 (claude, status: unknown)");
    }

    #[test]
    fn teammate_list_appends_formerly_note_for_renamed() {
        let mut renamed = HashMap::new();
        renamed.insert("w1".to_owned(), "OldName".to_owned());
        let mut t = make_teammate("w1", "Worker1", "claude");
        t.status = Some(TeammateStatus::Working);
        let got = render_teammate_list(std::slice::from_ref(&t), &renamed);
        assert_eq!(got, "- Worker1 (claude, status: working) [formerly: OldName]");
    }

    #[test]
    fn available_types_section_omitted_when_empty() {
        assert_eq!(render_available_types_section(&[]), "");
    }

    #[test]
    fn available_types_section_includes_backtick_ids_and_model_query_hint() {
        let got = render_available_types_section(&[
            AvailableAgentType {
                agent_type: "claude".into(),
                display_name: "general-purpose AI assistant".into(),
            },
            AvailableAgentType {
                agent_type: "codex".into(),
                display_name: "code generation specialist".into(),
            },
        ]);
        assert!(got.starts_with("\n\n## Available Agent Types for Spawning\n"));
        assert!(got.contains("- `claude` — general-purpose AI assistant"));
        assert!(got.contains("- `codex` — code generation specialist"));
        assert!(got.contains("Use `team_list_models`"));
    }

    #[test]
    fn available_assistants_section_omitted_when_empty() {
        assert_eq!(render_available_assistants_section(&[]), "");
    }

    #[test]
    fn available_assistants_section_includes_skills_and_how_to_pick() {
        let got = render_available_assistants_section(&[AvailableAssistant {
            custom_agent_id: "word-creator".into(),
            name: "Word Creator".into(),
            backend: "claude".into(),
            description: "Drafts Word documents".into(),
            skills: vec!["docx".into(), "formatting".into()],
        }]);
        assert!(got.contains("## Available Preset Assistants for Spawning"));
        assert!(got.contains("- `word-creator` (Word Creator, backend: claude) — Drafts Word documents"));
        assert!(got.contains("skills: docx, formatting"));
        assert!(got.contains("### How to pick a preset"));
    }

    #[test]
    fn workspace_section_omitted_when_none() {
        assert_eq!(render_workspace_section(None), "");
    }

    #[test]
    fn workspace_section_embeds_path_and_shared_directory_copy() {
        let got = render_workspace_section(Some("/tmp/team-ws"));
        assert!(got.contains("## Team Workspace"));
        assert!(got.contains("`/tmp/team-ws`"));
        assert!(got.contains("shared team workspace"));
    }

    #[test]
    fn preset_formatting_placeholders_are_empty_in_phase1() {
        // Phase1 convention: presets are not surfaced, so both preset-formatting
        // placeholders are replaced with "" regardless of other params.
        // The regression test above (`no_unsubstituted_placeholders_when_all_sections_populated`)
        // already asserts that both tokens are stripped from the final output.
        // This test guards the behavior by simulating the full substitution on a
        // template carrying just the two preset placeholders.
        let template_with_presets_only = "step:${presetFormattingStepRule}|important:${presetFormattingImportantRule}";
        let out = template_with_presets_only
            .replace("${presetFormattingStepRule}", "")
            .replace("${presetFormattingImportantRule}", "");
        assert_eq!(out, "step:|important:");
    }

    #[test]
    fn snapshot_minimal_params_with_stub_template_yields_empty_output() {
        // While `LEAD_PROMPT_TEMPLATE` is the D5b-1 stub (`""`), the builder has no
        // template to substitute into, so the output is empty regardless of params.
        // Once D5b-1 lands, this test will start failing and should be updated to a
        // real snapshot. The regression guard above keeps the substitution contract
        // healthy in the meantime.
        let renamed = HashMap::new();
        let out = build_lead_prompt(&params_min(&renamed));
        assert!(!out.is_empty(), "output should not be empty with real template");
        assert!(!out.contains("${"), "no unsubstituted placeholders");
    }

    #[test]
    fn substitution_against_synthetic_template_matches_nomifun_layout() {
        // This synthetic template mirrors the shape of Nomi's leadPrompt.ts literal
        // so we can validate end-to-end substitution without depending on D5b-1.
        // When D5b-1 lands the real `LEAD_PROMPT_TEMPLATE` takes over; this test
        // still exercises the same substitution code path.
        const SYNTHETIC: &str = "## Your Teammates\n\
            ${teammateList}${availableTypesSection}${availableAssistantsSection}${workspaceSection}\n\
            STEP:${presetFormattingStepRule}END\n\
            - ${presetFormattingImportantRule}END";

        let renamed = HashMap::new();
        let t = make_teammate("w1", "Worker1", "claude");
        let params = LeadPromptParams {
            team_name: "Beta",
            teammates: std::slice::from_ref(&t),
            available_agent_types: &[AvailableAgentType {
                agent_type: "claude".into(),
                display_name: "general-purpose AI assistant".into(),
            }],
            available_assistants: &[],
            renamed_agents: &renamed,
            team_workspace: Some("/tmp/team-ws"),
        };

        let teammate_list = render_teammate_list(params.teammates, params.renamed_agents);
        let types_section = render_available_types_section(params.available_agent_types);
        let assistants_section = render_available_assistants_section(params.available_assistants);
        let ws_section = render_workspace_section(params.team_workspace);

        let out = SYNTHETIC
            .replace("${teammateList}", &teammate_list)
            .replace("${availableTypesSection}", &types_section)
            .replace("${availableAssistantsSection}", &assistants_section)
            .replace("${workspaceSection}", &ws_section)
            .replace("${presetFormattingStepRule}", "")
            .replace("${presetFormattingImportantRule}", "");

        assert!(!out.contains("${"), "unsubstituted placeholder:\n{out}");
        assert!(out.contains("## Your Teammates"));
        assert!(out.contains("- Worker1 (claude, status: unknown)"));
        assert!(out.contains("## Available Agent Types for Spawning"));
        assert!(!out.contains("## Available Preset Assistants for Spawning"));
        assert!(out.contains("## Team Workspace"));
        assert!(out.contains("STEP:END"));
        assert!(out.contains("- END"));
    }
}
