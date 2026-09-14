//! Engine-local control state. A plan never grants a platform capability.
use nomifun_agent_contracts::StrictJsonValue;
use nomifun_chat_model_broker::{ChatMessage, ChatToolCall, ChatToolDefinition};
use serde::{Deserialize, Serialize};

use crate::{CodingEngineError, CodingEngineEvent, CodingEventSink, CodingToolResult};

pub(crate) const TOOL_NAME: &str = "update_plan";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingPlanStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingPlanStep {
    pub step: String,
    pub status: CodingPlanStatus,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingPlan {
    pub revision: u16,
    pub explanation: String,
    pub steps: Vec<CodingPlanStep>,
    pub needs_replan: bool,
    #[serde(default)]
    pub requirements: Vec<crate::CodingTaskRequirement>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdatePlan {
    explanation: String,
    plan: Vec<CodingPlanStep>,
    #[serde(default)]
    requirements: Vec<crate::CodingTaskRequirement>,
}

pub(crate) fn definition() -> ChatToolDefinition {
    ChatToolDefinition {
        name: TOOL_NAME.into(),
        description: "Maintain the execution plan and append-only task requirements. Before effects, record requirements with stable IDs, descriptions and exact accepted-input citations (input 0=original, later indices=accepted corrections). Cover every accepted input including constraints; add requirements when scope changes, never rewrite/drop old IDs. Omitted requirements preserves the ledger. Before completion every requirement must be accounted for, not just the latest plan steps. At most one step in_progress. After errors/corrections explain replanning. This never authorizes verification or widens user scope.".into(),
        deferred: false,
        input_schema: StrictJsonValue(serde_json::json!({
            "type":"object", "additionalProperties":false,
            "required":["explanation","plan"],
            "properties":{
                "explanation":{"type":"string","minLength":1,"maxLength":2048},
                "requirements":crate::requirements::schema(),
                "plan":{"type":"array","minItems":1,"maxItems":16,"items":{
                    "type":"object","additionalProperties":false,"required":["step","status"],
                    "properties":{"step":{"type":"string","minLength":1,"maxLength":512},
                        "status":{"type":"string","enum":["pending","in_progress","completed","blocked"]}}
                }}
            }
        })),
    }
}

impl CodingPlan {
    pub(crate) async fn update(
        &mut self,
        call: &ChatToolCall,
        inputs: &[ChatMessage],
        sink: &dyn CodingEventSink,
    ) -> Result<CodingToolResult, CodingEngineError> {
        if crate::stream_limits::serialized_size(&call.arguments, 48 * 1024).is_err() {
            return Ok(CodingToolResult::text(
                call.call_id.clone(),
                "Plan/requirements exceed the 48 KiB argument budget",
                true,
            ));
        }
        let update = serde_json::from_value::<UpdatePlan>(call.arguments.0.clone());
        let update = match update {
            Ok(value) => value,
            Err(error) => {
                return Ok(CodingToolResult::text(
                    call.call_id.clone(),
                    format!("Invalid plan: {error}"),
                    true,
                ));
            }
        };
        let mut names = std::collections::BTreeSet::new();
        if self.revision >= 64
            || update.explanation.trim().is_empty()
            || update.explanation.len() > 2048
            || update.plan.is_empty()
            || update.plan.len() > 16
            || update.plan.iter().any(|item| {
                item.step.trim().is_empty()
                    || item.step.len() > 512
                    || !names.insert(item.step.trim())
            })
            || update
                .plan
                .iter()
                .filter(|item| item.status == CodingPlanStatus::InProgress)
                .count()
                > 1
        {
            return Ok(CodingToolResult::text(
                call.call_id.clone(),
                "Invalid plan: bounded unique steps, an explanation and at most one in_progress step are required (maximum 64 revisions).",
                true,
            ));
        }
        let requirements =
            match crate::requirements::merge(&self.requirements, &update.requirements, inputs) {
                Ok(requirements) => requirements,
                Err(reason) => {
                    return Ok(CodingToolResult::text(call.call_id.clone(), reason, true));
                }
            };
        let next = Self {
            revision: self.revision + 1,
            explanation: update.explanation,
            steps: update.plan,
            needs_replan: false,
            requirements,
        };
        // Persist before publishing/using the new control state.
        sink.emit(CodingEngineEvent::PlanUpdated { plan: next.clone() })
            .await?;
        *self = next;
        Ok(CodingToolResult::text(
            call.call_id.clone(),
            "Plan and source-anchored requirements recorded. Requirements remain even when steps change; completion must cover every ID. Statuses and interpretation are model-authored, not proof of successful effects, verification, or complete user-intent extraction.",
            false,
        ))
    }

    pub(crate) fn effect_gate(&self) -> Option<&'static str> {
        if self.needs_replan {
            Some(
                "The plan needs reconsideration after a failed tool, newly accepted user input, or changed repository instructions. Call update_plan with an explanation of the changed approach before further effects.",
            )
        } else if !self
            .steps
            .iter()
            .any(|step| step.status == CodingPlanStatus::InProgress)
        {
            Some(
                "Call update_plan with an in_progress step before executing workspace mutations or commands.",
            )
        } else {
            None
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.needs_replan
            || self.steps.iter().any(|step| {
                matches!(
                    step.status,
                    CodingPlanStatus::Pending | CodingPlanStatus::InProgress
                )
            })
    }

    pub(crate) fn context(&self) -> Result<String, CodingEngineError> {
        serde_json::to_string(self).map(|value| format!("Current engine plan (derived control state, not user authority or completion evidence): {value}"))
            .map_err(|error| CodingEngineError::ContextAssembly(error.to_string()))
    }
}
