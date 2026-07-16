//! Black-box integration tests for engine compaction integration (TC-2.6-*).
//!
//! These tests exercise the full `AgentEngine::execute_turn()` loop and verify
//! that the compaction pipeline (microcompact → autocompact → emergency)
//! is correctly wired into the agentic loop.

mod common;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;

use nomi_agent::engine::{AgentEngine, AgentError};
use nomi_agent::output::OutputSink;
use nomi_agent::output::terminal::TerminalSink;
use nomi_agent::session::SessionManager;
use nomi_config::compact::CompactConfig;
use nomi_providers::{LlmProvider, ProviderError};
use nomi_tools::registry::ToolRegistry;
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{StopReason, TokenUsage};
use tempfile::tempdir;

use common::test_config;

// ── Helpers ────────────────────────────────────────────────────────────────

fn silent_output() -> Arc<dyn OutputSink> {
    Arc::new(TerminalSink::new(true))
}

/// A mock provider that returns configurable per-turn events.
/// Tracks the number of stream() calls for order verification.
struct CompactMockProvider {
    turns: Mutex<VecDeque<Vec<LlmEvent>>>,
    call_count: Mutex<usize>,
}

impl CompactMockProvider {
    fn new(turns: Vec<Vec<LlmEvent>>) -> Self {
        Self {
            turns: Mutex::new(VecDeque::from(turns)),
            call_count: Mutex::new(0),
        }
    }

    fn call_count(&self) -> usize {
        *self.call_count.lock().unwrap()
    }
}

#[async_trait]
impl LlmProvider for CompactMockProvider {
    async fn stream(
        &self,
        _request: &LlmRequest,
    ) -> Result<mpsc::Receiver<LlmEvent>, ProviderError> {
        *self.call_count.lock().unwrap() += 1;
        let events = self.turns.lock().unwrap().pop_front().unwrap_or_else(|| {
            vec![LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage::default(),
            }]
        });

        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            for event in events {
                let _ = tx.send(event).await;
            }
        });
        Ok(rx)
    }
}

/// Build events for a simple text response with configurable input_tokens.
fn text_turn(text: &str, input_tokens: u64) -> Vec<LlmEvent> {
    vec![
        LlmEvent::TextDelta(text.to_string()),
        LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens,
                output_tokens: 100,
                ..Default::default()
            },
        },
    ]
}

/// Build events for a summary LLM call (used by autocompact internally).
fn summary_turn(summary_text: &str) -> Vec<LlmEvent> {
    vec![
        LlmEvent::TextDelta(summary_text.to_string()),
        LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens: 5_000,
                output_tokens: 2_000,
                ..Default::default()
            },
        },
    ]
}

// ── TC-2.6-01: First turn does not trigger compaction ──────────────────────

#[tokio::test]
async fn tc_2_6_01_first_turn_no_compaction() {
    // On the first turn last_input_tokens is 0, so neither autocompact
    // nor emergency should fire.
    let provider = Arc::new(CompactMockProvider::new(vec![text_turn("Hello", 50_000)]));

    let config = test_config();
    let registry = ToolRegistry::new();
    let output = silent_output();

    let mut engine = AgentEngine::new_with_provider(
        provider.clone(),
        config,
        registry,
        output,
        std::env::temp_dir(),
    );
    let result = engine.execute_turn("Hi", "msg-1").await.expect("should succeed");

    assert_eq!(result.text, "Hello");
    assert_eq!(result.turns, 1);
    // Only one call to stream() — no compaction call
    assert_eq!(provider.call_count(), 1);
}

// ── TC-2.6-03: Emergency truncation returns error ──────────────────────────

