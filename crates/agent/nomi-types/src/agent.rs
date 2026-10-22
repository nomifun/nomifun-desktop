use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The lifecycle vocabulary for one durable Agent Execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionStatus {
    Planning,
    Running,
    Paused,
    WaitingInput,
    Completed,
    CompletedWithFailures,
    Failed,
    Cancelled,
}

impl AgentExecutionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::WaitingInput => "waiting_input",
            Self::Completed => "completed",
            Self::CompletedWithFailures => "completed_with_failures",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::CompletedWithFailures | Self::Failed | Self::Cancelled
        )
    }

    /// The complete legal transition table for a persisted execution. A no-op
    /// update is accepted so idempotent recovery commands remain safe.
    pub const fn can_transition_to(self, next: Self) -> bool {
        if self as u8 == next as u8 {
            return true;
        }
        match self {
            Self::Planning => matches!(
                next,
                Self::Running | Self::Failed | Self::Cancelled
            ),
            Self::Running => matches!(
                next,
                Self::Paused
                    | Self::WaitingInput
                    | Self::Completed
                    | Self::CompletedWithFailures
                    | Self::Failed
                    | Self::Cancelled
            ),
            Self::Paused => matches!(
                next,
                Self::Running
                    | Self::WaitingInput
                    | Self::Failed
                    | Self::Cancelled
            ),
            Self::WaitingInput => matches!(
                next,
                Self::Running
                    | Self::Paused
                    | Self::Failed
                    | Self::Cancelled
            ),
            // A settled result may be explicitly reopened by a versioned
            // retry/adopt command. Runtime attempt settlement still refuses
            // to touch a settled aggregate, so late events cannot revive it.
            Self::Completed | Self::CompletedWithFailures | Self::Failed => {
                matches!(next, Self::Running)
            }
            Self::Cancelled => false,
        }
    }
}

impl fmt::Display for AgentExecutionStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Parse failure for the canonical Agent Execution lifecycle vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownAgentExecutionStatus(String);

impl fmt::Display for UnknownAgentExecutionStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown AgentExecutionStatus value: {}", self.0)
    }
}

impl std::error::Error for UnknownAgentExecutionStatus {}

impl FromStr for AgentExecutionStatus {
    type Err = UnknownAgentExecutionStatus;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "planning" => Ok(Self::Planning),
            "running" => Ok(Self::Running),
            "paused" => Ok(Self::Paused),
            "waiting_input" => Ok(Self::WaitingInput),
            "completed" => Ok(Self::Completed),
            "completed_with_failures" => Ok(Self::CompletedWithFailures),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(UnknownAgentExecutionStatus(value.to_owned())),
        }
    }
}

/// Acceptance receipt for one durable Agent Execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentExecutionReceipt {
    pub execution_id: String,
    pub status: AgentExecutionStatus,
    pub message: String,
    /// Steps appended to an already-running execution. Empty for a new execution.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub step_ids: Vec<String>,
}

impl AgentExecutionReceipt {
    pub fn new(
        execution_id: impl Into<String>,
        status: AgentExecutionStatus,
        message: impl Into<String>,
    ) -> Self {
        Self {
            execution_id: execution_id.into(),
            status,
            message: message.into(),
            step_ids: Vec::new(),
        }
    }

    pub fn with_step_ids(mut self, step_ids: Vec<String>) -> Self {
        self.step_ids = step_ids;
        self
    }
}

/// Fixed wire discriminator for the shared parallel delegation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParallelDelegationStrategy {
    Parallel,
}

/// The one `strategy=parallel` request DTO consumed by every
/// `nomi_delegate` deployment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParallelDelegationRequest {
    /// Exactly `parallel`. Parallel requests never include planned aggregate
    /// settings such as `work_dir`, `model_pool`, or `max_parallel`.
    pub strategy: ParallelDelegationStrategy,
    /// A native JSON array of independent Agent task objects, never a JSON string.
    #[schemars(length(min = 1, max = 16))]
    pub tasks: Vec<AgentDelegationTask>,
}

impl ParallelDelegationRequest {
    pub const MAX_TASKS: usize = 16;

    /// Enforce the semantic constraints represented in JSON Schema for callers
    /// that deserialize outside a schema-aware model runtime.
    pub fn validate(&self) -> Result<(), String> {
        if self.tasks.is_empty() || self.tasks.len() > Self::MAX_TASKS {
            return Err(format!(
                "parallel delegation requires 1-{} tasks",
                Self::MAX_TASKS
            ));
        }
        for (index, task) in self.tasks.iter().enumerate() {
            task.validate()
                .map_err(|error| format!("parallel task {index}: {error}"))?;
        }
        Ok(())
    }
}

/// Explicit per-Agent narrowing of inherited tool authority.
///
/// This type is the single policy vocabulary shared by embedded invocation and
/// persisted Agent Execution. A free-form role is descriptive metadata only
/// and must never be interpreted as permission.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AgentToolPolicy {
    #[default]
    Full,
    ReadOnly,
    ReadShell,
}

impl AgentToolPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::ReadOnly => "read_only",
            Self::ReadShell => "read_shell",
        }
    }
}

impl std::fmt::Display for AgentToolPolicy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::str::FromStr for AgentToolPolicy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "full" => Ok(Self::Full),
            "read_only" => Ok(Self::ReadOnly),
            "read_shell" => Ok(Self::ReadShell),
            _ => Err(format!("unknown Agent tool policy '{value}'")),
        }
    }
}

