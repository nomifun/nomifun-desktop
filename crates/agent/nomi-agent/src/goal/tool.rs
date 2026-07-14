use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use nomi_protocol::events::ToolCategory;
use nomi_tools::Tool;
use nomi_types::tool::{JsonSchema, ToolResult};

use crate::goal::state::{GoalState, GoalStatus};

/// Lets the model declare the terminal state of the current session goal.
/// Engine-internal (no `RequirementSink`) — deliberately NOT reusing
/// `requirement_complete`, which routes to the AutoWork runner.
pub struct UpdateGoalTool {
    state: Arc<Mutex<GoalState>>,
}

impl UpdateGoalTool {
    pub fn new(state: Arc<Mutex<GoalState>>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Tool for UpdateGoalTool {
    fn name(&self) -> &str {
        "update_goal"
    }

    fn description(&self) -> &str {
        "标记当前会话目标的终态。仅在目标确实已达成、且无任何必须完成的工作残留时，\
         用 status=\"complete\"；仅在同一阻塞条件连续多个目标轮次重复、确实陷入僵局时，\
         用 status=\"blocked\"。不要因为工作困难/缓慢/不确定/想停下来就调用本工具。\
         可选地用 evidence 逐条列出证明完成或阻塞的权威证据（文件/命令输出/测试结果）。"
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["complete", "blocked"],
                    "description": "complete=目标已达成且经完成审计证明；blocked=连续多轮同一僵局。"
                },
                "evidence": {
                    "type": "string",
                    "description": "可选。逐条列出证明完成/阻塞的权威证据（文件/命令输出/测试结果）。"
                }
            },
            "required": ["status"]
        })
    }

    // Not deferred: the continuation prompt asks the model to call this directly,
    // so its schema must be visible from the start.
    fn is_deferred(&self) -> bool {
        false
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let status = match input.get("status").and_then(|v| v.as_str()) {
            Some("complete") => GoalStatus::Complete,
            Some("blocked") => GoalStatus::Blocked,
            _ => return ToolResult::error("update_goal: status 只能是 complete 或 blocked"),
        };
        let evidence = input.get("evidence").and_then(|v| v.as_str()).unwrap_or("");

        {
            let mut g = self.state.lock().unwrap();
            // Only Active -> terminal; re-calling on a terminal goal is a no-op (idempotent).
            if g.status == GoalStatus::Active {
                g.status = status;
            }
        }

        let payload = json!({
            "kind": "goal_update",
            "status": match status {
                GoalStatus::Complete => "complete",
                GoalStatus::Blocked => "blocked",
                GoalStatus::Active => "active",
            },
            "evidence": evidence,
        });
        ToolResult::text(serde_json::to_string(&payload).unwrap_or_default())
    }

    fn describe(&self, input: &Value) -> String {
        let status = input.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        format!("Update goal: {status}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_with_state() -> (UpdateGoalTool, Arc<Mutex<GoalState>>) {
        let state = Arc::new(Mutex::new(GoalState::new("do X".into(), 8)));
        (UpdateGoalTool::new(Arc::clone(&state)), state)
    }

    #[tokio::test]
    async fn complete_sets_state() {
        let (tool, state) = tool_with_state();
        let r = tool.execute(json!({ "status": "complete" })).await;
        assert!(!r.is_error);
        assert_eq!(state.lock().unwrap().status, GoalStatus::Complete);
        assert!(r.content.contains("complete"));
    }

    #[tokio::test]
    async fn blocked_sets_state() {
        let (tool, state) = tool_with_state();
        let r = tool.execute(json!({ "status": "blocked", "evidence": "stuck on auth" })).await;
        assert!(!r.is_error);
        assert_eq!(state.lock().unwrap().status, GoalStatus::Blocked);
    }

    #[tokio::test]
    async fn invalid_status_errors() {
        let (tool, _state) = tool_with_state();
        let r = tool.execute(json!({ "status": "done" })).await;
        assert!(r.is_error);
    }

    #[tokio::test]
    async fn terminal_is_idempotent() {
        let (tool, state) = tool_with_state();
        let _ = tool.execute(json!({ "status": "complete" })).await;
        // A later "blocked" must not override a reached "complete".
        let _ = tool.execute(json!({ "status": "blocked" })).await;
        assert_eq!(state.lock().unwrap().status, GoalStatus::Complete);
    }
}
