// ---------------------------------------------------------------------------
// Phase 2 tests — run_compaction()
// ---------------------------------------------------------------------------

use super::MAX_PROVIDER_REQUEST_IMAGES;
use std::sync::{Arc, Mutex};

use super::USER_IMAGE_HISTORY_PLACEHOLDER;
use nomi_config::compact::CompactConfig;
use nomi_providers::{LlmProvider, ProviderError};
use nomi_tools::registry::ToolRegistry;
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, Message, Role, StopReason};
use serde_json::json;

use crate::compact::state::CompactState;
use crate::confirm::ToolConfirmer;
use crate::output::OutputSink;
use crate::output::null_sink::NullSink;
use crate::session::EditableTurnCheckpoint;

#[derive(Default)]
struct RecordingOutput {
    tool_results: Mutex<Vec<(String, String, bool, String)>>,
}

impl OutputSink for RecordingOutput {
    fn emit_text_delta(&self, _: &str, _: &str) {}
    fn emit_thinking(&self, _: &str, _: &str) {}
    fn emit_tool_call(&self, _: &str, _: &str, _: &str) {}
    fn emit_tool_result(&self, tool_use_id: &str, name: &str, is_error: bool, content: &str) {
        self.tool_results.lock().unwrap().push((
            tool_use_id.to_string(),
            name.to_string(),
            is_error,
            content.to_string(),
        ));
    }
    fn emit_stream_start(&self, _: &str) {}
    fn emit_output_discarded(&self, _: &str, _: u32) {}
    fn emit_stream_end(&self, _: &str, _: usize, _: u64, _: u64, _: u64, _: u64) {}
    fn emit_error(&self, _: &str) {}
    fn emit_info(&self, _: &str) {}
}

struct NullProvider;
#[async_trait::async_trait]
impl LlmProvider for NullProvider {
    async fn stream(
        &self,
        _: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Ok(rx)
    }
}

#[derive(Default)]
struct RecordingProvider {
    request_image_counts: Mutex<Vec<usize>>,
}

#[async_trait::async_trait]
impl LlmProvider for RecordingProvider {
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        self.request_image_counts
            .lock()
            .unwrap()
            .push(count_images(&request.messages));
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tx.try_send(LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
        })
        .unwrap();
        Ok(rx)
    }
}

fn make_compact_engine(
    compact_config: CompactConfig,
    compact_state: CompactState,
    messages: Vec<Message>,
) -> super::AgentEngine {
    make_compact_engine_with_output(
        compact_config,
        compact_state,
        messages,
        Arc::new(NullSink),
    )
}

fn make_compact_engine_with_output(
    compact_config: CompactConfig,
    compact_state: CompactState,
    messages: Vec<Message>,
    output: Arc<dyn OutputSink>,
) -> super::AgentEngine {
    super::AgentEngine {
        provider: Arc::new(NullProvider),
        tools: ToolRegistry::new(),
        messages,
        system_prompt: String::new(),
        model: "test-model".to_string(),
        output_max_tokens: Some(4096),
        max_turns: Some(10),
        total_usage: Default::default(),
        thinking: None,
        compat: nomi_config::compat::ProviderCompat::anthropic_defaults(),
        confirmer: Arc::new(Mutex::new(ToolConfirmer::new(true, vec![]))),
        hooks: None,
        session_manager: None,
        current_session: None,
        output,
        current_msg_id: String::new(),
        approval_manager: None,
        protocol_writer: None,
        allow_list: vec![],
        current_reasoning_effort: None,
        compact_config,
        compact_state,
        plan_state: Default::default(),
        plan_active_flag: None,
        cache_detector: super::CacheBreakDetector::new(),
        compaction_level: nomi_compact::CompactionLevel::default(),
        toon_enabled: false,
        max_recent_images: 3,
        commands: crate::commands::default_registry(),
        goal: None,
        stagnation_guard: crate::loop_guard::StagnationGuard::new(crate::engine::STAGNATION_THRESHOLD),
        context_contributors: Vec::new(),
        steering_inbox: None,
        system_resource_inbox: None,
        process_supervisor: None,
        editable_turn: None,
        host_context: Default::default(),
    }
}

fn tool_use_msg(id: &str, name: &str) -> Message {
    Message::new(
        Role::Assistant,
        vec![ContentBlock::ToolUse {
            id: id.to_string(),
            name: name.to_string(),
            input: json!({}),
            extra: None,
        }],
    )
}

