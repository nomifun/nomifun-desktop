//! Parent-Session-scoped controls for background delegated Agents.
//!
//! `agent.delegate` remains the only spawn authority. A trusted host wraps that
//! tool with [`DelegationHandleRecordingTool`] and records the durable child
//! identities returned by the delegate owner. The model receives only opaque
//! handles; owner, AgentSession, execution, and step identities never appear in
//! the `subagent_send` or `subagent_wait` input contracts.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;

use nomi_protocol::events::ToolCategory;
use nomi_tools::{Tool, ToolExecutionContext};
use nomi_types::tool::{JsonSchema, ToolResult};

pub const SUBAGENT_SEND_TOOL_NAME: &str = "subagent_send";
pub const SUBAGENT_WAIT_TOOL_NAME: &str = "subagent_wait";
pub const SUBAGENT_SEND_OUTCOME_UNKNOWN_CODE: &str = "SUBAGENT_SEND_OUTCOME_UNKNOWN";

pub const MAX_SUBAGENT_HANDLES: usize = 16;
pub const MAX_SUBAGENT_WAIT_HANDLES: usize = 8;
pub const MAX_SUBAGENT_MESSAGE_BYTES: usize = 16 * 1024;
pub const MAX_SUBAGENT_RESULT_BYTES: usize = 8 * 1024;
pub const MAX_SUBAGENT_WAIT_MS: u64 = 30_000;

const SUBAGENT_MAILBOX_CAPACITY: usize = 8;
#[cfg(not(test))]
const SUBAGENT_SEND_ACK_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const SUBAGENT_SEND_ACK_TIMEOUT: Duration = Duration::from_millis(25);
const DEFAULT_SUBAGENT_WAIT_MS: u64 = 10_000;
const MAX_HANDLE_BYTES: usize = 128;

/// One host-owned child discovered from a successful `agent.delegate` receipt.
///
/// `child_id` is private host state. Only the generated opaque handle is ever
/// projected back to the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSubagentChild {
    pub child_id: String,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentRunState {
    Pending,
    Running,
    WaitingInput,
    Completed,
    Failed,
    Cancelled,
}

impl SubagentRunState {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// Bounded host result for one exact child identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSubagentResult {
    pub child_id: String,
    pub state: SubagentRunState,
    pub output: Option<String>,
}

/// Backend retained by one parent AgentSession registry.
///
/// Implementations are constructed from already-authenticated Session state.
/// They must not resolve owner/session authority from model input.
#[async_trait]
pub trait SubagentHost: Send + Sync {
    /// Resolve the children created by this exact successful delegation.
    async fn children_for_delegation(
        &self,
        execution_id: &str,
    ) -> Result<Vec<HostSubagentChild>, String>;

    /// Deliver one queued message to one host child.
    async fn send(
        &self,
        child_id: &str,
        message: &str,
        operation_id: &str,
    ) -> Result<Value, String>;

    /// Observe only the supplied host child identities for at most `timeout`.
    async fn wait(
        &self,
        child_ids: &[String],
        timeout: Duration,
        operation_id: &str,
    ) -> Result<Vec<HostSubagentResult>, String>;

    /// Teardown hook. It must be non-blocking; implementations may enqueue or
    /// spawn their bounded cancellation command on an existing runtime.
    fn cancel(&self, child_id: &str);
}

struct QueuedMessage {
    message: String,
    operation_id: String,
    reply: oneshot::Sender<Result<Value, String>>,
}

struct ChildEntry {
    child: HostSubagentChild,
    sender: mpsc::Sender<QueuedMessage>,
    dispatcher: AbortHandle,
    latest: HostSubagentResult,
    _sequence: u64,
}

impl ChildEntry {
    fn active(&self) -> bool {
        !self.latest.state.is_terminal()
    }
}

#[derive(Default)]
struct RegistryState {
    entries: BTreeMap<String, ChildEntry>,
    by_child_id: BTreeMap<String, String>,
    terminal_order: VecDeque<String>,
    next_sequence: u64,
}

struct RegistryInner {
    host: Arc<dyn SubagentHost>,
    state: Mutex<RegistryState>,
}

impl Drop for RegistryInner {
    fn drop(&mut self) {
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for entry in state.entries.values() {
            entry.dispatcher.abort();
            if entry.active() {
                self.host.cancel(&entry.child.child_id);
            }
        }
    }
}

/// A bounded handle and mailbox registry owned by one parent AgentSession.
///
/// Clones share the same Session lifetime. Once the last clone is dropped all
/// dispatchers are aborted and every still-active child is cancelled through
/// the host, so runtime teardown cannot leak descendant work.
#[derive(Clone)]
pub struct ParentScopedSubagentRegistry {
    inner: Arc<RegistryInner>,
}

