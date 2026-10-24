//! Numbered input selectors for optional exact-result publication.
//! Source selectors never prove semantic coverage or grant execution authority.
use nomifun_chat_model_broker::{ChatContentPart, ChatMessage, ChatRole};

fn multi_item_input(input: &ChatMessage) -> bool {
    if input.role != ChatRole::User { return false; }
    input.content.iter().any(|part| {
        let ChatContentPart::Text { text } = part else { return false; };
        let mut fenced = false;
        let mut numbers = std::collections::BTreeSet::new();
        for line in text.lines().map(str::trim_start) {
            if line.starts_with("```") || line.starts_with("~~~") { fenced = !fenced; continue; }
            if fenced { continue; }
            let end = line.find(|c: char| !c.is_ascii_digit()).unwrap_or(line.len());
            if end == 0 || end > 2 { continue; }
            let suffix = &line[end..];
            if suffix.starts_with(". ") || suffix.starts_with(") ") || suffix.starts_with('、') {
                numbers.insert(&line[..end]);
            }
        }
        numbers.len() >= 2
    })
}

pub(crate) fn multi_item_task(inputs: &[ChatMessage]) -> bool {
    inputs.iter().any(multi_item_input)
}

/// Syntactic delivery slots only. They preserve source text; they do not
/// interpret intent, grant actions, or prove semantic coverage.
pub(crate) fn delivery_slots(inputs: &[ChatMessage]) -> Vec<(String, String)> {
    let mut slots = Vec::new();
    for (input_index, input) in inputs.iter().enumerate() {
        if !multi_item_input(input) { continue; }
        for (part_index, part) in input.content.iter().enumerate() {
            let ChatContentPart::Text { text } = part else { continue; };
            let mut fenced = false;
            for (line_index, line) in text.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("```") || trimmed.starts_with("~~~") { fenced = !fenced; continue; }
                if fenced { continue; }
                let end = trimmed.find(|c: char| !c.is_ascii_digit()).unwrap_or(trimmed.len());
                if end == 0 || end > 2 { continue; }
                let suffix = &trimmed[end..];
                if suffix.starts_with(". ") || suffix.starts_with(") ") || suffix.starts_with('、') {
                    slots.push((format!("input_{input_index}_part_{part_index}_line_{line_index}"), trimmed.to_owned()));
                }
            }
        }
    }
    slots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(text: &str) -> Vec<ChatMessage> {
        vec![crate::context_lifecycle::text_message(ChatRole::User, text.into())]
    }

    #[test]
    fn numbered_result_slots_keep_source_identity_and_ignore_examples() {
        let inputs = input("1. Read the actual directory.\n2. Report its exact entries.");
        assert_eq!(delivery_slots(&inputs), vec![
            ("input_0_part_0_line_0".into(), "1. Read the actual directory.".into()),
            ("input_0_part_0_line_1".into(), "2. Report its exact entries.".into()),
        ]);
        for text in ["Read one file.", "1. Read one file.", "```text\n1. example\n2. example\n```", "> 1. quote\n> 2. quote"] {
            assert!(!multi_item_task(&input(text)));
            assert!(delivery_slots(&input(text)).is_empty());
        }
        assert!(multi_item_task(&input("1、读取\n2、报告")));
    }
}