fn tool_use_msg_with_two_calls(first_id: &str, second_id: &str) -> Message {
    Message::new(
        Role::Assistant,
        vec![
            ContentBlock::ToolUse {
                id: first_id.to_string(),
                name: "Read".to_string(),
                input: json!({}),
                extra: None,
            },
            ContentBlock::ToolUse {
                id: second_id.to_string(),
                name: "Bash".to_string(),
                input: json!({}),
                extra: None,
            },
        ],
    )
}

fn tool_result_msg(id: &str, content: &str) -> Message {
    Message::new(
        Role::User,
        vec![ContentBlock::ToolResult {
            tool_use_id: id.to_string(),
            content: content.to_string(),
            is_error: false,
            images: Vec::new(),
        }],
    )
}

fn tool_result_msg_with_image(id: &str) -> Message {
    tool_result_msg_with_image_data(id, "aGk=".to_string())
}

fn tool_result_msg_with_image_data(id: &str, data: String) -> Message {
    Message::new(
        Role::User,
        vec![ContentBlock::ToolResult {
            tool_use_id: id.to_string(),
            content: "screenshot".to_string(),
            is_error: false,
            images: vec![nomi_types::tool::ToolImage {
                media_type: "image/png".to_string(),
                data,
            }],
        }],
    )
}

fn count_images(messages: &[Message]) -> usize {
    messages
        .iter()
        .flat_map(|m| &m.content)
        .map(|b| match b {
            ContentBlock::ToolResult { images, .. } => images.len(),
            _ => 0,
        })
        .sum()
}

#[test]
fn prune_old_tool_images_keeps_most_recent() {
    let mut engine = make_compact_engine(
        CompactConfig::default(),
        CompactState::new(),
        (0..5).map(|i| tool_result_msg_with_image(&format!("call_{i}"))).collect(),
    );
    engine.messages[0].provider_round_id = Some("resp_before_prune".to_owned());
    engine.max_recent_images = 3;
    engine.prune_old_tool_images();
    assert_eq!(count_images(&engine.messages), 3);
    assert!(
        engine.messages.iter().all(|message| message.provider_round_id.is_none()),
        "rewriting any provider-visible history invalidates every retained round id"
    );
    // The two oldest lost their images; text content survives.
    for (i, msg) in engine.messages.iter().enumerate() {
        if let ContentBlock::ToolResult { images, content, .. } = &msg.content[0] {
            assert_eq!(images.is_empty(), i < 2, "msg {i}");
            assert!(content.starts_with("screenshot"));
            assert_eq!(content.contains("attachment(s)"), i < 2);
        }
    }
}

#[test]
fn prune_old_tool_images_noop_under_limit() {
    let mut engine = make_compact_engine(
        CompactConfig::default(),
        CompactState::new(),
        vec![tool_result_msg_with_image("call_0")],
    );
    engine.messages[0].provider_round_id = Some("resp_preserved".to_owned());
    engine.max_recent_images = 3;
    engine.prune_old_tool_images();
    assert_eq!(count_images(&engine.messages), 1);
    assert_eq!(
        engine.messages[0].provider_round_id.as_deref(),
        Some("resp_preserved"),
        "a byte-identical no-op must not break the response chain"
    );
}

#[test]
fn prune_old_tool_images_counts_images_inside_one_result() {
    let mut message = tool_result_msg_with_image("batch");
    let ContentBlock::ToolResult { images, .. } = &mut message.content[0] else {
        unreachable!();
    };
    let image = images[0].clone();
    images.resize(25, image);
    let mut engine = make_compact_engine(
        CompactConfig::default(),
        CompactState::new(),
        vec![message],
    );
    engine.max_recent_images = 100;

    engine.prune_old_tool_images();

    assert_eq!(count_images(&engine.messages), MAX_PROVIDER_REQUEST_IMAGES);
    let ContentBlock::ToolResult { content, .. } = &engine.messages[0].content[0] else {
        unreachable!();
    };
    assert!(content.contains("5 later attachment(s) were omitted"));
}

#[test]
fn prune_old_tool_images_enforces_cumulative_base64_budget() {
    let image_data_len = 3 * 1024 * 1024;
    let mut engine = make_compact_engine(
        CompactConfig::default(),
        CompactState::new(),
        (0..3)
            .map(|i| tool_result_msg_with_image_data(&format!("call_{i}"), "A".repeat(image_data_len)))
            .collect(),
    );
    engine.max_recent_images = 3;

    engine.prune_old_tool_images();

    assert_eq!(count_images(&engine.messages), 2);
    for (index, message) in engine.messages.iter().enumerate() {
        let ContentBlock::ToolResult { images, content, .. } = &message.content[0] else {
            unreachable!();
        };
        assert_eq!(images.is_empty(), index == 0, "message {index}");
        assert_eq!(content.contains("payload budget"), index == 0);
    }
}