impl ParentScopedSubagentRegistry {
    pub fn new(host: Arc<dyn SubagentHost>) -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                host,
                state: Mutex::new(RegistryState::default()),
            }),
        }
    }

    /// Associate a successful delegate receipt and return model-visible handles.
    /// Replayed receipts reuse the existing handles for the same host children.
    pub async fn associate_delegation(
        &self,
        execution_id: &str,
    ) -> Result<Vec<SubagentHandle>, String> {
        let execution_id = execution_id.trim();
        if execution_id.is_empty() || execution_id.len() > MAX_HANDLE_BYTES {
            return Err("delegate receipt contained an invalid execution identity".to_owned());
        }
        let children = self
            .inner
            .host
            .children_for_delegation(execution_id)
            .await?;
        if children.is_empty() {
            return Err("delegate owner returned no background child handles".to_owned());
        }
        if children.len() > MAX_SUBAGENT_HANDLES {
            return Err(format!(
                "delegate owner returned {} children, above the {} handle limit",
                children.len(),
                MAX_SUBAGENT_HANDLES
            ));
        }

        let mut unique_child_ids = std::collections::BTreeSet::new();
        for child in &children {
            if child.child_id.trim().is_empty() || child.child_id.len() > 512 {
                return Err("delegate owner returned an invalid child identity".to_owned());
            }
            if child.label.len() > MAX_SUBAGENT_RESULT_BYTES {
                return Err("delegate owner returned an oversized child label".to_owned());
            }
            if !unique_child_ids.insert(child.child_id.as_str()) {
                return Err("delegate owner returned duplicate child identities".to_owned());
            }
        }

        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let new_count = children
            .iter()
            .filter(|child| !state.by_child_id.contains_key(&child.child_id))
            .count();
        let active_count = state.entries.values().filter(|entry| entry.active()).count();
        if active_count.saturating_add(new_count) > MAX_SUBAGENT_HANDLES {
            return Err(format!(
                "this parent AgentSession would exceed the {MAX_SUBAGENT_HANDLES} active child handle limit"
            ));
        }

        // Every child and the complete target capacity were validated before
        // mutation. From here association is an infallible all-or-none commit.
        while state.entries.len().saturating_add(new_count) > MAX_SUBAGENT_HANDLES {
            let Some(handle) = state.terminal_order.pop_front() else {
                unreachable!("active-capacity preflight guarantees enough terminal evictions");
            };
            if let Some(entry) = state.entries.remove(&handle) {
                state.by_child_id.remove(&entry.child.child_id);
                entry.dispatcher.abort();
            }
        }

        let mut handles = Vec::with_capacity(children.len());
        for child in children {
            if let Some(handle) = state.by_child_id.get(&child.child_id).cloned() {
                let label = state
                    .entries
                    .get(&handle)
                    .map(|entry| entry.child.label.clone())
                    .unwrap_or(child.label);
                handles.push(SubagentHandle { handle, label });
                continue;
            }

            state.next_sequence = state.next_sequence.saturating_add(1);
            let sequence = state.next_sequence;
            let handle = format!("child-{}", nomifun_common::generate_id());
            let (sender, mut receiver) = mpsc::channel::<QueuedMessage>(SUBAGENT_MAILBOX_CAPACITY);
            let host = Arc::clone(&self.inner.host);
            let dispatcher_child_id = child.child_id.clone();
            let dispatcher = tokio::spawn(async move {
                while let Some(message) = receiver.recv().await {
                    let result = host
                        .send(
                            &dispatcher_child_id,
                            &message.message,
                            &message.operation_id,
                        )
                        .await;
                    let _ = message.reply.send(result);
                }
            });
            state.by_child_id.insert(child.child_id.clone(), handle.clone());
            state.entries.insert(
                handle.clone(),
                ChildEntry {
                    latest: HostSubagentResult {
                        child_id: child.child_id.clone(),
                        state: SubagentRunState::Pending,
                        output: None,
                    },
                    child: child.clone(),
                    sender,
                    dispatcher: dispatcher.abort_handle(),
                    _sequence: sequence,
                },
            );
            handles.push(SubagentHandle {
                handle,
                label: child.label,
            });
        }
        Ok(handles)
    }

    async fn send(
        &self,
        handle: &str,
        message: String,
        operation_id: String,
    ) -> Result<Value, SubagentSendFailure> {
        let result = {
            let state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let entry = state
                .entries
                .get(handle)
                .ok_or_else(|| {
                    SubagentSendFailure::Rejected(
                        "unknown child handle for this parent AgentSession".to_owned(),
                    )
                })?;
            if !entry.active() {
                return Err(SubagentSendFailure::Rejected(
                    "the selected child handle is already terminal".to_owned(),
                ));
            }
            let (reply, result) = oneshot::channel();
            entry
                .sender
                .try_send(QueuedMessage {
                    message,
                    operation_id,
                    reply,
                })
                .map_err(|error| SubagentSendFailure::Rejected(match error {
                    mpsc::error::TrySendError::Full(_) => {
                        "the selected child mailbox is full; wait before sending again".to_owned()
                    }
                    mpsc::error::TrySendError::Closed(_) => {
                        "the selected child mailbox is closed".to_owned()
                    }
                }))?;
            result
        };
        tokio::time::timeout(SUBAGENT_SEND_ACK_TIMEOUT, result)
            .await
            .map_err(|_| {
                SubagentSendFailure::OutcomeUnknown(
                    "child message acknowledgement timed out after mailbox acceptance; delivery may already have been applied"
                        .to_owned(),
                )
            })?
            .map_err(|_| {
                SubagentSendFailure::OutcomeUnknown(
                    "child message dispatcher stopped after mailbox acceptance; delivery outcome is unknown"
                        .to_owned(),
                )
            })?
            .map_err(SubagentSendFailure::Rejected)
    }

    async fn wait(
        &self,
        handles: &[String],
        timeout: Duration,
        operation_id: &str,
    ) -> Result<SubagentWaitResponse, String> {
        let mut active_child_ids = Vec::new();
        let mut cached = BTreeMap::new();
        {
            let state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for handle in handles {
                let entry = state.entries.get(handle).ok_or_else(|| {
                    "unknown child handle for this parent AgentSession".to_owned()
                })?;
                if entry.latest.state.is_terminal() {
                    cached.insert(
                        handle.clone(),
                        (entry.child.label.clone(), entry.latest.clone()),
                    );
                } else {
                    active_child_ids.push(entry.child.child_id.clone());
                }
            }
        }

        let mut timed_out = false;
        let observed = if active_child_ids.is_empty() {
            Vec::new()
        } else {
            match tokio::time::timeout(
                timeout.saturating_add(Duration::from_millis(250)),
                self.inner
                    .host
                    .wait(&active_child_ids, timeout, operation_id),
            )
            .await
            {
                Ok(result) => result?,
                Err(_) => {
                    timed_out = true;
                    Vec::new()
                }
            }
        };

        let requested_child_ids = active_child_ids.iter().collect::<std::collections::BTreeSet<_>>();
        for result in &observed {
            if !requested_child_ids.contains(&result.child_id) {
                return Err("subagent host returned a child outside the requested handle set".to_owned());
            }
        }
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for result in observed {
            let Some(handle) = state.by_child_id.get(&result.child_id).cloned() else {
                return Err("subagent host returned an unregistered child".to_owned());
            };
            let became_terminal = result.state.is_terminal()
                && state
                    .entries
                    .get(&handle)
                    .is_some_and(|entry| !entry.latest.state.is_terminal());
            if let Some(entry) = state.entries.get_mut(&handle) {
                if result.state.is_terminal() {
                    entry.dispatcher.abort();
                }
                entry.latest = bound_result(result);
            }
            if became_terminal {
                state.terminal_order.push_back(handle);
            }
        }
        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            let (label, result) = match state.entries.get(handle) {
                Some(entry) => (entry.child.label.clone(), Some(&entry.latest)),
                None => {
                    let Some((label, result)) = cached.get(handle) else {
                        return Err("child handle expired while waiting".to_owned());
                    };
                    (label.clone(), Some(result))
                }
            };
            results.push(SubagentWaitItem {
                handle: handle.clone(),
                label,
                state: result.map(|result| result.state).unwrap_or(SubagentRunState::Pending),
                output: result.and_then(|result| result.output.clone()),
            });
        }
        if results.iter().all(|result| !result.state.is_terminal()) {
            timed_out = true;
        }
        Ok(SubagentWaitResponse { timed_out, results })
    }

    #[cfg(test)]
    fn active_len(&self) -> usize {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entries
            .values()
            .filter(|entry| entry.active())
            .count()
    }
}

