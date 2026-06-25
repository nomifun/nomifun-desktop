//! Black-box integration tests for the microcompact subsystem.
//!
//! These tests correspond to TC-2.3-01 through TC-2.3-11 in the test plan.
//! They treat `should_microcompact` and `microcompact` as opaque functions
//! and validate observable behaviour only (inputs → outputs).

use chrono::{Duration, Utc};
use nomi_agent::compact::micro::{
    CLEARED_TOOL_RESULT, MicrocompactResult, microcompact, should_microcompact,
};
use nomi_config::compact::CompactConfig;
use nomi_types::message::{ContentBlock, Message, Role};
use serde_json::json;

// ── Helpers ─────────────────────────────────────────────────────────────────

fn tool_use(id: &str, name: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: id.into(),
        name: name.into(),
        input: json!({}),
        extra: None,
    }
}

fn tool_result(id: &str, content: &str) -> ContentBlock {
    ContentBlock::ToolResult {
        tool_use_id: id.into(),
        content: content.into(),
        is_error: false,
        images: Vec::new(),
    }
}

fn text(s: &str) -> ContentBlock {
    ContentBlock::Text { text: s.into() }
}

fn assistant(blocks: Vec<ContentBlock>) -> Message {
    Message::new(Role::Assistant, blocks)
}

fn user(blocks: Vec<ContentBlock>) -> Message {
    Message::new(Role::User, blocks)
}

fn assistant_at(blocks: Vec<ContentBlock>, ts: chrono::DateTime<Utc>) -> Message {
    Message {
        role: Role::Assistant,
        content: blocks,
        timestamp: Some(ts),
    }
}

fn get_tool_result_content(msg: &Message, block_idx: usize) -> &str {
    match &msg.content[block_idx] {
        ContentBlock::ToolResult { content, .. } => content.as_str(),
        other => panic!("expected ToolResult, got {other:?}"),
    }
}

// ── TC-2.3-01: Basic clearing ───────────────────────────────────────────────

#[test]
fn tc_2_3_01_basic_clearing() {
    // 10 messages containing 8 tool results (Read x3, Bash x3, Grep x2).
    // keep_recent = 3 → oldest 5 cleared.
    let tool_specs = [
        ("r1", "Read"),
        ("b1", "Bash"),
        ("g1", "Grep"),
        ("r2", "Read"),
        ("b2", "Bash"),
        ("g2", "Grep"),
        ("r3", "Read"),
        ("b3", "Bash"),
    ];
    let mut msgs: Vec<Message> = Vec::new();
    for (id, name) in &tool_specs {
        msgs.push(assistant(vec![tool_use(id, name)]));
        msgs.push(user(vec![tool_result(id, &format!("output-{id}"))]));
    }

    let config = CompactConfig {
        micro_keep_recent: 3,
        ..Default::default()
    };

    let result = microcompact(&mut msgs, &config);
    assert_eq!(result.cleared_count, 5);

    // First 5 user messages (indices 1, 3, 5, 7, 9) are cleared.
    for i in 0..5 {
        let user_msg_idx = i * 2 + 1;
        assert_eq!(
            get_tool_result_content(&msgs[user_msg_idx], 0),
            CLEARED_TOOL_RESULT,
            "tool result at msg index {user_msg_idx} should be cleared"
        );
    }
    // Last 3 user messages (indices 11, 13, 15) retain original content.
    for (idx, &(id, _name)) in tool_specs.iter().enumerate().skip(5) {
        let user_msg_idx = idx * 2 + 1;
        assert_eq!(
            get_tool_result_content(&msgs[user_msg_idx], 0),
            format!("output-{id}"),
            "tool result at msg index {user_msg_idx} should be preserved"
        );
    }
}

// ── TC-2.3-02: Tool results insufficient — no clearing ──────────────────────

