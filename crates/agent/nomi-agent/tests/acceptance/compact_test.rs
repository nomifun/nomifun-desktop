// Acceptance tests for context compression (all three compaction levels).
//
// TC-A2-01 and TC-A2-03 are purely local (no LLM call).
// TC-A2-02 makes a real LLM call and is skipped when OPENAI_API_KEY is absent.

use nomi_agent::compact::auto::{BOUNDARY_PREFIX, autocompact, should_autocompact};
use nomi_agent::compact::emergency::is_at_emergency_limit;
use nomi_agent::compact::micro::{CLEARED_TOOL_RESULT, microcompact};
use nomi_agent::compact::state::CompactState;
use nomi_config::compact::CompactConfig;
use nomi_types::message::{ContentBlock, Message, Role};
use serde_json::json;

use crate::helpers;

// ── Helpers ────────────────────────────────────────────────────────────────

fn tool_use_block(id: &str, name: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: id.to_string(),
        name: name.to_string(),
        input: json!({}),
        extra: None,
    }
}

fn tool_result_block(id: &str, content: &str) -> ContentBlock {
    ContentBlock::ToolResult {
        tool_use_id: id.to_string(),
        content: content.to_string(),
        is_error: false,
        images: Vec::new(),
    }
}

// ── TC-A2-01: Microcompact clears old tool results (LOCAL) ─────────────────

/// Construct a message history with more than `micro_keep_recent * 2`
/// compactable tool results (each with a matching ToolUse block), run
/// microcompact, and verify that old results are cleared while the most
/// recent `micro_keep_recent` are preserved.
#[test]
fn microcompact_clears_old_tool_results() {
    let keep_recent: usize = 3;
    // We need MORE than keep_recent * 2 = 6 compactable results, so use 8.
    let total_results: usize = 8;

    let config = CompactConfig {
        micro_keep_recent: keep_recent,
        compactable_tools: vec!["Read".to_string()],
        ..CompactConfig::default()
    };

    // Build messages: alternating ToolUse (assistant) and ToolResult (user)
    let mut messages: Vec<Message> = Vec::with_capacity(total_results * 2);
    for i in 0..total_results {
        let id = format!("tool_{i}");
        messages.push(Message::new(
            Role::Assistant,
            vec![tool_use_block(&id, "Read")],
        ));
        messages.push(Message::new(
            Role::User,
            vec![tool_result_block(&id, &format!("content of file {i}"))],
        ));
    }

    let result = microcompact(&mut messages, &config);

    // Verify cleared count is positive
    assert!(
        result.cleared_count > 0,
        "microcompact should clear at least one tool result, got cleared_count=0"
    );

    // Exactly total_results - keep_recent should be cleared
    let expected_cleared = total_results - keep_recent;
    assert_eq!(
        result.cleared_count, expected_cleared,
        "expected {expected_cleared} cleared, got {}",
        result.cleared_count
    );

    // Verify old results (first `expected_cleared`) are replaced with placeholder
    for i in 0..expected_cleared {
        let user_msg_idx = i * 2 + 1; // user messages are at odd indices
        match &messages[user_msg_idx].content[0] {
            ContentBlock::ToolResult { content, .. } => {
                assert_eq!(
                    content, CLEARED_TOOL_RESULT,
                    "tool result at index {i} should be cleared"
                );
            }
            other => panic!("expected ToolResult at index {user_msg_idx}, got {other:?}"),
        }
    }

    // Verify most recent `keep_recent` results are preserved
    for i in expected_cleared..total_results {
        let user_msg_idx = i * 2 + 1;
        match &messages[user_msg_idx].content[0] {
            ContentBlock::ToolResult { content, .. } => {
                let expected = format!("content of file {i}");
                assert_eq!(
                    content, &expected,
                    "tool result at index {i} should be preserved with original content"
                );
            }
            other => panic!("expected ToolResult at index {user_msg_idx}, got {other:?}"),
        }
    }
}

// ── TC-A2-02: Autocompact triggers LLM summary (REAL API CALL) ────────────

