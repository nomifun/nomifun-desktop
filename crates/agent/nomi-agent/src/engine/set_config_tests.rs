// ---------------------------------------------------------------------------
// set_config tests — apply_config_update()
// ---------------------------------------------------------------------------

use std::sync::{Arc, Mutex};

use super::{
    AgentError, MAX_PROVIDER_TURN_TOOL_CALLS, REQUEST_SCOPED_TOOL_AUTHORITY_HEADER,
    REQUEST_SCOPED_TOOL_AUTHORITY_RULE, SYSTEM_RESOURCE_CONTEXT_HEADER, USER_IMAGE_HISTORY_PLACEHOLDER,
};
use nomi_protocol::events::ToolCategory;
use nomi_providers::{LlmProvider, ProviderError};
use nomi_tools::{Tool, registry::ToolRegistry};
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, Role};
use nomi_types::tool::ToolResult;
use serde_json::Value;

use crate::confirm::ToolConfirmer;
use crate::output::null_sink::NullSink;
use crate::output::{OutputSink, ToolMediaDelivery, artifact_contract};
use crate::session::EditableTurnCheckpoint;

#[derive(Default)]
struct ArtifactIdentityOutput {
    running_identities: Mutex<Vec<String>>,
    result_identities: Mutex<Vec<String>>,
}

impl OutputSink for ArtifactIdentityOutput {
    fn emit_text_delta(&self, _: &str, _: &str) {}
    fn emit_thinking(&self, _: &str, _: &str) {}
    fn emit_tool_call(&self, _: &str, _: &str, _: &str) {}
    fn emit_tool_call_with_artifact_identity(
        &self,
        _: &str,
        _: &str,
        artifact_identity: &str,
        _: &str,
    ) {
        self.running_identities
            .lock()
            .unwrap()
            .push(artifact_identity.to_owned());
    }
    fn emit_tool_result(&self, _: &str, _: &str, _: bool, _: &str) {}
    fn emit_tool_result_with_images_and_artifact_identity(
        &self,
        _: &str,
        _: &str,
        artifact_identity: &str,
        is_error: bool,
        _: &str,
        images: &[nomi_types::tool::ToolImage],
    ) -> ToolMediaDelivery {
        self.result_identities
            .lock()
            .unwrap()
            .push(artifact_identity.to_owned());
        let contract = artifact_contract(artifact_identity)
            .expect("the untruncated exporter identity must create a contract");
        if !is_error && images.is_empty() {
            ToolMediaDelivery::Failed {
                error: format!("tool returned no {}", contract.label()),
            }
        } else {
            ToolMediaDelivery::Unmanaged
        }
    }
    fn emit_stream_start(&self, _: &str) {}
    fn emit_stream_end(&self, _: &str, _: usize, _: u64, _: u64, _: u64, _: u64) {}
    fn emit_error(&self, _: &str) {}
    fn emit_info(&self, _: &str) {}
}

#[derive(Default)]
struct ToolLifecycleRecordingOutput {
    tool_calls: std::sync::atomic::AtomicUsize,
    tool_results: std::sync::atomic::AtomicUsize,
    tool_inputs: Mutex<Vec<Value>>,
}

impl OutputSink for ToolLifecycleRecordingOutput {
    fn emit_text_delta(&self, _: &str, _: &str) {}
    fn emit_thinking(&self, _: &str, _: &str) {}
    fn emit_tool_call(&self, _: &str, _: &str, input: &str) {
        self.tool_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Ok(input) = serde_json::from_str(input) {
            self.tool_inputs.lock().unwrap().push(input);
        }
    }
    fn emit_tool_result(&self, _: &str, _: &str, _: bool, _: &str) {
        self.tool_results
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    fn emit_stream_start(&self, _: &str) {}
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
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tx.send(LlmEvent::Done {
            stop_reason: nomi_types::message::StopReason::EndTurn,
            usage: Default::default(),
        })
        .await
        .unwrap();
        Ok(rx)
    }
}

struct RecordingProvider {
    requests: Mutex<Vec<LlmRequest>>,
    fail: bool,
}

impl RecordingProvider {
    fn successful() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            fail: false,
        }
    }

    fn failing() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            fail: true,
        }
    }

    fn requests(&self) -> Vec<LlmRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl LlmProvider for RecordingProvider {
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        self.requests.lock().unwrap().push(request.clone());
        if self.fail {
            return Err(ProviderError::Connection("test provider failure".into()));
        }

        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let _ = tx
            .send(LlmEvent::Done {
                stop_reason: nomi_types::message::StopReason::EndTurn,
                usage: Default::default(),
            })
            .await;
        Ok(rx)
    }
}

struct CompactThenFailProvider {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl LlmProvider for CompactThenFailProvider {
    async fn stream(
        &self,
        _: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if call > 0 {
            return Err(ProviderError::Connection("post-compact provider failure".into()));
        }
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tx.send(LlmEvent::TextDelta(
            "<summary>Earlier stable conversation.</summary>".into(),
        ))
        .await
        .unwrap();
        tx.send(LlmEvent::Done {
            stop_reason: nomi_types::message::StopReason::EndTurn,
            usage: Default::default(),
        })
        .await
        .unwrap();
        Ok(rx)
    }
}

/// Emits one tool call every turn forever — used to verify the runaway-loop
/// safety net. With `max_turns: None` the engine must still terminate.
///
/// Provider tool-use ids are scoped to the root user turn and must be
/// unique across rounds. Keep this fixture focused on the stagnation guard
/// rather than accidentally exercising the protocol-violation path.
struct LoopProvider {
    calls: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl LlmProvider for LoopProvider {
    async fn stream(
        &self,
        _: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        let call = self
            .calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        let _ = tx
            .send(LlmEvent::ToolUse {
                id: format!("loop-{call}"),
                name: "noop".to_string(),
                input: serde_json::json!({}),
                extra: None,
            })
            .await;
        let _ = tx
            .send(LlmEvent::Done {
                stop_reason: nomi_types::message::StopReason::ToolUse,
                usage: Default::default(),
            })
            .await;
        Ok(rx)
    }
}

struct FiniteLoopProvider {
    calls: std::sync::atomic::AtomicUsize,
    tool_turns: usize,
    tool_name: &'static str,
}

#[async_trait::async_trait]
impl LlmProvider for FiniteLoopProvider {
    async fn stream(
        &self,
        _: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        let turn = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        if turn < self.tool_turns {
            let _ = tx
                .send(LlmEvent::ToolUse {
                    id: format!("loop-{turn}"),
                    name: self.tool_name.to_string(),
                    input: serde_json::json!({}),
                    extra: None,
                })
                .await;
            let _ = tx
                .send(LlmEvent::Done {
                    stop_reason: nomi_types::message::StopReason::ToolUse,
                    usage: Default::default(),
                })
                .await;
        } else {
            let _ = tx
                .send(LlmEvent::Done {
                    stop_reason: nomi_types::message::StopReason::EndTurn,
                    usage: Default::default(),
                })
                .await;
        }
        Ok(rx)
    }
}

/// Alternates two failing tools while emitting assistant filler before each
/// call. Neither the changing signature nor the filler may evade the
/// consecutive-all-failed guard.
struct AlternatingFailureProvider {
    turns: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl LlmProvider for AlternatingFailureProvider {
    async fn stream(
        &self,
        _: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        let turn = self.turns.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (tx, rx) = tokio::sync::mpsc::channel(3);
        tx.send(LlmEvent::TextDelta("Trying another tool. ".to_string()))
            .await
            .unwrap();
        tx.send(LlmEvent::ToolUse {
            id: format!("alternating-failure-{turn}"),
            name: if turn % 2 == 0 { "create" } else { "update" }.to_string(),
            input: serde_json::json!({}),
            extra: None,
        })
        .await
        .unwrap();
        tx.send(LlmEvent::Done {
            stop_reason: nomi_types::message::StopReason::ToolUse,
            usage: Default::default(),
        })
        .await
        .unwrap();
        Ok(rx)
    }
}

/// Emits a configurable burst of complete tool calls in its first provider
/// turn, then a plain EndTurn response so an accepted burst can finish.
struct ToolBurstProvider {
    turns: std::sync::atomic::AtomicUsize,
    tool_calls: usize,
    tool_name: &'static str,
}

#[async_trait::async_trait]
impl LlmProvider for ToolBurstProvider {
    async fn stream(
        &self,
        _: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        let turn = self.turns.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if turn != 0 {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            tx.send(LlmEvent::Done {
                stop_reason: nomi_types::message::StopReason::EndTurn,
                usage: Default::default(),
            })
            .await
            .unwrap();
            return Ok(rx);
        }

        let (tx, rx) = tokio::sync::mpsc::channel(self.tool_calls + 1);
        for index in 0..self.tool_calls {
            tx.send(LlmEvent::ToolUse {
                id: format!("burst-{index}"),
                name: self.tool_name.to_string(),
                input: serde_json::json!({"index": index}),
                extra: None,
            })
            .await
            .unwrap();
        }
        tx.send(LlmEvent::Done {
            stop_reason: nomi_types::message::StopReason::ToolUse,
            usage: Default::default(),
        })
        .await
        .unwrap();
        Ok(rx)
    }
}

struct FixedToolCallsProvider {
    calls: Vec<(&'static str, &'static str)>,
    as_deltas: bool,
}

#[async_trait::async_trait]
impl LlmProvider for FixedToolCallsProvider {
    async fn stream(
        &self,
        _: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        let (tx, rx) = tokio::sync::mpsc::channel(self.calls.len() + 1);
        for (id, name) in &self.calls {
            let event = if self.as_deltas {
                LlmEvent::ToolUseDelta {
                    id: (*id).to_string(),
                    name: (*name).to_string(),
                    input: Some(serde_json::json!({})),
                }
            } else {
                LlmEvent::ToolUse {
                    id: (*id).to_string(),
                    name: (*name).to_string(),
                    input: serde_json::json!({}),
                    extra: None,
                }
            };
            tx.send(event).await.unwrap();
        }
        tx.send(LlmEvent::Done {
            stop_reason: if self.as_deltas {
                nomi_types::message::StopReason::EndTurn
            } else {
                nomi_types::message::StopReason::ToolUse
            },
            usage: Default::default(),
        })
        .await
        .unwrap();
        Ok(rx)
    }
}

struct PreviewThenCompleteProvider {
    turns: std::sync::atomic::AtomicUsize,
    preview: (&'static str, &'static str),
    complete: (&'static str, &'static str),
}

#[async_trait::async_trait]
impl LlmProvider for PreviewThenCompleteProvider {
    async fn stream(
        &self,
        _: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        if self.turns.fetch_add(1, std::sync::atomic::Ordering::SeqCst) != 0 {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            tx.send(LlmEvent::Done {
                stop_reason: nomi_types::message::StopReason::EndTurn,
                usage: Default::default(),
            })
            .await
            .unwrap();
            return Ok(rx);
        }
        let (tx, rx) = tokio::sync::mpsc::channel(3);
        tx.send(LlmEvent::ToolUseDelta {
            id: self.preview.0.to_string(),
            name: self.preview.1.to_string(),
            input: Some(serde_json::json!({})),
        })
        .await
        .unwrap();
        tx.send(LlmEvent::ToolUse {
            id: self.complete.0.to_string(),
            name: self.complete.1.to_string(),
            input: serde_json::json!({}),
            extra: None,
        })
        .await
        .unwrap();
        tx.send(LlmEvent::Done {
            stop_reason: nomi_types::message::StopReason::ToolUse,
            usage: Default::default(),
        })
        .await
        .unwrap();
        Ok(rx)
    }
}

struct ConstantResultTool {
    name: &'static str,
    polling: bool,
    category: ToolCategory,
    calls: Arc<std::sync::atomic::AtomicUsize>,
    steer_on_call: Option<(
        usize,
        Arc<Mutex<std::collections::VecDeque<String>>>,
    )>,
}

struct ArtifactIdentityTool {
    provider_name: String,
    artifact_identity: String,
}

#[async_trait::async_trait]
impl Tool for ArtifactIdentityTool {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn artifact_identity(&self) -> &str {
        &self.artifact_identity
    }

