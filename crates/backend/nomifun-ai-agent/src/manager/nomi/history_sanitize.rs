//! Sanitize a resumed nomi session's message history before it is replayed
//! to a provider.
//!
//! Background: when the user clicks "Stop" on a tool-call mid-stream, nomi
//! may persist an assistant message that contains `ToolUse` content blocks
//! but whose tool calls were never followed up by the matching `ToolResult`
//! blocks. On the next turn, the engine replays history verbatim and strict
//! providers reject the request:
//!   - Ollama-compatible providers (e.g. `qwen3:8b`) return
//!     `400 invalid message content type: <nil>` because the assistant
//!     message has `tool_calls != null` but `content == null`.
//!   - Some OpenAI-compatible proxies (e.g. DeepSeek behind a strict gateway)
//!     return `400 invalid_request_error` for the same reason.
//!
//! Fix: drop assistant messages that
//!   1. contain at least one `ToolUse` block,
//!   2. have NO non-empty `Text` content, AND
//!   3. have NO subsequent `ToolResult` block (in any later message) that
//!      references one of those tool-use ids.
//!
//! A complete `assistant(tool_use) → user(tool_result)` pair is left intact —
//! that shape is valid and required by every provider.
//!
//! This logic is intentionally a free function (not a method on
//! `NomiAgentManager`) so it can be unit-tested in isolation and so we do
//! not add yet another field to a manager (per `AGENTS.md`).

use std::collections::HashSet;

use nomi_types::message::{ContentBlock, Message, Role};

/// Drop orphaned assistant tool-call messages from a session's history.
///
/// Returns the number of messages removed.
///
/// Operates in-place on `messages`. Safe to call on an empty vector.
pub fn sanitize_session_messages(messages: &mut Vec<Message>) -> usize {
    if messages.is_empty() {
        return 0;
    }

    // Collect every tool_use_id that has a matching tool_result anywhere
    // in the entire history. We do this in one pass so that the lookup
    // for each candidate assistant message is O(1).
    let mut answered_tool_use_ids: HashSet<String> = HashSet::new();
    for msg in messages.iter() {
        for block in &msg.content {
            if let ContentBlock::ToolResult { tool_use_id, .. } = block {
                answered_tool_use_ids.insert(tool_use_id.clone());
            }
        }
    }

    let original_len = messages.len();
    messages.retain(|msg| !is_orphaned_assistant_tool_call(msg, &answered_tool_use_ids));
    original_len - messages.len()
}

