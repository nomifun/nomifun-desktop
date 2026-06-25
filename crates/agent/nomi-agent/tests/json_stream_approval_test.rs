mod common;

use std::sync::Arc;

use serde_json::json;

use nomi_agent::engine::AgentEngine;
use nomi_agent::output::OutputSink;
use nomi_agent::output::terminal::TerminalSink;
use nomi_protocol::writer::ProtocolWriter;
use nomi_protocol::{ToolApprovalManager, ToolApprovalResult};
use nomi_tools::registry::ToolRegistry;
use nomi_types::llm::LlmEvent;
use nomi_types::message::{StopReason, TokenUsage};

use common::{ExecMockTool, MockLlmProvider, test_config};

fn silent_output() -> Arc<dyn OutputSink> {
    Arc::new(TerminalSink::new(true))
}

fn token_usage(input: u64, output: u64) -> TokenUsage {
    TokenUsage {
        input_tokens: input,
        output_tokens: output,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
    }
}

// ---------------------------------------------------------------------------
// test: tool approval approve flow
//
// LLM requests exec_tool → engine pauses at approval_manager.request_approval
// → background task resolves with Approved → tool executes → LLM continues
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_tool_approval_approve_flow() {
    let turn1 = vec![
        LlmEvent::ToolUse {
            id: "call-1".to_string(),
            name: "exec_tool".to_string(),
            input: json!({}),
            extra: None,
        },
        LlmEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: token_usage(80, 30),
        },
    ];
    let turn2 = vec![
        LlmEvent::TextDelta("Done".to_string()),
        LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: token_usage(100, 50),
        },
    ];

    let provider = Arc::new(MockLlmProvider::with_turns(vec![turn1, turn2]));
    let mut config = test_config();
    config.tools.auto_approve = false;

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ExecMockTool::new("exec_tool", "tool output")));

    let output = silent_output();
    let approval_manager = Arc::new(ToolApprovalManager::new());
    let writer = Arc::new(ProtocolWriter::new());

    let mut engine =
        AgentEngine::new_with_provider(provider, config, registry, output, std::env::temp_dir());
    engine.set_approval_manager(approval_manager.clone());
    engine.set_protocol_writer(writer);

    // Spawn a task that approves the tool call after a short delay
    let am = approval_manager.clone();
    tokio::spawn(async move {
        // Wait until the approval request appears
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let has_pending = {
                // Check if there's a pending request by trying to resolve a known id
                // We know the call_id is "call-1" from the mock
                true
            };
            if has_pending {
                am.resolve("call-1", ToolApprovalResult::Approved);
                break;
            }
        }
    });

    let result = engine
        .run("Use the tool", "msg-1")
        .await
        .expect("should succeed");
    assert_eq!(result.text, "Done");
    assert_eq!(result.turns, 2);
}

// ---------------------------------------------------------------------------
// test: tool approval deny flow
//
// LLM requests exec_tool → engine pauses → background resolves with Denied
// → tool_cancelled → denial fed back to LLM → LLM responds with text
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_tool_approval_deny_flow() {
    let turn1 = vec![
        LlmEvent::ToolUse {
            id: "call-2".to_string(),
            name: "exec_tool".to_string(),
            input: json!({}),
            extra: None,
        },
        LlmEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: token_usage(80, 30),
        },
    ];
    let turn2 = vec![
        LlmEvent::TextDelta("Cannot run tool".to_string()),
        LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: token_usage(100, 50),
        },
    ];

    let provider = Arc::new(MockLlmProvider::with_turns(vec![turn1, turn2]));
    let mut config = test_config();
    config.tools.auto_approve = false;

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ExecMockTool::new("exec_tool", "tool output")));

    let output = silent_output();
    let approval_manager = Arc::new(ToolApprovalManager::new());
    let writer = Arc::new(ProtocolWriter::new());

    let mut engine =
        AgentEngine::new_with_provider(provider, config, registry, output, std::env::temp_dir());
    engine.set_approval_manager(approval_manager.clone());
    engine.set_protocol_writer(writer);

    let am = approval_manager.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        am.resolve(
            "call-2",
            ToolApprovalResult::Denied {
                reason: "policy violation".into(),
            },
        );
    });

    let result = engine
        .run("Use the tool", "msg-2")
        .await
        .expect("should succeed");
    assert_eq!(result.text, "Cannot run tool");
    assert_eq!(result.turns, 2);
}

