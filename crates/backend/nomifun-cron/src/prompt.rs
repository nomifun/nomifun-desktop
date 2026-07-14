/// The fixed filename agents write skill suggestions to in the workspace root.
pub const SKILL_SUGGEST_FILENAME: &str = "SKILL_SUGGEST.md";

/// New-conversation mode for a model-only schedule. It keeps the recurring
/// task framing but never asks the Agent to create a host file it is not
/// authorized to write.
pub fn build_new_conversation_prompt(
    task_name: &str,
    schedule_desc: &str,
    user_prompt: &str,
) -> String {
    format!(
        "[Scheduled Task Context]\nTask: {task_name}\nSchedule: {schedule_desc}\n\nRules:\n1. Execute the task directly — do NOT ask clarifying questions.\n2. Focus on producing useful, actionable output.\n[/Scheduled Task Context]\n\n{user_prompt}"
    )
}

/// New-conversation mode without a saved skill for agents that need the
/// `SKILL_SUGGEST.md` request inline.
pub fn build_new_conversation_prompt_with_skill_suggest(
    task_name: &str,
    schedule_desc: &str,
    user_prompt: &str,
) -> String {
    format!(
        "[Scheduled Task Context]\nTask: {task_name}\nSchedule: {schedule_desc}\n\nRules:\n1. Execute the task directly — do NOT ask clarifying questions.\n2. Focus on producing useful, actionable output.\n3. If the task requires external data (news, weather, etc.), search for the latest information.\n4. After completing the task above, create a file named \"{SKILL_SUGGEST_FILENAME}\" in the current working directory (see instructions at the end).\n[/Scheduled Task Context]\n\n{user_prompt}\n\n---\n\n[Post-Task] After you have fully completed the task above, create a file named \"{SKILL_SUGGEST_FILENAME}\" in the current working directory to help future runs stay consistent. The file should follow this format:\n\n```markdown\n---\nname: <short kebab-case name, e.g. daily-greeting>\ndescription: <one-line description of what this task does>\n---\n\n<Instructions capturing the pattern you used: output format, tone, sources checked, steps taken, quality criteria. Use concrete details from this execution, not placeholders.>\n```\n\nIf you think the task is too simple or one-off to benefit from a skill file, you can skip this step."
    )
}

/// New-conversation mode with an existing saved skill already linked into the
/// agent workspace.
pub fn build_new_conversation_with_skill_prompt(task_name: &str, user_prompt: &str) -> String {
    format!(
        "[Scheduled Task Context]\nTask: {task_name}\n\nThis is a scheduled task execution. A skill file with detailed instructions has been loaded\ninto your workspace. You MUST read and follow the skill instructions precisely.\n\nRules:\n1. Execute the task directly — do NOT ask clarifying questions.\n2. Follow the output format, tone, sources, and steps defined in the skill.\n3. If the task requires external data (news, weather, etc.), search for the latest information.\n[/Scheduled Task Context]\n\n{user_prompt}"
    )
}

/// Existing-conversation mode: wrap the raw task text so the model treats it as
/// an automatic task instruction rather than as user chat.
pub fn build_existing_conversation_prompt(task_name: &str, schedule_desc: &str, user_prompt: &str) -> String {
    format!(
        "[Scheduled Task Execution]\nTask: {task_name}\nSchedule: {schedule_desc}\n\nThis message is NOT a conversation from the user — it is a scheduled task triggered automatically.\nThe text below is a TASK INSTRUCTION that you must execute, not something the user is saying to you.\n\nRules:\n1. Treat the instruction as a command to perform, not as a chat message to respond to.\n2. Execute it directly — do NOT ask clarifying questions.\n3. If the task requires external data (news, weather, etc.), search for the latest information.\n\nTask instruction:\n{user_prompt}"
    )
}

/// Follow-up request asking the agent to write `SKILL_SUGGEST.md` after it has
/// already completed the recurring task.
pub fn build_skill_suggest_prompt(task_name: &str) -> String {
    format!(
        "The task \"{task_name}\" is a recurring scheduled task. Based on what you just did, please create a file named \"{SKILL_SUGGEST_FILENAME}\" in the current working directory to help future runs stay consistent.\n\nThe file should follow this format:\n\n```markdown\n---\nname: <short kebab-case name, e.g. daily-greeting>\ndescription: <one-line description of what this task does>\n---\n\n<Instructions capturing the pattern you used: output format, tone, sources checked, steps taken, quality criteria. Use concrete details from this execution, not placeholders.>\n```\n\nIf you think the task is too simple or one-off to benefit from a skill file, you can skip this."
    )
}