/// One task in the shared `strategy=parallel` request vocabulary.
///
/// Both embedded and platform hosts deserialize this exact DTO. `role` is
/// intentionally free-form; `tool_policy` is the only field that narrows
/// tools. When present, `role` is injected as explicit invocation context by
/// both deployments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentDelegationTask {
    /// Short descriptive name shown in output or the execution canvas.
    pub name: String,
    /// Complete task prompt for the invoked Agent.
    pub prompt: String,
    /// Optional human-readable role used as invocation context and for
    /// persistent routing/display. It never grants tool authority.
    #[serde(default)]
    pub role: Option<String>,
    /// Explicit narrowing of inherited tool authority.
    #[serde(default)]
    pub tool_policy: AgentToolPolicy,
}

/// Apply the one shared execution meaning of a delegation role.
///
/// Role is prompt context, not a permission or a second Agent type. Taking an
/// owned prompt lets hosts use this without cloning, and keeping the exact
/// wording here prevents embedded and persisted execution from drifting.
pub fn apply_agent_role_context(prompt: String, role: Option<&str>) -> String {
    let Some(role) = role.map(str::trim).filter(|role| !role.is_empty()) else {
        return prompt;
    };
    format!(
        "DELEGATED ROLE CONTEXT: {role}\n\
         This role describes the focus for this invocation and does not grant or widen tool permissions.\n\n\
         {prompt}"
    )
}

impl AgentDelegationTask {
    /// Validate task fields identically for embedded and platform deployments.
    /// Keeping this next to the shared wire DTO prevents one host from
    /// accepting a task another rejects.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Agent delegation task name must not be blank".to_owned());
        }
        if self.prompt.trim().is_empty() {
            return Err("Agent delegation task prompt must not be blank".to_owned());
        }
        if self.role.as_ref().is_some_and(|role| role.trim().is_empty()) {
            return Err("Agent delegation task role must not be blank when provided".to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AgentDelegationTask, AgentExecutionReceipt, AgentExecutionStatus,
        AgentToolPolicy, ParallelDelegationRequest, apply_agent_role_context,
    };

    #[test]
    fn shared_task_role_is_free_form_and_policy_is_strict() {
        let task: AgentDelegationTask = serde_json::from_value(serde_json::json!({
            "name": "实现",
            "prompt": "修改代码",
            "role": "中文自定义角色",
            "tool_policy": "full"
        }))
        .unwrap();
        assert_eq!(task.role.as_deref(), Some("中文自定义角色"));
        assert_eq!(task.tool_policy, AgentToolPolicy::Full);

        assert!(serde_json::from_value::<AgentDelegationTask>(serde_json::json!({
            "name": "bad",
            "prompt": "bad",
            "tool_policy": "admin"
        }))
        .is_err());
    }

    #[test]
    fn shared_task_blank_validation_is_canonical() {
        let task = |name: &str, prompt: &str, role: Option<&str>| AgentDelegationTask {
            name: name.to_owned(),
            prompt: prompt.to_owned(),
            role: role.map(str::to_owned),
            tool_policy: AgentToolPolicy::Full,
        };
        assert!(task("name", "prompt", Some("任意角色")).validate().is_ok());
        assert!(task(" ", "prompt", None).validate().is_err());
        assert!(task("name", "\n", None).validate().is_err());
        assert!(task("name", "prompt", Some("\t")).validate().is_err());
    }

    #[test]
    fn role_context_has_one_non_authorizing_prompt_semantic() {
        let prompt = apply_agent_role_context(
            "Inspect the persistence boundary.".to_owned(),
            Some("  database reviewer  "),
        );
        assert!(prompt.starts_with("DELEGATED ROLE CONTEXT: database reviewer\n"));
        assert!(prompt.contains("does not grant or widen tool permissions"));
        assert!(prompt.ends_with("Inspect the persistence boundary."));
        assert_eq!(
            apply_agent_role_context("unchanged".to_owned(), None),
            "unchanged"
        );
    }

    #[test]
    fn parallel_request_is_one_strict_shared_wire_contract() {
        let request: ParallelDelegationRequest = serde_json::from_value(serde_json::json!({
            "strategy": "parallel",
            "tasks": [{"name":"scan","prompt":"inspect"}]
        }))
        .unwrap();
        assert!(request.validate().is_ok());
        assert!(serde_json::from_value::<ParallelDelegationRequest>(serde_json::json!({
            "strategy": "parallel",
            "tasks": [{"name":"scan","prompt":"inspect"}],
            "work_dir": "/model/must/not/choose"
        }))
        .is_err());
    }

    #[test]
    fn receipt_omits_deployment_specific_empty_projection_fields() {
        let receipt = AgentExecutionReceipt::new(
            "exec_test",
            AgentExecutionStatus::Running,
            "accepted",
        );
        let value = serde_json::to_value(receipt).unwrap();
        assert_eq!(value["execution_id"], "exec_test");
        assert_eq!(value["status"], "running");
        assert_eq!(value["message"], "accepted");
        for absent in ["mode", "execution_mode", "step_ids", "results", "synthesis", "summary"] {
            assert!(value.get(absent).is_none(), "unexpected field {absent}: {value}");
        }
    }

    #[test]
    fn receipt_keeps_exact_appended_step_ids() {
        let receipt = AgentExecutionReceipt::new(
            "exec_test", AgentExecutionStatus::Running, "accepted",
        ).with_step_ids(vec!["execstep_1".to_owned()]);
        let value = serde_json::to_value(receipt).unwrap();
        assert_eq!(value["step_ids"][0], "execstep_1");
        assert!(serde_json::from_value::<AgentExecutionReceipt>(serde_json::json!({
            "execution_id":"exec_test", "status":"running", "message":"accepted", "results":[]
        })).is_err(), "retired synchronous results cannot reenter the receipt contract");
    }
}