/// True iff `msg` is an assistant message that has tool_use blocks, no
/// non-empty text, and at least one of its tool_use ids has no matching
/// tool_result anywhere in the history.
fn is_orphaned_assistant_tool_call(msg: &Message, answered: &HashSet<String>) -> bool {
    if msg.role != Role::Assistant {
        return false;
    }

    let mut has_tool_use = false;
    let mut has_unanswered = false;
    let mut has_text = false;

    for block in &msg.content {
        match block {
            ContentBlock::ToolUse { id, .. } => {
                has_tool_use = true;
                if !answered.contains(id) {
                    has_unanswered = true;
                }
            }
            ContentBlock::Text { text } => {
                if !text.trim().is_empty() {
                    has_text = true;
                }
            }
            // Thinking, ToolResult, and Image blocks do not change the orphan
            // determination. ToolResult should not appear on assistant
            // messages, but if it does we ignore it here.
            ContentBlock::Thinking { .. } | ContentBlock::ToolResult { .. } | ContentBlock::Image { .. } => {}
        }
    }

    has_tool_use && has_unanswered && !has_text
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomi_types::message::{Message, Role};
    use serde_json::json;

    fn assistant_tool_call(ids: &[&str]) -> Message {
        let blocks = ids
            .iter()
            .map(|id| ContentBlock::ToolUse {
                id: (*id).to_owned(),
                name: "search".to_owned(),
                input: json!({}),
                extra: None,
            })
            .collect();
        Message::new(Role::Assistant, blocks)
    }

    fn assistant_text_plus_tool_call(text: &str, id: &str) -> Message {
        Message::new(
            Role::Assistant,
            vec![
                ContentBlock::Text { text: text.to_owned() },
                ContentBlock::ToolUse {
                    id: id.to_owned(),
                    name: "search".to_owned(),
                    input: json!({}),
                    extra: None,
                },
            ],
        )
    }

    fn user_tool_result(tool_use_id: &str) -> Message {
        Message::new(
            Role::User,
            vec![ContentBlock::ToolResult {
                tool_use_id: tool_use_id.to_owned(),
                content: "ok".to_owned(),
                is_error: false,
                images: Vec::new(),
            }],
        )
    }

    fn user_text(text: &str) -> Message {
        Message::new(Role::User, vec![ContentBlock::Text { text: text.to_owned() }])
    }

    fn assistant_text(text: &str) -> Message {
        Message::new(Role::Assistant, vec![ContentBlock::Text { text: text.to_owned() }])
    }

    #[test]
    fn drops_orphaned_assistant_tool_call_with_no_matching_result() {
        // user → assistant(tool_use, no text) — Stop pressed before tool_result
        let mut messages = vec![user_text("do thing"), assistant_tool_call(&["call_orphan"])];
        let removed = sanitize_session_messages(&mut messages);
        assert_eq!(removed, 1);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::User);
    }

    #[test]
    fn keeps_assistant_tool_call_with_matching_tool_result() {
        // user → assistant(tool_use) → user(tool_result) — valid pair
        let mut messages = vec![
            user_text("do thing"),
            assistant_tool_call(&["call_ok"]),
            user_tool_result("call_ok"),
        ];
        let removed = sanitize_session_messages(&mut messages);
        assert_eq!(removed, 0);
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn keeps_regular_assistant_text_message() {
        let mut messages = vec![user_text("hi"), assistant_text("hello there")];
        let removed = sanitize_session_messages(&mut messages);
        assert_eq!(removed, 0);
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn keeps_assistant_with_text_and_orphan_tool_call() {
        // Assistant emitted some streamed text before the tool was cancelled.
        // Provider will accept this because content is non-null even though
        // tool_calls is unmatched. We keep it to preserve the visible turn.
        // (A future iteration could strip just the orphan ToolUse blocks; for
        // now we only drop messages that would crash the provider.)
        let mut messages = vec![
            user_text("hi"),
            assistant_text_plus_tool_call("partial reply", "call_orphan"),
        ];
        let removed = sanitize_session_messages(&mut messages);
        assert_eq!(removed, 0);
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn drops_orphan_when_some_calls_are_answered_but_not_all() {
        // assistant fired two tool calls; only one got a result before Stop.
        // The whole assistant message is still invalid for strict providers
        // because at least one call_id has no tool_result, so we drop it.
        let mut messages = vec![
            user_text("do two things"),
            assistant_tool_call(&["call_a", "call_b"]),
            user_tool_result("call_a"),
        ];
        let removed = sanitize_session_messages(&mut messages);
        assert_eq!(removed, 1);
        // The dangling tool_result for call_a now has no matching tool_use,
        // but it is structurally a user message and providers tolerate that
        // shape. Dropping it would risk losing user-visible context. We only
        // sanitize the assistant side here.
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[1].role, Role::User);
    }

    #[test]
    fn keeps_full_history_when_all_pairs_match() {
        let mut messages = vec![
            user_text("first"),
            assistant_tool_call(&["c1"]),
            user_tool_result("c1"),
            assistant_text("done"),
            user_text("again"),
            assistant_tool_call(&["c2", "c3"]),
            user_tool_result("c2"),
            user_tool_result("c3"),
            assistant_text("all done"),
        ];
        let original_len = messages.len();
        let removed = sanitize_session_messages(&mut messages);
        assert_eq!(removed, 0);
        assert_eq!(messages.len(), original_len);
    }

    #[test]
    fn no_op_on_empty_history() {
        let mut messages: Vec<Message> = Vec::new();
        let removed = sanitize_session_messages(&mut messages);
        assert_eq!(removed, 0);
        assert!(messages.is_empty());
    }

    #[test]
    fn drops_orphan_assistant_with_only_empty_text_and_tool_call() {
        // Some providers stream an empty text delta before the tool call.
        // Empty/whitespace text should NOT save the assistant message.
        let msg = Message::new(
            Role::Assistant,
            vec![
                ContentBlock::Text { text: "   ".to_owned() },
                ContentBlock::ToolUse {
                    id: "call_empty".to_owned(),
                    name: "search".to_owned(),
                    input: json!({}),
                    extra: None,
                },
            ],
        );
        let mut messages = vec![user_text("hi"), msg];
        let removed = sanitize_session_messages(&mut messages);
        assert_eq!(removed, 1);
        assert_eq!(messages.len(), 1);
    }
}
