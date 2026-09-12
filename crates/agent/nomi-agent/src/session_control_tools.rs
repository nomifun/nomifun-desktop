//! Native, current-session-only control tools for the in-process Nomi agent.
//!
//! The backend supplies a [`SessionControlSink`] already bound to the runtime's
//! authenticated owner and Conversation/AgentSession. Model-visible input can
//! therefore describe only the operation; it can never select an owner,
//! session, execution, or child handle.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use nomi_protocol::events::ToolCategory;
use nomi_tools::{Tool, ToolExecutionContext};
use nomi_types::tool::{JsonSchema, ToolResult};

pub const AGENT_EXECUTION_OBSERVE_TOOL_NAME: &str = "agent_execution_observe";
pub const AGENT_EXECUTION_STEER_TOOL_NAME: &str = "agent_execution_steer";
pub const AGENT_FORK_TOOL_NAME: &str = "agent_fork";

const DEFAULT_OBSERVE_LIMIT: u32 = 50;
const MAX_OBSERVE_LIMIT: u32 = 100;
const MAX_STEER_BYTES: usize = 16 * 1024;
const MAX_FORK_TITLE_BYTES: usize = 256;

/// Host boundary for operations on the exact AgentSession that owns the
/// currently executing Nomi runtime.
///
/// Implementations must retain the owner/session binding captured at runtime
/// construction. In particular, no implementation may resolve a target from
/// free-form model input.
#[async_trait]
pub trait SessionControlSink: Send + Sync {
    async fn observe(&self, after_seq: u64, limit: u32) -> Result<Value, String>;

    async fn steer(&self, message: &str, operation_id: &str) -> Result<Value, String>;

    async fn fork(&self, title: Option<&str>, operation_id: &str) -> Result<Value, String>;
}

fn ok(value: Value) -> ToolResult {
    ToolResult {
        content: value.to_string(),
        is_error: false,
        images: Vec::new(),
    }
}

