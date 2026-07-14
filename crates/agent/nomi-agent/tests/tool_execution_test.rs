mod common;

use std::sync::Arc;

use common::{MockTool, auto_approve_confirmer};
use nomi_agent::tool_execution::{execute_tool_calls, execute_tool_calls_with_approval};
use nomi_compact::CompactionLevel;
use nomi_config::hooks::{HookDef, HookEngine, HooksConfig};
use nomi_protocol::events::ProtocolEvent;
use nomi_protocol::writer::ProtocolEmitter;
use nomi_tools::registry::ToolRegistry;
use nomi_types::message::ContentBlock;
use nomi_types::tool::ToolResult;
use serde_json::json;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_tool_use(id: &str, name: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: id.to_string(),
        name: name.to_string(),
        input: json!({}),
        extra: None,
    }
}

fn make_pre_hook(name: &str, tool_match: &str, command: &str) -> HookDef {
    HookDef {
        name: name.to_string(),
        tool_match: vec![tool_match.to_string()],
        file_match: vec![],
        command: command.to_string(),
        timeout_ms: 5_000,
    }
}

fn make_post_hook(name: &str, tool_match: &str, command: &str) -> HookDef {
    HookDef {
        name: name.to_string(),
        tool_match: vec![tool_match.to_string()],
        file_match: vec![],
        command: command.to_string(),
        timeout_ms: 5_000,
    }
}

fn make_sequential_tool(name: &str, result: &str, is_error: bool) -> MockTool {
    MockTool {
        tool_name: name.to_string(),
        tool_description: format!("Mock sequential tool: {name}"),
        concurrent_safe: false,
        result: Mutex::new(ToolResult {
            content: result.to_string(),
            is_error,
            images: Vec::new(),
        }),
    }
}

#[derive(Default)]
struct CapturingEmitter {
    events: Mutex<Vec<String>>,
}

impl ProtocolEmitter for CapturingEmitter {
    fn emit(&self, event: &ProtocolEvent) -> std::io::Result<()> {
        let encoded = serde_json::to_string(event)
            .map_err(|e| std::io::Error::other(format!("serialize protocol event: {e}")))?;
        self.events.lock().unwrap().push(encoded);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Single tool call executes and returns the expected result
#[tokio::test]
async fn test_execute_single_tool_call() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MockTool::new("echo", "hello", false)));

    let tool_calls = vec![make_tool_use("call-1", "echo")];
    let confirmer = auto_approve_confirmer();

    let results = execute_tool_calls(
        &registry,
        &tool_calls,
        &confirmer,
        None,
        CompactionLevel::Off,
        false,
    )
    .await
    .expect("execution should succeed");

    assert_eq!(results.len(), 1);
    match &results[0] {
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
            ..
        } => {
            assert_eq!(tool_use_id, "call-1");
            assert_eq!(content, "hello");
            assert!(!is_error);
        }
        other => panic!("expected ToolResult, got {:?}", other),
    }
}

/// Two concurrent-safe tools execute in parallel and both return results
#[tokio::test]
async fn test_execute_concurrent_safe_tools() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MockTool::new("tool_a", "result_a", false)));
    registry.register(Box::new(MockTool::new("tool_b", "result_b", false)));

    let tool_calls = vec![
        make_tool_use("id-a", "tool_a"),
        make_tool_use("id-b", "tool_b"),
    ];
    let confirmer = auto_approve_confirmer();

    let results = execute_tool_calls(
        &registry,
        &tool_calls,
        &confirmer,
        None,
        CompactionLevel::Off,
        false,
    )
    .await
    .expect("execution should succeed");

    assert_eq!(results.len(), 2);

    // Collect content strings keyed by tool_use_id for order-independent assertion
    let content_map: std::collections::HashMap<_, _> = results
        .iter()
        .filter_map(|r| match r {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => Some((tool_use_id.as_str(), content.as_str())),
            _ => None,
        })
        .collect();

    assert_eq!(content_map.get("id-a"), Some(&"result_a"));
    assert_eq!(content_map.get("id-b"), Some(&"result_b"));
}