    fn reserved_provider_name_prefix(&self) -> Option<&'static str> {
        Some("mcp__")
    }

    fn description(&self) -> &str {
        "test MCP exporter whose provider alias omits its semantic suffix"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({"type": "object"})
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        ToolResult::text("artifact generated successfully")
    }
}

#[async_trait::async_trait]
impl Tool for ConstantResultTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "test tool returning a constant result"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({"type": "object"})
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn is_polling_invocation(&self, _input: &Value) -> bool {
        self.polling
    }

    fn category(&self) -> ToolCategory {
        self.category
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        if let Some((steer_at, inbox)) = &self.steer_on_call
            && call == *steer_at
        {
            inbox.lock().unwrap().push_back("new direction".to_string());
        }
        ToolResult::text("unchanged")
    }
}

struct ConstantErrorTool {
    name: &'static str,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

struct DiagnosticImageErrorTool;

struct SuccessfulImageTool;

#[derive(Default)]
struct DeliveredMediaOutput;

#[derive(Default)]
struct FailedMediaOutput;

impl OutputSink for DeliveredMediaOutput {
    fn emit_text_delta(&self, _: &str, _: &str) {}
    fn emit_thinking(&self, _: &str, _: &str) {}
    fn emit_tool_call(&self, _: &str, _: &str, _: &str) {}
    fn emit_tool_result(&self, _: &str, _: &str, _: bool, _: &str) {}
    fn emit_tool_result_with_images_and_artifact_identity(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: bool,
        _: &str,
        _: &[nomi_types::tool::ToolImage],
    ) -> ToolMediaDelivery {
        ToolMediaDelivery::Delivered {
            context: "Verified artifact receipt: nomifun-artifacts/image.png".to_owned(),
        }
    }
    fn emit_stream_start(&self, _: &str) {}
    fn emit_stream_end(&self, _: &str, _: usize, _: u64, _: u64, _: u64, _: u64) {}
    fn emit_error(&self, _: &str) {}
    fn emit_info(&self, _: &str) {}
}

impl OutputSink for FailedMediaOutput {
    fn emit_text_delta(&self, _: &str, _: &str) {}
    fn emit_thinking(&self, _: &str, _: &str) {}
    fn emit_tool_call(&self, _: &str, _: &str, _: &str) {}
    fn emit_tool_result(&self, _: &str, _: &str, _: bool, _: &str) {}
    fn emit_tool_result_with_images_and_artifact_identity(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: bool,
        _: &str,
        _: &[nomi_types::tool::ToolImage],
    ) -> ToolMediaDelivery {
        ToolMediaDelivery::Failed {
            error: "durable image persistence failed".to_owned(),
        }
    }
    fn emit_stream_start(&self, _: &str) {}
    fn emit_stream_end(&self, _: &str, _: usize, _: u64, _: u64, _: u64, _: u64) {}
    fn emit_error(&self, _: &str) {}
    fn emit_info(&self, _: &str) {}
}

struct StrictImageThenStopProvider {
    requests: Mutex<Vec<LlmRequest>>,
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl LlmProvider for StrictImageThenStopProvider {
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        self.requests.lock().unwrap().push(request.clone());
        let turn = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        if turn == 0 {
            tx.send(LlmEvent::ToolUse {
                id: "image-call".to_owned(),
                name: "image_gen".to_owned(),
                input: serde_json::json!({"prompt": "fox"}),
                extra: None,
            })
            .await
            .unwrap();
            tx.send(LlmEvent::Done {
                stop_reason: nomi_types::message::StopReason::ToolUse,
                usage: Default::default(),
            })
            .await
            .unwrap();
        } else {
            tx.send(LlmEvent::Done {
                stop_reason: nomi_types::message::StopReason::EndTurn,
                usage: Default::default(),
            })
            .await
            .unwrap();
        }
        Ok(rx)
    }
}

struct RequiredKbIdTool {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

struct StringifiedNestedArgsTool {
    seen_inputs: Arc<Mutex<Vec<Value>>>,
}

#[async_trait::async_trait]
impl Tool for StringifiedNestedArgsTool {
    fn name(&self) -> &str {
        "delegate_proxy"
    }

    fn description(&self) -> &str {
        "root union schema fixture matching a dynamic delegation proxy"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "$defs": {
                "Planned": {
                    "type": "object",
                    "properties": {
                        "strategy": {"type": "string", "const": "planned"},
                        "goal": {"type": "string"}
                    },
                    "required": ["strategy", "goal"],
                    "additionalProperties": false
                },
                "Parallel": {
                    "type": "object",
                    "properties": {
                        "strategy": {"type": "string", "const": "parallel"},
                        "tasks": {
                            "type": "array",
                            "minItems": 1,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "name": {"type": "string"},
                                    "prompt": {"type": "string"}
                                },
                                "required": ["name", "prompt"],
                                "additionalProperties": false
                            }
                        },
                        "synthesize": {"type": "boolean"}
                    },
                    "required": ["strategy", "tasks"],
                    "additionalProperties": false
                }
            },
            "type": "object",
            "properties": {},
            "anyOf": [
                {"$ref": "#/$defs/Planned"},
                {"$ref": "#/$defs/Parallel"}
            ]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }

    async fn execute(&self, input: Value) -> ToolResult {
        self.seen_inputs.lock().unwrap().push(input);
        ToolResult::text("accepted")
    }
}

#[async_trait::async_trait]
impl Tool for RequiredKbIdTool {
    fn name(&self) -> &str {
        "knowledge_search"
    }

    fn description(&self) -> &str {
        "schema preview lifecycle fixture"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": { "kb_id": { "type": "string", "minLength": 1 } },
            "required": ["kb_id"],
            "additionalProperties": false
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ToolResult::text("should not execute")
    }
}

#[async_trait::async_trait]
impl Tool for ConstantErrorTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "test tool returning an error"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({"type": "object"})
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ToolResult::error("missing required field")
    }
}

#[async_trait::async_trait]
impl Tool for DiagnosticImageErrorTool {
    fn name(&self) -> &str {
        "noop"
    }

    fn description(&self) -> &str {
        "test tool returning an error with diagnostic image bytes"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({"type": "object"})
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        ToolResult::error("diagnostic failure").with_images(vec![
            nomi_types::tool::ToolImage {
                media_type: "image/png".to_owned(),
                data: "ZGlhZ25vc3RpYw==".to_owned(),
            },
        ])
    }
}

#[async_trait::async_trait]
impl Tool for SuccessfulImageTool {
    fn name(&self) -> &str {
        "image_gen"
    }

    fn description(&self) -> &str {
        "test image generator"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({"type": "object"})
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }

    fn requires_explicit_route(&self) -> bool {
        true
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        ToolResult::text("generated").with_images(vec![nomi_types::tool::ToolImage {
            media_type: "image/png".to_owned(),
            data: "aW1hZ2UtYnl0ZXM=".to_owned(),
        }])
    }
}

/// A real deferred catalog entry used to exercise ToolSearch through the
/// complete AgentEngine dispatch path.
struct DeferredProbeTool;

#[async_trait::async_trait]
impl Tool for DeferredProbeTool {
    fn name(&self) -> &str {
        "deferred_probe"
    }

    fn description(&self) -> &str {
        "deferred loop-guard probe"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({"type": "object"})
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        ToolResult::text("probe")
    }
}

