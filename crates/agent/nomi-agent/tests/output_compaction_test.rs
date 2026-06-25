mod common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;

use common::{MockLlmProvider, MockTool, auto_approve_confirmer, test_config};
use nomi_agent::context::{SystemPromptCache, build_system_prompt};
use nomi_agent::engine::AgentEngine;
use nomi_agent::orchestration::execute_tool_calls;
use nomi_agent::output::OutputSink;
use nomi_agent::output::null_sink::NullSink;
use nomi_compact::CompactionLevel;
use nomi_providers::{LlmProvider, ProviderError};
use nomi_tools::registry::ToolRegistry;
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, StopReason, TokenUsage};
use serde_json::json;

const TEST_OUTPUT: &str = "\x1b[32mSTATUS: OK\x1b[0m\n\n\n\n50%\r100%\nCompiling dep-0 v1.0.0\nCompiling dep-1 v1.0.0\nCompiling dep-2 v1.0.0\nCompiling dep-3 v1.0.0\nCompiling dep-4 v1.0.0\n{\n    \"id\": 1,\n    \"name\": \"Alice Wonderland\",\n    \"email\": \"alice@example.com\",\n    \"age\": 30,\n    \"address\": \"123 Main Street, Anytown, USA 12345\",\n    \"phone\": \"+1-555-0123\"\n}";

const TOON_INPUT: &str =
    r#"[{"id":1,"name":"Alice","role":"admin"},{"id":2,"name":"Bob","role":"user"}]"#;

fn make_tool_use(id: &str, name: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: id.to_string(),
        name: name.to_string(),
        input: json!({}),
        extra: None,
    }
}

fn extract_tool_result_content(blocks: &[ContentBlock]) -> &str {
    for block in blocks {
        if let ContentBlock::ToolResult { content, .. } = block {
            return content;
        }
    }
    panic!("no ToolResult found in blocks");
}

// ---------------------------------------------------------------------------
// A Layer: Case 1-3 (Off / Safe / Full)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn case_1_off_passthrough() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MockTool::new("test_tool", TEST_OUTPUT, false)));

    let tool_calls = vec![make_tool_use("c1", "test_tool")];
    let confirmer = auto_approve_confirmer();

    let outcome = execute_tool_calls(
        &registry,
        &tool_calls,
        &confirmer,
        None,
        CompactionLevel::Off,
        false,
    )
    .await
    .expect("should succeed");

    let content = extract_tool_result_content(&outcome);
    eprintln!("[compaction:A] === Case 1: Off passthrough ===");
    eprintln!(
        "[compaction:A] raw ({} chars): {:?}",
        TEST_OUTPUT.len(),
        &TEST_OUTPUT[..60]
    );
    eprintln!(
        "[compaction:A] result ({} chars): {:?}",
        content.len(),
        &content[..60]
    );

    assert_eq!(
        content, TEST_OUTPUT,
        "Off level should pass content through unchanged"
    );
    eprintln!("[compaction:A] ✓ content unchanged");
}

#[tokio::test]
async fn case_2_safe_sanitizes() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MockTool::new("test_tool", TEST_OUTPUT, false)));

    let tool_calls = vec![make_tool_use("c2", "test_tool")];
    let confirmer = auto_approve_confirmer();

    let outcome = execute_tool_calls(
        &registry,
        &tool_calls,
        &confirmer,
        None,
        CompactionLevel::Safe,
        false,
    )
    .await
    .expect("should succeed");

    let content = extract_tool_result_content(&outcome);
    eprintln!("[compaction:A] === Case 2: Safe sanitizes ===");
    eprintln!("[compaction:A] raw ({} chars)", TEST_OUTPUT.len());
    eprintln!(
        "[compaction:A] result ({} chars): {:?}",
        content.len(),
        content
    );

    assert!(!content.contains("\x1b"), "Safe should strip ANSI escapes");
    assert!(!content.contains("\n\n\n"), "Safe should merge blank lines");
    assert!(!content.contains("\r"), "Safe should collapse CR lines");
    assert!(
        content.contains("Compiling dep-0"),
        "Safe should keep all repeated lines"
    );
    assert!(
        content.contains("Compiling dep-4"),
        "Safe should keep all repeated lines"
    );
    assert!(
        content.contains("    \"id\""),
        "Safe should preserve original JSON indentation"
    );

    eprintln!(
        "[compaction:A] ✓ ANSI stripped, blanks merged, CR collapsed, repeats & JSON untouched"
    );
}