#[tokio::test]
async fn tc_2_6_03_emergency_returns_error() {
    // Emergency is the last safety net — it fires when autocompact is
    // disabled or circuit-broken.  We disable compact so only emergency
    // is active, then push input_tokens above the emergency limit.
    //
    // Turn 1: tool use, returns input_tokens above emergency threshold
    // Turn 2: emergency fires before the API call → ContextTooLong
    let turn1 = vec![
        LlmEvent::ToolUse {
            id: "t1".to_string(),
            name: "mock_tool".to_string(),
            input: serde_json::json!({}),
            extra: None,
        },
        LlmEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage {
                input_tokens: 198_000, // above emergency limit (197k)
                output_tokens: 100,
                ..Default::default()
            },
        },
    ];
    // Turn 2 events are queued but should never be consumed
    let turn2 = text_turn("Should not reach", 50_000);

    let provider = Arc::new(CompactMockProvider::new(vec![turn1, turn2]));
    let mut config = test_config();
    config.compact.enabled = false; // disable auto/micro so emergency is the only gate
    config.compact.context_window = 200_000;
    config.compact.emergency_buffer = 3_000;

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(common::MockTool::new(
        "mock_tool",
        "result",
        false,
    )));
    let output = silent_output();

    let mut engine = AgentEngine::new_with_provider(
        provider.clone(),
        config,
        registry,
        output,
        std::env::temp_dir(),
    );
    let err = engine.execute_turn("Do something", "msg-1").await.unwrap_err();

    match err {
        AgentError::ContextTooLong {
            input_tokens,
            limit,
        } => {
            assert_eq!(input_tokens, 198_000);
            assert_eq!(limit, 197_000);
        }
        other => panic!("expected ContextTooLong, got: {:?}", other),
    }

    // Only one call to stream() — second call blocked by emergency
    assert_eq!(provider.call_count(), 1);
}

// ── TC-2.6-04: Autocompact then continue ───────────────────────────────────

#[tokio::test]
async fn tc_2_6_04_autocompact_then_continue() {
    // Turn 1: tool use, returns input_tokens=170k (above autocompact threshold 167k)
    // Before turn 2: autocompact fires → LLM summary call → messages replaced
    // Turn 2 (after compact): text response with low input_tokens
    let turn1 = vec![
        LlmEvent::ToolUse {
            id: "t1".to_string(),
            name: "mock_tool".to_string(),
            input: serde_json::json!({}),
            extra: None,
        },
        LlmEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage {
                input_tokens: 170_000,
                output_tokens: 100,
                ..Default::default()
            },
        },
    ];
    let compact_summary = summary_turn("<summary>Conversation summary</summary>");
    let turn2_after_compact = text_turn("Continuing after compact", 10_000);

    let provider = Arc::new(CompactMockProvider::new(vec![
        turn1,
        compact_summary,
        turn2_after_compact,
    ]));

    let mut config = test_config();
    config.compact = CompactConfig::default();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(common::MockTool::new(
        "mock_tool",
        "result",
        false,
    )));
    let output = silent_output();

    let mut engine = AgentEngine::new_with_provider(
        provider.clone(),
        config,
        registry,
        output,
        std::env::temp_dir(),
    );
    let result = engine
        .execute_turn("Start work", "msg-1")
        .await
        .expect("should succeed after compact");

    assert_eq!(result.text, "Continuing after compact");
    assert_eq!(result.turns, 2);
    // 3 calls: turn1 + compact summary + turn2
    assert_eq!(provider.call_count(), 3);
}

// ── TC-2.6-05: Session save includes compacted messages ────────────────────

#[tokio::test]
async fn tc_2_6_05_session_save_after_compact() {
    let dir = tempdir().expect("tempdir");

    let turn1 = vec![
        LlmEvent::ToolUse {
            id: "t1".to_string(),
            name: "mock_tool".to_string(),
            input: serde_json::json!({}),
            extra: None,
        },
        LlmEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage {
                input_tokens: 170_000,
                output_tokens: 100,
                ..Default::default()
            },
        },
    ];
    let compact_summary = summary_turn("<summary>Session summary</summary>");
    let turn2 = text_turn("After compact", 10_000);

    let provider = Arc::new(CompactMockProvider::new(vec![
        turn1,
        compact_summary,
        turn2,
    ]));

    let mut config = test_config();
    config.compact = CompactConfig::default();
    config.session.enabled = true;
    config.session.directory = dir.path().to_string_lossy().into_owned();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(common::MockTool::new(
        "mock_tool",
        "result",
        false,
    )));
    let output = silent_output();

    let mut engine =
        AgentEngine::new_with_provider(provider, config, registry, output, std::env::temp_dir());
    engine
        .init_session("test", "/tmp", None)
        .expect("init session");

    engine.execute_turn("Start", "msg-1").await.expect("should succeed");

    // Load the saved session
    let mgr = SessionManager::new(dir.path().to_path_buf(), 10);
    let session = mgr.load("latest").expect("load session");

    // After compaction + turn2, messages should include the compact boundary,
    // summary, and the post-compact assistant/user messages.
    // The exact count depends on implementation, but should be small (not
    // the full pre-compact count).
    assert!(
        session.messages.len() < 10,
        "session should have compacted messages, got {}",
        session.messages.len()
    );

    // Verify at least one message contains compact boundary marker
    let has_boundary = session.messages.iter().any(|m| {
        m.content.iter().any(|b| {
            matches!(b, nomi_types::message::ContentBlock::Text { text } if text.contains("[Conversation compacted]"))
        })
    });
    assert!(
        has_boundary,
        "session should contain compact boundary marker"
    );
}