/// Two sequential (non-concurrent) tools execute one after the other and both succeed
#[tokio::test]
async fn test_execute_non_concurrent_tools_sequential() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MockTool::sequential("seq_a", "seq_result_a")));
    registry.register(Box::new(MockTool::sequential("seq_b", "seq_result_b")));

    let tool_calls = vec![
        make_tool_use("id-a", "seq_a"),
        make_tool_use("id-b", "seq_b"),
    ];
    let confirmer = auto_approve_confirmer();

    let results = execute_tool_calls(
        &registry,
        &tool_calls,
        &confirmer,
        None,
        CompactionLevel::Off,
        false,
    )
    .await
    .expect("execution should succeed");

    assert_eq!(results.len(), 2);

    let content_map: std::collections::HashMap<_, _> = results
        .iter()
        .filter_map(|r| match r {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => Some((tool_use_id.as_str(), content.as_str())),
            _ => None,
        })
        .collect();

    assert_eq!(content_map.get("id-a"), Some(&"seq_result_a"));
    assert_eq!(content_map.get("id-b"), Some(&"seq_result_b"));
}

/// A failed sequential tool should stop later same-turn tools so the model can
/// inspect the failure and decide whether to retry, extend timeout, or change
/// strategy before dependent work runs.
#[tokio::test]
async fn test_execute_non_concurrent_tools_stops_after_error() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(make_sequential_tool("seq_fail", "timeout", true)));
    registry.register(Box::new(make_sequential_tool("seq_later", "must_not_run", false)));

    let tool_calls = vec![
        make_tool_use("id-fail", "seq_fail"),
        make_tool_use("id-later", "seq_later"),
    ];
    let confirmer = auto_approve_confirmer();

    let results = execute_tool_calls(
        &registry,
        &tool_calls,
        &confirmer,
        None,
        CompactionLevel::Off,
        false,
    )
    .await
    .expect("execution should succeed");

    assert_eq!(results.len(), 2);
    assert!(matches!(
        &results[0],
        ContentBlock::ToolResult { tool_use_id, content, is_error, .. }
            if tool_use_id == "id-fail" && content == "timeout" && *is_error
    ));
    assert!(matches!(
        &results[1],
        ContentBlock::ToolResult { tool_use_id, content, is_error, .. }
            if tool_use_id == "id-later"
                && *is_error
                && content.contains("Skipped because a previous tool call")
                && !content.contains("must_not_run")
    ));
}

#[tokio::test]
async fn test_protocol_execution_stops_after_sequential_error() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(make_sequential_tool("seq_fail", "timeout", true)));
    registry.register(Box::new(make_sequential_tool("seq_later", "must_not_run", false)));

    let tool_calls = vec![
        make_tool_use("id-fail", "seq_fail"),
        make_tool_use("id-later", "seq_later"),
    ];
    let approval_manager = Arc::new(nomi_protocol::ToolApprovalManager::new());
    let writer_capture = Arc::new(CapturingEmitter::default());
    let writer: Arc<dyn ProtocolEmitter> = writer_capture.clone();

    let outcome = execute_tool_calls_with_approval(
        &registry,
        &tool_calls,
        &approval_manager,
        &writer,
        "msg-sequential-error",
        true,
        &[],
        None,
        CompactionLevel::Off,
        false,
    )
    .await
    .expect("execution should succeed");

    assert_eq!(outcome.results.len(), 2);
    assert!(matches!(
        &outcome.results[0],
        ContentBlock::ToolResult { tool_use_id, content, is_error, .. }
            if tool_use_id == "id-fail" && content == "timeout" && *is_error
    ));
    assert!(matches!(
        &outcome.results[1],
        ContentBlock::ToolResult { tool_use_id, content, is_error, .. }
            if tool_use_id == "id-later"
                && *is_error
                && content.contains("Skipped because a previous tool call")
                && !content.contains("must_not_run")
    ));

    let events = writer_capture.events.lock().unwrap();
    let later_result = events.iter().find(|event| {
        event.contains(r#""call_id":"id-later""#)
            && event.contains("Skipped because a previous tool call")
    });
    assert!(later_result.is_some(), "skipped call should still emit a paired tool result");
}

/// Calling a tool that is not registered returns an error ToolResult with "Unknown tool"
#[tokio::test]
async fn test_unknown_tool_returns_error() {
    let registry = ToolRegistry::new(); // empty registry

    let tool_calls = vec![make_tool_use("id-x", "nonexistent_tool")];
    let confirmer = auto_approve_confirmer();

    let results = execute_tool_calls(
        &registry,
        &tool_calls,
        &confirmer,
        None,
        CompactionLevel::Off,
        false,
    )
    .await
    .expect("execute_tool_calls itself should not fail");

    assert_eq!(results.len(), 1);
    match &results[0] {
        ContentBlock::ToolResult {
            content, is_error, ..
        } => {
            assert!(*is_error, "unknown tool should produce is_error = true");
            assert!(
                content.contains("Unknown tool"),
                "error message should mention 'Unknown tool', got: {}",
                content
            );
        }
        other => panic!("expected ToolResult, got {:?}", other),
    }
}