#[test]
fn prune_old_tool_images_drops_individually_oversized_legacy_image() {
    let padded_five_mib = (5usize * 1024 * 1024).div_ceil(3) * 4;
    let mut engine = make_compact_engine(
        CompactConfig::default(),
        CompactState::new(),
        vec![tool_result_msg_with_image_data(
            "oversized",
            "A".repeat(padded_five_mib + 4),
        )],
    );

    engine.prune_old_tool_images();

    assert_eq!(count_images(&engine.messages), 0);
    let ContentBlock::ToolResult { content, .. } = &engine.messages[0].content[0] else {
        unreachable!();
    };
    assert_eq!(content.matches("payload budget").count(), 1);
}

#[tokio::test]
async fn first_request_prunes_images_from_preloaded_or_resumed_history() {
    let mut message = tool_result_msg_with_image("legacy-batch");
    let ContentBlock::ToolResult { images, .. } = &mut message.content[0] else {
        unreachable!();
    };
    let image = images[0].clone();
    images.resize(25, image);

    let provider = Arc::new(RecordingProvider::default());
    let mut engine = make_compact_engine(
        CompactConfig::default(),
        CompactState::new(),
        vec![message],
    );
    engine.provider = provider.clone();
    engine.max_recent_images = 100;

    engine
        .execute_turn("continue", "resume-image-limit")
        .await
        .unwrap();

    assert_eq!(
        *provider.request_image_counts.lock().unwrap(),
        vec![MAX_PROVIDER_REQUEST_IMAGES]
    );
    assert_eq!(count_images(&engine.messages), MAX_PROVIDER_REQUEST_IMAGES);
}

#[test]
fn abort_current_turn_closes_pending_tool_uses() {
    let output = Arc::new(RecordingOutput::default());
    let mut engine = make_compact_engine_with_output(
        CompactConfig::default(),
        CompactState::new(),
        vec![
            Message::new(
                Role::User,
                vec![ContentBlock::Text {
                    text: "run tools".to_string(),
                }],
            ),
            tool_use_msg_with_two_calls("call_read", "call_bash"),
        ],
        output.clone(),
    );

    engine.abort_current_turn("Tool execution canceled by user");

    let last = engine.messages.last().expect("synthetic result message");
    assert_eq!(last.role, Role::User);
    assert_eq!(last.content.len(), 2);
    assert!(
        matches!(&last.content[0], ContentBlock::ToolResult { tool_use_id, content, is_error, .. }
            if tool_use_id == "call_read" && content == "Tool execution canceled by user" && *is_error)
    );
    assert!(
        matches!(&last.content[1], ContentBlock::ToolResult { tool_use_id, content, is_error, .. }
            if tool_use_id == "call_bash" && content == "Tool execution canceled by user" && *is_error)
    );

    let emitted = output.tool_results.lock().unwrap();
    assert_eq!(emitted.len(), 2);
    assert_eq!(
        emitted[0],
        (
            "call_read".into(),
            "Read".into(),
            true,
            "Tool execution canceled by user".into()
        )
    );
    assert_eq!(
        emitted[1],
        (
            "call_bash".into(),
            "Bash".into(),
            true,
            "Tool execution canceled by user".into()
        )
    );
}

#[test]
fn abort_current_turn_redacts_an_image_before_any_assistant_response() {
    let mut engine = make_compact_engine(
        CompactConfig::default(),
        CompactState::new(),
        vec![Message::new(
            Role::User,
            vec![
                ContentBlock::Text {
                    text: "inspect this".to_string(),
                },
                ContentBlock::Image {
                    media_type: "image/png".to_string(),
                    data: "large-base64-payload".to_string(),
                },
            ],
        )],
    );
    engine.editable_turn = Some(EditableTurnCheckpoint {
        source_message_id: "message-image".into(),
        start_len: 0,
        prior_host_context: Default::default(),
    });

    engine.abort_current_turn("Canceled by user");

    assert_eq!(engine.messages.len(), 1);
    assert!(engine.messages[0]
        .content
        .iter()
        .all(|block| !matches!(block, ContentBlock::Image { .. })));
    assert!(engine.messages[0].content.iter().any(|block| {
        matches!(block, ContentBlock::Text { text } if text == USER_IMAGE_HISTORY_PLACEHOLDER)
    }));
}

// -- Emergency check fires when at limit --

