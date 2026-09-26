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
    pub revision: u32,
    pub explanation: String,
    pub steps: Vec<AgentPlanStep>,
    pub needs_replan: bool,
    #[serde(default)]
    pub requirements: Vec<crate::AgentTaskRequirement>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdatePlan {
    #[serde(default)]
    explanation: Option<String>,
    plan: Vec<AgentPlanStep>,
    #[serde(default)]
    requirements: Vec<crate::AgentTaskRequirement>,
}

pub(crate) fn definition() -> ChatToolDefinition {
    ChatToolDefinition {
        name: TOOL_NAME.into(),
        description: "Maintain a concise execution plan. At most one step in_progress. requirements is optional: the engine captures each otherwise-unaccounted accepted input as a full-scope requirement, while preserving the complete original input and constraints. For optional finer-grained requirements provide stable IDs and short exact accepted-input citations (input 0=original, later indices=corrections). On status updates omit requirements unless adding new IDs; old obligations cannot be rewritten or dropped. The response lists the ledger IDs that report_completion must cover. Repeated unchanged plans succeed idempotently but are not progress. Replan after uncertain effects or changed scope. This never authorizes verification or widens user scope.".into(),
        deferred: false,
        input_schema: StrictJsonValue(serde_json::json!({
            "type":"object", "additionalProperties":false,
            "required":["plan"],
            "properties":{
                "explanation":{"type":"string","minLength":1,"maxLength":2048,
                    "description":"Optional short explanation of the plan or its revision. The engine preserves accepted requirements independently of this text."},
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
            return Ok(self.feedback(call, "rejected", "Plan/requirements exceed the 48 KiB argument budget"));
        }
        let update = serde_json::from_value::<UpdatePlan>(call.arguments.0.clone());
        let update = match update {
            Ok(value) => value,
            Err(error) => {
                return Ok(self.feedback(call, "rejected", &format!("Invalid plan: {error}")));
            }
        };
        let explanation = update.explanation.unwrap_or_else(|| {
            if self.explanation.is_empty() { "Execution plan for the accepted task.".to_owned() }
            else { self.explanation.clone() }
        });
        let mut names = std::collections::BTreeSet::new();
        if explanation.trim().is_empty()
            || explanation.chars().count() > 2048
            || update.plan.is_empty()
            || update.plan.len() > 16
            || update.plan.iter().any(|item| {
                item.step.trim().is_empty()
                    || item.step.chars().count() > 512
                    || !names.insert(item.step.trim())
            })
            || update
                .plan
                .iter()
                .filter(|item| item.status == AgentPlanStatus::InProgress)
                .count()
                > 1
        {
            return Ok(self.feedback(call, "rejected",
                "Invalid plan: provide 1..16 unique steps (1..512 characters each), a nonempty explanation (up to 2048 characters), and at most one in_progress step. The current plan was not changed."));
        }
        let requirements =
            match crate::requirements::merge(&self.requirements, &update.requirements, inputs) {
                Ok(requirements) => requirements,
                Err(reason) => {
                    return Ok(self.feedback(call, "rejected", &reason));
                }
            };
        let ignored_restatements = update.requirements.iter().filter(|item| {
            self.requirements.iter().any(|existing| existing.id == item.id
                && (existing.description != item.description || existing.source != item.source))
        }).map(|item| item.id.as_str()).collect::<Vec<_>>();
        if !self.needs_replan && update.plan == self.steps && requirements == self.requirements {
            return Ok(self.feedback(call, "unchanged",
                "Plan already has these step statuses and immutable requirements; this idempotent update succeeded without recording a new revision or invalidating completion evidence. Perform the next authorized action, or report_completion if the work is finished. Repeating this update is not task progress."));
        }
        let next = Self {
            revision: self.revision.checked_add(1).ok_or_else(|| {
                AgentEngineError::InvalidContract("plan revision counter exhausted".into())
            })?,
            explanation,
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
        Ok(self.feedback(call, "updated", &feedback))
    }

    fn feedback(&self, call: &ChatToolCall, status: &str, message: &str) -> AgentToolResult {
        AgentToolResult::text(call.call_id.clone(), serde_json::json!({
            "status": status,
            "plan_revision": self.revision,
            "needs_replan": self.needs_replan,
            "plan": self.steps,
            "requirement_ids": self.requirements.iter().map(|item| &item.id).collect::<Vec<_>>(),
            "message": message,
        }).to_string(), status == "rejected")
    }

    pub(crate) fn effect_gate(&self) -> Option<&'static str> {
        if self.needs_replan {
            Some(
                "The plan needs reconsideration after an uncertain effect, newly accepted user input, or changed repository instructions. This proposed effect was not executed. Call update_plan alone now: explain the recovery and put one step in_progress. Preserve existing requirements; add exact accepted-input citations only for genuinely new requirements or inputs.",
            )
        } else if self.revision == 0 && self.steps.is_empty() {
            // Adaptive accounting may start after the model has already
            // proposed a valid batch. An absent optional plan is not a closed
            // plan and must not retroactively block that batch or hide tools.
            // Completion still requires accounting for every accepted input.
            None
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

    #[derive(Default)]
    struct RecordingSink(std::sync::Mutex<Vec<AgentEngineEvent>>);

    #[async_trait::async_trait]
    impl AgentEventSink for RecordingSink {
        async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
            self.0.lock().unwrap().push(event);
            Ok(())
        }
    }

    struct FailingSink;

    #[async_trait::async_trait]
    impl AgentEventSink for FailingSink {
        async fn emit(&self, _: AgentEngineEvent) -> Result<(), AgentEngineError> {
            Err(AgentEngineError::InvalidContract("fixture persistence failure".into()))
        }
    }

    fn inputs() -> Vec<ChatMessage> {
        vec![crate::context_lifecycle::text_message(ChatRole::User, "inspect".into())]
    }

    fn update_call(status: &str) -> ChatToolCall {
        ChatToolCall {
            call_id: "plan-call".into(), name: TOOL_NAME.into(), provider_metadata: None,
            arguments: StrictJsonValue(serde_json::json!({
                "explanation":"Inspect and verify the requested work",
                "plan":[{"step":"inspect", "status":status}],
                "requirements":[{"id":"R1", "description":"inspect", "source":{"input":0,"quote":"inspect"}}],
            })),
        }
    }

    #[tokio::test]
    async fn a_schema_valid_plan_without_requirements_is_executable_and_idempotent() {
        let mut call = update_call("in_progress");
        call.arguments.0.as_object_mut().unwrap().remove("requirements");
        let mut plan = AgentPlan::default();
        let sink = RecordingSink::default();
        assert!(!plan.update(&call, &inputs(), &sink).await.unwrap().is_error);
        assert_eq!(plan.requirements[0].id, "input_0");
        assert!(plan.effect_gate().is_none());
        assert!(!plan.update(&call, &inputs(), &sink).await.unwrap().is_error);
        assert_eq!(plan.revision, 1);
        assert_eq!(sink.0.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn replaying_plan_is_idempotent_but_replanning_is_a_real_transition() {
        let sink = RecordingSink::default();
        let mut plan = AgentPlan::default();
        let call = update_call("in_progress");
        assert!(!plan.update(&call, &inputs(), &sink).await.unwrap().is_error);
        let original = plan.clone();
        for _ in 0..10 {
            assert!(!plan.update(&call, &inputs(), &sink).await.unwrap().is_error);
        }
        assert_eq!(plan, original);
        assert_eq!(sink.0.lock().unwrap().len(), 1);

        plan.needs_replan = true;
        assert!(!plan.update(&call, &inputs(), &sink).await.unwrap().is_error);
        assert_eq!(plan.revision, 2);
        assert!(!plan.needs_replan);
        assert_eq!(sink.0.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn revisions_survive_long_tasks_without_an_arbitrary_sixty_four_update_limit() {
        let mut plan = AgentPlan::default();
        assert!(!plan.update(&update_call("in_progress"), &inputs(), &NoopAgentEventSink).await.unwrap().is_error);
        for index in 2..=130 {
            let status = if index % 2 == 0 { "pending" } else { "in_progress" };
            assert!(!plan.update(&update_call(status), &inputs(), &NoopAgentEventSink).await.unwrap().is_error);
            assert_eq!(plan.revision, index);
        }
        plan.revision = u32::from(u16::MAX);
        assert!(!plan.update(&update_call("completed"), &inputs(), &NoopAgentEventSink).await.unwrap().is_error);
        assert_eq!(plan.revision, 65_536);
        plan.revision = u32::MAX;
        let before = plan.clone();
        assert!(plan.update(&update_call("in_progress"), &inputs(), &NoopAgentEventSink).await.is_err());
        assert_eq!(plan, before, "revision overflow cannot wrap or mutate the plan");
    }

    #[tokio::test]
    async fn persistence_failure_and_invalid_proposal_preserve_the_committed_plan() {
        let mut plan = AgentPlan::default();
        plan.update(&update_call("in_progress"), &inputs(), &NoopAgentEventSink).await.unwrap();
        let before = plan.clone();
        assert!(plan.update(&update_call("completed"), &inputs(), &FailingSink).await.is_err());
        assert_eq!(plan, before);
        let mut invalid = update_call("in_progress");
        invalid.arguments.0["plan"] = serde_json::json!([
            {"step":"first", "status":"in_progress"},
            {"step":"second", "status":"in_progress"},
        ]);
        let result = plan.update(&invalid, &inputs(), &NoopAgentEventSink).await.unwrap();
        assert!(result.is_error);
        let feedback: serde_json::Value = serde_json::from_str(&result.output_text()).unwrap();
        assert_eq!(feedback["status"], "rejected");
        assert_eq!(feedback["plan_revision"], 1);
        assert_eq!(feedback["plan"][0]["step"], "inspect");
        assert_eq!(plan, before);
    }

    #[tokio::test]
    async fn schema_character_limits_do_not_reject_valid_chinese_plans_as_byte_overflows() {
        let quote = "检查".repeat(200);
        let input = vec![crate::context_lifecycle::text_message(ChatRole::User, quote.clone())];
        let mut call = update_call("in_progress");
        call.arguments.0["explanation"] = serde_json::Value::String("解释".repeat(800));
        call.arguments.0["plan"][0]["step"] = serde_json::Value::String("步骤".repeat(256));
        call.arguments.0["requirements"][0]["description"] = serde_json::Value::String("需求".repeat(256));
        call.arguments.0["requirements"][0]["source"]["quote"] = serde_json::Value::String(quote);
        let mut plan = AgentPlan::default();
        let result = plan.update(&call, &input, &NoopAgentEventSink).await.unwrap();
        assert!(!result.is_error, "{}", result.output_text());
        assert_eq!(plan.steps[0].step.chars().count(), 512);
    }

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
        assert!(!repeated.is_error);
        assert_eq!(serde_json::from_str::<serde_json::Value>(&repeated.output_text()).unwrap()["status"], "unchanged");
        assert_eq!(plan.revision, revision);
    }

    #[tokio::test]
    async fn optional_explanation_does_not_block_revisions_or_drop_accepted_requirements() {
        let mut plan = AgentPlan::default();
        let mut status = update_call("completed");
        status.arguments.0.as_object_mut().unwrap().remove("explanation");
        assert!(!plan.update(&status, &inputs(), &NoopAgentEventSink).await.unwrap().is_error);
        plan.update(&update_call("in_progress"), &inputs(), &NoopAgentEventSink).await.unwrap();
        let requirements = plan.requirements.clone();
        let explanation = plan.explanation.clone();
        let result = plan.update(&status, &inputs(), &NoopAgentEventSink).await.unwrap();
        assert!(!result.is_error, "{}", result.output_text());
        assert_eq!(plan.requirements, requirements);
        assert_eq!(plan.explanation, explanation);
        assert_eq!(plan.steps[0].status, AgentPlanStatus::Completed);
        status.arguments.0["plan"][0]["step"] = serde_json::json!("Different work");
        assert!(!plan.update(&status, &inputs(), &NoopAgentEventSink).await.unwrap().is_error);
        assert_eq!(plan.requirements, requirements);
        plan.needs_replan = true;
        let mut recovery = update_call("in_progress");
        recovery.arguments.0.as_object_mut().unwrap().remove("explanation");
        assert!(!plan.update(&recovery, &inputs(), &NoopAgentEventSink).await.unwrap().is_error);
        assert!(!plan.needs_replan);
        assert_eq!(plan.requirements, requirements);
    }

    #[test]
    fn absent_plan_allows_work_but_recovery_and_explicit_closure_still_gate_effects() {
        let mut plan = AgentPlan::default();
        assert!(plan.effect_gate().is_none());
        plan.needs_replan = true;
        assert!(plan.effect_gate().is_some());
        plan.needs_replan = false;
        plan.revision = 1;
        plan.steps.push(AgentPlanStep { step: "Create files".into(), status: AgentPlanStatus::Completed });
        assert!(plan.effect_gate().is_some());
        plan.steps[0].status = AgentPlanStatus::InProgress;
        assert!(plan.effect_gate().is_none());
    }
}