fn bound_result(mut result: HostSubagentResult) -> HostSubagentResult {
    if let Some(output) = result.output.as_mut()
        && output.len() > MAX_SUBAGENT_RESULT_BYTES
    {
        *output = format!(
            "{}\n[truncated at {MAX_SUBAGENT_RESULT_BYTES} bytes]",
            nomi_tools::truncate_utf8(output, MAX_SUBAGENT_RESULT_BYTES)
        );
    }
    result
}

enum SubagentSendFailure {
    Rejected(String),
    OutcomeUnknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubagentHandle {
    pub handle: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SubagentWaitItem {
    handle: String,
    label: String,
    state: SubagentRunState,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SubagentWaitResponse {
    timed_out: bool,
    results: Vec<SubagentWaitItem>,
}

fn tool_ok(value: impl Serialize) -> ToolResult {
    ToolResult::text(
        serde_json::to_string(&value).expect("bounded subagent tool result is serializable"),
    )
}

fn tool_error(message: impl Into<String>) -> ToolResult {
    ToolResult::error(message.into())
}

fn parse<T: for<'de> Deserialize<'de>>(input: Value, tool: &str) -> Result<T, ToolResult> {
    serde_json::from_value(input)
        .map_err(|error| tool_error(format!("invalid {tool} input: {error}")))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SendInput {
    handle: String,
    message: String,
}

pub struct SubagentSendTool {
    registry: ParentScopedSubagentRegistry,
}

impl SubagentSendTool {
    pub fn new(registry: ParentScopedSubagentRegistry) -> Self {
        Self { registry }
    }

    async fn execute_inner(
        &self,
        input: Value,
        context: Option<&ToolExecutionContext>,
    ) -> ToolResult {
        let input: SendInput = match parse(input, SUBAGENT_SEND_TOOL_NAME) {
            Ok(input) => input,
            Err(error) => return error,
        };
        let handle = input.handle.trim();
        let message = input.message.trim();
        if handle.is_empty() || handle.len() > MAX_HANDLE_BYTES {
            return tool_error("subagent_send handle is invalid");
        }
        if message.is_empty() {
            return tool_error("subagent_send message must not be empty");
        }
        if message.len() > MAX_SUBAGENT_MESSAGE_BYTES {
            return tool_error(format!(
                "subagent_send message exceeds {MAX_SUBAGENT_MESSAGE_BYTES} bytes"
            ));
        }
        let Some(context) = context else {
            return tool_error("subagent_send requires engine-owned invocation identity");
        };
        match self
            .registry
            .send(
                handle,
                message.to_owned(),
                context.operation_id().to_owned(),
            )
            .await
        {
            Ok(receipt) => tool_ok(json!({
                "handle": handle,
                "status": "delivered",
                "receipt": receipt,
            })),
            Err(SubagentSendFailure::Rejected(error)) => tool_error(error),
            Err(SubagentSendFailure::OutcomeUnknown(message)) => tool_ok(json!({
                "handle": handle,
                "status": "outcome_unknown",
                "code": SUBAGENT_SEND_OUTCOME_UNKNOWN_CODE,
                "retry_safe": false,
                "message": message,
            })),
        }
    }
}

#[async_trait]
impl Tool for SubagentSendTool {
    fn name(&self) -> &str {
        SUBAGENT_SEND_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Send a bounded message to an active child created by this parent AgentSession. Accepts only the opaque child handle returned by agent.delegate."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "handle": {"type": "string", "minLength": 1, "maxLength": MAX_HANDLE_BYTES},
                "message": {"type": "string", "minLength": 1, "maxLength": MAX_SUBAGENT_MESSAGE_BYTES}
            },
            "required": ["handle", "message"],
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

    fn execution_timeout(&self, _input: &Value) -> Duration {
        SUBAGENT_SEND_ACK_TIMEOUT.saturating_add(Duration::from_secs(1))
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitInput {
    handles: Vec<String>,
    #[serde(default = "default_wait_ms")]
    timeout_ms: u64,
}

fn default_wait_ms() -> u64 {
    DEFAULT_SUBAGENT_WAIT_MS
}

pub struct SubagentWaitTool {
    registry: ParentScopedSubagentRegistry,
}

impl SubagentWaitTool {
    pub fn new(registry: ParentScopedSubagentRegistry) -> Self {
        Self { registry }
    }

    async fn execute_inner(
        &self,
        input: Value,
        context: Option<&ToolExecutionContext>,
    ) -> ToolResult {
        let input: WaitInput = match parse(input, SUBAGENT_WAIT_TOOL_NAME) {
            Ok(input) => input,
            Err(error) => return error,
        };
        if input.handles.is_empty() || input.handles.len() > MAX_SUBAGENT_WAIT_HANDLES {
            return tool_error(format!(
                "subagent_wait requires 1-{MAX_SUBAGENT_WAIT_HANDLES} handles"
            ));
        }
        if input.timeout_ms > MAX_SUBAGENT_WAIT_MS {
            return tool_error(format!(
                "subagent_wait timeout_ms must be at most {MAX_SUBAGENT_WAIT_MS}"
            ));
        }
        let mut handles = Vec::with_capacity(input.handles.len());
        for handle in input.handles {
            let handle = handle.trim();
            if handle.is_empty() || handle.len() > MAX_HANDLE_BYTES {
                return tool_error("subagent_wait contains an invalid handle");
            }
            if handles.iter().any(|existing| existing == handle) {
                return tool_error("subagent_wait handles must be unique");
            }
            handles.push(handle.to_owned());
        }
        let Some(context) = context else {
            return tool_error("subagent_wait requires engine-owned invocation identity");
        };
        match self
            .registry
            .wait(
                &handles,
                Duration::from_millis(input.timeout_ms),
                context.operation_id(),
            )
            .await
        {
            Ok(result) => tool_ok(result),
            Err(error) => tool_error(error),
        }
    }
}

#[async_trait]
impl Tool for SubagentWaitTool {
    fn name(&self) -> &str {
        SUBAGENT_WAIT_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Wait for up to eight child handles created by this parent AgentSession. The timeout is finite and cancellation stops only this wait, not the child."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "handles": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_SUBAGENT_WAIT_HANDLES,
                    "items": {"type": "string", "minLength": 1, "maxLength": MAX_HANDLE_BYTES}
                },
                "timeout_ms": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_SUBAGENT_WAIT_MS,
                    "default": DEFAULT_SUBAGENT_WAIT_MS
                }
            },
            "required": ["handles"],
            "additionalProperties": false
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
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

    fn execution_timeout(&self, input: &Value) -> Duration {
        let timeout_ms = input
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_SUBAGENT_WAIT_MS)
            .min(MAX_SUBAGENT_WAIT_MS);
        Duration::from_millis(timeout_ms).saturating_add(Duration::from_secs(2))
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }

    fn is_polling_invocation(&self, _input: &Value) -> bool {
        true
    }
}

/// Decorates the existing persistent `agent.delegate` owner. It never spawns a
/// second execution; it records the returned execution into this Session's
/// bounded registry and augments the receipt with opaque child handles.
pub struct DelegationHandleRecordingTool {
    inner: Box<dyn Tool>,
    registry: ParentScopedSubagentRegistry,
}

impl DelegationHandleRecordingTool {
    pub fn new(inner: Box<dyn Tool>, registry: ParentScopedSubagentRegistry) -> Self {
        Self { inner, registry }
    }

    async fn record(&self, mut result: ToolResult) -> ToolResult {
        if result.is_error {
            return result;
        }
        let Ok(mut payload) = serde_json::from_str::<Value>(&result.content) else {
            return result;
        };
        let Some(execution_id) = payload
            .get("result")
            .and_then(|result| result.get("execution_id"))
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return result;
        };
        let status = payload
            .get("result")
            .and_then(|result| result.get("status"))
            .and_then(Value::as_str);
        if matches!(
            status,
            Some("completed" | "completed_with_failures" | "failed" | "cancelled")
        ) {
            return result;
        }
        match self.registry.associate_delegation(&execution_id).await {
            Ok(handles) => {
                if let Some(object) = payload
                    .get_mut("result")
                    .and_then(Value::as_object_mut)
                {
                    object.insert("child_handles".to_owned(), json!(handles));
                    result.content = serde_json::to_string(&payload)
                        .expect("delegate receipt with bounded child handles is serializable");
                }
            }
            Err(error) => {
                if let Some(object) = payload
                    .get_mut("result")
                    .and_then(Value::as_object_mut)
                {
                    object.insert("child_handle_error".to_owned(), json!(error));
                    result.content = serde_json::to_string(&payload)
                        .expect("delegate receipt with handle error is serializable");
                }
            }
        }
        result
    }
}

#[async_trait]
impl Tool for DelegationHandleRecordingTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn activation_identity(&self) -> &str {
        self.inner.activation_identity()
    }

    fn artifact_identity(&self) -> &str {
        self.inner.artifact_identity()
    }

    fn reserved_provider_name_prefix(&self) -> Option<&'static str> {
        self.inner.reserved_provider_name_prefix()
    }

