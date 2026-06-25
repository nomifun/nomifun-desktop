pub mod lead;

use std::collections::HashMap;

use crate::prompts::lead::{AvailableAgentType, LeadPromptParams};
use crate::types::{MailboxMessage, MailboxMessageType, TaskStatus, TeamAgent, TeamTask};

/// Build the leader system prompt.
///
/// Delegates to [`lead::build_lead_prompt`], which mirrors the Nomi
/// `leadPrompt.ts` template verbatim. A one-line `Team: "<name>"` header
/// is prepended so the leader knows which team it belongs to (Nomi
/// surfaces this through other channels, but the backend session has no
/// other place to inject it).
///
/// `available_agent_types` carries `(backend_id, display_name)` pairs that
/// feed the `## Available Agent Types for Spawning` section; callers
/// should source these from the team-capable backend whitelist.
pub fn build_lead_prompt(team_name: &str, members: &[TeamAgent], available_agent_types: &[(String, String)]) -> String {
    let agent_types: Vec<AvailableAgentType> = available_agent_types
        .iter()
        .map(|(backend, display)| AvailableAgentType {
            agent_type: backend.clone(),
            display_name: display.clone(),
        })
        .collect();
    let renamed: HashMap<String, String> = HashMap::new();

    let params = LeadPromptParams {
        team_name,
        teammates: members,
        available_agent_types: &agent_types,
        available_assistants: &[],
        renamed_agents: &renamed,
        team_workspace: None,
    };

    let body = lead::build_lead_prompt(&params);
    format!("Team: \"{team_name}\"\n\n{body}")
}

pub fn build_teammate_prompt(agent: &TeamAgent, team_name: &str) -> String {
    let mut prompt = String::with_capacity(1024);

    prompt.push_str(&format!(
        "You are **{}**, a Teammate Agent in team \"{}\". \
         Your slot ID is `{}`.\n\n",
        agent.name, team_name, agent.slot_id,
    ));

    prompt.push_str("## Your Role\n\n");
    prompt.push_str(
        "You execute tasks assigned by the Lead Agent. Focus on completing your \
         assigned work thoroughly and reporting back.\n\n",
    );

    prompt.push_str("## Communication Protocol\n\n");
    prompt.push_str(
        "- Use `team_send_message` to report progress or ask questions to the Lead.\n\
         - Use `team_task_update` to update task status as you work \
         (pending → in_progress → completed).\n\
         - When your assigned work is done, send an idle notification. \
         The system will notify the Lead.\n\
         - If you receive a `shutdown_request`, finish any critical work, \
         then respond with \"shutdown_approved\" or \"shutdown_rejected: <reason>\".\n",
    );

    prompt
}

