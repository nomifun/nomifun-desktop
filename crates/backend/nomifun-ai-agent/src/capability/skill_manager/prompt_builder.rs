use super::{SkillDefinition, SkillIndex};

/// Build a formatted text block listing available skills for injection.
///
/// The output includes skill names with descriptions and instructions
/// on how to request loading via `[LOAD_SKILL: name]`.
pub fn build_skills_index_text(skills: &[SkillIndex]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut lines = Vec::with_capacity(skills.len() + 4);
    lines.push("## Available Skills".to_string());
    lines.push(String::new());
    lines.push("To load a skill, include `[LOAD_SKILL: skill-name]` in your response.".to_string());
    lines.push(String::new());

    for skill in skills {
        lines.push(format!("- **{}**: {}", skill.name, skill.description));
    }

    lines.join("\n")
}

/// Build system instructions text with full skill content (for Gemini).
pub fn build_system_instructions(base_instructions: &str, skills: &[SkillDefinition]) -> String {
    if skills.is_empty() {
        return base_instructions.to_string();
    }

    let mut parts = vec![base_instructions.to_string()];

    for skill in skills {
        if let Some(body) = &skill.body {
            parts.push(format!("\n## Skill: {}\n\n{}", skill.name, body));
        }
    }

    parts.join("\n")
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

/// Build system instructions with skills index only (for Gemini index-only mode).
///
/// Unlike [`build_system_instructions`] which injects full skill bodies,
/// this variant injects only the skill index (name + description) and
/// the `[LOAD_SKILL]` protocol, allowing the agent to request full content on demand.
pub fn build_system_instructions_with_skills_index(base_instructions: &str, skills: &[SkillIndex]) -> String {
    let index_text = build_skills_index_text(skills);
    if index_text.is_empty() {
        return base_instructions.to_string();
    }

    format!("{base_instructions}\n\n{index_text}")
}

/// Prepare the first message with full skill content (for Gemini).
///
/// Prepends `[Assistant Rules]` block with complete skill bodies.
pub fn prepare_first_message(content: &str, skills: &[SkillDefinition], preset_context: Option<&str>) -> String {
    let mut parts = Vec::new();
    let has_rules = !skills.is_empty() || preset_context.is_some();

    if has_rules {
        parts.push("[Assistant Rules]".to_string());

        if let Some(ctx) = preset_context
            && !ctx.is_empty()
        {
            parts.push(ctx.to_string());
        }

        for skill in skills {
            if let Some(body) = &skill.body {
                parts.push(format!("## Skill: {}\n\n{}", skill.name, body));
            }
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
    use std::path::PathBuf;

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
        assert!(text.contains("[LOAD_SKILL: skill-name]"));
        assert!(text.contains("- **review**: Code review"));
        assert!(text.contains("- **debug**: Debugging helper"));
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
    fn prepare_first_message_with_full_skills() {
        let skills = vec![SkillDefinition {
            name: "review".into(),
            description: "Review".into(),
            location: PathBuf::new(),
            source: nomifun_extension::SkillSource::Custom,
            relative_location: None,
            body: Some("Full review instructions here.".into()),
        }];
        let result = prepare_first_message("Hello", &skills, None);
        assert!(result.contains("[Assistant Rules]"));
        assert!(result.contains("## Skill: review"));
        assert!(result.contains("Full review instructions here."));
        assert!(result.contains("[/Assistant Rules]"));
        assert!(result.ends_with("Hello"));
    }

    #[test]
    fn prepare_first_message_no_skills_no_context() {
        let result = prepare_first_message("Hello", &[], None);
        assert_eq!(result, "Hello");
    }

    #[test]
    fn prepare_first_message_context_only() {
        let result = prepare_first_message_with_skills_index("Hello", &[], Some("Rules here."));
        assert!(result.contains("[Assistant Rules]"));
        assert!(result.contains("Rules here."));
        assert!(result.ends_with("Hello"));
    }

    // -----------------------------------------------------------------------
    // System instructions builder
    // -----------------------------------------------------------------------

    #[test]
    fn build_system_instructions_no_skills() {
        let result = build_system_instructions("Base prompt", &[]);
        assert_eq!(result, "Base prompt");
    }

    #[test]
    fn build_system_instructions_with_skills() {
        let skills = vec![SkillDefinition {
            name: "helper".into(),
            description: "A helper".into(),
            location: PathBuf::new(),
            source: nomifun_extension::SkillSource::Custom,
            relative_location: None,
            body: Some("Helper body content.".into()),
        }];
        let result = build_system_instructions("Base prompt", &skills);
        assert!(result.starts_with("Base prompt"));
        assert!(result.contains("## Skill: helper"));
        assert!(result.contains("Helper body content."));
    }

    #[test]
    fn build_system_instructions_with_skills_index_no_skills() {
        let result = build_system_instructions_with_skills_index("Base prompt", &[]);
        assert_eq!(result, "Base prompt");
    }

    #[test]
    fn build_system_instructions_with_skills_index_includes_index() {
        let skills = vec![SkillIndex {
            name: "helper".into(),
            description: "A helper skill".into(),
        }];
        let result = build_system_instructions_with_skills_index("Base prompt", &skills);
        assert!(result.starts_with("Base prompt"));
        assert!(result.contains("## Available Skills"));
        assert!(result.contains("- **helper**: A helper skill"));
        assert!(result.contains("[LOAD_SKILL: skill-name]"));
    }

    #[test]
    fn build_system_instructions_skips_unloaded_skills() {
        let skills = vec![SkillDefinition {
            name: "unloaded".into(),
            description: "Not loaded".into(),
            location: PathBuf::new(),
            source: nomifun_extension::SkillSource::Custom,
            relative_location: None,
            body: None,
        }];
        let result = build_system_instructions("Base", &skills);
        assert_eq!(result, "Base");
    }
}