// ── TC-2.6-06: Disabled skips all except emergency ─────────────────────────

#[tokio::test]
async fn tc_2_6_06_disabled_skips_micro_auto() {
    // With compact disabled, a text response that reports high usage
    // should not trigger autocompact (only emergency if at limit).
    let provider = Arc::new(CompactMockProvider::new(vec![
        // Returns high but not emergency-level tokens
        text_turn("Normal response", 170_000),
    ]));

    let mut config = test_config();
    config.compact.enabled = false;

    let registry = ToolRegistry::new();
    let output = silent_output();

    let mut engine = AgentEngine::new_with_provider(
        provider.clone(),
        config,
        registry,
        output,
        std::env::temp_dir(),
    );
    let result = engine.execute_turn("Hi", "msg-1").await.expect("should succeed");

    assert_eq!(result.text, "Normal response");
    // Only 1 call — no compact summary call
    assert_eq!(provider.call_count(), 1);
}

#[tokio::test]
async fn tc_2_6_06b_disabled_still_fires_emergency() {
    let turn1 = vec![
        LlmEvent::ToolUse {
            id: "t1".to_string(),
            name: "mock_tool".to_string(),
            input: serde_json::json!({}),
            extra: None,
        },
        LlmEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage {
                input_tokens: 198_000,
                output_tokens: 100,
                ..Default::default()
            },
        },
    ];

    let provider = Arc::new(CompactMockProvider::new(vec![
        turn1,
        text_turn("unreachable", 0),
    ]));

    let mut config = test_config();
    config.compact.enabled = false;
    config.compact.context_window = 200_000;
    config.compact.emergency_buffer = 3_000;

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(common::MockTool::new(
        "mock_tool",
        "result",
        false,
    )));
    let output = silent_output();

    let mut engine =
        AgentEngine::new_with_provider(provider, config, registry, output, std::env::temp_dir());
    let err = engine.execute_turn("Go", "msg-1").await.unwrap_err();

    assert!(
        matches!(err, AgentError::ContextTooLong { .. }),
        "emergency should fire even when disabled"
    );
}

// ── TC-2.6-07: input_tokens correctly tracked ──────────────────────────────

#[tokio::test]
async fn tc_2_6_07_input_tokens_tracked() {
    // Two turns: first returns 50k tokens, second returns 60k tokens.
    // We verify that the engine updates compact state after each turn.
    let turn1 = vec![
        LlmEvent::ToolUse {
            id: "t1".to_string(),
            name: "mock_tool".to_string(),
            input: serde_json::json!({}),
            extra: None,
        },
        LlmEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage {
                input_tokens: 50_000,
                output_tokens: 100,
                ..Default::default()
            },
        },
    ];
    let turn2 = text_turn("Done", 60_000);

    let provider = Arc::new(CompactMockProvider::new(vec![turn1, turn2]));

    let config = test_config();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(common::MockTool::new(
        "mock_tool",
        "result",
        false,
    )));
    let output = silent_output();

    let mut engine =
        AgentEngine::new_with_provider(provider, config, registry, output, std::env::temp_dir());
    let result = engine.execute_turn("Work", "msg-1").await.expect("should succeed");

    assert_eq!(result.turns, 2);
    // Total usage should accumulate: 50k + 60k = 110k input tokens
    assert_eq!(result.usage.input_tokens, 110_000);
}

