use nomifun_api_types::Requirement;
use nomifun_common::AgentType;

use crate::attachments::PromptAttachment;

/// Whether a chat-style engine has the native `requirement_complete` /
/// `requirement_update_status` tools registered into its tool bus at session
/// build time — and therefore whether the platform should EXPECT an explicit
/// verdict rather than assuming a clean turn means success.
///
/// This must mirror the runtime registration logic: only the Nomi factory
/// (`crates/backend/nomifun-ai-agent/src/factory/nomi.rs`) consumes the
/// `requirement_sink`, and only `NomiAgentManager` registers
/// `RequirementCompleteTool` / `RequirementUpdateStatusTool` on the engine.
///
/// Keep this in lock-step with the registration site if engines ever change.
/// A prompt must never name a tool the session lacks: the agent would try to
/// call a missing tool and break the turn.
pub fn has_native_requirement_tools(agent_type: AgentType) -> bool {
    matches!(agent_type, AgentType::Nomi)
}

/// Render the attachments section appended to every requirement prompt
/// variant. Empty input renders nothing. The model is explicitly told to view
/// the images with its file-reading tool BEFORE starting — this is the
/// path-plus-guidance contract (same pattern as the knowledge context builder).
fn render_attachments_section(attachments: &[PromptAttachment]) -> String {
    if attachments.is_empty() {
        return String::new();
    }
    let mut s = format!("\n## Requirement attachments ({} images)\n", attachments.len());
    for a in attachments {
        if a.missing {
            s.push_str(&format!(
                "- {} — (missing: the original file could not be found)\n",
                a.file_name
            ));
        } else {
            s.push_str(&format!("- {} — {}\n", a.file_name, a.path));
        }
    }
    s.push_str(
        "Before starting the work, view each attached image above with your file-reading tool — \
         they are part of the requirement description.\n",
    );
    s
}

/// Build the message injected into the agent for a claimed requirement.
/// Tells the agent exactly what to do and how to report completion. The agent
/// must NOT pick the next requirement — the platform hands it the next one.
///
/// The current chat engine is Nomi, whose native requirement tools are expected
/// to be registered by host wiring. Terminal MCP prompts are built separately.
pub fn build_requirement_prompt(
    tag: &str,
    req: &Requirement,
    claim_generation: i64,
    claim_token: &str,
    agent_type: AgentType,
    attachments: &[PromptAttachment],
) -> String {
    match agent_type {
        AgentType::Nomi => build_native_tools_prompt(
            &format!("[AutoWork] You are working through requirements in tag \"{tag}\"."),
            req,
            claim_generation,
            claim_token,
            attachments,
            "",
        ),
    }
}

/// Single source of truth for the native-tools prompt shape. The chat and
/// terminal variants differ only in their opening line and optional extra
/// sections, so sharing the template keeps the requirement block and the
/// completion contract from drifting between the two surfaces.
fn build_native_tools_prompt(
    header: &str,
    req: &Requirement,
    claim_generation: i64,
    claim_token: &str,
    attachments: &[PromptAttachment],
    extra_sections: &str,
) -> String {
    format!(
        "{header}\n\n\
         ## Current requirement\n\
         id: {id}\n\
         claim_generation: {claim_generation}\n\
         claim_token: {claim_token}\n\
         title: {title}\n\
         order: {order}\n\n\
         {content}\n\
         {attachments_section}\
         {extra_sections}\n\
         ## When finished\n\
         - Call `requirement_complete({{\"id\":\"{id}\",\"claim_generation\":{claim_generation},\
         \"claim_token\":\"{claim_token}\",\"completion_note\":\"what you did\"}})` when done.\n\
         - If you cannot complete it, call `requirement_update_status({{\"id\":\"{id}\",\
         \"claim_generation\":{claim_generation},\"claim_token\":\"{claim_token}\",\
         \"status\":\"failed\",\"note\":\"reason\"}})`.\n\
         Do not pick the next requirement yourself — the platform will hand you the next one.",
        header = header,
        id = req.requirement_id,
        claim_generation = claim_generation,
        claim_token = claim_token,
        title = req.title,
        order = req.order_key,
        content = req.content,
        attachments_section = render_attachments_section(attachments),
        extra_sections = extra_sections,
    )
}