#[test]
fn tc_2_3_02_insufficient_results_no_clearing() {
    let mut msgs = vec![
        assistant(vec![tool_use("t1", "Read")]),
        user(vec![tool_result("t1", "data-1")]),
        assistant(vec![tool_use("t2", "Bash")]),
        user(vec![tool_result("t2", "data-2")]),
    ];
    let config = CompactConfig {
        micro_keep_recent: 5,
        ..Default::default()
    };

    let result = microcompact(&mut msgs, &config);
    assert_eq!(result.cleared_count, 0);
    assert_eq!(
        result,
        MicrocompactResult {
            cleared_count: 0,
            estimated_tokens_freed: 0,
        }
    );
}

// ── TC-2.3-03: Only compactable tools are cleared ───────────────────────────

#[test]
fn tc_2_3_03_only_compactable_tools_cleared() {
    let mut msgs = vec![
        assistant(vec![tool_use("t1", "Read")]),
        user(vec![tool_result("t1", "read-output")]),
        assistant(vec![tool_use("t2", "Bash")]),
        user(vec![tool_result("t2", "bash-output")]),
        assistant(vec![tool_use("t3", "Skill")]),
        user(vec![tool_result("t3", "skill-output")]),
        assistant(vec![tool_use("t4", "Read")]),
        user(vec![tool_result("t4", "read-output-2")]),
    ];

    // compactable_tools does NOT include "Skill".
    let config = CompactConfig {
        micro_keep_recent: 1,
        compactable_tools: vec!["Read".into(), "Bash".into()],
        ..Default::default()
    };

    let result = microcompact(&mut msgs, &config);
    // 3 compactable (t1-Read, t2-Bash, t4-Read), keep 1 → clear 2.
    assert_eq!(result.cleared_count, 2);

    // Skill result (t3) must be untouched.
    assert_eq!(get_tool_result_content(&msgs[5], 0), "skill-output");
    // Most recent compactable (t4) must be preserved.
    assert_eq!(get_tool_result_content(&msgs[7], 0), "read-output-2");
}

// ── TC-2.3-04: Time trigger — exceeds threshold ────────────────────────────

#[test]
fn tc_2_3_04_time_trigger_exceeds_threshold() {
    let old_ts = Utc::now() - Duration::seconds(3660); // 61 minutes ago
    let msgs = vec![assistant_at(vec![text("thinking")], old_ts)];
    let config = CompactConfig {
        micro_gap_seconds: 3600,
        ..Default::default()
    };
    assert!(should_microcompact(&msgs, &config));
}

// ── TC-2.3-05: Time trigger — within threshold ─────────────────────────────

#[test]
fn tc_2_3_05_time_trigger_within_threshold() {
    let recent_ts = Utc::now() - Duration::seconds(1800); // 30 minutes ago
    let msgs = vec![assistant_at(vec![text("thinking")], recent_ts)];
    let config = CompactConfig {
        micro_gap_seconds: 3600,
        ..Default::default()
    };
    assert!(!should_microcompact(&msgs, &config));
}

// ── TC-2.3-06: Count trigger ────────────────────────────────────────────────

#[test]
fn tc_2_3_06_count_trigger() {
    // 12 compactable results, keep_recent=5 → threshold = 10.
    // 12 > 10 → should trigger.
    let mut msgs = Vec::new();
    for i in 0..12 {
        let id = format!("t{i}");
        msgs.push(assistant(vec![tool_use(&id, "Read")]));
        msgs.push(user(vec![tool_result(&id, "data")]));
    }
    let config = CompactConfig {
        micro_keep_recent: 5,
        ..Default::default()
    };
    assert!(should_microcompact(&msgs, &config));
}

// ── TC-2.3-07: No timestamp — time check skipped ───────────────────────────

#[test]
fn tc_2_3_07_no_timestamp_skips_time_check() {
    // All messages have no timestamp.
    // Only 2 compactable results with keep_recent=5 → count trigger also false.
    let msgs = vec![
        assistant(vec![tool_use("t1", "Read")]),
        user(vec![tool_result("t1", "data-1")]),
        assistant(vec![tool_use("t2", "Read")]),
        user(vec![tool_result("t2", "data-2")]),
    ];
    let config = CompactConfig {
        micro_keep_recent: 5,
        micro_gap_seconds: 3600,
        ..Default::default()
    };
    // No timestamp → time trigger skipped.
    // 2 results ≤ 5*2=10 → count trigger false.
    assert!(!should_microcompact(&msgs, &config));
}