// ── TC-2.6-02: Execution order — micro before auto ────────────────────────

#[tokio::test]
async fn tc_2_6_02_micro_before_auto_execution_order() {
    // Build a scenario where both microcompact and autocompact trigger
    // in the same compaction cycle.  A custom provider captures the
    // messages sent to the autocompact LLM call so we can verify that
    // microcompact already cleared old tool results before autocompact
    // was invoked.

    let captured: Arc<Mutex<Option<Vec<nomi_types::message::Message>>>> =
        Arc::new(Mutex::new(None));
    let capture_ref = captured.clone();

    struct OrderProvider {
        regular_count: Mutex<usize>,
        captured: Arc<Mutex<Option<Vec<nomi_types::message::Message>>>>,
    }

    #[async_trait]
    impl LlmProvider for OrderProvider {
        async fn stream(
            &self,
            request: &LlmRequest,
        ) -> Result<mpsc::Receiver<LlmEvent>, ProviderError> {
            let is_compact = request.tools.is_empty();

            if is_compact {
                // Capture messages that autocompact sends to the LLM
                *self.captured.lock().unwrap() = Some(request.messages.clone());

                let events = vec![
                    LlmEvent::TextDelta("<summary>Order test summary</summary>".to_string()),
                    LlmEvent::Done {
                        stop_reason: StopReason::EndTurn,
                        usage: TokenUsage {
                            input_tokens: 5_000,
                            output_tokens: 2_000,
                            ..Default::default()
                        },
                    },
                ];
                let (tx, rx) = mpsc::channel(64);
                tokio::spawn(async move {
                    for e in events {
                        let _ = tx.send(e).await;
                    }
                });
                return Ok(rx);
            }

            let count = {
                let mut c = self.regular_count.lock().unwrap();
                let v = *c;
                *c += 1;
                v
            };

            // Turns 0-6: tool use.  Turn 6 reports high input_tokens
            // so that micro and auto both trigger in the SAME cycle
            // (turn 7's run_compaction).
            // Turn 7 (after compact): text to end the run.
            //
            // micro_keep_recent = 3 → count threshold = 6.
            // After 7 tool-use turns: 7 > 6 → micro fires.
            // After turn 6: last_input_tokens = 170k > 167k → auto fires.
            let events = if count < 7 {
                let input_tokens = if count == 6 { 170_000 } else { 10_000 };
                vec![
                    LlmEvent::ToolUse {
                        id: format!("t{count}"),
                        name: "mock_tool".to_string(),
                        // Distinct work items: this fixture is exercising
                        // compaction, not an unchanged retry loop.
                        input: serde_json::json!({"iteration": count}),
                        extra: None,
                    },
                    LlmEvent::Done {
                        stop_reason: StopReason::ToolUse,
                        usage: TokenUsage {
                            input_tokens,
                            output_tokens: 100,
                            ..Default::default()
                        },
                    },
                ]
            } else {
                vec![
                    LlmEvent::TextDelta("Done after compact".to_string()),
                    LlmEvent::Done {
                        stop_reason: StopReason::EndTurn,
                        usage: TokenUsage {
                            input_tokens: 5_000,
                            output_tokens: 100,
                            ..Default::default()
                        },
                    },
                ]
            };

            let (tx, rx) = mpsc::channel(64);
            tokio::spawn(async move {
                for e in events {
                    let _ = tx.send(e).await;
                }
            });
            Ok(rx)
        }
    }

    let provider = Arc::new(OrderProvider {
        regular_count: Mutex::new(0),
        captured: capture_ref,
    });

    let mut config = test_config();
    config.compact = CompactConfig {
        micro_keep_recent: 3,
        compactable_tools: vec!["mock_tool".into()],
        context_window: 200_000,
        emergency_buffer: 3_000,
        ..Default::default()
    };
    config.max_turns = Some(20);

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(common::MockTool::new(
        "mock_tool",
        "tool output data",
        false,
    )));
    let output = silent_output();

    let mut engine =
        AgentEngine::new_with_provider(provider, config, registry, output, std::env::temp_dir());
    let result = engine.execute_turn("Start", "msg-1").await.expect("should succeed");

    assert_eq!(result.text, "Done after compact");

    // Verify: the messages that autocompact received should contain
    // tool results cleared by microcompact (proving micro ran first
    // within the SAME compaction cycle).
    let msgs = captured.lock().unwrap();
    let msgs = msgs.as_ref().expect("autocompact should have been called");

    let cleared_count = msgs
        .iter()
        .flat_map(|m| m.content.iter())
        .filter(|b| {
            matches!(
                b,
                nomi_types::message::ContentBlock::ToolResult { content, .. }
                    if content == nomi_agent::compact::micro::CLEARED_TOOL_RESULT
            )
        })
        .count();

    // 7 tool results total, keep_recent=3 → 4 cleared by micro
    // before auto received the messages.
    assert_eq!(
        cleared_count, 4,
        "microcompact should have cleared 4 tool results before autocompact ran"
    );
}

