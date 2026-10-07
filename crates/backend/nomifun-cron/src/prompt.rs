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