#[tokio::test]
async fn case_3_full_folds_and_compacts() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MockTool::new("test_tool", TEST_OUTPUT, false)));

    let tool_calls = vec![make_tool_use("c3", "test_tool")];
    let confirmer = auto_approve_confirmer();

    let outcome = execute_tool_calls(
        &registry,
        &tool_calls,
        &confirmer,
        None,
        CompactionLevel::Full,
        false,
    )
    .await
    .expect("should succeed");

    let content = extract_tool_result_content(&outcome);
    eprintln!("[compaction:A] === Case 3: Full folds and compacts ===");
    eprintln!("[compaction:A] raw ({} chars)", TEST_OUTPUT.len());
    eprintln!(
        "[compaction:A] result ({} chars): {:?}",
        content.len(),
        content
    );

    assert!(!content.contains("\x1b"), "Full should strip ANSI");
    assert!(
        content.contains("similar lines") || content.contains("identical lines"),
        "Full should fold repeated lines: {content}"
    );
    assert!(
        content.len() < TEST_OUTPUT.len(),
        "Full should produce shorter output: {} vs {}",
        content.len(),
        TEST_OUTPUT.len()
    );

    eprintln!("[compaction:A] ✓ ANSI stripped, lines folded, output shorter");
}

// ---------------------------------------------------------------------------
// A Layer: Case 4-5 (TOON on / off)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn case_4_toon_encodes_array() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MockTool::new("test_tool", TOON_INPUT, false)));

    let tool_calls = vec![make_tool_use("c4", "test_tool")];
    let confirmer = auto_approve_confirmer();

    let outcome = execute_tool_calls(
        &registry,
        &tool_calls,
        &confirmer,
        None,
        CompactionLevel::Full,
        true,
    )
    .await
    .expect("should succeed");

    let content = extract_tool_result_content(&outcome);
    eprintln!("[compaction:A] === Case 4: TOON encodes array ===");
    eprintln!("[compaction:A] raw: {TOON_INPUT}");
    eprintln!("[compaction:A] result: {content}");

    assert!(
        content.contains("[2]{id,name,role}:"),
        "TOON should produce header: {content}"
    );
    assert!(content.contains("Alice"), "TOON should contain data");
    assert!(content.contains("Bob"), "TOON should contain data");

    eprintln!("[compaction:A] ✓ TOON header present with data rows");
}

#[tokio::test]
async fn case_5_toon_disabled_no_encoding() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MockTool::new("test_tool", TOON_INPUT, false)));

    let tool_calls = vec![make_tool_use("c5", "test_tool")];
    let confirmer = auto_approve_confirmer();

    let outcome = execute_tool_calls(
        &registry,
        &tool_calls,
        &confirmer,
        None,
        CompactionLevel::Full,
        false,
    )
    .await
    .expect("should succeed");

    let content = extract_tool_result_content(&outcome);
    eprintln!("[compaction:A] === Case 5: TOON disabled ===");
    eprintln!("[compaction:A] raw: {TOON_INPUT}");
    eprintln!("[compaction:A] result: {content}");

    assert!(
        !content.contains("[2]{id,name,role}:"),
        "TOON off should not produce TOON header: {content}"
    );

    eprintln!("[compaction:A] ✓ no TOON encoding when disabled");
}

// ---------------------------------------------------------------------------
// CapturingProvider — wraps MockLlmProvider, records each LlmRequest
// ---------------------------------------------------------------------------

struct CapturingProvider {
    inner: MockLlmProvider,
    captured: Arc<Mutex<Vec<LlmRequest>>>,
}

#[async_trait]
impl LlmProvider for CapturingProvider {
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<mpsc::Receiver<LlmEvent>, ProviderError> {
        self.captured.lock().unwrap().push(request.clone());
        self.inner.stream(request).await
    }
}

// ---------------------------------------------------------------------------
// B Layer: Case 6 (compressed content reaches LLM)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn case_6_compressed_content_reaches_llm() {
    let captured: Arc<Mutex<Vec<LlmRequest>>> = Arc::new(Mutex::new(Vec::new()));

    let provider = CapturingProvider {
        inner: MockLlmProvider::with_turns(vec![
            vec![
                LlmEvent::ToolUse {
                    id: "t1".to_string(),
                    name: "test_tool".to_string(),
                    input: json!({}),
                    extra: None,
                },
                LlmEvent::Done {
                    stop_reason: StopReason::ToolUse,
                    usage: TokenUsage::default(),
                },
            ],
            vec![
                LlmEvent::TextDelta("done".to_string()),
                LlmEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    usage: TokenUsage::default(),
                },
            ],
        ]),
        captured: captured.clone(),
    };

    let mut config = test_config();
    config.compact.compaction = CompactionLevel::Full;
    config.compact.toon = false;

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MockTool::new("test_tool", TEST_OUTPUT, false)));

    let output: Arc<dyn OutputSink> = Arc::new(NullSink);
    let mut engine = AgentEngine::new_with_provider(
        Arc::new(provider),
        config,
        registry,
        output,
        std::env::temp_dir(),
    );

    engine
        .run("call test_tool", "")
        .await
        .expect("engine.run should succeed");

    let requests = captured.lock().unwrap();
    eprintln!("[compaction:B] === Case 6: Compressed content reaches LLM ===");
    eprintln!("[compaction:B] captured {} LlmRequests", requests.len());
    assert!(
        requests.len() >= 2,
        "should have at least 2 requests (initial + after tool)"
    );

    let second_req = &requests[1];
    let mut found_tool_result = false;
    for msg in &second_req.messages {
        for block in &msg.content {
            if let ContentBlock::ToolResult { content, .. } = block {
                eprintln!(
                    "[compaction:B] tool_result content ({} chars): {:?}",
                    content.len(),
                    content
                );
                assert!(!content.contains("\x1b"), "LLM should not see ANSI escapes");
                assert!(
                    content.contains("similar lines") || content.contains("identical lines"),
                    "LLM should see folded lines: {content}"
                );
                found_tool_result = true;
            }
        }
    }
    assert!(
        found_tool_result,
        "second request should contain a ToolResult"
    );

    eprintln!("[compaction:B] ✓ LLM received compressed content");
}