struct InputLoopProvider {
    calls: std::sync::atomic::AtomicUsize,
    tool_turns: usize,
    tool_name: &'static str,
    input: Value,
}

#[async_trait::async_trait]
impl LlmProvider for InputLoopProvider {
    async fn stream(
        &self,
        _: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        let turn = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        if turn < self.tool_turns {
            let _ = tx
                .send(LlmEvent::ToolUse {
                    id: format!("input-loop-{turn}"),
                    name: self.tool_name.to_string(),
                    input: self.input.clone(),
                    extra: None,
                })
                .await;
            let _ = tx
                .send(LlmEvent::Done {
                    stop_reason: nomi_types::message::StopReason::ToolUse,
                    usage: Default::default(),
                })
                .await;
        } else {
            let _ = tx
                .send(LlmEvent::Done {
                    stop_reason: nomi_types::message::StopReason::EndTurn,
                    usage: Default::default(),
                })
                .await;
        }
        Ok(rx)
    }
}

enum InputLoopSemantics {
    WriteStdin,
    ReadOnlyAction(&'static str),
}

struct InputClassifiedLoopTool {
    name: &'static str,
    semantics: InputLoopSemantics,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl Tool for InputClassifiedLoopTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "input-sensitive loop-guard test tool"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "number" },
                "chars": { "type": "string" },
                "action": { "type": "string" }
            }
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn is_polling_invocation(&self, input: &Value) -> bool {
        matches!(self.semantics, InputLoopSemantics::WriteStdin)
            && match input.get("chars") {
                None => true,
                Some(Value::String(chars)) => chars.is_empty(),
                Some(_) => false,
            }
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }

    fn category_for(&self, input: &Value) -> ToolCategory {
        match self.semantics {
            InputLoopSemantics::ReadOnlyAction(action)
                if input.get("action").and_then(Value::as_str) == Some(action) =>
            {
                ToolCategory::Info
            }
            _ => ToolCategory::Exec,
        }
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ToolResult::text("unchanged")
    }
}

/// Turn 1 issues a single tool call (then ToolUse stop); turn 2 ends the
/// turn (EndTurn stop, no tools). Used to verify steering injection point A
/// rides along the tool-result message.
struct ToolThenStopProvider {
    calls: std::sync::atomic::AtomicUsize,
    request_image_counts: Option<Arc<Mutex<Vec<usize>>>>,
}

struct NamedToolThenStopProvider {
    calls: std::sync::atomic::AtomicUsize,
    provider_name: String,
    requests: Mutex<Vec<LlmRequest>>,
}

#[async_trait::async_trait]
impl LlmProvider for NamedToolThenStopProvider {
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        self.requests.lock().unwrap().push(request.clone());
        let turn = self
            .calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        if turn == 0 {
            tx.send(LlmEvent::ToolUse {
                id: "artifact-call".to_owned(),
                name: self.provider_name.clone(),
                input: serde_json::json!({}),
                extra: None,
            })
            .await
            .unwrap();
            tx.send(LlmEvent::Done {
                stop_reason: nomi_types::message::StopReason::ToolUse,
                usage: Default::default(),
            })
            .await
            .unwrap();
        } else {
            tx.send(LlmEvent::Done {
                stop_reason: nomi_types::message::StopReason::EndTurn,
                usage: Default::default(),
            })
            .await
            .unwrap();
        }
        Ok(rx)
    }
}
#[async_trait::async_trait]
impl LlmProvider for ToolThenStopProvider {
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        if let Some(counts) = &self.request_image_counts {
            let image_count = request
                .messages
                .iter()
                .flat_map(|message| &message.content)
                .map(|block| match block {
                    ContentBlock::ToolResult { images, .. } => images.len(),
                    _ => 0,
                })
                .sum();
            counts.lock().unwrap().push(image_count);
        }
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        if n == 0 {
            let _ = tx
                .send(LlmEvent::ToolUse {
                    id: "t1".to_string(),
                    name: "noop".to_string(),
                    input: serde_json::json!({}),
                    extra: None,
                })
                .await;
            let _ = tx
                .send(LlmEvent::Done {
                    stop_reason: nomi_types::message::StopReason::ToolUse,
                    usage: Default::default(),
                })
                .await;
        } else {
            let _ = tx
                .send(LlmEvent::Done {
                    stop_reason: nomi_types::message::StopReason::EndTurn,
                    usage: Default::default(),
                })
                .await;
        }
        Ok(rx)
    }
}

fn make_engine(model: &str) -> super::AgentEngine {
    super::AgentEngine {
        provider: Arc::new(NullProvider),
        tools: ToolRegistry::new(),
        messages: vec![],
        system_prompt: String::new(),
        model: model.to_string(),
        max_tokens: 4096,
        max_turns: Some(10),
        total_usage: Default::default(),
        thinking: None,
        compat: nomi_config::compat::ProviderCompat::anthropic_defaults(),
        confirmer: Arc::new(Mutex::new(ToolConfirmer::new(true, vec![]))),
        hooks: None,
        session_manager: None,
        current_session: None,
        output: Arc::new(NullSink),
        current_msg_id: String::new(),
        approval_manager: None,
        protocol_writer: None,
        allow_list: vec![],
        current_reasoning_effort: None,
        compact_config: nomi_config::compact::CompactConfig::default(),
        compact_state: super::CompactState::new(),
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

#[test]
fn context_accessors_report_window_and_last_input() {
    let mut engine = make_engine("ctx-accessors");
    assert_eq!(engine.context_window(), engine.compact_config.context_window as u64);
    engine.compact_state.last_input_tokens = 12_345;
    assert_eq!(engine.context_tokens(), 12_345);
}

#[tokio::test]
async fn system_resource_notice_is_system_context_not_a_user_message() {
    let mut engine = make_engine("resource-notice");
    let provider = Arc::new(RecordingProvider::successful());
    engine.provider = provider.clone();
    engine.system_prompt = "base system".to_owned();
    let inbox = Arc::new(Mutex::new(std::collections::VecDeque::from([
        "terminal term-1 was closed by the user".to_owned(),
    ])));
    engine.set_system_resource_inbox(Some(inbox.clone()));

    engine
        .execute_turn("continue the task", "msg-resource-notice")
        .await
        .expect("resource notice should not prevent the real user turn");

    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].system.contains(SYSTEM_RESOURCE_CONTEXT_HEADER));
    assert!(
        requests[0]
            .system
            .contains("terminal term-1 was closed by the user")
    );
    assert!(
        requests[0].messages.iter().all(|message| {
            message.content.iter().all(|block| {
                !matches!(
                    block,
                    ContentBlock::Text { text }
                        if text.contains("terminal term-1 was closed by the user")
                )
            })
        }),
        "trusted resource state must never be serialized as a conversation message"
    );
    assert!(inbox.lock().unwrap().is_empty());

    engine
        .execute_turn("next real user turn", "msg-after-resource-notice")
        .await
        .unwrap();
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        !requests[1].system.contains(SYSTEM_RESOURCE_CONTEXT_HEADER),
        "a consumed notice should not be replayed forever"
    );
}

#[tokio::test]
async fn execute_turn_with_content_sends_image_once_then_redacts_it_from_history() {
    let mut engine = make_engine("vision-model");
    let provider = Arc::new(RecordingProvider::successful());
    engine.provider = provider.clone();
    let result = engine
        .execute_turn_with_content(
            vec![
                ContentBlock::Text {
                    text: "What is in this image?".into(),
                },
                ContentBlock::Image {
                    media_type: "image/png".into(),
                    data: "cG5n".into(),
                },
            ],
            "msg-vision",
        )
        .await
        .expect("multimodal turn should run");

    assert_eq!(result.turns, 1);
    let first_requests = provider.requests();
    assert_eq!(first_requests.len(), 1);
    assert!(first_requests[0].messages.iter().any(|message| {
        message.role == Role::User
            && message.content.iter().any(|block| {
                matches!(
                    block,
                    ContentBlock::Image { media_type, data }
                        if media_type == "image/png" && data == "cG5n"
                )
            })
    }));

    assert_eq!(engine.messages[0].role, Role::User);
    assert!(engine.messages[0]
        .content
        .iter()
        .all(|block| !matches!(block, ContentBlock::Image { .. })));
    assert!(engine.messages[0].content.iter().any(|block| {
        matches!(block, ContentBlock::Text { text } if text == USER_IMAGE_HISTORY_PLACEHOLDER)
    }));

    engine
        .execute_turn("What about its color?", "msg-follow-up")
        .await
        .expect("follow-up turn should run");
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].messages.iter().all(|message| {
        message
            .content
            .iter()
            .all(|block| !matches!(block, ContentBlock::Image { .. }))
    }));
    assert!(requests[1].messages.iter().any(|message| {
        message.content.iter().any(|block| {
            matches!(block, ContentBlock::Text { text } if text == USER_IMAGE_HISTORY_PLACEHOLDER)
        })
    }));
}

#[tokio::test]
async fn execute_turn_with_content_redacts_user_image_after_provider_error() {
    let mut engine = make_engine("vision-model");
    let provider = Arc::new(RecordingProvider::failing());
    engine.provider = provider.clone();

    let error = engine
        .execute_turn_with_content(
            vec![
                ContentBlock::Text {
                    text: "Inspect this image.".into(),
                },
                ContentBlock::Image {
                    media_type: "image/png".into(),
                    data: "cG5n".into(),
                },
            ],
            "msg-provider-error",
        )
        .await
        .expect_err("the provider failure should surface");

    assert!(matches!(error, super::AgentError::Provider(_)));
    assert_eq!(provider.requests().len(), 1);
    assert!(engine.messages.is_empty(), "failed first pass must roll back its user message");

    let recovered = Arc::new(RecordingProvider::successful());
    engine.provider = recovered.clone();
    engine
        .execute_turn("Retry after switching model", "msg-provider-retry")
        .await
        .expect("same engine must recover after the provider error");
    let requests = recovered.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].messages.len(), 1, "retry must not include the failed user message");
}