pub fn build_wake_payload(agent: &TeamAgent, tasks: &[TeamTask], unread_messages: &[MailboxMessage]) -> String {
    let mut payload = String::with_capacity(2048);

    if !unread_messages.is_empty() {
        payload.push_str("## New Messages\n\n");
        for msg in unread_messages {
            let type_label = match msg.msg_type {
                MailboxMessageType::Message => "message",
                MailboxMessageType::IdleNotification => "idle_notification",
                MailboxMessageType::ShutdownRequest => "shutdown_request",
            };
            payload.push_str(&format!(
                "- From `{}` [{}]: {}\n",
                msg.from_agent_id, type_label, msg.content,
            ));
            if let Some(ref summary) = msg.summary {
                payload.push_str(&format!("  Summary: {summary}\n"));
            }
        }
        payload.push('\n');
    } else {
        payload.push_str("## New Messages\n\nNo new messages.\n\n");
    }

    if !tasks.is_empty() {
        payload.push_str("## Current Task Board\n\n");
        payload.push_str("| ID | Subject | Status | Owner | Blocked By |\n");
        payload.push_str("|---|---|---|---|---|\n");
        for task in tasks {
            let status = match task.status {
                TaskStatus::Pending => "pending",
                TaskStatus::InProgress => "in_progress",
                TaskStatus::Completed => "completed",
                TaskStatus::Deleted => "deleted",
            };
            let owner = task.owner.as_deref().unwrap_or("-");
            let blocked = if task.blocked_by.is_empty() {
                "-".to_owned()
            } else {
                task.blocked_by.join(", ")
            };
            // `task_{uuidv7}` ids share a long common head (type prefix +
            // timestamp); the random tail is the distinctive part, so
            // truncate from the front.
            let short_id = if task.id.len() > 8 {
                format!("…{}", &task.id[task.id.len() - 8..])
            } else {
                task.id.clone()
            };
            payload.push_str(&format!(
                "| {short_id} | {} | {status} | {owner} | {blocked} |\n",
                task.subject,
            ));
        }
        payload.push('\n');
    } else {
        payload.push_str("## Current Task Board\n\nNo tasks on the board.\n\n");
    }

    payload.push_str(&format!(
        "You are **{}** (role: {}). Proceed with your work.\n",
        agent.name, agent.role,
    ));

    payload
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TeammateRole;

    fn make_lead() -> TeamAgent {
        TeamAgent {
            slot_id: "lead-1".into(),
            name: "Lead".into(),
            role: TeammateRole::Lead,
            conversation_id: "conv-1".into(),
            backend: "acp".into(),
            model: "claude".into(),
            custom_agent_id: None,
            status: None,
            conversation_type: None,
            cli_path: None,
        }
    }

    fn make_teammate(slot_id: &str, name: &str) -> TeamAgent {
        TeamAgent {
            slot_id: slot_id.into(),
            name: name.into(),
            role: TeammateRole::Teammate,
            conversation_id: format!("conv-{slot_id}"),
            backend: "acp".into(),
            model: "claude".into(),
            custom_agent_id: None,
            status: None,
            conversation_type: None,
            cli_path: None,
        }
    }

    fn make_task(id: &str, subject: &str, status: TaskStatus) -> TeamTask {
        TeamTask {
            id: id.into(),
            team_id: "t1".into(),
            subject: subject.into(),
            description: None,
            status,
            owner: Some("worker-1".into()),
            blocked_by: vec![],
            blocks: vec![],
            metadata: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn make_message(from: &str, content: &str, msg_type: MailboxMessageType) -> MailboxMessage {
        MailboxMessage {
            id: 1,
            team_id: "t1".into(),
            to_agent_id: "lead-1".into(),
            from_agent_id: from.into(),
            msg_type,
            content: content.into(),
            summary: None,
            files: None,
            read: false,
            created_at: 0,
        }
    }

    // -- Lead prompt ----------------------------------------------------------

    fn default_agent_types() -> Vec<(String, String)> {
        vec![
            ("claude".into(), "Claude".into()),
            ("codex".into(), "Codex".into()),
            ("gemini".into(), "Gemini".into()),
        ]
    }

    #[test]
    fn lead_prompt_contains_team_name() {
        let types = default_agent_types();
        let prompt = build_lead_prompt("Alpha", &[], &types);
        assert!(prompt.contains("\"Alpha\""));
    }

    #[test]
    fn lead_prompt_contains_member_list() {
        let types = default_agent_types();
        let members = vec![make_lead(), make_teammate("w1", "Worker1")];
        let prompt = build_lead_prompt("Alpha", &members, &types);

        // Nomi bullet format: `- {name} ({backend}, status: {status})`
        assert!(prompt.contains("- Lead (acp, status:"));
        assert!(prompt.contains("- Worker1 (acp, status:"));
    }

    #[test]
    fn lead_prompt_contains_core_sections() {
        let types = default_agent_types();
        let prompt = build_lead_prompt("Alpha", &[], &types);

        // Workflow — 15-step procedure with model listing at step 3
        assert!(prompt.contains("## Workflow"));
        assert!(prompt.contains("FIRST call `team_list_models`"));
        assert!(prompt.contains("Wait for explicit confirmation before using team_spawn_agent"));
        assert!(prompt.contains("End your turn after the proposal"));

        // Model Selection Guidelines
        assert!(prompt.contains("## Model Selection Guidelines"));
        assert!(prompt.contains("exact model ID strings"));
        assert!(prompt.contains("omit the model parameter"));

        // Conversation Style — don't pitch proposals up-front
        assert!(prompt.contains("## Conversation Style"));
        assert!(prompt.contains("reply warmly and naturally"));

        // Idle, sequencing, shutdown, important rules
        assert!(prompt.contains("## Teammate Idle State"));
        assert!(prompt.contains("## Sequencing Dependent Work"));
        assert!(prompt.contains("## Shutting Down Teammates"));
        assert!(prompt.contains("team_shutdown_agent"));
        assert!(prompt.contains("## Important Rules"));

        // Team coordination tool list still referenced
        assert!(prompt.contains("team_send_message"));
        assert!(prompt.contains("team_spawn_agent"));
        assert!(prompt.contains("team_members"));
        assert!(prompt.contains("team_task_list"));
        assert!(prompt.contains("team_rename_agent"));
    }

    #[test]
    fn lead_prompt_includes_available_agent_types_section() {
        let types = default_agent_types();
        let prompt = build_lead_prompt("Alpha", &[], &types);

        assert!(prompt.contains("## Available Agent Types for Spawning"));
        assert!(prompt.contains("- `claude` — Claude"));
        assert!(prompt.contains("- `codex` — Codex"));
        assert!(prompt.contains("- `gemini` — Gemini"));
        assert!(prompt.contains("Use `team_list_models`"));
    }

    #[test]
    fn lead_prompt_omits_agent_types_section_when_empty() {
        let prompt = build_lead_prompt("Alpha", &[], &[]);
        assert!(!prompt.contains("## Available Agent Types for Spawning"));
    }

    #[test]
    fn lead_prompt_no_members_shows_empty_lineup_copy() {
        let types = default_agent_types();
        let prompt = build_lead_prompt("Solo", &[], &types);
        assert!(prompt.contains("(no teammates yet"));
        assert!(prompt.contains("propose the lineup to the user first"));
    }

    #[test]
    fn lead_prompt_has_no_unsubstituted_placeholders() {
        let types = default_agent_types();
        let members = vec![make_lead(), make_teammate("w1", "Worker1")];
        let prompt = build_lead_prompt("Alpha", &members, &types);
        assert!(
            !prompt.contains("${"),
            "unsubstituted template placeholder leaked:\n{prompt}"
        );
    }

    // -- Teammate prompt ------------------------------------------------------

    #[test]
    fn teammate_prompt_contains_agent_identity() {
        let agent = make_teammate("w1", "Worker1");
        let prompt = build_teammate_prompt(&agent, "Alpha");

        assert!(prompt.contains("**Worker1**"));
        assert!(prompt.contains("\"Alpha\""));
        assert!(prompt.contains("`w1`"));
    }

    #[test]
    fn teammate_prompt_contains_communication_protocol() {
        let agent = make_teammate("w1", "Worker1");
        let prompt = build_teammate_prompt(&agent, "Alpha");

        assert!(prompt.contains("team_send_message"));
        assert!(prompt.contains("team_task_update"));
        assert!(prompt.contains("idle notification"));
        assert!(prompt.contains("shutdown_request"));
        assert!(prompt.contains("shutdown_approved"));
    }

    #[test]
    fn teammate_prompt_contains_team_name() {
        let agent = make_teammate("w1", "W");
        let prompt = build_teammate_prompt(&agent, "Beta Team");
        assert!(prompt.contains("\"Beta Team\""));
    }

    // -- Wake payload ---------------------------------------------------------

    #[test]
    fn wake_payload_with_messages() {
        let agent = make_lead();
        let msgs = vec![make_message("w1", "Task A done", MailboxMessageType::Message)];
        let payload = build_wake_payload(&agent, &[], &msgs);

        assert!(payload.contains("New Messages"));
        assert!(payload.contains("`w1`"));
        assert!(payload.contains("[message]"));
        assert!(payload.contains("Task A done"));
    }

    #[test]
    fn wake_payload_with_idle_notification() {
        let agent = make_lead();
        let mut msg = make_message("w1", "idle", MailboxMessageType::IdleNotification);
        msg.summary = Some("Finished feature X".into());
        let payload = build_wake_payload(&agent, &[], &[msg]);

        assert!(payload.contains("[idle_notification]"));
        assert!(payload.contains("Summary: Finished feature X"));
    }

    #[test]
    fn wake_payload_with_shutdown_request() {
        let agent = make_teammate("w1", "W");
        let msg = make_message("lead-1", "No longer needed", MailboxMessageType::ShutdownRequest);
        let payload = build_wake_payload(&agent, &[], &[msg]);

        assert!(payload.contains("[shutdown_request]"));
        assert!(payload.contains("No longer needed"));
    }

    #[test]
    fn wake_payload_with_tasks() {
        let agent = make_lead();
        let tasks = vec![
            make_task(
                "task_0190aaaa-1234-5678-9abc-def0aaaa1111",
                "Implement X",
                TaskStatus::InProgress,
            ),
            make_task("task_0190aaaa-1234-5678-9abc-def0bbbb2222", "Test Y", TaskStatus::Pending),
        ];
        let payload = build_wake_payload(&agent, &tasks, &[]);

        assert!(payload.contains("Current Task Board"));
        assert!(payload.contains("Implement X"));
        assert!(payload.contains("in_progress"));
        assert!(payload.contains("Test Y"));
        assert!(payload.contains("pending"));
        assert!(payload.contains("…aaaa1111"));
        assert!(payload.contains("…bbbb2222"));
    }

    #[test]
    fn wake_payload_with_task_dependencies() {
        let agent = make_lead();
        let mut task = make_task("cccccccc-1234-5678-9abc-def012345678", "Deploy", TaskStatus::Pending);
        task.blocked_by = vec!["task-a".into(), "task-b".into()];
        let payload = build_wake_payload(&agent, &[task], &[]);

        assert!(payload.contains("task-a, task-b"));
    }

    #[test]
    fn wake_payload_empty() {
        let agent = make_lead();
        let payload = build_wake_payload(&agent, &[], &[]);

        assert!(payload.contains("No new messages"));
        assert!(payload.contains("No tasks on the board"));
        assert!(payload.contains("**Lead**"));
    }

    #[test]
    fn wake_payload_contains_agent_identity() {
        let agent = make_teammate("w1", "Worker1");
        let payload = build_wake_payload(&agent, &[], &[]);

        assert!(payload.contains("**Worker1**"));
        assert!(payload.contains("teammate"));
    }

    #[test]
    fn wake_payload_short_task_id_no_truncation() {
        let agent = make_lead();
        let task = make_task("short", "Short ID Task", TaskStatus::Pending);
        let payload = build_wake_payload(&agent, &[task], &[]);
        assert!(payload.contains("| short |"));
    }
}
