//! Session-scoped subagent controls over the existing persistent Gateway
//! AgentExecution owner.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use nomi_agent::subagent_tools::{
    HostSubagentChild, HostSubagentResult, SubagentHost, SubagentRunState,
};
use nomi_mcp::manager::{McpCallOutput, McpManager};
use nomi_tools::ToolExecutionContext;
use nomi_types::agent::AgentExecutionStatus;
use nomifun_api_types::{AgentExecutionDetail, GatewayMcpConfig};
use nomifun_common::{ExecutionStepKind, ExecutionStepStatus};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

const DELEGATE_TOOL: &str = "nomi_delegate";
const EXECUTION_GET_TOOL: &str = "nomi_execution_get";
const EXECUTION_UPDATE_TOOL: &str = "nomi_execution_update";
const CHILD_SEPARATOR: char = '#';
const AGGREGATE_CHILD: &str = "*";
const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) fn gateway_delegate_provider_name() -> String {
    nomi_mcp::tool_proxy::canonical_mcp_display_name(
        GatewayMcpConfig::SERVER_NAME,
        DELEGATE_TOOL,
    )
}

pub(crate) struct GatewaySubagentHost {
    manager: Arc<McpManager>,
    scheduled_cancellations: Mutex<BTreeSet<String>>,
}

impl GatewaySubagentHost {
    pub(crate) fn new(manager: Arc<McpManager>) -> Self {
        Self {
            manager,
            scheduled_cancellations: Mutex::new(BTreeSet::new()),
        }
    }

    async fn get_execution(&self, execution_id: &str) -> Result<AgentExecutionDetail, String> {
        let output = self
            .manager
            .call_tool(
                GatewayMcpConfig::SERVER_NAME,
                EXECUTION_GET_TOOL,
                json!({"execution_id": execution_id}),
            )
            .await
            .map_err(|error| format!("AgentExecution inspection failed: {error}"))?;
        parse_gateway_result(output)
    }

    fn child_ref(execution_id: &str, step_id: &str) -> String {
        format!("{execution_id}{CHILD_SEPARATOR}{step_id}")
    }

    fn parse_child_ref(child_id: &str) -> Result<(&str, &str), String> {
        child_id
            .split_once(CHILD_SEPARATOR)
            .filter(|(execution_id, step_id)| !execution_id.is_empty() && !step_id.is_empty())
            .ok_or_else(|| "stored subagent child identity is malformed".to_owned())
    }

    fn step_result(
        detail: &AgentExecutionDetail,
        child_id: &str,
        step_id: &str,
    ) -> Result<HostSubagentResult, String> {
        let step = detail
            .steps
            .iter()
            .find(|step| step.step_id == step_id)
            .ok_or_else(|| "delegated child step no longer exists".to_owned())?;
        let state = match step.status {
            ExecutionStepStatus::Pending => SubagentRunState::Pending,
            ExecutionStepStatus::Running => SubagentRunState::Running,
            ExecutionStepStatus::WaitingInput => SubagentRunState::WaitingInput,
            ExecutionStepStatus::Completed => SubagentRunState::Completed,
            ExecutionStepStatus::Failed => SubagentRunState::Failed,
            ExecutionStepStatus::Skipped | ExecutionStepStatus::Cancelled => {
                SubagentRunState::Cancelled
            }
        };
        let output = if state.is_terminal() {
            detail
                .attempts
                .iter()
                .filter(|attempt| attempt.step_id == step_id)
                .max_by_key(|attempt| attempt.attempt_no)
                .and_then(|attempt| {
                    attempt
                        .output_summary
                        .clone()
                        .or_else(|| attempt.error.clone())
                })
                .or_else(|| Some(format!("{}: {}", step.title, step.status.as_str())))
        } else {
            None
        };
        Ok(HostSubagentResult {
            child_id: child_id.to_owned(),
            state,
            output,
        })
    }

    fn aggregate_result(
        detail: &AgentExecutionDetail,
        child_id: &str,
    ) -> HostSubagentResult {
        let state = match detail.execution.status {
            AgentExecutionStatus::Planning => SubagentRunState::Pending,
            AgentExecutionStatus::Running | AgentExecutionStatus::Paused => {
                SubagentRunState::Running
            }
            AgentExecutionStatus::WaitingInput => SubagentRunState::WaitingInput,
            AgentExecutionStatus::Completed => SubagentRunState::Completed,
            AgentExecutionStatus::CompletedWithFailures | AgentExecutionStatus::Failed => {
                SubagentRunState::Failed
            }
            AgentExecutionStatus::Cancelled => SubagentRunState::Cancelled,
        };
        HostSubagentResult {
            child_id: child_id.to_owned(),
            state,
            output: state
                .is_terminal()
                .then(|| detail.execution.summary.clone())
                .flatten(),
        }
    }

    async fn cancel_execution(manager: Arc<McpManager>, execution_id: String) {
        let get = manager
            .call_tool(
                GatewayMcpConfig::SERVER_NAME,
                EXECUTION_GET_TOOL,
                json!({"execution_id": execution_id}),
            )
            .await;
        let Ok(get) = get else { return };
        let Ok(detail) = parse_gateway_result::<AgentExecutionDetail>(get) else {
            return;
        };
        if detail.execution.status.is_terminal() {
            return;
        }
        let _ = manager
            .call_tool(
                GatewayMcpConfig::SERVER_NAME,
                EXECUTION_UPDATE_TOOL,
                json!({
                    "operation": "cancel",
                    "execution_id": detail.execution.execution_id,
                    "expected_version": detail.execution.version,
                }),
            )
            .await;
    }
}