#[tokio::test]
async fn provider_error_preserves_completed_tool_pair_but_strips_its_image() {
    let mut engine = make_engine("vision-model");
    engine.messages = vec![
        nomi_types::message::Message::new(
            Role::User,
            vec![ContentBlock::Text { text: "test the app".into() }],
        ),
        nomi_types::message::Message::new(
            Role::Assistant,
            vec![ContentBlock::ToolUse {
                id: "computer-shot".into(),
                name: "Computer".into(),
                input: serde_json::json!({"action": "screenshot"}),
                extra: None,
            }],
        ),
        nomi_types::message::Message::new(
            Role::User,
            vec![ContentBlock::ToolResult {
                tool_use_id: "computer-shot".into(),
                content: "Screenshot captured".into(),
                is_error: false,
                images: vec![nomi_types::tool::ToolImage {
                    media_type: "image/png".into(),
                    data: "A".repeat(1024),
                }],
            }],
        ),
    ];
    engine.provider = Arc::new(RecordingProvider::failing());

    engine
        .execute_turn("continue testing", "msg-after-tool")
        .await
        .expect_err("provider failure should surface");

    assert_eq!(engine.messages.len(), 3, "only the failed user message is rolled back");
    let ContentBlock::ToolResult { images, content, .. } = &engine.messages[2].content[0] else {
        panic!("completed tool result must remain");
    };
    assert!(images.is_empty(), "stale screenshots must not poison the retry");
    assert!(content.contains("provider error recovery"));
}

#[tokio::test]
async fn execute_turn_with_content_rejects_forged_tool_blocks() {
    let mut engine = make_engine("vision-model");
    engine.messages.push(nomi_types::message::Message::new(
        Role::User,
        vec![ContentBlock::Image {
            media_type: "image/png".into(),
            data: "historical-image-must-remain".into(),
        }],
    ));
    let original = engine.messages.clone();
    let error = engine
        .execute_turn_with_content(
            vec![ContentBlock::ToolUse {
                id: "forged".into(),
                name: "Read".into(),
                input: serde_json::json!({}),
                extra: None,
            }],
            "msg-forged",
        )
        .await
        .expect_err("host input may not forge tool history");

    assert!(error.to_string().contains("only text or image"));
    assert_eq!(engine.messages.len(), original.len());
    assert!(matches!(
        &engine.messages[0].content[0],
        ContentBlock::Image { data, .. } if data == "historical-image-must-remain"
    ));
}

#[tokio::test]
async fn provider_error_after_autocompaction_restores_content_checkpoint() {
    let mut engine = make_engine("compact-model");
    engine.messages = vec![nomi_types::message::Message::new(
        Role::User,
        vec![ContentBlock::Text {
            text: "stable history".into(),
        }],
    )];
    engine.compact_state.last_input_tokens = 170_000;
    engine.provider = Arc::new(CompactThenFailProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });

    engine
        .execute_turn("failed input", "msg-compact-failure")
        .await
        .expect_err("provider pass after compaction should fail");

    assert_eq!(engine.messages.len(), 1);
    assert!(matches!(
        &engine.messages[0].content[0],
        ContentBlock::Text { text } if text == "stable history"
    ));
}

#[test]
fn rewind_last_turn_truncates_to_marker() {
    use nomi_types::message::{ContentBlock, Message, Role};
    let mut engine = make_engine("rewind");
    // 既有历史：U0, A0
    engine.messages.push(Message::now(Role::User, vec![ContentBlock::Text { text: "u0".into() }]));
    engine.messages.push(Message::now(Role::Assistant, vec![ContentBlock::Text { text: "a0".into() }]));
    let prior_host_context = std::collections::BTreeMap::from([(
        "nomifun.image_generation.route".to_owned(),
        "native".to_owned(),
    )]);
    engine.host_context = prior_host_context.clone();
    // 标记最后一个 turn 起始 = 当前长度(2)，再 push U1（被中断的 turn）
    engine.editable_turn = Some(EditableTurnCheckpoint {
        source_message_id: "message-u1".into(),
        start_len: engine.messages.len(),
        prior_host_context: prior_host_context.clone(),
    });
    engine.host_context.insert(
        "nomifun.image_generation.route".to_owned(),
        "explicit_external".to_owned(),
    );
    engine.messages.push(Message::now(Role::User, vec![ContentBlock::Text { text: "u1".into() }]));
    assert_eq!(engine.messages.len(), 3);

    assert!(engine.can_rewind_last_turn("message-u1"));
    assert!(engine.rewind_last_turn("message-u1"));
    assert_eq!(engine.messages.len(), 2); // U1 被回退
    assert!(engine.editable_turn.is_none()); // 锚点被消费
    assert_eq!(engine.host_context, prior_host_context);

    // 再次回退无锚点 → false
    assert!(!engine.rewind_last_turn("message-u1"));
}

#[test]
fn rewind_last_turn_rejects_stale_marker() {
    let mut engine = make_engine("rewind-stale");
    // 锚点越界（如压缩后未清理的极端情况）→ 拒绝
    engine.editable_turn = Some(EditableTurnCheckpoint {
        source_message_id: "message-stale".into(),
        start_len: 5,
        prior_host_context: Default::default(),
    });
    assert!(!engine.rewind_last_turn("message-stale"));
}

#[test]
fn rewind_last_turn_allows_only_an_empty_checkpointless_transcript() {
    use nomi_types::message::{ContentBlock, Message, Role};

    let mut empty = make_engine("rewind-empty-no-checkpoint");
    assert!(empty.can_rewind_last_turn("message-empty"));
    assert!(empty.rewind_last_turn("message-empty"));
    assert!(empty.messages.is_empty());
    assert!(empty.editable_turn.is_none());

    let mut non_empty = make_engine("rewind-nonempty-no-checkpoint");
    non_empty.messages.push(Message::now(
        Role::User,
        vec![ContentBlock::Text {
            text: "legacy history".into(),
        }],
    ));
    assert!(!non_empty.can_rewind_last_turn("message-legacy"));
    assert!(!non_empty.rewind_last_turn("message-legacy"));
    assert_eq!(non_empty.messages.len(), 1);
}

#[tokio::test]
async fn continuation_passes_keep_the_root_user_checkpoint() {
    let mut engine = make_engine("rewind-continuation");
    engine.provider = Arc::new(RecordingProvider::successful());

    engine
        .execute_turn_with_content_for_source(
            vec![ContentBlock::Text {
                text: "root request".into(),
            }],
            "wire-segment-1",
            "message-root",
        )
        .await
        .unwrap();
    assert_eq!(
        engine.editable_turn,
        Some(EditableTurnCheckpoint {
            source_message_id: "message-root".into(),
            start_len: 0,
            prior_host_context: Default::default(),
        })
    );

    engine
        .execute_turn_with_content_for_source(
            vec![ContentBlock::Text {
                text: "automatic continuation".into(),
            }],
            "wire-segment-2",
            "message-root",
        )
        .await
        .unwrap();
    assert_eq!(
        engine.editable_turn.as_ref().map(|checkpoint| checkpoint.start_len),
        Some(0),
        "a continuation must not move the root turn boundary"
    );

    let next_root_start = engine.messages.len();
    engine
        .execute_turn_with_content_for_source(
            vec![ContentBlock::Text {
                text: "next user request".into(),
            }],
            "wire-segment-3",
            "message-next",
        )
        .await
        .unwrap();
    assert_eq!(
        engine.editable_turn,
        Some(EditableTurnCheckpoint {
            source_message_id: "message-next".into(),
            start_len: next_root_start,
            prior_host_context: Default::default(),
        })
    );
    assert!(!engine.can_rewind_last_turn("message-root"));
    assert!(engine.can_rewind_last_turn("message-next"));
}

#[tokio::test]
async fn turn_tool_allowlist_is_exact_and_does_not_leak_to_the_next_turn() {
    let provider = Arc::new(RecordingProvider::successful());
    let mut engine = make_engine("turn-tool-route");
    engine.provider = provider.clone();
    assert!(engine.tools.register(Box::new(SuccessfulImageTool)));
    assert!(engine.tools.register(Box::new(ConstantResultTool {
        name: "browser",
        polling: false,
        category: ToolCategory::Exec,
        calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        steer_on_call: None,
    })));

    let image_only = std::collections::HashSet::from(["image_gen".to_owned()]);
    engine
        .execute_turn_with_content_for_source_and_tool_allowlist(
            vec![ContentBlock::Text {
                text: "generate an image".into(),
            }],
            "wire-image",
            "message-image",
            Some(&image_only),
        )
        .await
        .unwrap();
    engine
        .execute_turn_with_content_for_source(
            vec![ContentBlock::Text {
                text: "open a website".into(),
            }],
            "wire-normal",
            "message-normal",
        )
        .await
        .unwrap();

    let requests = provider.requests();
    let first_names = requests[0]
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(first_names, vec!["image_gen"]);
    let second_names = requests[1]
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(second_names, std::collections::HashSet::from(["browser"]));
}