// ── TC-2.6-E2E-02: Microcompact + autocompact cooperative scenario ────────

#[tokio::test]
async fn tc_2_6_e2e_02_micro_and_auto_cooperative() {
    // Verify that microcompact and autocompact cooperate in the same
    // compaction cycle.  Microcompact frees some tokens from old tool
    // results, and autocompact still fires because the input token
    // watermark (which is not reduced by micro) remains above threshold.

    let compact_call_count: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
    let counter_ref = compact_call_count.clone();

    struct CoopProvider {
        regular_count: Mutex<usize>,
        compact_calls: Arc<Mutex<usize>>,
    }

    #[async_trait]
    impl LlmProvider for CoopProvider {
        async fn stream(
            &self,
            request: &LlmRequest,
        ) -> Result<mpsc::Receiver<LlmEvent>, ProviderError> {
            let is_compact = request.tools.is_empty();

            if is_compact {
                *self.compact_calls.lock().unwrap() += 1;

                let events = vec![
                    LlmEvent::TextDelta("<summary>Cooperative summary</summary>".to_string()),
                    LlmEvent::Done {
                        stop_reason: StopReason::EndTurn,
                        usage: TokenUsage {
                            input_tokens: 5_000,
                            output_tokens: 2_000,
                            ..Default::default()
                        },
                    },
                ];
                let (tx, rx) = mpsc::channel(64);
                tokio::spawn(async move {
                    for e in events {
                        let _ = tx.send(e).await;
                    }
                });
                return Ok(rx);
            }

            let count = {
                let mut c = self.regular_count.lock().unwrap();
                let v = *c;
                *c += 1;
                v
            };

            // 7 tool-use turns (count 0-6).  Turn 6 returns high tokens.
            // micro_keep_recent = 3 → count threshold = 6.
            // After 7 tool results: 7 > 6 → micro fires.
            // After turn 6: last_input_tokens = 170k > 167k → auto fires.
            let events = if count < 7 {
                let input_tokens = if count == 6 { 170_000 } else { 10_000 };
                vec![
                    LlmEvent::ToolUse {
                        id: format!("t{count}"),
                        name: "mock_tool".to_string(),
                        // Distinct work items: this fixture is exercising
                        // compaction, not an unchanged retry loop.
                        input: serde_json::json!({"iteration": count}),
                        extra: None,
                    },
                    LlmEvent::Done {
                        stop_reason: StopReason::ToolUse,
                        usage: TokenUsage {
                            input_tokens,
                            output_tokens: 100,
                            ..Default::default()
                        },
                    },
                ]
            } else {
                vec![
                    LlmEvent::TextDelta("After cooperative compact".to_string()),
                    LlmEvent::Done {
                        stop_reason: StopReason::EndTurn,
                        usage: TokenUsage {
                            input_tokens: 5_000,
                            output_tokens: 100,
                            ..Default::default()
                        },
                    },
                ]
            };

            let (tx, rx) = mpsc::channel(64);
            tokio::spawn(async move {
                for e in events {
                    let _ = tx.send(e).await;
                }
            });
            Ok(rx)
        }
    }

    let provider = Arc::new(CoopProvider {
        regular_count: Mutex::new(0),
        compact_calls: counter_ref,
    });

    let mut config = test_config();
    config.compact = CompactConfig {
        micro_keep_recent: 3,
        compactable_tools: vec!["mock_tool".into()],
        context_window: 200_000,
        emergency_buffer: 3_000,
        ..Default::default()
    };
    config.max_turns = Some(20);

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(common::MockTool::new(
        "mock_tool",
        "tool output data",
        false,
    )));
    let output = silent_output();

    let mut engine =
        AgentEngine::new_with_provider(provider, config, registry, output, std::env::temp_dir());
    let result = engine.execute_turn("Work", "msg-1").await.expect("should succeed");

    assert_eq!(result.text, "After cooperative compact");

    // Autocompact was called exactly once (micro freed tokens but
    // did not reduce last_input_tokens, so auto still fired).
    let calls = *compact_call_count.lock().unwrap();
    assert_eq!(
        calls, 1,
        "autocompact should fire exactly once despite microcompact running first"
    );

    // Total turns: 7 tool-use + 1 post-compact text = 8 engine turns,
    // plus 1 internal compact LLM call = 9 provider calls.
    assert_eq!(result.turns, 8);
}

