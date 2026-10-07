use nomifun_api_types::Requirement;
use nomifun_common::AppError;

use crate::attachments::{PromptAttachment, validate_prompt_attachment_path};

fn render_attachments_section(
    requirement_id: &str,
    attachments: &[PromptAttachment],
) -> Result<String, AppError> {
    if attachments.is_empty() {
        return Ok(String::new());
    }
    let mut rendered = format!(
        "\n## Requirement attachments ({} images)\n",
        attachments.len()
    );
    for attachment in attachments {
        let display_name = serde_json::to_string(&attachment.file_name).map_err(|error| {
            AppError::Internal(format!("encode attachment display name: {error}"))
        })?;
        if attachment.missing {
            if !attachment.path.is_empty() {
                return Err(AppError::Conflict(
                    "missing AutoWork attachment unexpectedly has a prompt path".to_owned(),
                ));
            }
            rendered.push_str(&format!(
                "- {} — (missing: the original file could not be found)\n",
                display_name
            ));
        } else {
            validate_prompt_attachment_path(requirement_id, &attachment.path)?;
            rendered.push_str(&format!(
                "- {} — {}\n",
                display_name, attachment.path
            ));
        }
    }
    rendered.push_str(
        "Before starting, view each available image with the authorized file-reading action; \
         the images are part of the requirement.\n",
    );
    Ok(rendered)
}

/// Build the durable AgentExecution goal for one queue-selected Requirement.
///
/// Completion is derived from the AgentExecution aggregate. No model-visible
/// claim token, Runtime overlay, status declaration tool, or second delivery
/// receipt is embedded in this prompt.
pub fn build_agent_execution_requirement_goal(
    tag: &str,
    requirement: &Requirement,
    attachments: &[PromptAttachment],
) -> Result<String, AppError> {
    Ok(format!(
        "[AutoWork] Complete only the queue-selected requirement below.\n\n\
         ## Requirement\n\
         id: {id}\n\
         tag: {tag}\n\
         title: {title}\n\
         order: {order}\n\n\
         {content}\n\
         {attachments_section}\
         The platform owns queue advancement and completion state. Do not select another \
         requirement or invent a parallel completion receipt.",
        id = requirement.requirement_id,
        title = requirement.title,
        order = requirement.order_key,
        content = requirement.content,
        attachments_section = render_attachments_section(
            &requirement.requirement_id,
            attachments,
        )?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::RequirementStatus;

    fn requirement() -> Requirement {
        Requirement {
            requirement_id: "0190f5fe-7c00-7a00-8000-000000000001".to_owned(),
            display_no: 1,
            title: "Inspect the architecture".to_owned(),
            content: "Use only the canonical execution path.".to_owned(),
            tag: "architecture".to_owned(),
            order_key: "1".to_owned(),
            status: RequirementStatus::InProgress,
            completion_note: None,
            owner_conversation_id: None,
            owner_terminal_id: None,
            started_at: None,
            completed_at: None,
            attempt_count: 1,
            created_by: "user".to_owned(),
            created_at: 0,
            updated_at: 0,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn execution_goal_contains_business_fact_but_no_claim_or_receipt_protocol() {
        let goal =
            build_agent_execution_requirement_goal("architecture", &requirement(), &[]).unwrap();
        assert!(goal.contains("Inspect the architecture"));
        assert!(goal.contains("canonical execution path"));
        for forbidden in [
            "claim_token",
            "claim_generation",
            "requirement_complete",
            "requirement_update_status",
        ] {
            assert!(!goal.contains(forbidden));
        }
    }

    #[test]
    fn attachments_are_explicit_without_becoming_authority() {
        let requirement = requirement();
        let goal = build_agent_execution_requirement_goal(
            "architecture",
            &requirement,
            &[PromptAttachment {
                file_name: "diagram.png".to_owned(),
                path: format!(
                    "./.nomi/requirement-attachments/{}/0190f5fe-7c00-7a00-8000-000000000002.png",
                    requirement.requirement_id
                ),
                missing: false,
            }],
        )
        .unwrap();
        assert!(goal.contains("diagram.png"));
        assert!(goal.contains("authorized file-reading action"));
    }

    #[test]
    fn attachment_source_paths_are_rejected_without_echoing_them() {
        let secret_source = "C:/private/data/attachments/secret.png";
        let error = build_agent_execution_requirement_goal(
            "architecture",
            &requirement(),
            &[PromptAttachment {
                file_name: "diagram.png".to_owned(),
                path: secret_source.to_owned(),
                missing: false,
            }],
        )
        .unwrap_err();
        assert!(!error.to_string().contains(secret_source));
    }
}