#[tokio::test]
async fn strict_route_prefixes_authority_over_hidden_knowledge_tool_promises() {
    let provider = Arc::new(RecordingProvider::successful());
    let mut engine = make_engine("strict-tool-authority");
    engine.provider = provider.clone();
    engine.system_prompt = "## Knowledge bases (extended knowledge source)\nCall the `knowledge_search` tool BEFORE answering, then call `knowledge_read`.".to_owned();
    assert!(engine.tools.register(Box::new(SuccessfulImageTool)));
    let image_only = std::collections::HashSet::from(["image_gen".to_owned()]);

    engine
        .execute_turn_with_content_for_source_and_tool_allowlist(
            vec![ContentBlock::Text {
                text: "generate an image".into(),
            }],
            "wire-strict-authority",
            "message-strict-authority",
            Some(&image_only),
        )
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec!["image_gen"]
    );
    assert!(requests[0].system.starts_with(REQUEST_SCOPED_TOOL_AUTHORITY_HEADER));
    assert!(requests[0].system.contains(REQUEST_SCOPED_TOOL_AUTHORITY_RULE));
    assert!(
        requests[0]
            .system
            .contains("Declared tools for this request: `image_gen`")
    );
    assert!(
        requests[0].system.contains("knowledge_search"),
        "the authority rule must override hidden promises without brittle prompt-string removal"
    );
}

#[test]
fn deterministic_host_turn_is_persisted_with_a_rewind_checkpoint() {
    let mut engine = make_engine("host-turn");
    engine
        .record_host_text_turn("generate a fox", "configure an image model", "message-host")
        .unwrap();

    assert_eq!(engine.messages.len(), 2);
    assert_eq!(engine.messages[0].role, Role::User);
    assert_eq!(engine.messages[1].role, Role::Assistant);
    assert_eq!(
        engine.editable_turn,
        Some(EditableTurnCheckpoint {
            source_message_id: "message-host".into(),
            start_len: 0,
            prior_host_context: Default::default(),
        })
    );
    assert!(engine.can_rewind_last_turn("message-host"));
}

fn make_engine_with_compat(
    model: &str,
    compat: nomi_config::compat::ProviderCompat,
) -> super::AgentEngine {
    let mut engine = make_engine(model);
    engine.compat = compat;
    engine
}

#[tokio::test]
async fn stagnation_guard_caps_unchanged_loop() {
    // A model stuck in an unchanged tool-call/result loop should terminate
    // at the stagnation guard well before the generic 200-turn safety net.
    let mut engine = make_engine("safety-net-model");
    engine.max_turns = None;
    engine.provider = Arc::new(LoopProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    engine.tools.register(Box::new(ConstantResultTool {
        name: "noop",
        polling: false,
        category: ToolCategory::Exec,
        calls: Arc::clone(&tool_calls),
        steer_on_call: None,
    }));

    let res = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        engine.execute_turn("go", "msg-safety"),
    )
    .await
    .expect("engine.execute_turn must terminate via the stagnation guard, not hang forever");

    assert!(
        matches!(res, Err(AgentError::Stagnation(_))),
        "an unchanged non-polling cycle must fail closed: {res:?}"
    );
    assert_eq!(tool_calls.load(std::sync::atomic::Ordering::SeqCst), 6);
}

#[tokio::test]
async fn alternating_all_failed_tools_with_assistant_filler_are_bounded() {
    let mut engine = make_engine("alternating-failure-model");
    engine.max_turns = None;
    engine.provider = Arc::new(AlternatingFailureProvider {
        turns: std::sync::atomic::AtomicUsize::new(0),
    });
    let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for name in ["create", "update"] {
        engine.tools.register(Box::new(ConstantErrorTool {
            name,
            calls: Arc::clone(&tool_calls),
        }));
    }

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        engine.execute_turn("keep trying", "msg-alternating-failures"),
    )
    .await
    .expect("alternating failures must terminate via the stagnation guard");

    assert!(
        matches!(result, Err(AgentError::Stagnation(_))),
        "assistant filler and alternating tool names must not evade all-failed detection: {result:?}"
    );
    assert_eq!(
        tool_calls.load(std::sync::atomic::Ordering::SeqCst),
        6,
        "the guard must nudge at 3 all-failed turns and abort at 6"
    );
}

#[tokio::test]
async fn repeated_tool_search_is_bounded_by_stagnation_guard() {
    let mut engine = make_engine("tool-search-loop-model");
    engine.max_turns = None;
    let provider = Arc::new(InputLoopProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
        tool_turns: usize::MAX,
        tool_name: "ToolSearch",
        input: serde_json::json!({"query": "deferred_probe"}),
    });
    engine.provider = provider.clone();

    let deferred_state = engine.tools.deferred_state();
    engine
        .tools
        .register(Box::new(nomi_tools::tool_search::ToolSearchTool::new(
            deferred_state,
        )));
    engine.tools.register(Box::new(DeferredProbeTool));

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        engine.execute_turn("find the probe", "msg-tool-search-loop"),
    )
    .await
    .expect("ToolSearch loop must terminate before the generic turn cap");

    assert!(
        matches!(result, Err(AgentError::Stagnation(_))),
        "an unchanged ToolSearch result must fail closed: {result:?}"
    );
    assert_eq!(
        provider.calls.load(std::sync::atomic::Ordering::SeqCst),
        6,
        "the real ToolSearch dispatch path must nudge at 3 and abort at 6"
    );
}

#[tokio::test]
async fn polling_invocations_can_repeat_unchanged_until_external_progress() {
    let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut engine = make_engine("polling-model");
    engine.max_turns = None;
    engine.provider = Arc::new(FiniteLoopProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
        tool_turns: 7,
        tool_name: "poll",
    });
    engine.tools.register(Box::new(ConstantResultTool {
        name: "poll",
        polling: true,
        category: ToolCategory::Exec,
        calls: Arc::clone(&tool_calls),
        steer_on_call: None,
    }));

    let result = engine
        .execute_turn("wait until ready", "msg-poll")
        .await
        .expect("unchanged polling is not stagnation");

    assert_eq!(result.turns, 8);
    assert_eq!(tool_calls.load(std::sync::atomic::Ordering::SeqCst), 7);
    assert!(!engine.messages.iter().any(|message| {
        message.content.iter().any(|block| {
            matches!(block, ContentBlock::Text { text } if text == crate::loop_guard::STAGNATION_NUDGE)
        })
    }));
}

#[tokio::test]
async fn provider_turn_accepts_exactly_128_tool_calls() {
    let executed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut engine = make_engine("max-tool-burst-model");
    engine.provider = Arc::new(ToolBurstProvider {
        turns: std::sync::atomic::AtomicUsize::new(0),
        tool_calls: MAX_PROVIDER_TURN_TOOL_CALLS,
        tool_name: "burst",
    });
    engine.tools.register(Box::new(ConstantResultTool {
        name: "burst",
        polling: true,
        category: ToolCategory::Info,
        calls: Arc::clone(&executed),
        steer_on_call: None,
    }));

    let result = engine
        .execute_turn("run the burst", "msg-max-tool-burst")
        .await
        .expect("the exact per-turn tool-call limit must be accepted");

    assert_eq!(result.stop_reason, nomi_types::message::StopReason::EndTurn);
    assert_eq!(result.turns, 2);
    assert_eq!(
        executed.load(std::sync::atomic::Ordering::SeqCst),
        MAX_PROVIDER_TURN_TOOL_CALLS
    );
}

#[tokio::test]
async fn provider_turn_rejects_129_tool_calls_before_any_execution() {
    let executed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut engine = make_engine("oversized-tool-burst-model");
    engine.provider = Arc::new(ToolBurstProvider {
        turns: std::sync::atomic::AtomicUsize::new(0),
        tool_calls: MAX_PROVIDER_TURN_TOOL_CALLS + 1,
        tool_name: "burst",
    });
    engine.tools.register(Box::new(ConstantResultTool {
        name: "burst",
        polling: true,
        category: ToolCategory::Info,
        calls: Arc::clone(&executed),
        steer_on_call: None,
    }));

    let result = engine
        .execute_turn("run the oversized burst", "msg-oversized-tool-burst")
        .await;

    assert!(
        matches!(
            &result,
            Err(AgentError::ApiError(message))
                if message.contains("exceeded the maximum of 128 complete tool calls")
        ),
        "the 129th complete tool call must fail the provider turn: {result:?}"
    );
    assert_eq!(
        executed.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "an oversized provider turn must fail before any tool dispatch"
    );
}