/// Build the message injected into a terminal CLI (claude/codex over a PTY)
/// for a claimed requirement.
///
/// The agent is instructed to declare completion via the `requirement_complete`
/// / `requirement_update_status` MCP tools (injected by Task 2 into every
/// AutoWork-enabled agent terminal). This mirrors the native-tools branch of
/// `build_requirement_prompt`: the tools ARE present, so the agent
/// SHOULD call them. A clean turn-end where the agent did NOT call them → the
/// platform parks the requirement as `needs_review` (not silently done).
pub fn build_terminal_requirement_prompt(
    tag: &str,
    req: &Requirement,
    claim_generation: i64,
    claim_token: &str,
    attachments: &[PromptAttachment],
    knowledge_mounted: bool,
) -> String {
    // One line, only when the workspace ACTUALLY has bases mounted (resolved
    // live by the driver): points at the injected retrieval tool without
    // re-listing bases (the MCP initialize instructions carry those). The old
    // static TERMINAL_KNOWLEDGE_HINT was removed because it referenced file
    // paths unconditionally; this replacement is gated and tool-based.
    let knowledge_section = if knowledge_mounted {
        "\n## Knowledge\n\
         Curated knowledge bases are mounted for this workspace. Call `knowledge_search` \
         before answering from memory when the requirement touches a topic they may cover.\n"
    } else {
        ""
    };
    build_native_tools_prompt(
        &format!(
            "[AutoWork] You are working through requirements in tag \"{tag}\". Complete ONLY the \
             requirement below."
        ),
        req,
        claim_generation,
        claim_token,
        attachments,
        knowledge_section,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::RequirementStatus;

    const ATTACHMENT_REQ_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const CLAIM_TOKEN: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn req() -> Requirement {
        Requirement {
            requirement_id: ATTACHMENT_REQ_ID.into(),
            display_no: 1,
            title: "Do X".into(),
            content: "Detailed body".into(),
            tag: "t".into(),
            order_key: "1.2".into(),
            status: RequirementStatus::InProgress,
            completion_note: None,
            owner_conversation_id: None,
            owner_terminal_id: None,
            started_at: None,
            completed_at: None,
            attempt_count: 1,
            created_by: "user".into(),
            created_at: 0,
            updated_at: 0,
            attachments: vec![],
        }
    }

    fn atts() -> Vec<crate::attachments::PromptAttachment> {
        vec![
            crate::attachments::PromptAttachment {
                file_name: "设计稿.png".into(),
                path: format!(
                    "./.nomi/requirement-attachments/{ATTACHMENT_REQ_ID}/设计稿.png"
                ),
                missing: false,
            },
            crate::attachments::PromptAttachment {
                file_name: "gone.png".into(),
                path: String::new(),
                missing: true,
            },
        ]
    }

    #[test]
    fn attachments_section_lists_paths_and_missing_marker() {
        for at in [AgentType::Nomi] {
            let p = build_requirement_prompt("t", &req(), 7, CLAIM_TOKEN, at, &atts());
            assert!(p.contains("Requirement attachments"));
            assert!(p.contains(&format!(
                "./.nomi/requirement-attachments/{ATTACHMENT_REQ_ID}/设计稿.png"
            )));
            assert!(p.contains("设计稿.png"));
            assert!(p.contains("missing"), "vanished originals are flagged, not silently dropped");
            assert!(p.contains("view each attached image"), "must instruct the model to read the images");
        }
        let p =
            build_terminal_requirement_prompt("t", &req(), 7, CLAIM_TOKEN, &atts(), false);
        assert!(p.contains("Requirement attachments"));
        assert!(p.contains("设计稿.png"));
    }

    #[test]
    fn no_attachments_means_no_section() {
        let p = build_requirement_prompt(
            "t",
            &req(),
            7,
            CLAIM_TOKEN,
            AgentType::Nomi,
            &[],
        );
        assert!(!p.contains("Requirement attachments"));
        let p = build_terminal_requirement_prompt("t", &req(), 7, CLAIM_TOKEN, &[], false);
        assert!(!p.contains("Requirement attachments"));
    }

    #[test]
    fn nomi_prompt_contains_id_and_native_tool_instructions() {
        let p = build_requirement_prompt(
            "t",
            &req(),
            7,
            CLAIM_TOKEN,
            AgentType::Nomi,
            &[],
        );
        assert!(p.contains(&format!("id: {ATTACHMENT_REQ_ID}")));
        assert!(p.contains("claim_generation: 7"));
        assert!(p.contains("\"claim_generation\":7"));
        assert!(p.contains(&format!("claim_token: {CLAIM_TOKEN}")));
        assert!(p.contains(&format!("\"claim_token\":\"{CLAIM_TOKEN}\"")));
        assert!(p.contains("Detailed body"));
        assert!(
            p.contains("requirement_complete"),
            "Nomi prompt MUST instruct calling requirement_complete (tool is registered for Nomi)"
        );
        assert!(
            p.contains("requirement_update_status"),
            "Nomi prompt MUST instruct calling requirement_update_status on failure"
        );
    }

    #[test]
    fn has_native_requirement_tools_holds_for_nomi() {
        assert!(has_native_requirement_tools(AgentType::Nomi));
    }

    #[test]
    fn terminal_prompt_instructs_requirement_complete_and_gates_knowledge_hint() {
        let p = build_terminal_requirement_prompt("t", &req(), 7, CLAIM_TOKEN, &[], false);
        assert!(p.contains(&format!("id: {ATTACHMENT_REQ_ID}")));
        assert!(p.contains("claim_generation: 7"));
        assert!(p.contains("\"claim_generation\":7"));
        assert!(p.contains(&format!("claim_token: {CLAIM_TOKEN}")));
        assert!(p.contains(&format!("\"claim_token\":\"{CLAIM_TOKEN}\"")));
        assert!(p.contains("Detailed body"));
        // Must instruct the agent to call the requirement completion tools
        // (they are injected via the requirement MCP server — Task 2).
        assert!(
            p.contains("requirement_complete"),
            "terminal prompt MUST instruct calling requirement_complete"
        );
        assert!(
            p.contains("requirement_update_status"),
            "terminal prompt MUST instruct calling requirement_update_status on failure"
        );
        // No mounts → zero knowledge prose (the old unconditional
        // TERMINAL_KNOWLEDGE_HINT stays dead).
        assert!(
            !p.contains("knowledge"),
            "unmounted terminal prompt must NOT contain a knowledge hint"
        );
        // The old printed-marker protocol is gone.
        assert!(!p.contains("NOMI_AUTOWORK_END"), "terminal prompt must not ask for a marker");

        // Mounted → exactly the tool-based one-liner (RC-5), no file paths.
        let mounted = build_terminal_requirement_prompt("t", &req(), 7, CLAIM_TOKEN, &[], true);
        assert!(
            mounted.contains("knowledge_search"),
            "mounted terminal prompt must point at the retrieval tool"
        );
        assert!(
            !mounted.contains(".nomi/knowledge"),
            "the hint is tool-based, never a file-path contract"
        );
    }

}
