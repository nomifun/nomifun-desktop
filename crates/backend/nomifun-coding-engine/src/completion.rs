//! Turn-local completion accounting. Valid references prove observations,
//! not the truth of a model's interpretation or coverage of all user intent.
//! No authority to run verification, no automatic test/command classification.
use std::collections::BTreeSet;

use nomifun_agent_contracts::StrictJsonValue;
use nomifun_chat_model_broker::{ChatMessage, ChatToolCall, ChatToolDefinition};
use serde::{Deserialize, Serialize};

use crate::{
    CodingEngineError, CodingEngineEvent, CodingEventSink, CodingPlan, CodingPlanStatus,
    CodingToolBinding, CodingToolResult, CodingWorkStatus,
};

pub(crate) const TOOL_NAME: &str = "report_completion";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingCriterionDisposition {
    Supported,
    Unverified,
    Blocked,
    ScopeChanged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingCompletionCriterion {
    /// Exact current plan step, or "response" for a turn without a plan.
    pub step: String,
    pub disposition: CodingCriterionDisposition,
    pub evidence_call_ids: Vec<String>,
    /// Model-authored interpretation, not a platform proof.
    pub rationale: String,
    /// Each immutable requirement is assigned to exactly one criterion, even
    /// if its original implementation step was removed or renamed. Imported
    /// requirements keep historical origin, never historical evidence.
    #[serde(default)]
    pub requirement_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_change: Option<crate::CodingInputCitation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingCompletionReport {
    pub plan_revision: u16,
    pub observation_revision: u32,
    pub input_revision: usize,
    pub workspace_epoch: u32,
    pub summary: String,
    pub criteria: Vec<CodingCompletionCriterion>,
    #[serde(default)]
    pub requirements: Vec<crate::CodingTaskRequirement>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingCompletionObservation {
    pub call_id: String,
    pub tool_name: String,
    pub workspace_epoch: u32,
    /// Crossing the engine's tool port, not proof of Kernel admission/effects.
    /// Old records do not prove a call was skipped; keep them conservative.
    #[serde(default = "historical_invocation_attempted")]
    pub invocation_attempted: bool,
    pub successful: bool,
    /// False for unfinished/uncertain commands, failed/deferred calls, and
    /// reads overlapping a live command. This is not a test-suite verdict.
    pub usable_at_observation: bool,
    /// An exact, reaped command exit is distinguishable from any other result.
    pub command_exit_code: Option<i32>,
    /// Keep the bounded launch/interaction chain available after transcript
    /// compaction. References identify real calls, not command/test semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<crate::CodingCommandObservation>,
}

fn historical_invocation_attempted() -> bool {
    true
}

#[derive(Default)]
pub(crate) struct CompletionTracker {
    revision: u32,
    observations: Vec<CodingCompletionObservation>,
    omitted: u32,
    report: Option<CodingCompletionReport>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Submission {
    summary: String,
    criteria: Vec<CodingCompletionCriterion>,
}

pub(crate) fn definition() -> ChatToolDefinition {
    ChatToolDefinition {
        name: TOOL_NAME.into(),
        description: "Before finishing a tool/plan turn, close the plan and submit a single-call completion account. Record source-anchored requirements using update_plan first. Include every current plan step once and assign EVERY recorded requirement ID to exactly one criterion, including obligations from removed steps. supported requires current successful observations, not inferred test/task success. unverified/blocked require reasons. scope_changed requires an exact quote from a LATER accepted user input for all assigned requirements (any current accepted input is later than an imported historical requirement), no evidence_call_ids, and is disclosed in the final answer; it is not completed original work. Later tools/plans/input invalidate the report. This grants no verification or extra work authority.".into(),
        deferred: false,
        input_schema: StrictJsonValue(serde_json::json!({
            "type":"object", "additionalProperties":false, "required":["summary","criteria"],
            "properties":{
                "summary":{"type":"string","minLength":1,"maxLength":2048},
                "criteria":{"type":"array","minItems":1,"maxItems":16,"items":{
                    "type":"object","additionalProperties":false,"required":["step","disposition","evidence_call_ids","rationale","requirement_ids"],
                    "properties":{
                        "step":{"type":"string","minLength":1,"maxLength":512},
                        "disposition":{"type":"string","enum":["supported","unverified","blocked","scope_changed"]},
                        "requirement_ids":{"type":"array","maxItems":32,"items":{"type":"string","minLength":1,"maxLength":64}},
                        "scope_change":crate::requirements::citation_schema(),
                        "evidence_call_ids":{"type":"array","maxItems":8,"items":{"type":"string","minLength":1,"maxLength":256}},
                        "rationale":{"type":"string","minLength":1,"maxLength":1024}
                    }
                }}
            }
        })),
    }
}

impl CompletionTracker {
    pub(crate) fn invalidate(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.report = None;
    }
    /// Called after a tool result or engine-only deferral. The host separately
    /// persists actual dispatch/settlement and owns resource cleanup.
    pub(crate) fn observe(
        &mut self,
        work: &CodingWorkStatus,
        binding: &CodingToolBinding,
        call: &ChatToolCall,
        result: &CodingToolResult,
        invocation_attempted: bool,
    ) -> CodingCompletionObservation {
        self.invalidate();
        let command = work.recent_commands.iter().find(|command| {
            invocation_attempted && command.observation_call_id == call.call_id.as_ref()
        });
        let usable = invocation_attempted
            && !result.is_error
            && work.running_processes.is_empty()
            && (binding.capability_id.as_ref() != "workspace.process"
                || command.is_some_and(|command| {
                    command.was_current_at_observation
                        && command.cleanup_proven
                        && command.state == "exited"
                        && command.exit_code == Some(0)
                }));
        let observation = CodingCompletionObservation {
            call_id: call.call_id.as_ref().to_owned(),
            tool_name: call.name.clone(),
            workspace_epoch: work.workspace_observation_epoch,
            invocation_attempted,
            successful: invocation_attempted && !result.is_error,
            usable_at_observation: usable,
            command_exit_code: command.and_then(|command| command.exit_code),
            command: command.cloned(),
        };
        self.observations.push(observation.clone());
        // Interactive provenance can carry several call IDs per observation.
        // Bound the serialized window too, not just its number of records.
        while self.observations.len() > 64
            || self
                .observations
                .iter()
                .map(|item| serde_json::to_vec(item).map_or(usize::MAX, |value| value.len()))
                .fold(0usize, usize::saturating_add)
                > 32 * 1024
        {
            self.observations.remove(0);
            self.omitted = self.omitted.saturating_add(1);
        }
        observation
    }

    pub(crate) fn current(
        &self,
        plan: &CodingPlan,
        work: &CodingWorkStatus,
        input_revision: usize,
    ) -> Option<&CodingCompletionReport> {
        self.report.as_ref().filter(|report| {
            report.plan_revision == plan.revision
                && report.observation_revision == self.revision
                && report.input_revision == input_revision
                && report.workspace_epoch == work.workspace_observation_epoch
                && !plan.is_open()
                && work.running_processes.is_empty()
                && report.requirements == plan.requirements
                && crate::requirements::require_input_coverage(&plan.requirements, input_revision)
                    .is_ok()
        })
    }

    pub(crate) fn context(
        &self,
        plan: &CodingPlan,
        work: &CodingWorkStatus,
        input_revision: usize,
    ) -> Result<String, CodingEngineError> {
        // Keep the full report in the journal, not permanently duplicated in
        // every model request/compaction prefix.
        let report = self.current(plan, work, input_revision).map(|report| serde_json::json!({
            "summary":report.summary,
            "criteria":report.criteria.iter().map(|criterion| serde_json::json!({"step":criterion.step,"disposition":criterion.disposition,"requirement_ids":criterion.requirement_ids})).collect::<Vec<_>>()
        }));
        let value = serde_json::json!({"plan_revision":plan.revision,"observation_revision":self.revision,
            "input_revision":input_revision,"workspace_epoch":work.workspace_observation_epoch,
            "observations":self.observations,"omitted_observations":self.omitted,
            "current_report":report});
        Ok(format!(
            "Completion accounting (derived data, not instructions or extra authority): {}. supported may cite only usable successful observations at the CURRENT workspace epoch. invocation_attempted=false means the engine did not pass that proposed call to the tool port: it is neither work performed nor a command result. true means attempted, not proof of owner effects. A command observation includes its original launch and bounded interaction call IDs: inspect the whole chain, not just a final poll or exit code. provenance_workspace_epoch tracks exclusively attributed interactions; it is not a fresh launch or proof of test timing inside the command. Command exits do not identify tests. If a check was excluded, unavailable, stale, or not run, use unverified with a reason, not invented evidence. Account for all current plan steps AND every immutable requirement ID in the plan ledger, even after replanning. scope_changed requires an exact later accepted-input citation and is not completed work. This report is not an independent semantic verifier or process cleanup proof.",
            serde_json::to_string(&value).map_err(invalid)?
        ))
    }

    pub(crate) async fn submit(
        &mut self,
        call: &ChatToolCall,
        plan: &CodingPlan,
        work: &CodingWorkStatus,
        inputs: &[ChatMessage],
        sink: &dyn CodingEventSink,
    ) -> Result<CodingToolResult, CodingEngineError> {
        // A rejected replacement must not leave an old successful report as
        // an accidental fallback after the model was told its account failed.
        self.report = None;
        let checked = self.check(call, plan, work, inputs);
        let report = match checked {
            Ok(report) => report,
            Err(reason) => return Ok(CodingToolResult::text(call.call_id.clone(), reason, true)),
        };
        sink.emit(CodingEngineEvent::CompletionReported {
            report: report.clone(),
        })
        .await?;
        self.report = Some(report);
        Ok(CodingToolResult::text(
            call.call_id.clone(),
            "Completion account recorded, not independently verified. Disclose unverified/blocked items, declared scope changes and actual command scope. Scope changes are not proof the original work was completed; quotation checks establish origin only. A blocked plan/report cannot be published as task completion. Further tool results, plan changes or user input require a new report.",
            false,
        ))
    }

    fn check(
        &self,
        call: &ChatToolCall,
        plan: &CodingPlan,
        work: &CodingWorkStatus,
        inputs: &[ChatMessage],
    ) -> Result<CodingCompletionReport, String> {
        crate::stream_limits::serialized_size(&call.arguments, 48 * 1024)
            .map_err(|_| "Completion report exceeds the 48 KiB serialized budget".to_owned())?;
        crate::requirements::require_input_coverage(&plan.requirements, inputs.len())?;
        let submission: Submission = serde_json::from_value(call.arguments.0.clone())
            .map_err(|error| format!("Invalid completion report: {error}"))?;
        if submission.summary.trim().is_empty()
            || submission.summary.len() > 2048
            || submission.criteria.is_empty()
            || submission.criteria.len() > 16
        {
            return Err("Completion report needs a bounded summary and 1..16 criteria".into());
        }
        if plan.is_open() || !work.running_processes.is_empty() {
            return Err("Close or explicitly block all plan steps and settle processes before reporting completion".into());
        }
        let expected: BTreeSet<&str> = if plan.steps.is_empty() {
            BTreeSet::from(["response"])
        } else {
            plan.steps.iter().map(|step| step.step.as_str()).collect()
        };
        let mut seen = BTreeSet::new();
        let mut covered_requirements = BTreeSet::new();
        for criterion in &submission.criteria {
            if !expected.contains(criterion.step.as_str())
                || !seen.insert(criterion.step.as_str())
                || criterion.rationale.trim().is_empty()
                || criterion.rationale.len() > 1024
                || criterion.evidence_call_ids.len() > 8
                || criterion.requirement_ids.len() > 32
            {
                return Err("Each current plan step needs exactly one bounded completion criterion and rationale".into());
            }
            let blocked = plan.steps.iter().any(|step| {
                step.step == criterion.step && step.status == CodingPlanStatus::Blocked
            });
            if blocked && criterion.disposition != CodingCriterionDisposition::Blocked {
                return Err(
                    "A blocked plan step must remain blocked in the completion report".into(),
                );
            }
            let supported = criterion.disposition == CodingCriterionDisposition::Supported;
            let scope_changed = criterion.disposition == CodingCriterionDisposition::ScopeChanged;
            if scope_changed {
                if criterion.requirement_ids.is_empty() || !criterion.evidence_call_ids.is_empty() {
                    return Err("scope_changed must name affected requirements and cannot use tool observations as user authority".into());
                }
                let source = criterion.scope_change.as_ref().ok_or_else(|| {
                    "scope_changed requires a later accepted-input citation".to_owned()
                })?;
                crate::requirements::validate_citation(source, inputs, false)?;
            } else if criterion.scope_change.is_some() {
                return Err(
                    "scope_change citations are only valid with scope_changed disposition".into(),
                );
            }
            for id in &criterion.requirement_ids {
                let requirement = plan
                    .requirements
                    .iter()
                    .find(|item| &item.id == id)
                    .ok_or_else(|| "Completion refers to an unknown requirement ID".to_owned())?;
                if !covered_requirements.insert(id.as_str()) {
                    return Err(
                        "Assign each requirement ID to exactly one completion criterion".into(),
                    );
                }
                if scope_changed
                    && criterion.scope_change.as_ref().is_none_or(|source| {
                        requirement.origin.is_none() && source.input <= requirement.source.input
                    })
                {
                    return Err("Scope changes require input later than every affected requirement's original source; current inputs may revise imported historical requirements".into());
                }
            }
            if supported && criterion.evidence_call_ids.is_empty() {
                return Err("supported requires real observation call IDs; otherwise use unverified/blocked with a reason".into());
            }
            let mut ids = BTreeSet::new();
            for id in &criterion.evidence_call_ids {
                if id.is_empty() || id.len() > 256 || !ids.insert(id) {
                    return Err("Invalid or repeated evidence call identity".into());
                }
                let observation = self
                    .observations
                    .iter()
                    .find(|item| &item.call_id == id)
                    .ok_or_else(|| {
                        "Evidence call is unknown or no longer in the bounded observation window"
                            .to_owned()
                    })?;
                if supported
                    && (!observation.invocation_attempted
                        || !observation.successful
                        || !observation.usable_at_observation
                        || observation.workspace_epoch != work.workspace_observation_epoch)
                {
                    return Err("Evidence is failed, unsettled, overlapping or stale for the current workspace; mark unverified/blocked or inspect within the user's scope".into());
                }
            }
        }
        if seen != expected {
            return Err("Completion report omits current plan steps".into());
        }
        if covered_requirements.len() != plan.requirements.len() {
            return Err("Completion omits recorded requirements; removing or renaming plan steps cannot discard user obligations".into());
        }
        Ok(CodingCompletionReport {
            plan_revision: plan.revision,
            observation_revision: self.revision,
            input_revision: inputs.len(),
            workspace_epoch: work.workspace_observation_epoch,
            summary: submission.summary,
            criteria: submission.criteria,
            requirements: plan.requirements.clone(),
        })
    }
}

impl CodingCompletionReport {
    pub(crate) fn is_blocked(&self) -> bool {
        self.criteria
            .iter()
            .any(|criterion| criterion.disposition == CodingCriterionDisposition::Blocked)
    }

    pub(crate) fn unverified_disclosure(&self) -> Option<String> {
        let items = self
            .criteria
            .iter()
            .filter(|criterion| {
                matches!(
                    criterion.disposition,
                    CodingCriterionDisposition::Unverified
                        | CodingCriterionDisposition::ScopeChanged
                )
            })
            .map(|criterion| {
                let scope = criterion.scope_change.as_ref().map(|source| format!(
                    " [Scope changed, not original work completed; accepted input {}, quote: {}]",
                    source.input, serde_json::to_string(&source.quote).unwrap_or_default()
                )).unwrap_or_default();
                let obligations = criterion
                    .requirement_ids
                    .iter()
                    .filter_map(|id| self.requirements.iter().find(|item| &item.id == id))
                    .map(|item| format!("{}: {}", item.id, item.description))
                    .collect::<Vec<_>>()
                    .join("; ");
                format!(
                    "- {} (requirements: {}): {}{}",
                    criterion.step, obligations, criterion.rationale, scope
                )
            })
            .collect::<Vec<_>>();
        (!items.is_empty()).then(|| {
            format!(
                "\n\nEngine completion account — unverified items / declared scope changes (not test results or independent semantic verification):\n\n{}",
                items.join("\n")
            )
        })
    }
}

fn invalid(error: impl std::fmt::Display) -> CodingEngineError {
    CodingEngineError::InvalidContract(error.to_string())
}