// ── TC-2.3-08: Token estimation after clearing ──────────────────────────────

#[test]
fn tc_2_3_08_token_estimation() {
    // 3 tool results with known content lengths, clear all but 1.
    let content_a = "x".repeat(200); // 50 tokens
    let content_b = "y".repeat(400); // 100 tokens
    let content_c = "z".repeat(80); // 20 tokens — kept
    let mut msgs = vec![
        assistant(vec![tool_use("a", "Read")]),
        user(vec![tool_result("a", &content_a)]),
        assistant(vec![tool_use("b", "Bash")]),
        user(vec![tool_result("b", &content_b)]),
        assistant(vec![tool_use("c", "Grep")]),
        user(vec![tool_result("c", &content_c)]),
    ];
    let config = CompactConfig {
        micro_keep_recent: 1,
        ..Default::default()
    };

    let result = microcompact(&mut msgs, &config);
    assert_eq!(result.cleared_count, 2);
    assert!(result.estimated_tokens_freed > 0);
    // 200/4 + 400/4 = 50 + 100 = 150
    assert_eq!(result.estimated_tokens_freed, 150);
}

// ── TC-2.3-09: Already cleared content not re-cleared ───────────────────────

#[test]
fn tc_2_3_09_already_cleared_not_recounted() {
    let mut msgs = vec![
        assistant(vec![tool_use("t1", "Read")]),
        user(vec![tool_result("t1", CLEARED_TOOL_RESULT)]),
        assistant(vec![tool_use("t2", "Read")]),
        user(vec![tool_result("t2", "live-data")]),
    ];
    let config = CompactConfig {
        micro_keep_recent: 1,
        ..Default::default()
    };

    let result = microcompact(&mut msgs, &config);
    // t1 already cleared, only t2 is compactable and is the most recent → keep.
    assert_eq!(result.cleared_count, 0);
    assert_eq!(result.estimated_tokens_freed, 0);
}

// ── TC-2.3-10: Empty message list ───────────────────────────────────────────

#[test]
fn tc_2_3_10_empty_messages() {
    let mut msgs: Vec<Message> = vec![];
    let result = microcompact(&mut msgs, &CompactConfig::default());
    assert_eq!(
        result,
        MicrocompactResult {
            cleared_count: 0,
            estimated_tokens_freed: 0,
        }
    );
}

// ── TC-2.3-11: Message order preserved ──────────────────────────────────────

#[test]
fn tc_2_3_11_message_order_preserved() {
    let mut msgs = vec![
        assistant(vec![tool_use("t1", "Read")]),
        user(vec![tool_result("t1", "data-1")]),
        assistant(vec![text("thinking about it")]),
        user(vec![text("please continue")]),
        assistant(vec![tool_use("t2", "Bash")]),
        user(vec![tool_result("t2", "bash-out")]),
        assistant(vec![tool_use("t3", "Grep")]),
        user(vec![tool_result("t3", "grep-out")]),
    ];
    let original_len = msgs.len();
    let original_roles: Vec<Role> = msgs.iter().map(|m| m.role).collect();

    let config = CompactConfig {
        micro_keep_recent: 1,
        ..Default::default()
    };
    microcompact(&mut msgs, &config);

    // Message count unchanged.
    assert_eq!(msgs.len(), original_len);
    // Role sequence unchanged.
    let after_roles: Vec<Role> = msgs.iter().map(|m| m.role).collect();
    assert_eq!(after_roles, original_roles);
    // Non-tool-result content blocks unchanged.
    match &msgs[2].content[0] {
        ContentBlock::Text { text } => assert_eq!(text, "thinking about it"),
        _ => panic!("expected Text"),
    }
    match &msgs[3].content[0] {
        ContentBlock::Text { text } => assert_eq!(text, "please continue"),
        _ => panic!("expected Text"),
    }
}
