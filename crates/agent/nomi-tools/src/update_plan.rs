use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use nomi_protocol::events::ToolCategory;
use nomi_types::tool::{JsonSchema, ToolResult};

use crate::Tool;

/// Single step status. snake_case aligns with codex and the frontend
/// `entry.status` (`pending`/`in_progress`/`completed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    InProgress,
    Completed,
}

/// One plan step (argument). `step` is the step text; it is normalized to
/// `content` for the frontend plan renderer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanItemArg {
    pub step: String,
    pub status: StepStatus,
}

/// `update_plan` arguments — a stateless full snapshot of the plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePlanArgs {
    #[serde(default)]
    pub explanation: Option<String>,
    pub plan: Vec<PlanItemArg>,
}

/// codex-style todo/checklist tool. Stateless: the model submits the full step
/// list every call.
///
/// This is a different concept from nomi's Plan Mode
/// (`EnterPlanMode`/`ExitPlanMode`), which is a *mode* that restricts the tool
/// allow-list. `update_plan` is a *progress declaration* tool.
pub struct UpdatePlanTool;

impl UpdatePlanTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for UpdatePlanTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UpdatePlanTool {
    fn name(&self) -> &str {
        "update_plan"
    }

    fn description(&self) -> &str {
        "Update the task plan (a todo checklist shown to the user). \
         Provide an optional `explanation` and a full `plan`: the complete list of steps, \
         each with a one-line `step` and a `status` of pending, in_progress, or completed. \
         This is a stateless full snapshot — send the entire current plan every time, not a diff. \
         There should be exactly one in_progress step until all are completed; mark a step \
         completed before starting the next. Use it for non-trivial multi-step work; do not use \
         it for simple single-step queries, and do not pad with filler steps. After calling it, \
         do not repeat the full plan in your reply — just note what changed and the next step."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "explanation": {
                    "type": "string",
                    "description": "Optional rationale for this plan update."
                },
                "plan": {
                    "type": "array",
                    "description": "The full list of plan steps (complete snapshot).",
                    "items": {
                        "type": "object",
                        "properties": {
                            "step":   { "type": "string", "description": "Task step text (one short line)." },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"],
                                "description": "Step status."
                            }
                        },
                        "required": ["step", "status"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["plan"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        // Pure declaration, no side effects — safe to run concurrently.
        true
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }

    async fn execute(&self, input: Value) -> ToolResult {
        // 1) Parse arguments.
        let args: UpdatePlanArgs = match serde_json::from_value(input) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error(format!("update_plan: invalid arguments: {e}"));
            }
        };

        if args.plan.is_empty() {
            return ToolResult::error("update_plan: `plan` must contain at least one step.");
        }

        // 2) Soft constraint: at most one in_progress. More than one does NOT
        //    fail (avoids breaking the agent loop, matching codex) — we just
        //    warn so the model self-corrects.
        let in_progress = args
            .plan
            .iter()
            .filter(|p| p.status == StepStatus::InProgress)
            .count();

        // 3) Normalize into frontend entry shape: { content, status }
        //    (note step -> content).
        let entries: Vec<Value> = args
            .plan
            .iter()
            .map(|p| {
                json!({
                    "content": p.step,
                    "status": match p.status {
                        StepStatus::Pending => "pending",
                        StepStatus::InProgress => "in_progress",
                        StepStatus::Completed => "completed",
                    }
                })
            })
            .collect();

        // 4) Encode the structured snapshot into content (JSON). The backend
        //    bridge layer parses this to emit a Plan event; the same string is
        //    the (compact) tool_result returned to the model.
        let payload = json!({
            "kind": "plan_update",
            "explanation": args.explanation,
            "entries": entries,
        });
        let content = serde_json::to_string(&payload)
            .unwrap_or_else(|_| "{\"kind\":\"plan_update\",\"entries\":[]}".to_string());

        if in_progress > 1 {
            let warn = format!(
                "[note] {in_progress} steps are in_progress; convention is exactly one. Plan rendered as submitted.\n"
            );
            return ToolResult::text(format!("{warn}{content}"));
        }

        ToolResult::text(content)
    }

    fn describe(&self, input: &Value) -> String {
        let n = input
            .get("plan")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        format!("Update plan ({n} steps)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_category() {
        let t = UpdatePlanTool::new();
        assert_eq!(t.name(), "update_plan");
        assert!(matches!(t.category(), ToolCategory::Info));
    }

    #[test]
    fn schema_requires_plan() {
        let s = UpdatePlanTool::new().input_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "plan"));
        let item_req = s["properties"]["plan"]["items"]["required"].as_array().unwrap();
        assert!(item_req.iter().any(|v| v == "step"));
        assert!(item_req.iter().any(|v| v == "status"));
    }

    #[tokio::test]
    async fn execute_rejects_empty_plan() {
        let r = UpdatePlanTool::new().execute(json!({ "plan": [] })).await;
        assert!(r.is_error);
    }

    #[tokio::test]
    async fn execute_rejects_bad_args() {
        let r = UpdatePlanTool::new().execute(json!({ "plan": "nope" })).await;
        assert!(r.is_error);
    }

    #[tokio::test]
    async fn execute_normalizes_step_to_content() {
        let r = UpdatePlanTool::new()
            .execute(json!({
                "plan": [{ "step": "Read code", "status": "in_progress" }]
            }))
            .await;
        assert!(!r.is_error);
        let start = r.content.find('{').unwrap();
        let v: serde_json::Value = serde_json::from_str(&r.content[start..]).unwrap();
        assert_eq!(v["kind"], "plan_update");
        assert_eq!(v["entries"][0]["content"], "Read code");
        assert_eq!(v["entries"][0]["status"], "in_progress");
    }

    #[tokio::test]
    async fn execute_one_in_progress_no_warning() {
        let r = UpdatePlanTool::new()
            .execute(json!({
                "plan": [
                    { "step": "a", "status": "completed" },
                    { "step": "b", "status": "in_progress" },
                    { "step": "c", "status": "pending" }
                ]
            }))
            .await;
        assert!(!r.is_error);
        assert!(!r.content.contains("[note]"));
    }

    #[tokio::test]
    async fn execute_multi_in_progress_warns_but_succeeds() {
        let r = UpdatePlanTool::new()
            .execute(json!({
                "plan": [
                    { "step": "a", "status": "in_progress" },
                    { "step": "b", "status": "in_progress" }
                ]
            }))
            .await;
        assert!(!r.is_error);
        assert!(r.content.contains("[note]"));
        let start = r.content.find('{').unwrap();
        let v: serde_json::Value = serde_json::from_str(&r.content[start..]).unwrap();
        assert_eq!(v["entries"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn execute_carries_explanation() {
        let r = UpdatePlanTool::new()
            .execute(json!({
                "explanation": "re-scoping",
                "plan": [{ "step": "x", "status": "pending" }]
            }))
            .await;
        let start = r.content.find('{').unwrap();
        let v: serde_json::Value = serde_json::from_str(&r.content[start..]).unwrap();
        assert_eq!(v["explanation"], "re-scoping");
    }

    #[test]
    fn describe_reports_step_count() {
        let t = UpdatePlanTool::new();
        let d = t.describe(&json!({ "plan": [ {"step":"a","status":"pending"}, {"step":"b","status":"pending"} ] }));
        assert!(d.contains('2'));
    }
}
