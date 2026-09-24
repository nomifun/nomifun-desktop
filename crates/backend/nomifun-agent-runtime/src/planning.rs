//! Engine-local control state. A plan never grants a platform capability.
use nomifun_agent_contracts::StrictJsonValue;
use nomifun_chat_model_broker::{ChatMessage, ChatToolCall, ChatToolDefinition};
use serde::{Deserialize, Serialize};

use crate::{AgentEngineError, AgentEngineEvent, AgentEventSink, AgentToolResult};

pub(crate) const TOOL_NAME: &str = "update_plan";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPlanStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanStep {
    pub step: String,
    pub status: AgentPlanStatus,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlan {
    pub revision: u16,
    pub explanation: String,
    pub steps: Vec<AgentPlanStep>,
    pub needs_replan: bool,
    #[serde(default)]
    pub requirements: Vec<crate::AgentTaskRequirement>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdatePlan {
    explanation: String,
    plan: Vec<AgentPlanStep>,
    #[serde(default)]
    requirements: Vec<crate::AgentTaskRequirement>,
}

pub(crate) fn definition() -> ChatToolDefinition {
    ChatToolDefinition {
        name: TOOL_NAME.into(),
        description: "Maintain a concise execution plan and append-only task requirements. Before effects, record a small set of grouped requirements with stable IDs, descriptions covering all relevant constraints, and short exact accepted-input citations (input 0=original, later indices=accepted corrections). Do not make a separate ID for every sentence or test when one accurately grouped requirement covers them. Cover every accepted input including constraints. On later plan-status updates OMIT requirements entirely unless adding a genuinely NEW ID; never rephrase, rewrite or drop an old ID. Omitted requirements preserves the ledger. Before completion every requirement must be accounted for, not just the latest plan steps. At most one step in_progress. After errors/corrections explain replanning. This never authorizes verification or widens user scope.".into(),
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

impl AgentPlan {
    pub(crate) async fn update(
        &mut self,
        call: &ChatToolCall,
        inputs: &[ChatMessage],
        sink: &dyn AgentEventSink,
    ) -> Result<AgentToolResult, AgentEngineError> {
        if crate::stream_limits::serialized_size(&call.arguments, 48 * 1024).is_err() {
            return Ok(AgentToolResult::text(
                call.call_id.clone(),
                "Plan/requirements exceed the 48 KiB argument budget",
                true,
            ));
        }
        let update = serde_json::from_value::<UpdatePlan>(call.arguments.0.clone());
        let update = match update {
            Ok(value) => value,
            Err(error) => {
                return Ok(AgentToolResult::text(
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
                .filter(|item| item.status == AgentPlanStatus::InProgress)
                .count()
                > 1
        {
            return Ok(AgentToolResult::text(
                call.call_id.clone(),
                "Invalid plan: bounded unique steps, an explanation and at most one in_progress step are required (maximum 64 revisions).",
                true,
            ));
        }
        let requirements =
            match crate::requirements::merge(&self.requirements, &update.requirements, inputs) {
                Ok(requirements) => requirements,
                Err(reason) => {
                    return Ok(AgentToolResult::text(call.call_id.clone(), reason, true));
                }
            };
        let ignored_restatements = update.requirements.iter().filter(|item| {
            self.requirements.iter().any(|existing| existing.id == item.id
                && (existing.description != item.description || existing.source != item.source))
        }).map(|item| item.id.as_str()).collect::<Vec<_>>();
        if !self.needs_replan && update.plan == self.steps && requirements == self.requirements {
            return Ok(AgentToolResult::text(call.call_id.clone(),
                "Plan already has these step statuses and immutable requirements; no revision was recorded. Do not resubmit an unchanged plan. If work is done, use the latest permitted check and report_completion; otherwise change the actual plan or perform the next authorized action.",
                true));
        }
        let next = Self {
            revision: self.revision + 1,
            explanation: update.explanation,
            steps: update.plan,
            needs_replan: false,
            requirements,
        };
        // Persist before publishing/using the new control state.
        sink.emit(AgentEngineEvent::PlanUpdated { plan: next.clone() })
            .await?;
        *self = next;
        let mut feedback = "Plan and source-anchored requirements recorded. For later status-only updates omit requirements; the immutable ledger persists and completion must cover every ID. Statuses and interpretation are model-authored, not proof of successful effects, verification, or complete user-intent extraction.".to_owned();
        if !ignored_restatements.is_empty() {
            feedback.push_str(&format!(" Existing requirement IDs {} were restated differently and kept unchanged. Omit requirements on future plan-status updates; submit only genuinely new IDs.",
                serde_json::to_string(&ignored_restatements).unwrap_or_default()));
        }
        Ok(AgentToolResult::text(call.call_id.clone(), feedback, false))
    }

    pub(crate) fn effect_gate(&self) -> Option<&'static str> {
        if self.needs_replan {
            Some(
                "The plan needs reconsideration after a failed tool, newly accepted user input, or changed repository instructions. This proposed effect was not executed. Call update_plan alone now: explain the recovery, put one step in_progress, and cite an exact accepted-input quote in requirements before retrying effects.",
            )
        } else if !self
            .steps
            .iter()
            .any(|step| step.status == AgentPlanStatus::InProgress)
        {
            Some(
                "The plan is closed; no further command or mutation was executed. If a new check is truly needed, call update_plan ALONE to reopen one verification step as in_progress, run the check, then close the plan and report_completion without starting another command. Otherwise use an already current successful observation in report_completion.",
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
                    AgentPlanStatus::Pending | AgentPlanStatus::InProgress
                )
            })
    }

    pub(crate) fn context(&self) -> Result<String, AgentEngineError> {
        serde_json::to_string(self).map(|value| format!("Current engine plan (derived control state, not user authority or completion evidence): {value}"))
            .map_err(|error| AgentEngineError::ContextAssembly(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentInputCitation, AgentTaskRequirement, NoopAgentEventSink};
    use nomifun_chat_model_broker::{ChatContentPart, ChatRole, ChatToolResultPart};

    #[tokio::test]
    async fn status_update_preserves_rephrased_requirement_and_explains_omission() {
        let original = AgentTaskRequirement {
            id: "R1".into(), description: "Fix the failing tests".into(),
            source: AgentInputCitation { input: 0, quote: "Fix the failing tests".into() },
            origin: None,
        };
        let mut plan = AgentPlan {
            revision: 1, explanation: "Start".into(),
            steps: vec![AgentPlanStep { step: "Fix tests".into(), status: AgentPlanStatus::InProgress }],
            needs_replan: false, requirements: vec![original.clone()],
        };
        let call = ChatToolCall {
            call_id: "update-2".into(), name: TOOL_NAME.into(), provider_metadata: None,
            arguments: StrictJsonValue(serde_json::json!({
                "explanation":"Tests passed", "plan":[{"step":"Fix tests","status":"completed"}],
                "requirements":[{"id":"R1","description":"A weakened restatement",
                    "source":{"input":0,"quote":"Fix the failing tests"}}]
            })),
        };
        let input = ChatMessage { role: ChatRole::User, provider_round_id: None,
            content: vec![ChatContentPart::Text { text: "Fix the failing tests".into() }] };
        let result = plan.update(&call, &[input], &NoopAgentEventSink).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(plan.requirements, vec![original]);
        assert_eq!(plan.steps[0].status, AgentPlanStatus::Completed);
        assert!(plan.effect_gate().is_some_and(|reason|
            reason.contains("already current successful observation")));
        assert!(matches!(&result.output[0], ChatToolResultPart::Text { text }
            if text.contains("kept unchanged") && text.contains("Omit requirements")));
        let revision = plan.revision;
        let repeated = plan.update(&call, &[crate::context_lifecycle::text_message(
            ChatRole::User, "Fix the failing tests".into(),
        )], &NoopAgentEventSink).await.unwrap();
        assert!(repeated.is_error);
        assert_eq!(plan.revision, revision);
    }
}