#[async_trait]
impl SubagentHost for GatewaySubagentHost {
    async fn children_for_delegation(
        &self,
        execution_id: &str,
    ) -> Result<Vec<HostSubagentChild>, String> {
        let detail = self.get_execution(execution_id).await?;
        if detail.execution.execution_id != execution_id {
            return Err("Gateway returned a different AgentExecution identity".to_owned());
        }
        let mut children = detail
            .steps
            .iter()
            .filter(|step| {
                step.kind == ExecutionStepKind::Agent && step.superseded_in_revision.is_none()
            })
            .map(|step| HostSubagentChild {
                child_id: Self::child_ref(execution_id, &step.step_id),
                label: step.title.clone(),
            })
            .collect::<Vec<_>>();
        if children.is_empty() {
            // Planned delegation can briefly be visible before its DAG commits.
            // Retain one aggregate child; its future sends resolve an exact
            // active step from a fresh owner-scoped snapshot.
            children.push(HostSubagentChild {
                child_id: Self::child_ref(execution_id, AGGREGATE_CHILD),
                label: detail.execution.goal,
            });
        }
        Ok(children)
    }

    async fn send(
        &self,
        child_id: &str,
        message: &str,
        operation_id: &str,
    ) -> Result<Value, String> {
        let (execution_id, requested_step_id) = Self::parse_child_ref(child_id)?;
        let detail = self.get_execution(execution_id).await?;
        if detail.execution.status.is_terminal() {
            return Err("the delegated AgentExecution is already terminal".to_owned());
        }
        let step = if requested_step_id == AGGREGATE_CHILD {
            let mut candidates = detail.steps.iter().filter(|step| {
                step.kind == ExecutionStepKind::Agent
                    && matches!(
                        step.status,
                        ExecutionStepStatus::Running | ExecutionStepStatus::WaitingInput
                    )
            });
            let step = candidates.next().ok_or_else(|| {
                "the delegated AgentExecution has no active child that can receive a message"
                    .to_owned()
            })?;
            if candidates.next().is_some() {
                return Err(
                    "the aggregate child has multiple active steps; wait for an exact child handle"
                        .to_owned(),
                );
            }
            step
        } else {
            detail
                .steps
                .iter()
                .find(|step| step.step_id == requested_step_id)
                .ok_or_else(|| "delegated child step no longer exists".to_owned())?
        };
        if !matches!(
            step.status,
            ExecutionStepStatus::Running | ExecutionStepStatus::WaitingInput
        ) {
            return Err(format!(
                "delegated child is not active (status={})",
                step.status.as_str()
            ));
        }
        let context = ToolExecutionContext::from_scoped_tool_call(
            &format!("subagent-send:{operation_id}"),
            child_id,
        );
        let output = self
            .manager
            .call_tool_with_context(
                GatewayMcpConfig::SERVER_NAME,
                EXECUTION_UPDATE_TOOL,
                json!({
                    "operation": "steer",
                    "execution_id": execution_id,
                    "step_id": step.step_id,
                    "expected_execution_version": detail.execution.version,
                    "expected_step_version": step.version,
                    "text": message,
                }),
                &context,
            )
            .await
            .map_err(|error| format!("subagent message delivery failed: {error}"))?;
        parse_gateway_value(output)
    }

    async fn wait(
        &self,
        child_ids: &[String],
        timeout: Duration,
        _operation_id: &str,
    ) -> Result<Vec<HostSubagentResult>, String> {
        let targets = child_ids
            .iter()
            .map(|child_id| {
                let (execution_id, step_id) = Self::parse_child_ref(child_id)?;
                Ok((child_id.clone(), execution_id.to_owned(), step_id.to_owned()))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let deadline = Instant::now() + timeout;
        loop {
            let mut details = BTreeMap::new();
            for (_, execution_id, _) in &targets {
                if !details.contains_key(execution_id) {
                    details.insert(execution_id.clone(), self.get_execution(execution_id).await?);
                }
            }
            let results = targets
                .iter()
                .map(|(child_id, execution_id, step_id)| {
                    let detail = details
                        .get(execution_id)
                        .expect("every requested execution was fetched");
                    if step_id == AGGREGATE_CHILD {
                        Ok(Self::aggregate_result(detail, child_id))
                    } else {
                        Self::step_result(detail, child_id, step_id)
                    }
                })
                .collect::<Result<Vec<_>, String>>()?;
            if results.iter().any(|result| result.state.is_terminal())
                || timeout.is_zero()
                || Instant::now() >= deadline
            {
                return Ok(results);
            }
            tokio::time::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())))
                .await;
        }
    }

    fn cancel(&self, child_id: &str) {
        let Ok((execution_id, _)) = Self::parse_child_ref(child_id) else {
            return;
        };
        let mut scheduled = self
            .scheduled_cancellations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !scheduled.insert(execution_id.to_owned()) {
            return;
        }
        let manager = Arc::clone(&self.manager);
        let execution_id = execution_id.to_owned();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(Self::cancel_execution(manager, execution_id));
        }
    }
}

fn parse_gateway_value(output: McpCallOutput) -> Result<Value, String> {
    if output.is_error {
        return Err(if output.text.trim().is_empty() {
            "Gateway returned an error without details".to_owned()
        } else {
            output.text
        });
    }
    let payload: Value = serde_json::from_str(&output.text)
        .map_err(|error| format!("Gateway returned invalid JSON: {error}"))?;
    if let Some(error) = payload.get("error").and_then(Value::as_str) {
        return Err(error.to_owned());
    }
    payload
        .get("result")
        .cloned()
        .ok_or_else(|| "Gateway response has no result".to_owned())
}

fn parse_gateway_result<T: DeserializeOwned>(output: McpCallOutput) -> Result<T, String> {
    serde_json::from_value(parse_gateway_value(output)?)
        .map_err(|error| format!("Gateway result shape is invalid: {error}"))
}