/// Set a very low autocompact threshold, verify should_autocompact triggers,
/// then call autocompact with a real LLM provider and verify the result
/// contains the boundary prefix marker.
#[tokio::test]
async fn autocompact_triggers_llm_summary() {
    let api_key = match helpers::openai_api_key() {
        Some(k) => k,
        None => {
            eprintln!("[acceptance] OPENAI_API_KEY not set — skipping");
            return;
        }
    };

    // Use gpt-4.1-mini which supports up to 32768 output tokens.
    // The autocompact function requests COMPACT_MAX_OUTPUT_TOKENS (20000),
    // which exceeds gpt-4o-mini's 16384 limit.
    let config = {
        let base = helpers::openai_config(&api_key);
        nomi_config::config::Config {
            model: "gpt-4.1-mini".to_string(),
            ..base
        }
    };

    let compact_config = CompactConfig {
        context_window: 1000,
        output_reserve: 100,
        autocompact_buffer: 100,
        // threshold = 1000 - 100 - 100 = 800
        ..CompactConfig::default()
    };

    // Verify should_autocompact detects the threshold is exceeded
    assert!(
        should_autocompact(900, &compact_config),
        "900 tokens should exceed the threshold of 800"
    );
    assert!(
        !should_autocompact(700, &compact_config),
        "700 tokens should be below the threshold of 800"
    );

    // Build a simple conversation
    let messages = vec![
        Message::new(
            Role::User,
            vec![ContentBlock::Text {
                text: "Hello".to_string(),
            }],
        ),
        Message::new(
            Role::Assistant,
            vec![ContentBlock::Text {
                text: "Hi there!".to_string(),
            }],
        ),
        Message::new(
            Role::User,
            vec![ContentBlock::Text {
                text: "What is 2+2?".to_string(),
            }],
        ),
        Message::new(
            Role::Assistant,
            vec![ContentBlock::Text {
                text: "4".to_string(),
            }],
        ),
    ];

    // Create a real provider and run autocompact
    let provider = nomi_providers::create_provider(&config);

    let state = CompactState {
        last_input_tokens: 900, // above the threshold of 800
        ..CompactState::default()
    };
    // autocompact takes &mut state for recording success/failure
    let mut state = state;

    let result = autocompact(
        provider.as_ref(),
        &messages,
        &config.model,
        &compact_config,
        &mut state,
    )
    .await;

    let compact_result = result.expect("autocompact should succeed with a real LLM");

    // Verify the result messages contain the boundary prefix
    let has_boundary = compact_result.messages.iter().any(|msg| {
        msg.content.iter().any(|block| {
            if let ContentBlock::Text { text } = block {
                text.starts_with(BOUNDARY_PREFIX)
            } else {
                false
            }
        })
    });
    assert!(
        has_boundary,
        "autocompact result should contain a message with the boundary prefix"
    );

    // Verify metadata
    assert_eq!(compact_result.messages_summarized, messages.len());
    assert_eq!(compact_result.pre_compact_tokens, 900);
}

// ── TC-A2-03: Emergency truncation detection (LOCAL) ───────────────────────

/// Verify that is_at_emergency_limit correctly detects when the token count
/// is within the emergency buffer of the context window, and that it works
/// even when compact is disabled.
#[test]
fn emergency_truncation_detection() {
    let config = CompactConfig {
        context_window: 1000,
        emergency_buffer: 100,
        // limit = 1000 - 100 = 900
        ..CompactConfig::default()
    };

    // 950 >= 900 → true (at emergency limit)
    assert!(
        is_at_emergency_limit(950, &config),
        "950 tokens should be at the emergency limit (threshold = 900)"
    );

    // 800 < 900 → false (below emergency limit)
    assert!(
        !is_at_emergency_limit(800, &config),
        "800 tokens should be below the emergency limit (threshold = 900)"
    );

    // Verify emergency check works even when config.enabled = false
    let disabled_config = CompactConfig {
        context_window: 1000,
        emergency_buffer: 100,
        enabled: false,
        ..CompactConfig::default()
    };

    assert!(
        is_at_emergency_limit(950, &disabled_config),
        "emergency limit should apply even when compact is disabled"
    );
    assert!(
        !is_at_emergency_limit(800, &disabled_config),
        "below-limit should still return false when compact is disabled"
    );
}
