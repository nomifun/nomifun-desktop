//! Public task progress. Runtime requirements and effect ledgers stay private.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../../ui/src/common/protocolBindings/")]
pub enum TaskPlanStepStatus { Pending, InProgress, Completed, Blocked }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct TaskPlanStep {
    pub step: String,
    pub status: TaskPlanStepStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct TaskPlan {
    pub revision: u32,
    pub explanation: String,
    pub needs_replan: bool,
    pub steps: Vec<TaskPlanStep>,
}

/// One consistent read of the latest started turn, independently of transcript
/// pagination. A new turn without a plan explicitly clears the previous plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct TaskPlanSnapshot {
    pub conversation_id: String,
    #[ts(type = "number")]
    pub sequence: u64,
    pub turn_id: Option<String>,
    pub turn_status: Option<String>,
    pub plan: Option<TaskPlan>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_snapshot_preserves_blocked_and_reset_states() {
        let snapshot: TaskPlanSnapshot = serde_json::from_value(serde_json::json!({
            "conversation_id":"session", "sequence":42, "turn_id":"turn", "turn_status":"paused",
            "plan":{"revision":2,"explanation":"Wait for input","needs_replan":true,
                "steps":[{"step":"Obtain input","status":"blocked"}]}
        })).unwrap();
        assert_eq!(snapshot.plan.unwrap().steps[0].status, TaskPlanStepStatus::Blocked);
        let reset: TaskPlanSnapshot = serde_json::from_value(serde_json::json!({
            "conversation_id":"session", "sequence":43, "turn_id":"next", "turn_status":"running", "plan":null
        })).unwrap();
        assert!(reset.plan.is_none());
    }
}