fn err(code: &str, message: &str, retry_safe: bool) -> ToolResult {
    ToolResult {
        content: json!({
            "code": code,
            "message": message,
            "retry_safe": retry_safe,
        })
        .to_string(),
        is_error: true,
        images: Vec::new(),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(input: Value, tool: &str) -> Result<T, ToolResult> {
    serde_json::from_value(input).map_err(|_| {
        tracing::debug!(tool, "session control tool rejected an invalid payload");
        err(
            "INVALID_PAYLOAD",
            "The AgentSession control request is invalid.",
            false,
        )
    })
}

fn sink_error(
    operation: &str,
    code: &str,
    message: &'static str,
    retry_safe: bool,
    internal_error: String,
) -> ToolResult {
    let internal_error = nomi_redact::redact_secrets_owned(internal_error);
    tracing::warn!(
        operation,
        error = %internal_error,
        "AgentSession control owner failed"
    );
    err(code, message, retry_safe)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObserveInput {
    #[serde(default)]
    after_seq: u64,
    #[serde(default = "default_observe_limit")]
    limit: u32,
}

fn default_observe_limit() -> u32 {
    DEFAULT_OBSERVE_LIMIT
}

pub struct AgentExecutionObserveTool {
    sink: Arc<dyn SessionControlSink>,
}

impl AgentExecutionObserveTool {
    pub fn new(sink: Arc<dyn SessionControlSink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl Tool for AgentExecutionObserveTool {
    fn name(&self) -> &str {
        AGENT_EXECUTION_OBSERVE_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Observe the durable status and committed messages of this current AgentSession. The target session is fixed by the host and cannot be selected."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "after_seq": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Return committed messages after this sequence number."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_OBSERVE_LIMIT,
                    "default": DEFAULT_OBSERVE_LIMIT
                }
            },
            "additionalProperties": false
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let input: ObserveInput = match parse(input, self.name()) {
            Ok(input) => input,
            Err(error) => return error,
        };
        if input.limit == 0 || input.limit > MAX_OBSERVE_LIMIT {
            return err(
                "INVALID_PAYLOAD",
                "The observation limit must be between 1 and 100.",
                false,
            );
        }
        match self.sink.observe(input.after_seq, input.limit).await {
            Ok(value) => ok(value),
            Err(error) => sink_error(
                "observe",
                "AGENT_EXECUTION_OBSERVE_FAILED",
                "The current AgentSession could not be observed.",
                true,
                error,
            ),
        }
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SteerInput {
    message: String,
}

pub struct AgentExecutionSteerTool {
    sink: Arc<dyn SessionControlSink>,
}

impl AgentExecutionSteerTool {
    pub fn new(sink: Arc<dyn SessionControlSink>) -> Self {
        Self { sink }
    }

    async fn execute_inner(
        &self,
        input: Value,
        context: Option<&ToolExecutionContext>,
    ) -> ToolResult {
        let input: SteerInput = match parse(input, self.name()) {
            Ok(input) => input,
            Err(error) => return error,
        };
        let message = input.message.trim();
        if message.is_empty() {
            return err(
                "INVALID_PAYLOAD",
                "The steering message must not be empty.",
                false,
            );
        }
        if message.len() > MAX_STEER_BYTES {
            return err(
                "INVALID_PAYLOAD",
                "The steering message exceeds the safe size limit.",
                false,
            );
        }
        let Some(context) = context else {
            return err(
                "AGENT_SESSION_CONTEXT_REQUIRED",
                "The steering operation requires an engine-owned invocation identity.",
                false,
            );
        };
        match self.sink.steer(message, context.operation_id()).await {
            Ok(value) => ok(value),
            Err(error) => sink_error(
                "steer",
                "AGENT_EXECUTION_STEER_FAILED",
                "The current AgentSession could not be steered.",
                false,
                error,
            ),
        }
    }
}

#[async_trait]
impl Tool for AgentExecutionSteerTool {
    fn name(&self) -> &str {
        AGENT_EXECUTION_STEER_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Inject a durable steering message into the active turn of this current AgentSession. The host fixes both owner and session."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_STEER_BYTES,
                    "description": "Instruction to apply at the next safe model boundary of the active turn."
                }
            },
            "required": ["message"],
            "additionalProperties": false
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, input: Value) -> ToolResult {
        self.execute_inner(input, None).await
    }

    async fn execute_with_context(
        &self,
        input: Value,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        self.execute_inner(input, Some(context)).await
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ForkInput {
    #[serde(default)]
    title: Option<String>,
}

pub struct AgentForkTool {
    sink: Arc<dyn SessionControlSink>,
}

impl AgentForkTool {
    pub fn new(sink: Arc<dyn SessionControlSink>) -> Self {
        Self { sink }
    }

    async fn execute_inner(
        &self,
        input: Value,
        context: Option<&ToolExecutionContext>,
    ) -> ToolResult {
        let input: ForkInput = match parse(input, self.name()) {
            Ok(input) => input,
            Err(error) => return error,
        };
        let title = input
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty());
        if title.is_some_and(|title| title.len() > MAX_FORK_TITLE_BYTES) {
            return err(
                "INVALID_PAYLOAD",
                "The fork title exceeds the safe size limit.",
                false,
            );
        }
        let Some(context) = context else {
            return err(
                "AGENT_SESSION_CONTEXT_REQUIRED",
                "The fork operation requires an engine-owned invocation identity.",
                false,
            );
        };
        match self.sink.fork(title, context.operation_id()).await {
            Ok(value) => ok(value),
            Err(error) => sink_error(
                "fork",
                "AGENT_FORK_FAILED",
                "The current AgentSession could not be forked.",
                false,
                error,
            ),
        }
    }
}

#[async_trait]
impl Tool for AgentForkTool {
    fn name(&self) -> &str {
        AGENT_FORK_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Create a new isolated AgentSession from the durable committed history of this current AgentSession. The parent owner, parent session, and inherited Agent binding are fixed by the host."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "maxLength": MAX_FORK_TITLE_BYTES,
                    "description": "Optional title for the new child AgentSession."
                }
            },
            "additionalProperties": false
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, input: Value) -> ToolResult {
        self.execute_inner(input, None).await
    }

    async fn execute_with_context(
        &self,
        input: Value,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        self.execute_inner(input, Some(context)).await
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct RecordingSink {
        observes: Mutex<Vec<(u64, u32)>>,
        steers: Mutex<Vec<(String, String)>>,
        forks: Mutex<Vec<(Option<String>, String)>>,
    }

    struct LeakyFailureSink;

    #[async_trait]
    impl SessionControlSink for RecordingSink {
        async fn observe(&self, after_seq: u64, limit: u32) -> Result<Value, String> {
            self.observes.lock().unwrap().push((after_seq, limit));
            Ok(json!({"agent_session_id": "host-bound", "after_seq": after_seq}))
        }

        async fn steer(&self, message: &str, operation_id: &str) -> Result<Value, String> {
            self.steers
                .lock()
                .unwrap()
                .push((message.to_owned(), operation_id.to_owned()));
            Ok(json!({"status": "accepted"}))
        }

        async fn fork(&self, title: Option<&str>, operation_id: &str) -> Result<Value, String> {
            self.forks
                .lock()
                .unwrap()
                .push((title.map(ToOwned::to_owned), operation_id.to_owned()));
            Ok(json!({"child_agent_session_id": "host-created"}))
        }
    }

    #[async_trait]
    impl SessionControlSink for LeakyFailureSink {
        async fn observe(&self, _after_seq: u64, _limit: u32) -> Result<Value, String> {
            Err(private_internal_error())
        }

        async fn steer(&self, _message: &str, _operation_id: &str) -> Result<Value, String> {
            Err(private_internal_error())
        }

        async fn fork(&self, _title: Option<&str>, _operation_id: &str) -> Result<Value, String> {
            Err(private_internal_error())
        }
    }

    fn private_internal_error() -> String {
        "database request to https://session-owner.internal/private failed; api_key=sk-012345678901234567890123"
            .to_owned()
    }

    fn execution_context() -> ToolExecutionContext {
        ToolExecutionContext::from_scoped_tool_call("turn-1", "call-1")
    }

    #[tokio::test]
    async fn observe_is_bounded_and_has_no_model_selected_target() {
        let sink = Arc::new(RecordingSink::default());
        let tool = AgentExecutionObserveTool::new(sink.clone());
        let schema = tool.input_schema().to_string();
        assert!(!schema.contains("session_id"));
        assert!(!schema.contains("owner"));

        let result = tool.execute(json!({"after_seq": 7, "limit": 12})).await;
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(sink.observes.lock().unwrap().as_slice(), &[(7, 12)]);

        let rejected = tool.execute(json!({"session_id": "other"})).await;
        assert!(rejected.is_error);
        assert_eq!(sink.observes.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn steer_uses_engine_identity_and_rejects_target_override() {
        let sink = Arc::new(RecordingSink::default());
        let tool = AgentExecutionSteerTool::new(sink.clone());
        let schema = tool.input_schema().to_string();
        assert!(!schema.contains("session_id"));
        assert!(!schema.contains("owner"));

        let rejected = tool
            .execute_with_context(
                json!({"message": "continue", "session_id": "other"}),
                &execution_context(),
            )
            .await;
        assert!(rejected.is_error);
        assert!(sink.steers.lock().unwrap().is_empty());

        let context = execution_context();
        let result = tool
            .execute_with_context(json!({"message": " continue "}), &context)
            .await;
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(
            sink.steers.lock().unwrap().as_slice(),
            &[("continue".to_owned(), context.operation_id().to_owned())]
        );
    }

    #[tokio::test]
    async fn fork_inherits_host_scope_and_uses_engine_identity() {
        let sink = Arc::new(RecordingSink::default());
        let tool = AgentForkTool::new(sink.clone());
        let schema = tool.input_schema().to_string();
        assert!(!schema.contains("parent_session"));
        assert!(!schema.contains("agent_binding"));
        assert!(!schema.contains("owner"));

        let rejected = tool
            .execute_with_context(
                json!({"title": "child", "owner_id": "other"}),
                &execution_context(),
            )
            .await;
        assert!(rejected.is_error);
        assert!(sink.forks.lock().unwrap().is_empty());

        let context = execution_context();
        let result = tool
            .execute_with_context(json!({"title": " child "}), &context)
            .await;
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(
            sink.forks.lock().unwrap().as_slice(),
            &[(
                Some("child".to_owned()),
                context.operation_id().to_owned()
            )]
        );
    }

    #[tokio::test]
    async fn sink_diagnostics_never_enter_model_visible_control_errors() {
        let sink: Arc<dyn SessionControlSink> = Arc::new(LeakyFailureSink);
        let context = execution_context();
        let results = [
            AgentExecutionObserveTool::new(Arc::clone(&sink))
                .execute(json!({"after_seq": 0, "limit": 1}))
                .await,
            AgentExecutionSteerTool::new(Arc::clone(&sink))
                .execute_with_context(json!({"message": "continue"}), &context)
                .await,
            AgentForkTool::new(sink)
                .execute_with_context(json!({"title": "child"}), &context)
                .await,
        ];
        let expected_codes = [
            "AGENT_EXECUTION_OBSERVE_FAILED",
            "AGENT_EXECUTION_STEER_FAILED",
            "AGENT_FORK_FAILED",
        ];
        for (result, expected_code) in results.into_iter().zip(expected_codes) {
            assert!(result.is_error);
            let payload: Value = serde_json::from_str(&result.content).unwrap();
            assert_eq!(payload["code"], expected_code);
            for forbidden in [
                "session-owner.internal",
                "api_key",
                "sk-012345678901234567890123",
                "database request",
            ] {
                assert!(
                    !result.content.contains(forbidden),
                    "model-visible session control error leaked {forbidden}: {}",
                    result.content
                );
            }
        }
    }
}