// ---------------------------------------------------------------------------
// B Layer: Case 7 (runtime compaction switch)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn case_7_runtime_compaction_switch() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MockTool::new("test_tool", TEST_OUTPUT, false)));

    let tool_calls = vec![make_tool_use("c7", "test_tool")];
    let confirmer = auto_approve_confirmer();

    let outcome_off = execute_tool_calls(
        &registry,
        &tool_calls,
        &confirmer,
        None,
        CompactionLevel::Off,
        false,
    )
    .await
    .expect("should succeed");
    let content_off = extract_tool_result_content(&outcome_off).to_string();

    let outcome_full = execute_tool_calls(
        &registry,
        &tool_calls,
        &confirmer,
        None,
        CompactionLevel::Full,
        false,
    )
    .await
    .expect("should succeed");
    let content_full = extract_tool_result_content(&outcome_full).to_string();

    eprintln!("[compaction:B] === Case 7: Runtime compaction switch ===");
    eprintln!("[compaction:B] Off content ({} chars)", content_off.len());
    eprintln!("[compaction:B] Full content ({} chars)", content_full.len());

    assert_ne!(
        content_off, content_full,
        "Off and Full should produce different content"
    );
    assert!(content_off.contains("\x1b"), "Off should preserve ANSI");
    assert!(!content_full.contains("\x1b"), "Full should strip ANSI");
    assert!(
        content_full.contains("similar lines") || content_full.contains("identical lines"),
        "Full should fold lines"
    );

    // Verify apply_config_update works on the engine
    let mut config = test_config();
    config.compact.compaction = CompactionLevel::Off;
    let registry_engine = ToolRegistry::new();
    let output: Arc<dyn OutputSink> = Arc::new(NullSink);
    let mut engine = AgentEngine::new_with_provider(
        Arc::new(MockLlmProvider::with_text_response("ok")),
        config,
        registry_engine,
        output,
        std::env::temp_dir(),
    );
    assert_eq!(engine.compaction_level(), CompactionLevel::Off);

    let changes = engine.apply_config_update(None, None, None, None, Some("full".to_string()));
    assert!(!changes.is_empty(), "should report changes");
    assert_eq!(engine.compaction_level(), CompactionLevel::Full);
    eprintln!("[compaction:B] apply_config_update changes: {:?}", changes);

    eprintln!("[compaction:B] ✓ runtime switch from Off to Full verified");
}

// ---------------------------------------------------------------------------
// B Layer: Case 8 (TOON system prompt injection)
// ---------------------------------------------------------------------------

#[test]
fn case_8_toon_system_prompt_injection() {
    eprintln!("[compaction:B] === Case 8: TOON system prompt injection ===");

    // TOON enabled
    let mut cache_on = SystemPromptCache::new();
    let prompt_on = build_system_prompt(
        &mut cache_on,
        Some("You are a test assistant."),
        "/tmp",
        "test-model",
        &[],
        None,
        None,
        false,
        true, // toon_enabled
        false, // browser_enabled
    );

    eprintln!(
        "[compaction:B] TOON=true system prompt length: {} chars",
        prompt_on.len()
    );

    assert!(
        prompt_on.contains("TOON"),
        "TOON enabled: system prompt should mention TOON"
    );
    assert!(
        prompt_on.contains("Token-Oriented Object Notation"),
        "should contain full TOON description"
    );

    // TOON disabled
    let mut cache_off = SystemPromptCache::new();
    let prompt_off = build_system_prompt(
        &mut cache_off,
        Some("You are a test assistant."),
        "/tmp",
        "test-model",
        &[],
        None,
        None,
        false,
        false, // toon_enabled
        false, // browser_enabled
    );

    eprintln!(
        "[compaction:B] TOON=false system prompt length: {} chars",
        prompt_off.len()
    );

    assert!(
        !prompt_off.contains("TOON"),
        "TOON disabled: system prompt should NOT mention TOON"
    );

    eprintln!("[compaction:B] ✓ TOON system prompt injection verified");
}