#[tokio::test]
async fn whitespace_variant_tool_ids_are_rejected_before_any_dispatch() {
    let executed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let output = Arc::new(ToolLifecycleRecordingOutput::default());
    let mut engine = make_engine("whitespace-tool-id-model");
    engine.output = output.clone();
    engine.provider = Arc::new(FixedToolCallsProvider {
        calls: vec![("x", "noop"), (" x ", "noop")],
        as_deltas: false,
    });
    engine.tools.register(Box::new(ConstantResultTool {
        name: "noop",
        polling: false,
        category: ToolCategory::Exec,
        calls: Arc::clone(&executed),
        steer_on_call: None,
    }));

    let result = engine
        .execute_turn("run both calls", "msg-whitespace-tool-id")
        .await;

    assert!(
        matches!(
            &result,
            Err(AgentError::ApiError(message))
                if message.contains("tool_use_id has leading or trailing whitespace")
        ),
        "whitespace-equivalent IDs must fail the entire provider turn: {result:?}"
    );
    assert_eq!(
        executed.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "neither the canonical nor whitespace-variant call may dispatch"
    );
    assert_eq!(output.tool_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn tool_names_with_surrounding_whitespace_are_rejected_before_dispatch() {
    let executed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let output = Arc::new(ToolLifecycleRecordingOutput::default());
    let mut engine = make_engine("whitespace-tool-name-model");
    engine.output = output.clone();
    engine.provider = Arc::new(FixedToolCallsProvider {
        calls: vec![("x", " noop ")],
        as_deltas: false,
    });
    engine.tools.register(Box::new(ConstantResultTool {
        name: "noop",
        polling: false,
        category: ToolCategory::Exec,
        calls: Arc::clone(&executed),
        steer_on_call: None,
    }));

    let result = engine
        .execute_turn("run the call", "msg-whitespace-tool-name")
        .await;

    assert!(
        matches!(
            &result,
            Err(AgentError::ApiError(message))
                if message.contains("tool name for call 'x' has leading or trailing whitespace")
        ),
        "a tool name with surrounding whitespace must fail closed: {result:?}"
    );
    assert_eq!(executed.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(output.tool_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn invalid_tool_delta_identifiers_fail_before_their_preview_or_dispatch() {
    let cases = [
        (
            "whitespace-id",
            vec![("x", "noop"), (" x ", "noop")],
            "tool progress tool_use_id has leading or trailing whitespace",
        ),
        (
            "empty-id",
            vec![("", "noop")],
            "has an empty tool_use_id",
        ),
        (
            "whitespace-name",
            vec![("x", " noop ")],
            "tool progress name for call 'x' has leading or trailing whitespace",
        ),
        (
            "empty-name",
            vec![("x", "")],
            "tool progress 'x' has an empty name",
        ),
    ];

    for (case, calls, expected_error) in cases {
        let executed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut engine = make_engine("invalid-tool-delta-model");
        engine.provider = Arc::new(FixedToolCallsProvider {
            calls,
            as_deltas: true,
        });
        engine.tools.register(Box::new(ConstantResultTool {
            name: "noop",
            polling: false,
            category: ToolCategory::Exec,
            calls: Arc::clone(&executed),
            steer_on_call: None,
        }));

        let result = engine
            .execute_turn("stream the call", "msg-invalid-tool-delta")
            .await;

        assert!(
            matches!(&result, Err(AgentError::ApiError(message)) if message.contains(expected_error)),
            "{case} must fail closed before emitting its invalid preview: {result:?}"
        );
        assert_eq!(
            executed.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "{case} must never dispatch a tool"
        );
    }
}

#[tokio::test]
async fn tool_delta_identity_must_match_the_completed_call() {
    let cases = [
        (
            "changed-name",
            ("x", "first"),
            ("x", "second"),
            "changed its previewed name",
        ),
        (
            "changed-id",
            ("x", "first"),
            ("y", "first"),
            "had no matching completed ToolUse before Done",
        ),
    ];

    for (case, preview, complete, expected_error) in cases {
        let executed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let output = Arc::new(ToolLifecycleRecordingOutput::default());
        let mut engine = make_engine("mismatched-tool-preview-model");
        engine.output = output.clone();
        engine.provider = Arc::new(PreviewThenCompleteProvider {
            turns: std::sync::atomic::AtomicUsize::new(0),
            preview,
            complete,
        });
        for name in ["first", "second"] {
            engine.tools.register(Box::new(ConstantResultTool {
                name,
                polling: false,
                category: ToolCategory::Exec,
                calls: Arc::clone(&executed),
                steer_on_call: None,
            }));
        }

        let result = engine
            .execute_turn("stream the call", "msg-mismatched-tool-preview")
            .await;

        assert!(
            matches!(&result, Err(AgentError::ApiError(message)) if message.contains(expected_error)),
            "{case} must fail at the provider commit boundary: {result:?}"
        );
        assert_eq!(
            executed.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "{case} must fail before dispatch"
        );
        assert_eq!(
            output.tool_calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "{case} must fail before Running publication"
        );
    }
}

#[tokio::test]
async fn unadvertised_tool_delta_fails_before_running_preview() {
    let output = Arc::new(ToolLifecycleRecordingOutput::default());
    let mut engine = make_engine("unadvertised-tool-delta-model");
    engine.output = output.clone();
    engine.provider = Arc::new(FixedToolCallsProvider {
        calls: vec![("x", "not_advertised")],
        as_deltas: true,
    });

    let result = engine
        .execute_turn("stream the call", "msg-unadvertised-tool-delta")
        .await;

    assert!(
        matches!(&result, Err(AgentError::ApiError(message)) if message.contains("was not advertised in this request")),
        "an unadvertised delta must fail at the provider boundary: {result:?}"
    );
    assert_eq!(
        output.tool_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "an unauthorized call must never enter the Running preview lifecycle"
    );
}

#[tokio::test]
async fn deferred_tool_delta_does_not_create_running_preview() {
    let output = Arc::new(ToolLifecycleRecordingOutput::default());
    let mut engine = make_engine("deferred-tool-delta-model");
    engine.output = output.clone();
    assert!(engine.tools.register(Box::new(DeferredProbeTool)));
    engine.provider = Arc::new(FixedToolCallsProvider {
        calls: vec![("x", "deferred_probe")],
        as_deltas: true,
    });

    let result = engine
        .execute_turn("stream the call", "msg-deferred-tool-delta")
        .await;

    assert!(
        matches!(&result, Err(AgentError::ApiError(message)) if message.contains("had no matching completed ToolUse before Done")),
        "an unfinished deferred preview must fail terminal reconciliation: {result:?}"
    );
    assert_eq!(
        output.tool_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a deferred discovery stub must not enter the Running preview lifecycle"
    );
}

#[tokio::test]
async fn nested_stringified_tool_fields_are_canonical_before_running_and_dispatch() {
    let output = Arc::new(ToolLifecycleRecordingOutput::default());
    let seen_inputs = Arc::new(Mutex::new(Vec::new()));
    let mut engine = make_engine("stringified-nested-fields-model");
    engine.output = output.clone();
    engine.provider = Arc::new(InputLoopProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
        tool_turns: 1,
        tool_name: "delegate_proxy",
        input: serde_json::json!({
            "strategy": "parallel",
            "tasks": "[{\"name\":\"research\",\"prompt\":\"inspect\"}]",
            "synthesize": "True"
        }),
    });
    assert!(engine.tools.register(Box::new(StringifiedNestedArgsTool {
        seen_inputs: seen_inputs.clone(),
    })));

    engine
        .execute_turn("delegate work", "msg-stringified-nested-fields")
        .await
        .expect("schema-directed nested normalization should let the call execute");

    assert_eq!(
        output.tool_calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the validated canonical call must enter the Running lifecycle once"
    );
    assert_eq!(
        output.tool_results.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    let published = output.tool_inputs.lock().unwrap();
    assert_eq!(published.len(), 1);
    assert!(published[0]["tasks"].is_array());
    assert_eq!(published[0]["synthesize"], true);
    drop(published);

    let dispatched = seen_inputs.lock().unwrap();
    assert_eq!(dispatched.len(), 1);
    assert!(dispatched[0]["tasks"].is_array());
    assert_eq!(dispatched[0]["synthesize"], true);
}

#[tokio::test]
async fn schema_invalid_tool_call_emits_error_without_running_preview_or_dispatch() {
    let output = Arc::new(ToolLifecycleRecordingOutput::default());
    let executed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut engine = make_engine("schema-invalid-preview-model");
    engine.output = output.clone();
    engine.provider = Arc::new(InputLoopProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
        tool_turns: 1,
        tool_name: "knowledge_search",
        input: serde_json::json!({}),
    });
    assert!(engine.tools.register(Box::new(RequiredKbIdTool {
        calls: executed.clone(),
    })));

    engine
        .execute_turn("search knowledge", "msg-schema-invalid-preview")
        .await
        .expect("the local schema error must be returned to the model for correction");

    assert_eq!(
        output.tool_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "invalid arguments must never create a Running preview"
    );
    assert_eq!(
        output.tool_results.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "invalid arguments must still produce one paired local error result"
    );
    assert_eq!(executed.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn missing_required_field_delta_never_emits_running_preview() {
    let output = Arc::new(ToolLifecycleRecordingOutput::default());
    let executed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut engine = make_engine("schema-invalid-delta-model");
    engine.output = output.clone();
    engine.provider = Arc::new(PreviewThenCompleteProvider {
        turns: std::sync::atomic::AtomicUsize::new(0),
        preview: ("x", "knowledge_search"),
        complete: ("x", "knowledge_search"),
    });
    assert!(engine.tools.register(Box::new(RequiredKbIdTool {
        calls: executed.clone(),
    })));

    engine
        .execute_turn("search knowledge", "msg-schema-invalid-delta")
        .await
        .expect("the paired local schema error must let the model turn recover");

    assert_eq!(
        output.tool_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "an uncommitted delta with missing required fields must never be Running"
    );
    assert_eq!(output.tool_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(output.tool_results.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(executed.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn whole_string_tool_arguments_are_rejected_before_dispatch() {
    let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut engine = make_engine("stringified-write-stdin-model");
    engine.max_turns = None;
    engine.provider = Arc::new(InputLoopProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
        tool_turns: 7,
        tool_name: "write_stdin",
        input: Value::String(r#"{"session_id":7,"chars":"status"}"#.to_string()),
    });
    engine.tools.register(Box::new(InputClassifiedLoopTool {
        name: "write_stdin",
        semantics: InputLoopSemantics::WriteStdin,
        calls: Arc::clone(&tool_calls),
    }));

    let result = engine
        .execute_turn("send status", "msg-stringified-write-stdin")
        .await;

    assert!(
        matches!(&result, Err(AgentError::ApiError(message)) if message.contains("arguments are not a JSON object")),
        "a provider must not bypass the structured argument contract with a JSON string: {result:?}"
    );
    assert_eq!(tool_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn whole_string_read_arguments_are_rejected_without_execution() {
    for (tool_name, action) in [("Browser", "observe"), ("Computer", "screenshot")] {
        let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut engine = make_engine("stringified-read-only-model");
        engine.max_turns = None;
        engine.provider = Arc::new(InputLoopProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            tool_turns: 7,
            tool_name,
            input: Value::String(format!(r#"{{"action":"{action}"}}"#)),
        });
        engine.tools.register(Box::new(InputClassifiedLoopTool {
            name: tool_name,
            semantics: InputLoopSemantics::ReadOnlyAction(action),
            calls: Arc::clone(&tool_calls),
        }));

        let result = engine
            .execute_turn("observe", &format!("msg-stringified-{tool_name}"))
            .await;

        assert!(
            matches!(&result, Err(AgentError::ApiError(message)) if message.contains("arguments are not a JSON object")),
            "{tool_name} must reject a provider's whole-object JSON string: {result:?}"
        );
        assert_eq!(tool_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn unchanged_read_only_calls_nudge_then_abort() {
    let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut engine = make_engine("read-only-monitor-model");
    engine.max_turns = None;
    engine.provider = Arc::new(FiniteLoopProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
        tool_turns: 7,
        tool_name: "external_status",
    });
    engine.tools.register(Box::new(ConstantResultTool {
        name: "external_status",
        polling: false,
        category: ToolCategory::Info,
        calls: Arc::clone(&tool_calls),
        steer_on_call: None,
    }));

    let result = engine
        .execute_turn("monitor until ready", "msg-read-only-monitor")
        .await;

    assert!(
        matches!(result, Err(AgentError::Stagnation(_))),
        "read-only without explicit polling semantics must be bounded: {result:?}"
    );
    assert_eq!(tool_calls.load(std::sync::atomic::Ordering::SeqCst), 6);
    assert!(engine.messages.iter().any(|message| {
        message.content.iter().any(|block| {
            matches!(block, ContentBlock::Text { text } if text == crate::loop_guard::STAGNATION_NUDGE)
        })
    }));
}

#[tokio::test]
async fn steering_cancels_a_stale_stagnation_abort() {
    let inbox = Arc::new(Mutex::new(std::collections::VecDeque::new()));
    let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut engine = make_engine("steered-loop-model");
    engine.max_turns = None;
    engine.provider = Arc::new(FiniteLoopProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
        tool_turns: 6,
        tool_name: "constant",
    });
    engine.tools.register(Box::new(ConstantResultTool {
        name: "constant",
        polling: false,
        category: ToolCategory::Exec,
        calls: Arc::clone(&tool_calls),
        steer_on_call: Some((6, Arc::clone(&inbox))),
    }));
    engine.set_steering_inbox(Some(inbox));

    let result = engine
        .execute_turn("start", "msg-steered-loop")
        .await
        .expect("a steer arriving on the aborting outcome must reset the guard");

    assert_eq!(result.turns, 7);
    assert_eq!(tool_calls.load(std::sync::atomic::Ordering::SeqCst), 6);
    assert!(engine.messages.iter().any(|message| {
        message
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::Text { text } if text == "new direction"))
    }));
}

#[tokio::test]
async fn steering_extends_a_would_end_turn() {
    // NullProvider makes every turn a no-tool turn that would END. A steer
    // message present at turn-end must extend the turn by one (point B),
    // appended as a fresh User message (assistant→user ordering is valid).
    let mut engine = make_engine("steer-b");
    let inbox = std::sync::Arc::new(std::sync::Mutex::new(
        std::collections::VecDeque::from(["please also do X".to_string()]),
    ));
    engine.set_steering_inbox(Some(inbox.clone()));

    let res = engine
        .execute_turn("go", "m-b")
        .await
        .expect("engine.execute_turn ok");

    assert_eq!(res.turns, 2, "the steer message extends the turn by one");
    // [User "go", Assistant[], User "please also do X", Assistant[]]
    assert_eq!(engine.messages.len(), 4);
    let injected = &engine.messages[2];
    assert_eq!(injected.role, Role::User);
    assert!(
        injected
            .content
            .iter()
            .any(|b| matches!(b, ContentBlock::Text { text } if text == "please also do X")),
        "injected user message must carry the steer text"
    );
    assert!(inbox.lock().unwrap().is_empty(), "inbox drained");
}

#[tokio::test]
async fn steering_rides_along_tool_result_message() {
    // Turn 1 issues a tool call; the steer message must be appended as a
    // trailing Text block ON the tool-result User message (point A) — never
    // as a second consecutive User message.
    let mut engine = make_engine("steer-a");
    engine.provider = std::sync::Arc::new(ToolThenStopProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
        request_image_counts: None,
    });
    engine.tools.register(Box::new(ConstantResultTool {
        name: "noop",
        polling: false,
        category: ToolCategory::Info,
        calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        steer_on_call: None,
    }));
    let inbox = std::sync::Arc::new(std::sync::Mutex::new(
        std::collections::VecDeque::from(["wait, focus on Y".to_string()]),
    ));
    engine.set_steering_inbox(Some(inbox.clone()));

    let res = engine
        .execute_turn("go", "m-a")
        .await
        .expect("engine.execute_turn ok");

    assert_eq!(res.turns, 2);
    // messages: [User "go", Assistant[ToolUse], User[ToolResult, Text "wait, focus on Y"], Assistant[]]
    let tool_result_msg = &engine.messages[2];
    assert_eq!(tool_result_msg.role, Role::User);
    assert!(
        tool_result_msg
            .content
            .iter()
            .any(|b| matches!(b, ContentBlock::Text { text } if text == "wait, focus on Y")),
        "steer text must ride along the tool-result message"
    );
    for w in engine.messages.windows(2) {
        assert!(
            !(w[0].role == Role::User && w[1].role == Role::User),
            "must not create consecutive user messages"
        );
    }
    assert!(inbox.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_tool_diagnostic_images_are_not_replayed_to_the_provider() {
    let mut engine = make_engine("diagnostic-image-error");
    let request_image_counts = Arc::new(Mutex::new(Vec::new()));
    engine.provider = Arc::new(ToolThenStopProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
        request_image_counts: Some(request_image_counts.clone()),
    });
    engine.tools.register(Box::new(DiagnosticImageErrorTool));

    engine
        .execute_turn("run the diagnostic tool", "m-diagnostic-image")
        .await
        .expect("a handled tool error should reach the second provider turn");

    assert_eq!(
        *request_image_counts.lock().unwrap(),
        vec![0, 0],
        "diagnostic bytes from a failed tool must not enter the next request"
    );
    let ContentBlock::ToolResult {
        is_error, images, ..
    } = &engine.messages[2].content[0]
    else {
        panic!("the third message should contain the failed tool result");
    };
    assert!(*is_error);
    assert!(images.is_empty());
}

#[tokio::test]
async fn delivered_image_bytes_are_replaced_by_receipt_context_without_a_redundant_model_pass() {
    let mut engine = make_engine("delivered-image");
    let provider = Arc::new(StrictImageThenStopProvider {
        requests: Mutex::new(Vec::new()),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    engine.provider = provider.clone();
    engine.output = Arc::new(DeliveredMediaOutput);
    engine.tools.register(Box::new(SuccessfulImageTool));
    let allowlist = std::collections::HashSet::from(["image_gen".to_owned()]);

    engine
        .execute_turn_with_content_for_source_and_tool_allowlist(
            vec![ContentBlock::Text {
                text: "generate a fox".to_owned(),
            }],
            "m-delivered-image",
            "root-delivered-image",
            Some(&allowlist),
        )
        .await
        .expect("verified image delivery should return directly to the host commit gate");

    let requests = provider.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "a paid artifact result must not depend on a redundant text-only provider pass"
    );
    assert_eq!(requests[0].tools.len(), 1);
    drop(requests);

    let ContentBlock::ToolResult {
        content, images, ..
    } = &engine.messages[2].content[0]
    else {
        panic!("session history should retain the compact tool result");
    };
    assert!(content.contains("Verified artifact receipt"));
    assert!(images.is_empty(), "base64 image bytes must never persist in session history");
}

#[tokio::test]
async fn strict_image_delivery_failure_stops_before_an_empty_tool_provider_pass() {
    let mut engine = make_engine("failed-image-delivery");
    let provider = Arc::new(StrictImageThenStopProvider {
        requests: Mutex::new(Vec::new()),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    engine.provider = provider.clone();
    engine.output = Arc::new(FailedMediaOutput);
    engine.tools.register(Box::new(SuccessfulImageTool));
    let allowlist = std::collections::HashSet::from(["image_gen".to_owned()]);

    engine
        .execute_turn_with_content_for_source_and_tool_allowlist(
            vec![ContentBlock::Text {
                text: "generate a fox".to_owned(),
            }],
            "m-failed-image-delivery",
            "root-failed-image-delivery",
            Some(&allowlist),
        )
        .await
        .expect("the engine phase must return control to the host receipt gate");

    let requests = provider.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "artifact failure must not trigger a prose pass with tools=[]"
    );
    assert_eq!(requests[0].tools.len(), 1);
    drop(requests);

    let ContentBlock::ToolResult {
        content, is_error, ..
    } = &engine.messages[2].content[0]
    else {
        panic!("session history should retain the failed tool result");
    };
    assert!(*is_error);
    assert!(content.contains("Artifact delivery failed"));
}

#[tokio::test]
async fn bounded_mcp_alias_uses_untruncated_export_identity_and_rejects_text_only_success() {
    for semantic_tool in ["export_pdf", "render_video"] {
        // This is the shape of a 64-byte MCP provider alias after a long
        // server slug consumed the readable budget: neither output product
        // survives in the provider-facing name.
        let provider_name =
            "mcp__enterprise_content_gateway_with_a_very_l__abcdefghijklmnop".to_owned();
        assert!(!provider_name.contains(semantic_tool));
        let artifact_identity = format!(
            "enterprise_content_gateway_with_a_very_long_origin_name__{semantic_tool}"
        );
        let output = Arc::new(ArtifactIdentityOutput::default());
        let mut engine = make_engine("mcp-artifact-identity");
        let provider = Arc::new(NamedToolThenStopProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            provider_name: provider_name.clone(),
            requests: Mutex::new(Vec::new()),
        });
        engine.provider = provider.clone();
        engine.output = output.clone();
        engine.tools.register(Box::new(ArtifactIdentityTool {
            provider_name,
            artifact_identity: artifact_identity.clone(),
        }));

        engine
            .execute_turn("create the requested artifact", "m-mcp-artifact")
            .await
            .expect("a contract failure is a handled tool result");

        assert_eq!(
            *output.running_identities.lock().unwrap(),
            vec![artifact_identity.clone()]
        );
        assert_eq!(
            *output.result_identities.lock().unwrap(),
            vec![artifact_identity]
        );
        let ContentBlock::ToolResult {
            is_error,
            images,
            content,
            ..
        } = &engine.messages[2].content[0]
        else {
            panic!("the third message should contain the exporter result");
        };
        assert!(*is_error, "text-only {semantic_tool} must not remain successful");
        assert!(images.is_empty());
        assert!(content.contains("Artifact delivery failed"));
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].tools.len(), 1);
        assert!(
            requests[1].tools.is_empty(),
            "the handled contract failure must close artifact execution authority for the accepted turn"
        );
    }
}

#[test]
fn set_config_changes_model() {
    let mut engine = make_engine("old-model");
    let changes = engine.apply_config_update(Some("new-model".into()), None, None, None, None);
    assert_eq!(engine.model, "new-model");
    assert_eq!(changes.len(), 1);
    assert!(changes[0].contains("old-model"));
    assert!(changes[0].contains("new-model"));
}

#[test]
fn set_config_none_model_no_change() {
    let mut engine = make_engine("current");
    let changes = engine.apply_config_update(None, None, None, None, None);
    assert_eq!(engine.model, "current");
    assert!(changes.is_empty());
}

#[test]
fn set_config_same_model_still_reports_change() {
    let mut engine = make_engine("same");
    let changes = engine.apply_config_update(Some("same".into()), None, None, None, None);
    assert_eq!(changes.len(), 1);
}

#[test]
fn set_config_empty_string_model_accepted() {
    let mut engine = make_engine("real-model");
    engine.apply_config_update(Some(String::new()), None, None, None, None);
    assert_eq!(engine.model, "");
}

#[test]
fn set_config_model_does_not_affect_other_state() {
    let mut engine = make_engine("m");
    engine.current_reasoning_effort = Some("high".into());
    engine.apply_config_update(Some("new-m".into()), None, None, None, None);
    assert_eq!(engine.model, "new-m");
    assert_eq!(engine.current_reasoning_effort.as_deref(), Some("high"));
}

// --- Cycle 2: Effort config tests ---

#[test]
fn set_config_changes_effort() {
    let mut engine =
        make_engine_with_compat("m", nomi_config::compat::ProviderCompat::openai_defaults());
    assert!(engine.current_reasoning_effort.is_none());
    let changes = engine.apply_config_update(None, None, None, Some("high".into()), None);
    assert_eq!(engine.current_reasoning_effort.as_deref(), Some("high"));
    assert_eq!(changes.len(), 1);
    assert!(changes[0].contains("high"));
}

#[test]
fn set_config_clears_effort_with_empty_string() {
    let mut engine = make_engine("m");
    engine.current_reasoning_effort = Some("high".into());
    let changes = engine.apply_config_update(None, None, None, Some(String::new()), None);
    assert!(engine.current_reasoning_effort.is_none());
    assert_eq!(changes.len(), 1);
}

// --- Cycle 2: Thinking config tests ---

#[test]
fn set_config_enables_thinking() {
    let mut engine = make_engine("m");
    let changes =
        engine.apply_config_update(None, Some("enabled".into()), Some(16000), None, None);
    match &engine.thinking {
        Some(nomi_types::llm::ThinkingConfig::Enabled { budget_tokens }) => {
            assert_eq!(*budget_tokens, 16000);
        }
        other => panic!("expected Enabled, got: {other:?}"),
    }
    assert_eq!(changes.len(), 1);
}

#[test]
fn set_config_disables_thinking() {
    let mut engine = make_engine("m");
    engine.thinking = Some(nomi_types::llm::ThinkingConfig::Enabled {
        budget_tokens: 8000,
    });
    let changes = engine.apply_config_update(None, Some("disabled".into()), None, None, None);
    match &engine.thinking {
        Some(nomi_types::llm::ThinkingConfig::Disabled) => {}
        other => panic!("expected Disabled, got: {other:?}"),
    }
    assert_eq!(changes.len(), 1);
}

#[test]
fn set_config_thinking_enabled_default_budget() {
    let mut engine = make_engine("m");
    let changes = engine.apply_config_update(None, Some("enabled".into()), None, None, None);
    match &engine.thinking {
        Some(nomi_types::llm::ThinkingConfig::Enabled { budget_tokens }) => {
            assert!(*budget_tokens > 0);
        }
        other => panic!("expected Enabled with default budget, got: {other:?}"),
    }
    assert_eq!(changes.len(), 1);
}

#[test]
fn set_config_invalid_thinking_ignored() {
    let mut engine = make_engine("m");
    engine.thinking = Some(nomi_types::llm::ThinkingConfig::Enabled {
        budget_tokens: 8000,
    });
    let changes =
        engine.apply_config_update(None, Some("invalid_value".into()), None, None, None);
    match &engine.thinking {
        Some(nomi_types::llm::ThinkingConfig::Enabled { budget_tokens }) => {
            assert_eq!(*budget_tokens, 8000);
        }
        other => panic!("expected Enabled unchanged, got: {other:?}"),
    }
    assert_eq!(changes.len(), 1);
    assert!(changes[0].contains("invalid") || changes[0].contains("ignored"));
}

// --- Cycle 2: Combined fields test ---

#[test]
fn set_config_all_fields_at_once() {
    let compat = nomi_config::compat::ProviderCompat {
        supports_thinking: Some(true),
        supports_effort: Some(true),
        effort_levels: Some(vec!["low".into()]),
        ..Default::default()
    };
    let mut engine = make_engine_with_compat("old-model", compat);
    let changes = engine.apply_config_update(
        Some("new-model".into()),
        Some("enabled".into()),
        Some(12000),
        Some("low".into()),
        None,
    );
    assert_eq!(engine.model, "new-model");
    assert_eq!(engine.current_reasoning_effort.as_deref(), Some("low"));
    match &engine.thinking {
        Some(nomi_types::llm::ThinkingConfig::Enabled { budget_tokens }) => {
            assert_eq!(*budget_tokens, 12000);
        }
        other => panic!("expected Enabled, got: {other:?}"),
    }
    assert_eq!(changes.len(), 3);
}

// --- Cycle 2: White-box edge case tests ---

#[test]
fn set_config_thinking_budget_only_updates_existing_enabled() {
    let mut engine = make_engine("m");
    engine.thinking = Some(nomi_types::llm::ThinkingConfig::Enabled {
        budget_tokens: 5000,
    });
    let changes = engine.apply_config_update(None, None, Some(20000), None, None);
    match &engine.thinking {
        Some(nomi_types::llm::ThinkingConfig::Enabled { budget_tokens }) => {
            assert_eq!(*budget_tokens, 20000);
        }
        other => panic!("expected Enabled with 20000, got: {other:?}"),
    }
    assert_eq!(changes.len(), 1);
}

#[test]
fn set_config_thinking_budget_ignored_when_disabled() {
    let mut engine = make_engine("m");
    engine.thinking = Some(nomi_types::llm::ThinkingConfig::Disabled);
    let changes = engine.apply_config_update(None, None, Some(20000), None, None);
    match &engine.thinking {
        Some(nomi_types::llm::ThinkingConfig::Disabled) => {}
        other => panic!("expected Disabled unchanged, got: {other:?}"),
    }
    assert!(changes.is_empty());
}

#[test]
fn set_config_effort_valid_values() {
    let compat = nomi_config::compat::ProviderCompat {
        supports_effort: Some(true),
        effort_levels: Some(vec![
            "low".into(),
            "medium".into(),
            "high".into(),
            "max".into(),
        ]),
        ..Default::default()
    };
    for value in ["low", "medium", "high", "max"] {
        let mut engine = make_engine_with_compat("m", compat.clone());
        engine.apply_config_update(None, None, None, Some(value.to_string()), None);
        assert_eq!(
            engine.current_reasoning_effort.as_deref(),
            Some(value),
            "effort should be set to {value}"
        );
    }
}

// --- Capability validation tests ---

#[test]
fn set_config_thinking_rejected_when_unsupported() {
    let mut engine =
        make_engine_with_compat("m", nomi_config::compat::ProviderCompat::openai_defaults());
    let changes = engine.apply_config_update(None, Some("enabled".into()), None, None, None);
    assert!(changes.iter().any(|c| c.contains("not supported")));
    assert!(engine.thinking.is_none());
}

#[test]
fn set_config_effort_rejected_when_unsupported() {
    let mut engine = make_engine("m"); // anthropic defaults: supports_effort = false
    let changes = engine.apply_config_update(None, None, None, Some("high".into()), None);
    assert!(changes.iter().any(|c| c.contains("not supported")));
    assert!(engine.current_reasoning_effort.is_none());
}

#[test]
fn set_config_effort_rejected_invalid_level() {
    let mut engine =
        make_engine_with_compat("m", nomi_config::compat::ProviderCompat::openai_defaults());
    let changes = engine.apply_config_update(None, None, None, Some("max".into()), None);
    assert!(changes.iter().any(|c| c.contains("invalid")));
    assert!(engine.current_reasoning_effort.is_none());
}

#[test]
fn set_config_effort_clear_always_works() {
    let mut engine = make_engine("m"); // anthropic defaults: supports_effort = false
    engine.current_reasoning_effort = Some("high".into());
    let changes = engine.apply_config_update(None, None, None, Some(String::new()), None);
    assert!(engine.current_reasoning_effort.is_none());
    assert!(changes.iter().any(|c| c.contains("cleared")));
}
