use nomifun_cron::prompt::{
    SKILL_SUGGEST_FILENAME, build_existing_conversation_prompt,
    build_new_conversation_prompt_with_skill_suggest, build_new_conversation_with_skill_prompt,
    build_skill_suggest_prompt,
};

#[test]
fn build_new_conversation_prompt_with_skill_suggest_includes_follow_up_block() {
    let prompt = build_new_conversation_prompt_with_skill_suggest("Daily Report", "Every day at 9am", "Summarize it.");
    assert!(prompt.contains(&format!("create a file named \"{SKILL_SUGGEST_FILENAME}\"")));
    assert!(prompt.contains("short kebab-case name"));
    assert!(prompt.contains("If you think the task is too simple or one-off to benefit from a skill file"));
}

#[test]
fn build_new_conversation_with_skill_prompt_matches_frontend_copy() {
    let prompt = build_new_conversation_with_skill_prompt("Daily Report", "Summarize it.");
    assert_eq!(
        prompt,
        "[Scheduled Task Context]\nTask: Daily Report\n\nThis is a scheduled task execution. A skill file with detailed instructions has been loaded\ninto your workspace. You MUST read and follow the skill instructions precisely.\n\nRules:\n1. Execute the task directly — do NOT ask clarifying questions.\n2. Follow the output format, tone, sources, and steps defined in the skill.\n3. If the task requires external data (news, weather, etc.), search for the latest information.\n[/Scheduled Task Context]\n\nSummarize it."
    );
}

#[test]
fn build_existing_conversation_prompt_matches_frontend_copy() {
    let prompt = build_existing_conversation_prompt("Daily Report", "Every day at 9am", "Summarize it.");
    assert_eq!(
        prompt,
        "[Scheduled Task Execution]\nTask: Daily Report\nSchedule: Every day at 9am\n\nThis message is NOT a conversation from the user — it is a scheduled task triggered automatically.\nThe text below is a TASK INSTRUCTION that you must execute, not something the user is saying to you.\n\nRules:\n1. Treat the instruction as a command to perform, not as a chat message to respond to.\n2. Execute it directly — do NOT ask clarifying questions.\n3. If the task requires external data (news, weather, etc.), search for the latest information.\n\nTask instruction:\nSummarize it."
    );
}

#[test]
fn build_skill_suggest_prompt_matches_frontend_copy() {
    let prompt = build_skill_suggest_prompt("Daily Report");
    assert!(prompt.starts_with("The task \"Daily Report\" is a recurring scheduled task. Based on what you just did,"));
    assert!(prompt.contains("```markdown"));
    assert!(prompt.contains("Use concrete details from this execution, not placeholders."));
    assert!(
        prompt.ends_with(
            "If you think the task is too simple or one-off to benefit from a skill file, you can skip this."
        )
    );
}