    fn deferred_search_aliases(&self) -> Vec<String> {
        self.inner.deferred_search_aliases()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn input_schema(&self) -> JsonSchema {
        self.inner.input_schema()
    }

    fn is_concurrency_safe(&self, input: &Value) -> bool {
        self.inner.is_concurrency_safe(input)
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let result = self.inner.execute(input).await;
        self.record(result).await
    }

    fn execution_timeout(&self, input: &Value) -> Duration {
        self.inner.execution_timeout(input)
    }

    async fn execute_with_context(
        &self,
        input: Value,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        let result = self.inner.execute_with_context(input, context).await;
        self.record(result).await
    }

    fn take_delegated_effects(&self, context: &ToolExecutionContext) -> Vec<String> {
        self.inner.take_delegated_effects(context)
    }

    fn max_result_size(&self) -> usize {
        self.inner.max_result_size()
    }

    fn category(&self) -> ToolCategory {
        self.inner.category()
    }

    fn category_for(&self, input: &Value) -> ToolCategory {
        self.inner.category_for(input)
    }

    fn may_have_workspace_side_effects(&self, input: &Value) -> bool {
        self.inner.may_have_workspace_side_effects(input)
    }

    fn is_polling_invocation(&self, input: &Value) -> bool {
        self.inner.is_polling_invocation(input)
    }

    fn is_deferred(&self) -> bool {
        self.inner.is_deferred()
    }

    fn requires_explicit_route(&self) -> bool {
        self.inner.requires_explicit_route()
    }

    fn describe(&self, input: &Value) -> String {
        self.inner.describe(input)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    struct FakeHost {
        children: Mutex<BTreeMap<String, Vec<HostSubagentChild>>>,
        sent: Mutex<Vec<(String, String)>>,
        results: Mutex<BTreeMap<String, HostSubagentResult>>,
        cancelled: Mutex<Vec<String>>,
        sends_in_flight: AtomicUsize,
    }

    #[async_trait]
    impl SubagentHost for FakeHost {
        async fn children_for_delegation(
            &self,
            execution_id: &str,
        ) -> Result<Vec<HostSubagentChild>, String> {
            Ok(self
                .children
                .lock()
                .unwrap()
                .get(execution_id)
                .cloned()
                .unwrap_or_default())
        }

        async fn send(
            &self,
            child_id: &str,
            message: &str,
            _operation_id: &str,
        ) -> Result<Value, String> {
            self.sends_in_flight.fetch_add(1, Ordering::SeqCst);
            self.sent
                .lock()
                .unwrap()
                .push((child_id.to_owned(), message.to_owned()));
            self.sends_in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(json!({"accepted": true}))
        }

        async fn wait(
            &self,
            child_ids: &[String],
            _timeout: Duration,
            _operation_id: &str,
        ) -> Result<Vec<HostSubagentResult>, String> {
            let results = self.results.lock().unwrap();
            Ok(child_ids
                .iter()
                .filter_map(|child_id| results.get(child_id).cloned())
                .collect())
        }

        fn cancel(&self, child_id: &str) {
            self.cancelled.lock().unwrap().push(child_id.to_owned());
        }
    }

    fn child(id: &str) -> HostSubagentChild {
        HostSubagentChild {
            child_id: id.to_owned(),
            label: format!("child {id}"),
        }
    }

    fn context() -> ToolExecutionContext {
        ToolExecutionContext::from_scoped_tool_call("parent-session:turn-1", "call-1")
    }

    #[tokio::test]
    async fn send_accepts_only_current_parent_active_handles() {
        let host = Arc::new(FakeHost::default());
        host.children
            .lock()
            .unwrap()
            .insert("execution-a".to_owned(), vec![child("private-a")]);
        let registry = ParentScopedSubagentRegistry::new(host.clone());
        let handle = registry
            .associate_delegation("execution-a")
            .await
            .unwrap()
            .remove(0)
            .handle;
        assert!(!handle.contains("private-a"));

        let tool = SubagentSendTool::new(registry.clone());
        let denied = tool
            .execute_with_context(
                json!({"handle": "private-a", "message": "escape"}),
                &context(),
            )
            .await;
        assert!(denied.is_error);
        let sent = tool
            .execute_with_context(
                json!({"handle": handle, "message": "continue"}),
                &context(),
            )
            .await;
        assert!(!sent.is_error, "{}", sent.content);
        assert_eq!(
            host.sent.lock().unwrap().as_slice(),
            &[("private-a".to_owned(), "continue".to_owned())]
        );
    }

    #[tokio::test]
    async fn wait_is_bounded_marks_terminal_and_closes_send() {
        let host = Arc::new(FakeHost::default());
        host.children
            .lock()
            .unwrap()
            .insert("execution-a".to_owned(), vec![child("private-a")]);
        host.results.lock().unwrap().insert(
            "private-a".to_owned(),
            HostSubagentResult {
                child_id: "private-a".to_owned(),
                state: SubagentRunState::Completed,
                output: Some("x".repeat(MAX_SUBAGENT_RESULT_BYTES + 100)),
            },
        );
        let registry = ParentScopedSubagentRegistry::new(host);
        let handle = registry
            .associate_delegation("execution-a")
            .await
            .unwrap()
            .remove(0)
            .handle;
        let wait = SubagentWaitTool::new(registry.clone())
            .execute_with_context(
                json!({"handles": [handle.clone()], "timeout_ms": 5}),
                &context(),
            )
            .await;
        assert!(!wait.is_error, "{}", wait.content);
        assert!(wait.content.contains("truncated at 8192 bytes"));
        assert_eq!(registry.active_len(), 0);

        let send = SubagentSendTool::new(registry)
            .execute_with_context(
                json!({"handle": handle, "message": "too late"}),
                &context(),
            )
            .await;
        assert!(send.is_error);
        assert!(send.content.contains("terminal"));
    }

    #[tokio::test]
    async fn wait_preserves_real_non_terminal_state() {
        let host = Arc::new(FakeHost::default());
        host.children
            .lock()
            .unwrap()
            .insert("execution-a".to_owned(), vec![child("private-a")]);
        host.results.lock().unwrap().insert(
            "private-a".to_owned(),
            HostSubagentResult {
                child_id: "private-a".to_owned(),
                state: SubagentRunState::WaitingInput,
                output: None,
            },
        );
        let registry = ParentScopedSubagentRegistry::new(host);
        let handle = registry
            .associate_delegation("execution-a")
            .await
            .unwrap()
            .remove(0)
            .handle;

        let result = SubagentWaitTool::new(registry)
            .execute_with_context(
                json!({"handles": [handle], "timeout_ms": 0}),
                &context(),
            )
            .await;
        assert!(!result.is_error, "{}", result.content);
        let value: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(value["results"][0]["state"], "waiting_input");
        assert_eq!(value["timed_out"], true);
    }

    #[tokio::test]
    async fn delegation_association_is_atomic_on_shape_and_capacity_failure() {
        let host = Arc::new(FakeHost::default());
        host.children.lock().unwrap().insert(
            "invalid-shape".to_owned(),
            vec![child("valid"), child("")],
        );
        host.children.lock().unwrap().insert(
            "over-capacity".to_owned(),
            (0..=MAX_SUBAGENT_HANDLES)
                .map(|index| child(&format!("child-{index}")))
                .collect(),
        );
        let registry = ParentScopedSubagentRegistry::new(host);

        assert!(registry.associate_delegation("invalid-shape").await.is_err());
        assert_eq!(registry.active_len(), 0, "shape failure must leave no partial handles");
        assert!(registry.associate_delegation("over-capacity").await.is_err());
        assert_eq!(
            registry.active_len(),
            0,
            "capacity failure must leave no partial handles"
        );
    }

    struct SlowSendHost;

    #[async_trait]
    impl SubagentHost for SlowSendHost {
        async fn children_for_delegation(
            &self,
            _execution_id: &str,
        ) -> Result<Vec<HostSubagentChild>, String> {
            Ok(vec![child("private-a")])
        }

        async fn send(
            &self,
            _child_id: &str,
            _message: &str,
            _operation_id: &str,
        ) -> Result<Value, String> {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(json!({"accepted": true}))
        }

        async fn wait(
            &self,
            _child_ids: &[String],
            _timeout: Duration,
            _operation_id: &str,
        ) -> Result<Vec<HostSubagentResult>, String> {
            Ok(Vec::new())
        }

        fn cancel(&self, _child_id: &str) {}
    }

    #[tokio::test]
    async fn send_ack_timeout_returns_stable_outcome_unknown_without_retry_signal() {
        let registry = ParentScopedSubagentRegistry::new(Arc::new(SlowSendHost));
        let handle = registry
            .associate_delegation("execution-a")
            .await
            .unwrap()
            .remove(0)
            .handle;

        let result = SubagentSendTool::new(registry)
            .execute_with_context(
                json!({"handle": handle, "message": "apply once"}),
                &context(),
            )
            .await;
        assert!(!result.is_error, "an unknown applied outcome must not invite tool retry");
        let value: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(value["status"], "outcome_unknown");
        assert_eq!(value["code"], SUBAGENT_SEND_OUTCOME_UNKNOWN_CODE);
        assert_eq!(value["retry_safe"], false);
    }

    #[tokio::test]
    async fn registry_teardown_aborts_mailboxes_and_cancels_active_children() {
        let host = Arc::new(FakeHost::default());
        host.children
            .lock()
            .unwrap()
            .insert("execution-a".to_owned(), vec![child("private-a")]);
        {
            let registry = ParentScopedSubagentRegistry::new(host.clone());
            registry.associate_delegation("execution-a").await.unwrap();
        }
        assert_eq!(host.cancelled.lock().unwrap().as_slice(), &["private-a"]);
    }

    struct CancellationHost {
        active_waits: AtomicUsize,
    }

    struct ActiveWaitGuard<'a>(&'a AtomicUsize);

    impl Drop for ActiveWaitGuard<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl SubagentHost for CancellationHost {
        async fn children_for_delegation(
            &self,
            _execution_id: &str,
        ) -> Result<Vec<HostSubagentChild>, String> {
            Ok(vec![child("private-a")])
        }

        async fn send(
            &self,
            _child_id: &str,
            _message: &str,
            _operation_id: &str,
        ) -> Result<Value, String> {
            Ok(json!({"accepted": true}))
        }

        async fn wait(
            &self,
            _child_ids: &[String],
            _timeout: Duration,
            _operation_id: &str,
        ) -> Result<Vec<HostSubagentResult>, String> {
            self.active_waits.fetch_add(1, Ordering::SeqCst);
            let _guard = ActiveWaitGuard(&self.active_waits);
            std::future::pending().await
        }

        fn cancel(&self, _child_id: &str) {}
    }

    #[tokio::test]
    async fn cancelling_wait_drops_the_host_wait_without_cancelling_child() {
        let host = Arc::new(CancellationHost {
            active_waits: AtomicUsize::new(0),
        });
        let registry = ParentScopedSubagentRegistry::new(host.clone());
        let handle = registry
            .associate_delegation("execution-a")
            .await
            .unwrap()
            .remove(0)
            .handle;
        let cancelled = tokio::time::timeout(
            Duration::from_millis(10),
            registry.wait(&[handle], Duration::from_secs(30), "wait-1"),
        )
        .await;
        assert!(cancelled.is_err(), "the test timeout must cancel the wait future");
        tokio::task::yield_now().await;
        assert_eq!(host.active_waits.load(Ordering::SeqCst), 0);
        assert_eq!(registry.active_len(), 1, "cancelling wait must not cancel child work");
    }

    struct DelegateReceiptTool;

    #[async_trait]
    impl Tool for DelegateReceiptTool {
        fn name(&self) -> &str { "nomi_delegate" }
        fn description(&self) -> &str { "delegate" }
        fn input_schema(&self) -> JsonSchema {
            json!({"type":"object","additionalProperties":true})
        }
        fn is_concurrency_safe(&self, _input: &Value) -> bool { false }
        async fn execute(&self, _input: Value) -> ToolResult {
            ToolResult::text(json!({
                "result": {"execution_id":"execution-a","status":"running"}
            }).to_string())
        }
        fn category(&self) -> ToolCategory { ToolCategory::Exec }
    }

    #[tokio::test]
    async fn delegate_wrapper_adds_opaque_handles_without_replacing_spawn_owner() {
        let host = Arc::new(FakeHost::default());
        host.children
            .lock()
            .unwrap()
            .insert("execution-a".to_owned(), vec![child("private-a")]);
        let registry = ParentScopedSubagentRegistry::new(host);
        let tool = DelegationHandleRecordingTool::new(Box::new(DelegateReceiptTool), registry);
        let result = tool.execute(json!({})).await;
        assert!(!result.is_error);
        let value: Value = serde_json::from_str(&result.content).unwrap();
        let handle = value["result"]["child_handles"][0]["handle"]
            .as_str()
            .unwrap();
        assert!(handle.starts_with("child-"));
        assert!(!handle.contains("private-a"));
    }

    #[test]
    fn model_schemas_never_accept_owner_session_or_execution_selectors() {
        let host = Arc::new(FakeHost::default());
        let registry = ParentScopedSubagentRegistry::new(host);
        for schema in [
            SubagentSendTool::new(registry.clone()).input_schema(),
            SubagentWaitTool::new(registry).input_schema(),
        ] {
            let schema = schema.to_string();
            assert!(!schema.contains("owner"));
            assert!(!schema.contains("session"));
            assert!(!schema.contains("execution"));
            assert!(!schema.contains("child_id"));
        }
    }
}
