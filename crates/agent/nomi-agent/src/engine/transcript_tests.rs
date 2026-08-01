use super::{render_transcript, truncate_chars};
use nomi_types::message::{ContentBlock, Message, Role};
use serde_json::json;

#[test]
fn render_tags_roles_and_keeps_text() {
    let messages = vec![
        Message::new(Role::User, vec![ContentBlock::Text { text: "fix the bug".into() }]),
        Message::new(
            Role::Assistant,
            vec![ContentBlock::Text { text: "done".into() }],
        ),
    ];
    let t = render_transcript(&messages);
    assert!(t.contains("[user] fix the bug"));
    assert!(t.contains("[assistant] done"));
}

#[test]
fn render_drops_thinking() {
    let messages = vec![Message::new(
        Role::Assistant,
        vec![
            ContentBlock::Thinking {
                thinking: "secret reasoning".into(),
                signature: None,
            },
            ContentBlock::Text { text: "visible".into() },
        ],
    )];
    let t = render_transcript(&messages);
    assert!(!t.contains("secret reasoning"));
    assert!(t.contains("[assistant] visible"));
}

#[test]
fn render_compresses_tool_use_and_result() {
    let messages = vec![
        Message::new(
            Role::Assistant,
            vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "Read".into(),
                input: json!({"path": "/tmp/a.txt"}),
                extra: None,
            }],
        ),
        Message::new(
            Role::Tool,
            vec![ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                content: "file body".into(),
                is_error: false,
                images: vec![],
            }],
        ),
    ];
    let t = render_transcript(&messages);
    assert!(t.contains("[tool Read]"));
    assert!(t.contains("/tmp/a.txt"));
    assert!(t.contains("[tool result] file body"));
}

#[test]
fn render_marks_tool_error() {
    let messages = vec![Message::new(
        Role::Tool,
        vec![ContentBlock::ToolResult {
            tool_use_id: "t1".into(),
            content: "boom".into(),
            is_error: true,
            images: vec![],
        }],
    )];
    let t = render_transcript(&messages);
    assert!(t.contains("[tool result error] boom"));
}

#[test]
fn truncate_keeps_short_and_cuts_long() {
    assert_eq!(truncate_chars("short", 100), "short");
    let long = "x".repeat(700);
    let cut = truncate_chars(&long, 600);
    assert!(cut.contains("(truncated)"));
    assert!(cut.chars().count() < 700);
}
