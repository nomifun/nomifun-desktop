use nomi_types::message::{ContentBlock, Message};

const CHARS_PER_TOKEN_TEXT: usize = 4;

const CHARS_PER_TOKEN_JSON: usize = 3;

/// Flat per-image token estimate. A 1568px-edge screenshot costs roughly
/// 1100-1600 tokens on Anthropic's vision pricing; over-estimating keeps
/// compaction triggering early rather than late.
const TOKENS_PER_IMAGE: usize = 1600;

/// Estimate the total token count for a slice of messages.
///
/// Intentionally conservative (slightly over-estimates) to ensure
/// compaction triggers rather than being skipped.
pub fn estimate_tokens_from_messages(messages: &[Message]) -> u64 {
    let mut total_chars: usize = 0;
    let mut json_chars: usize = 0;
    let mut image_tokens: usize = 0;

    for msg in messages {
        for block in &msg.content {
            match block {
                ContentBlock::Text { text } => {
                    total_chars += text.len();
                }
                ContentBlock::Thinking { thinking, .. } => {
                    total_chars += thinking.len();
                }
                ContentBlock::ToolUse { name, input, .. } => {
                    let input_str = input.to_string();
                    json_chars += name.len() + input_str.len();
                }
                ContentBlock::ToolResult { content, images, .. } => {
                    total_chars += content.len();
                    image_tokens += images.len() * TOKENS_PER_IMAGE;
                }
                ContentBlock::Image { .. } => {
                    image_tokens += TOKENS_PER_IMAGE;
                }
            }
        }
    }

    let text_tokens = total_chars / CHARS_PER_TOKEN_TEXT;
    let json_tokens = json_chars / CHARS_PER_TOKEN_JSON;

    (text_tokens + json_tokens + image_tokens) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomi_types::message::{Message, Role};
    use serde_json::json;

    #[test]
    fn empty_messages_returns_zero() {
        assert_eq!(estimate_tokens_from_messages(&[]), 0);
    }

    #[test]
    fn text_only_message() {
        let text = "a".repeat(400);
        let msg = Message::new(Role::User, vec![ContentBlock::Text { text }]);
        assert_eq!(estimate_tokens_from_messages(&[msg]), 100);
    }

    #[test]
    fn tool_use_message_uses_json_ratio() {
        let input = json!({"cmd": "ls -la"});
        let input_len = "Bash".len() + input.to_string().len();
        let msg = Message::new(
            Role::Assistant,
            vec![ContentBlock::ToolUse {
                id: "call_1".into(),
                name: "Bash".into(),
                input,
                extra: None,
            }],
        );
        let result = estimate_tokens_from_messages(&[msg]);
        assert_eq!(result, (input_len / CHARS_PER_TOKEN_JSON) as u64);
    }

    #[test]
    fn tool_result_uses_text_ratio() {
        let content = "x".repeat(800);
        let msg = Message::new(
            Role::User,
            vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".into(),
                content,
                is_error: false,
                images: Vec::new(),
            }],
        );
        assert_eq!(estimate_tokens_from_messages(&[msg]), 200);
    }

    #[test]
    fn mixed_conversation_accumulates() {
        let messages = vec![
            Message::new(
                Role::User,
                vec![ContentBlock::Text {
                    text: "a".repeat(400),
                }],
            ),
            Message::new(
                Role::Assistant,
                vec![
                    ContentBlock::Text {
                        text: "b".repeat(200),
                    },
                    ContentBlock::ToolUse {
                        id: "c1".into(),
                        name: "Read".into(),
                        input: json!({"path": "/foo/bar.rs"}),
                        extra: None,
                    },
                ],
            ),
            Message::new(
                Role::User,
                vec![ContentBlock::ToolResult {
                    tool_use_id: "c1".into(),
                    content: "c".repeat(1200),
                    is_error: false,
                    images: Vec::new(),
                }],
            ),
        ];
        let estimate = estimate_tokens_from_messages(&messages);
        // text_tokens = (400 + 200 + 1200) / 4 = 450
        // json_tokens = ("Read".len() + json_string.len()) / 3
        assert!(estimate > 450);
        assert!(estimate < 600);
    }

    #[test]
    fn thinking_block_counted() {
        let thinking = "t".repeat(4000);
        let msg = Message::new(
            Role::Assistant,
            vec![ContentBlock::Thinking {
                thinking,
                signature: None,
            }],
        );
        assert_eq!(estimate_tokens_from_messages(&[msg]), 1000);
    }

    #[test]
    fn large_conversation_realistic_estimate() {
        let big_result = "x".repeat(400_000);
        let messages = vec![Message::new(
            Role::User,
            vec![ContentBlock::ToolResult {
                tool_use_id: "c1".into(),
                content: big_result,
                is_error: false,
                images: Vec::new(),
            }],
        )];
        let estimate = estimate_tokens_from_messages(&messages);
        assert_eq!(estimate, 100_000);
    }

    #[test]
    fn effective_watermark_uses_max() {
        let provider_reported: u64 = 500;
        let messages = vec![Message::new(
            Role::User,
            vec![ContentBlock::ToolResult {
                tool_use_id: "c1".into(),
                content: "x".repeat(400_000),
                is_error: false,
                images: Vec::new(),
            }],
        )];
        let local_estimate = estimate_tokens_from_messages(&messages);
        let effective = provider_reported.max(local_estimate);

        assert_eq!(effective, 100_000);
        assert!(effective > provider_reported);
    }
}
