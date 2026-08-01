use super::SkillIndex;

/// Build a formatted text block listing available skills for injection.
///
/// The output lists skill names with descriptions. No loading protocol is
/// advertised: nothing in the product watches agent output for load
/// requests, so the index is purely informational context.
pub fn build_skills_index_text(skills: &[SkillIndex]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut lines = Vec::with_capacity(skills.len() + 2);
    lines.push("## Available Skills".to_string());
    lines.push(String::new());

    for skill in skills {
        lines.push(format!("- **{}**: {}", skill.name, skill.description));
    }

    lines.join("\n")
}

/// Prepare the first message with skills index prefix (for ACP/Codex).
///
/// Prepends `[Assistant Rules]` block with skill index to the user content.
pub fn prepare_first_message_with_skills_index(
    content: &str,
    skills: &[SkillIndex],
    preset_context: Option<&str>,
) -> String {
    let mut parts = Vec::new();

    let index_text = build_skills_index_text(skills);
    let has_rules = !index_text.is_empty() || preset_context.is_some();

    if has_rules {
        parts.push("[Assistant Rules]".to_string());

        if let Some(ctx) = preset_context
            && !ctx.is_empty()
        {
            parts.push(ctx.to_string());
        }

        if !index_text.is_empty() {
            parts.push(index_text);
        }

        parts.push("[/Assistant Rules]".to_string());
        parts.push(String::new());
    }

    parts.push(content.to_string());
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Skills index text
    // -----------------------------------------------------------------------

    #[test]
    fn build_skills_index_text_empty() {
        assert!(build_skills_index_text(&[]).is_empty());
    }

    #[test]
    fn build_skills_index_text_with_skills() {
        let skills = vec![
            SkillIndex {
                name: "review".into(),
                description: "Code review".into(),
            },
            SkillIndex {
                name: "debug".into(),
                description: "Debugging helper".into(),
            },
        ];
        let text = build_skills_index_text(&skills);
        assert!(text.contains("## Available Skills"));
        assert!(text.contains("- **review**: Code review"));
        assert!(text.contains("- **debug**: Debugging helper"));
    }

    #[test]
    fn build_skills_index_text_does_not_advertise_dead_load_protocol() {
        let skills = vec![SkillIndex {
            name: "review".into(),
            description: "Code review".into(),
        }];
        let text = build_skills_index_text(&skills);
        assert!(
            !text.contains("LOAD_SKILL"),
            "nothing fulfills [LOAD_SKILL: ...] requests, so the index must not instruct the model to emit them"
        );
    }

    // -----------------------------------------------------------------------
    // First message preparation
    // -----------------------------------------------------------------------

    #[test]
    fn prepare_first_message_with_index_no_skills() {
        let result = prepare_first_message_with_skills_index("Hello", &[], None);
        assert_eq!(result, "Hello");
    }

    #[test]
    fn prepare_first_message_with_index_and_context() {
        let skills = vec![SkillIndex {
            name: "test".into(),
            description: "Testing".into(),
        }];
        let result = prepare_first_message_with_skills_index("Hello", &skills, Some("Be concise."));
        assert!(result.contains("[Assistant Rules]"));
        assert!(result.contains("Be concise."));
        assert!(result.contains("- **test**: Testing"));
        assert!(result.contains("[/Assistant Rules]"));
        assert!(result.ends_with("Hello"));
    }

    #[test]
    fn prepare_first_message_context_only() {
        let result = prepare_first_message_with_skills_index("Hello", &[], Some("Rules here."));
        assert!(result.contains("[Assistant Rules]"));
        assert!(result.contains("Rules here."));
        assert!(result.ends_with("Hello"));
    }
}