/// A tool that signals an error surfaces is_error = true in the result
#[tokio::test]
async fn test_tool_error_returns_error_result() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MockTool::new("fail_tool", "error message", true)));

    let tool_calls = vec![make_tool_use("id-fail", "fail_tool")];
    let confirmer = auto_approve_confirmer();

    let results = execute_tool_calls(
        &registry,
        &tool_calls,
        &confirmer,
        None,
        CompactionLevel::Off,
        false,
    )
    .await
    .expect("execution should succeed");

    assert_eq!(results.len(), 1);
    match &results[0] {
        ContentBlock::ToolResult {
            content, is_error, ..
        } => {
            assert!(*is_error, "tool error should propagate as is_error = true");
            assert_eq!(content, "error message");
        }
        other => panic!("expected ToolResult, got {:?}", other),
    }
}

/// A pre-tool-use hook that exits with a non-zero status blocks tool execution
#[tokio::test]
async fn test_pre_hook_blocks_tool() {
    let hook_config = HooksConfig {
        pre_tool_use: vec![make_pre_hook("blocker", "echo", "exit 1")],
        post_tool_use: vec![],
        stop: vec![],
    };
    let mut hook_engine = HookEngine::new(hook_config, std::env::temp_dir());

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MockTool::new("echo", "should not appear", false)));

    let tool_calls = vec![make_tool_use("id-blocked", "echo")];
    let confirmer = auto_approve_confirmer();

    let results = execute_tool_calls(
        &registry,
        &tool_calls,
        &confirmer,
        Some(&mut hook_engine),
        CompactionLevel::Off,
        false,
    )
    .await
    .expect("execute_tool_calls itself should not fail");

    assert_eq!(results.len(), 1);
    match &results[0] {
        ContentBlock::ToolResult {
            content, is_error, ..
        } => {
            assert!(
                *is_error,
                "blocked execution should produce is_error = true"
            );
            assert!(
                content.contains("Blocked by hook"),
                "result should mention 'Blocked by hook', got: {}",
                content
            );
        }
        other => panic!("expected ToolResult, got {:?}", other),
    }
}

/// A post-tool-use hook runs after the tool but does not alter the tool's result
#[tokio::test]
async fn test_post_hook_runs_after_tool() {
    let hook_config = HooksConfig {
        pre_tool_use: vec![],
        post_tool_use: vec![make_post_hook("post-logger", "echo", "echo done")],
        stop: vec![],
    };
    let mut hook_engine = HookEngine::new(hook_config, std::env::temp_dir());

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MockTool::new("echo", "result", false)));

    let tool_calls = vec![make_tool_use("id-post", "echo")];
    let confirmer = auto_approve_confirmer();

    let results = execute_tool_calls(
        &registry,
        &tool_calls,
        &confirmer,
        Some(&mut hook_engine),
        CompactionLevel::Off,
        false,
    )
    .await
    .expect("execution should succeed");

    assert_eq!(results.len(), 1);
    match &results[0] {
        ContentBlock::ToolResult {
            content, is_error, ..
        } => {
            // Post-hooks must not mutate the tool result
            assert!(!is_error);
            assert_eq!(content, "result");
        }
        other => panic!("expected ToolResult, got {:?}", other),
    }
}

/// Results that exceed max_result_size are truncated with a "[truncated N chars]" marker
#[tokio::test]
async fn test_tool_result_truncation() {
    // Default max_result_size is 50_000; build a result that exceeds it
    let long_result: String = "x".repeat(60_000);

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MockTool::new("big_tool", &long_result, false)));

    let tool_calls = vec![make_tool_use("id-big", "big_tool")];
    let confirmer = auto_approve_confirmer();

    let results = execute_tool_calls(
        &registry,
        &tool_calls,
        &confirmer,
        None,
        CompactionLevel::Off,
        false,
    )
    .await
    .expect("execution should succeed");

    assert_eq!(results.len(), 1);
    match &results[0] {
        ContentBlock::ToolResult {
            content, is_error, ..
        } => {
            assert!(!is_error);
            assert!(
                content.len() < long_result.len(),
                "truncated result should be shorter than the original"
            );
            assert!(
                content.contains("truncated"),
                "truncated result should contain the word 'truncated', got length {}",
                content.len()
            );
        }
        other => panic!("expected ToolResult, got {:?}", other),
    }
}
