mod common;

use std::sync::Arc;

use common::{MockLlmProvider, test_config};
use nomi_agent::spawner::{AgentSpawner, SubAgentConfig};
use nomi_types::llm::LlmEvent;
use nomi_types::message::{StopReason, TokenUsage};

// ---------------------------------------------------------------------------
// Helper: build a minimal SubAgentConfig for testing
// ---------------------------------------------------------------------------

fn make_sub_config(name: &str) -> SubAgentConfig {
    SubAgentConfig {
        name: name.to_string(),
        prompt: format!("Task for {}", name),
        max_turns: 5,
        max_tokens: 1024,
        system_prompt: None,
        allowed_tools: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Single sub-agent executes and returns the expected text result.
#[tokio::test]
async fn test_spawn_single_agent() {
    let provider = Arc::new(MockLlmProvider::with_text_response("Sub-agent done"));
    let spawner = AgentSpawner::new(provider, test_config(), std::env::temp_dir());

    let result = spawner.spawn_one(make_sub_config("agent-1")).await;

    assert_eq!(result.text, "Sub-agent done");
    assert!(!result.is_error, "expected no error, got: {}", result.text);
    assert_eq!(result.turns, 1);
    assert_eq!(result.name, "agent-1");
}

/// Parallel sub-agents all complete successfully and return distinct results.
#[tokio::test]
async fn test_spawn_parallel_agents() {
    // Provide one turn sequence per sub-agent; each stream() call pops one entry.
    let make_turn = |text: &str| {
        vec![
            LlmEvent::TextDelta(text.to_string()),
            LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    cache_creation_tokens: 0,
                    cache_read_tokens: 0,
                },
            },
        ]
    };

    let provider = Arc::new(MockLlmProvider::with_turns(vec![
        make_turn("result-A"),
        make_turn("result-B"),
        make_turn("result-C"),
    ]));

    let spawner = AgentSpawner::new(provider, test_config(), std::env::temp_dir());

    let sub_configs = vec![
        make_sub_config("agent-A"),
        make_sub_config("agent-B"),
        make_sub_config("agent-C"),
    ];

    let results = spawner.spawn_parallel(sub_configs).await;

    assert_eq!(results.len(), 3, "expected 3 results from 3 sub-agents");

    for result in &results {
        assert!(
            !result.is_error,
            "sub-agent '{}' returned an error: {}",
            result.name, result.text
        );
    }

    // Each result should contain one of the expected texts (order may vary due
    // to concurrent scheduling, so we just verify the full set is covered).
    let texts: std::collections::HashSet<&str> = results.iter().map(|r| r.text.as_str()).collect();
    assert!(texts.contains("result-A"), "missing result-A");
    assert!(texts.contains("result-B"), "missing result-B");
    assert!(texts.contains("result-C"), "missing result-C");
}

/// The same provider Arc is reused across sequentially spawned sub-agents.
#[tokio::test]
async fn test_spawn_shares_provider() {
    // Two turns: one for each sequential sub-agent call.
    let provider = Arc::new(MockLlmProvider::with_turns(vec![
        vec![
            LlmEvent::TextDelta("first".to_string()),
            LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    cache_creation_tokens: 0,
                    cache_read_tokens: 0,
                },
            },
        ],
        vec![
            LlmEvent::TextDelta("second".to_string()),
            LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    cache_creation_tokens: 0,
                    cache_read_tokens: 0,
                },
            },
        ],
    ]));

    // Both sub-agents share the same underlying provider via Arc.
    let provider_dyn: Arc<dyn nomi_providers::LlmProvider> = provider;
    let spawner = AgentSpawner::new(
        Arc::clone(&provider_dyn),
        test_config(),
        std::env::temp_dir(),
    );

    let result1 = spawner.spawn_one(make_sub_config("seq-1")).await;
    let result2 = spawner.spawn_one(make_sub_config("seq-2")).await;

    assert!(!result1.is_error, "seq-1 errored: {}", result1.text);
    assert!(!result2.is_error, "seq-2 errored: {}", result2.text);
    assert_eq!(result1.text, "first");
    assert_eq!(result2.text, "second");
}

/// An LLM error event causes the sub-agent result to be marked as an error.
#[tokio::test]
async fn test_spawn_agent_error_captured() {
    // Emit an Error event — the engine converts this to AgentError::ApiError,
    // which spawner catches and stores in SubAgentResult::is_error.
    let provider = Arc::new(MockLlmProvider::with_events(vec![LlmEvent::Error(
        "provider failed".to_string(),
    )]));

    let spawner = AgentSpawner::new(provider, test_config(), std::env::temp_dir());

    let result = spawner.spawn_one(make_sub_config("error-agent")).await;

    assert!(result.is_error, "expected is_error=true");
    assert!(
        result.text.to_lowercase().contains("error"),
        "expected error message to contain 'error', got: {}",
        result.text
    );
}