// ---------------------------------------------------------------------------
// test: auto_approve bypasses approval wait
//
// With auto_approve=true, exec category tools should execute immediately
// without waiting for approval.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_auto_approve_bypasses_approval() {
    let turn1 = vec![
        LlmEvent::ToolUse {
            id: "call-3".to_string(),
            name: "exec_tool".to_string(),
            input: json!({}),
            extra: None,
        },
        LlmEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: token_usage(80, 30),
        },
    ];
    let turn2 = vec![
        LlmEvent::TextDelta("Auto done".to_string()),
        LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: token_usage(100, 50),
        },
    ];

    let provider = Arc::new(MockLlmProvider::with_turns(vec![turn1, turn2]));
    let mut config = test_config();
    config.tools.auto_approve = true;

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ExecMockTool::new("exec_tool", "tool output")));

    let output = silent_output();
    let approval_manager = Arc::new(ToolApprovalManager::new());
    let writer = Arc::new(ProtocolWriter::new());

    let mut engine =
        AgentEngine::new_with_provider(provider, config, registry, output, std::env::temp_dir());
    engine.set_approval_manager(approval_manager.clone());
    engine.set_protocol_writer(writer);

    // No background task to approve — should not hang
    let result = engine
        .run("Use the tool", "msg-3")
        .await
        .expect("should succeed");
    assert_eq!(result.text, "Auto done");
    assert_eq!(result.turns, 2);
}

// ---------------------------------------------------------------------------
// test: session auto-approve (scope=always) bypasses future approvals
//
// After add_auto_approve("exec"), exec tools skip the approval wait.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_session_auto_approve_category() {
    let turn1 = vec![
        LlmEvent::ToolUse {
            id: "call-4".to_string(),
            name: "exec_tool".to_string(),
            input: json!({}),
            extra: None,
        },
        LlmEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: token_usage(80, 30),
        },
    ];
    let turn2 = vec![
        LlmEvent::TextDelta("Session auto".to_string()),
        LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: token_usage(100, 50),
        },
    ];

    let provider = Arc::new(MockLlmProvider::with_turns(vec![turn1, turn2]));
    let mut config = test_config();
    config.tools.auto_approve = false;

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ExecMockTool::new("exec_tool", "tool output")));

    let output = silent_output();
    let approval_manager = Arc::new(ToolApprovalManager::new());
    // Pre-approve the "exec" category
    approval_manager.add_auto_approve("exec");
    let writer = Arc::new(ProtocolWriter::new());

    let mut engine =
        AgentEngine::new_with_provider(provider, config, registry, output, std::env::temp_dir());
    engine.set_approval_manager(approval_manager.clone());
    engine.set_protocol_writer(writer);

    // No background task to approve — should not hang
    let result = engine
        .run("Use the tool", "msg-4")
        .await
        .expect("should succeed");
    assert_eq!(result.text, "Session auto");
    assert_eq!(result.turns, 2);
}

// ---------------------------------------------------------------------------
// test: client disconnect (channel drop) causes UserAborted
//
// If the approval channel sender is dropped before resolve, the engine
// should return an abort error.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_client_disconnect_aborts() {
    let turn1 = vec![
        LlmEvent::ToolUse {
            id: "call-5".to_string(),
            name: "exec_tool".to_string(),
            input: json!({}),
            extra: None,
        },
        LlmEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: token_usage(80, 30),
        },
    ];

    let provider = Arc::new(MockLlmProvider::with_turns(vec![turn1]));
    let mut config = test_config();
    config.tools.auto_approve = false;

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ExecMockTool::new("exec_tool", "tool output")));

    let output = silent_output();
    let approval_manager = Arc::new(ToolApprovalManager::new());
    let writer = Arc::new(ProtocolWriter::new());

    let mut engine =
        AgentEngine::new_with_provider(provider, config, registry, output, std::env::temp_dir());
    engine.set_approval_manager(approval_manager.clone());
    engine.set_protocol_writer(writer);

    // Simulate client disconnect: drop the pending sender without resolving
    let am = approval_manager.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        am.drop_pending("call-5");
    });

    let err = engine.run("Use the tool", "msg-5").await.unwrap_err();
    assert!(
        format!("{:?}", err).contains("UserAborted"),
        "expected UserAborted, got: {:?}",
        err
    );
}