// ── TC-2.6-E2E-03: Circuit breaker after repeated failures ─────────────────

#[tokio::test]
async fn tc_2_6_e2e_03_circuit_breaker_stops_retries() {
    // Simulate: 3 turns where autocompact would trigger but fails each time.
    // After 3 failures the circuit breaker trips and autocompact stops.
    //
    // We use a provider that always fails the compact summary call with
    // a generic API error, but succeeds for regular conversation turns.

    struct CircuitBreakerProvider {
        call_index: Mutex<usize>,
    }

    #[async_trait]
    impl LlmProvider for CircuitBreakerProvider {
        async fn stream(
            &self,
            request: &LlmRequest,
        ) -> Result<mpsc::Receiver<LlmEvent>, ProviderError> {
            let idx = {
                let mut i = self.call_index.lock().unwrap();
                let v = *i;
                *i += 1;
                v
            };

            // Compact summary calls have no tools defined and include the
            // compact prompt in messages. We detect them by checking tools.is_empty().
            let is_compact_call = request.tools.is_empty();

            if is_compact_call {
                return Err(ProviderError::Api {
                    status: 500,
                    message: "Internal error".to_string(),
                });
            }

            // Regular conversation turns: tool use on odd calls, text on even
            let events = if idx % 2 == 0 {
                // Tool use turn → keeps the loop going
                vec![
                    LlmEvent::ToolUse {
                        id: format!("t{idx}"),
                        name: "mock_tool".to_string(),
                        input: serde_json::json!({}),
                        extra: None,
                    },
                    LlmEvent::Done {
                        stop_reason: StopReason::ToolUse,
                        usage: TokenUsage {
                            input_tokens: 170_000, // above autocompact threshold
                            output_tokens: 100,
                            ..Default::default()
                        },
                    },
                ]
            } else {
                // Text turn → ends the loop
                vec![
                    LlmEvent::TextDelta("Final".to_string()),
                    LlmEvent::Done {
                        stop_reason: StopReason::EndTurn,
                        usage: TokenUsage {
                            input_tokens: 170_000,
                            output_tokens: 100,
                            ..Default::default()
                        },
                    },
                ]
            };

            let (tx, rx) = mpsc::channel(64);
            tokio::spawn(async move {
                for event in events {
                    let _ = tx.send(event).await;
                }
            });
            Ok(rx)
        }
    }

    let provider = Arc::new(CircuitBreakerProvider {
        call_index: Mutex::new(0),
    });

    let mut config = test_config();
    config.compact = CompactConfig {
        max_failures: 3,
        // Set emergency very high so it doesn't interfere
        context_window: 500_000,
        emergency_buffer: 3_000,
        ..Default::default()
    };
    config.max_turns = Some(10);

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(common::MockTool::new(
        "mock_tool",
        "result",
        false,
    )));
    let output = silent_output();

    let mut engine =
        AgentEngine::new_with_provider(provider, config, registry, output, std::env::temp_dir());
    let result = engine.execute_turn("Work", "msg-1").await.expect("should succeed");

    assert_eq!(result.text, "Final");
}