#[tokio::test]
async fn emergency_fires_when_at_limit() {
    let config = CompactConfig {
        context_window: 200_000,
        emergency_buffer: 3_000,
        ..Default::default()
    };
    let mut state = CompactState::new();
    state.last_input_tokens = 198_000; // >= 197k limit

    let mut engine = make_compact_engine(config, state, vec![]);
    let result = engine.run_compaction().await;

    match result {
        Err(super::AgentError::ContextTooLong {
            input_tokens,
            limit,
        }) => {
            assert_eq!(input_tokens, 198_000);
            assert_eq!(limit, 197_000);
        }
        other => panic!("expected ContextTooLong, got: {:?}", other),
    }
}

// -- Emergency does not fire when below limit --

#[tokio::test]
async fn emergency_silent_below_limit() {
    let config = CompactConfig::default();
    let mut state = CompactState::new();
    state.last_input_tokens = 190_000; // below 197k

    let mut engine = make_compact_engine(config, state, vec![]);
    assert!(engine.run_compaction().await.is_ok());
}

// -- Microcompact runs when count trigger fires --

#[tokio::test]
async fn microcompact_clears_old_results() {
    // 12 tool results with keep_recent=3 (threshold=6) → should clear 9
    let mut messages = Vec::new();
    for i in 0..12 {
        let id = format!("t{i}");
        messages.push(tool_use_msg(&id, "Read"));
        messages.push(tool_result_msg(&id, &format!("data-{i}")));
    }

    let config = CompactConfig {
        micro_keep_recent: 3,
        ..Default::default()
    };
    let state = CompactState::new();

    let mut engine = make_compact_engine(config, state, messages);
    engine.messages[0].provider_round_id = Some("resp_before_microcompact".to_owned());
    engine.run_compaction().await.unwrap();

    // Last 3 tool results should be preserved
    let cleared_count = engine
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .filter(|b| {
            matches!(b, ContentBlock::ToolResult { content, .. } if content == "[Tool result cleared]")
        })
        .count();

    assert_eq!(cleared_count, 9);
    assert!(
        engine.messages.iter().all(|message| message.provider_round_id.is_none()),
        "microcompact rewrites the full provider snapshot"
    );
}

// -- Disabled config skips micro and auto but not emergency --

#[tokio::test]
async fn disabled_config_skips_micro_auto() {
    let mut messages = Vec::new();
    for i in 0..12 {
        let id = format!("t{i}");
        messages.push(tool_use_msg(&id, "Read"));
        messages.push(tool_result_msg(&id, &format!("data-{i}")));
    }

    let config = CompactConfig {
        enabled: false,
        micro_keep_recent: 3,
        ..Default::default()
    };
    let state = CompactState::new();

    let mut engine = make_compact_engine(config, state, messages);
    engine.run_compaction().await.unwrap();

    // Nothing should be cleared (microcompact skipped)
    let cleared_count = engine
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .filter(|b| {
            matches!(b, ContentBlock::ToolResult { content, .. } if content == "[Tool result cleared]")
        })
        .count();

    assert_eq!(
        cleared_count, 0,
        "microcompact should be skipped when disabled"
    );
}

#[tokio::test]
async fn disabled_config_still_fires_emergency() {
    let config = CompactConfig {
        enabled: false,
        context_window: 200_000,
        emergency_buffer: 3_000,
        ..Default::default()
    };
    let mut state = CompactState::new();
    state.last_input_tokens = 198_000;

    let mut engine = make_compact_engine(config, state, vec![]);
    let result = engine.run_compaction().await;

    assert!(
        matches!(result, Err(super::AgentError::ContextTooLong { .. })),
        "emergency should fire even when disabled"
    );
}

// -- Zero tokens on first turn does not trigger anything --

#[tokio::test]
async fn first_turn_zero_tokens_no_compaction() {
    let config = CompactConfig::default();
    let state = CompactState::new(); // last_input_tokens = 0

    let mut engine = make_compact_engine(config, state, vec![]);
    assert!(engine.run_compaction().await.is_ok());
    assert_eq!(engine.compact_state.last_input_tokens, 0);
}

// -- Circuit broken prevents autocompact, emergency still fires --

#[tokio::test]
async fn circuit_broken_skips_auto_but_emergency_fires() {
    let config = CompactConfig {
        context_window: 200_000,
        emergency_buffer: 3_000,
        max_failures: 3,
        ..Default::default()
    };
    let mut state = CompactState::new();
    state.last_input_tokens = 198_000; // triggers both auto and emergency
    state.consecutive_failures = 3; // circuit broken

    let mut engine = make_compact_engine(config, state, vec![]);
    let result = engine.run_compaction().await;

    // Auto is skipped due to circuit breaker; emergency fires
    assert!(matches!(
        result,
        Err(super::AgentError::ContextTooLong { .. })
    ));
}
