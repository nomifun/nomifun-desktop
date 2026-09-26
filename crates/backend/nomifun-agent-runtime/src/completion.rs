//! Turn-local completion accounting. Valid references prove observations,
//! not the truth of a model's interpretation or coverage of all user intent.
//! No authority to run verification, no automatic test/command classification.
use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::StrictJsonValue;
use nomifun_chat_model_broker::{ChatMessage, ChatToolCall, ChatToolDefinition};
use serde::{Deserialize, Serialize};

use crate::{
    AgentEngineError, AgentEngineEvent, AgentEventSink, AgentPlan, AgentPlanStatus,
    AgentToolBinding, AgentToolResult, AgentWorkStatus,
};

pub(crate) const TOOL_NAME: &str = "report_completion";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCriterionDisposition {
    Supported,
    Unverified,
    Blocked,
    ScopeChanged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCompletionCriterion {
    /// Descriptive deliverable/check label; not an exact plan-label identity.
    pub step: String,
    pub disposition: AgentCriterionDisposition,
    pub evidence_call_ids: Vec<String>,
    /// Model-authored interpretation, not a platform proof.
    pub rationale: String,
    /// Each immutable requirement is covered by at least one criterion, even
    /// if its original implementation step was removed or renamed. Imported
    /// requirements keep historical origin, never historical evidence.
    #[serde(default)]
    pub requirement_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_change: Option<crate::AgentInputCitation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCompletionReport {
    pub plan_revision: u32,
    pub observation_revision: u32,
    pub input_revision: usize,
    pub workspace_epoch: u32,
    pub summary: String,
    pub criteria: Vec<AgentCompletionCriterion>,
    #[serde(default)]
    pub requirements: Vec<crate::AgentTaskRequirement>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCompletionObservation {
    pub call_id: String,
    pub tool_name: String,
    /// Owner-observed workspace target, for deterministic evidence selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
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
    pub command: Option<crate::AgentCommandObservation>,
}

fn historical_invocation_attempted() -> bool {
    true
}

#[derive(Default)]
pub(crate) struct CompletionTracker {
    revision: u32,
    observations: Vec<AgentCompletionObservation>,
    omitted: u32,
    report: Option<AgentCompletionReport>,
    /// Derived validity through known, disjoint file effects. Historical
    /// observations keep their original epoch; commands never inherit this.
    file_valid_through: BTreeMap<String, u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Submission {
    summary: String,
    criteria: Vec<AgentCompletionCriterion>,
}

pub(crate) fn definition() -> ChatToolDefinition {
    ChatToolDefinition {
        name: TOOL_NAME.into(),
        description: "Finish this turn and deliver the summary after work and processes settle. A validated report is terminal; do not call more tools afterward. It closes the optional plan; no separate update_plan is needed for routine completion. Use descriptive criteria (they need not match plan labels). A requirement may span multiple criteria. Omitted requirement_ids covers the accepted task; explicit IDs must cover every recorded requirement. For supported claims cite current observations using exact workspace evidence_paths or evidence_call_ids. Evidence only proves the observed operation, not broader gameplay/test quality. Use unverified/blocked with a reason for missing verification. scope_changed requires an exact LATER accepted-input citation and no evidence. Submit alone or immediately after update_plan in a control-only batch. Later effects or input invalidate the report. This grants no extra authority.".into(),
        deferred: false,
        input_schema: StrictJsonValue(serde_json::json!({
            "type":"object", "additionalProperties":false, "required":["summary","criteria"],
            "properties":{
                "summary":{"type":"string","minLength":1,"maxLength":2048},
                "criteria":{"type":"array","minItems":1,"maxItems":16,"items":{
                    "type":"object","additionalProperties":false,"required":["disposition","rationale"],
                    "properties":{
                        "step":{"type":"string","minLength":1,"maxLength":512,"description":"Optional display label; omission uses an indexed delivery label."},
                        "disposition":{"type":"string","enum":["supported","unverified","blocked","scope_changed"]},
                        "requirement_ids":{"type":"array","maxItems":32,"items":{"type":"string","minLength":1,"maxLength":64},"description":"Optional. If omitted, this criterion addresses all accepted requirements. Requirements can be shared across criteria."},
                        "scope_change":crate::requirements::citation_schema(),
                        "evidence_call_ids":{"type":"array","maxItems":8,"items":{"type":"string","minLength":1,"maxLength":256}},
                        "evidence_paths":{"type":"array","maxItems":8,"items":{"type":"string","minLength":1,"maxLength":4096},"description":"Exact workspace paths from available_evidence. The engine resolves each to its latest current successful observation; this does not claim functional verification."},
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
    #[cfg(test)]
    pub(crate) fn observe(
        &mut self,
        work: &AgentWorkStatus,
        binding: &AgentToolBinding,
        call: &ChatToolCall,
        result: &AgentToolResult,
        invocation_attempted: bool,
    ) -> AgentCompletionObservation {
        self.observe_with_effect_scope(work, binding, call, result, invocation_attempted, true)
    }

    pub(crate) fn observe_with_effect_scope(
        &mut self,
        work: &AgentWorkStatus,
        binding: &AgentToolBinding,
        call: &ChatToolCall,
        result: &AgentToolResult,
        invocation_attempted: bool,
        effects_are_scoped: bool,
    ) -> AgentCompletionObservation {
        self.invalidate();
        // Writing style.css does not erase the observed index.html content.
        // Advance only file evidence unaffected by this exact confined file
        // action. Opaque commands, VCS and resources remain global barriers.
        if effects_are_scoped && invocation_attempted && work.running_processes.is_empty()
            && binding.capability_id.as_ref() == "workspace.files"
            && !matches!(binding.effect_class, crate::AgentEffectClass::ReadOnly)
            && let Some(previous_epoch) = work.workspace_observation_epoch.checked_sub(1)
        {
            let targets: Option<Vec<String>> = match binding.action_id.as_ref() {
                "workspace.files/write" | "workspace.files/delete" => call.arguments.0["path"].as_str()
                    .and_then(|path| crate::agents_md::normalize_workspace_directory(path).ok()).map(|path| vec![path]),
                "workspace.files/patch" => call.arguments.0["files"].as_array().and_then(|files| files.iter()
                    .map(|file| file["path"].as_str().and_then(|path| crate::agents_md::normalize_workspace_directory(path).ok()))
                    .collect()),
                _ => None,
            };
            if let Some(targets) = targets.filter(|targets| !targets.is_empty()) {
                for observation in &self.observations {
                    if observation.path.as_ref().is_some_and(|path| !targets.iter().any(|target|
                        file_paths_may_overlap(path, target)))
                        && self.is_usable(observation, previous_epoch)
                    {
                        self.file_valid_through.insert(observation.call_id.clone(), work.workspace_observation_epoch);
                    }
                }
            }
        }
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
        let observation = AgentCompletionObservation {
            call_id: call.call_id.as_ref().to_owned(),
            tool_name: call.name.clone(),
            path: (binding.capability_id.as_ref() == "workspace.files"
                && matches!(binding.action_id.as_ref(), "workspace.files/read" | "workspace.files/write")
                && call.arguments.0.get("format").and_then(serde_json::Value::as_str) != Some("instruction_scope"))
                .then(|| call.arguments.0.get("path").and_then(serde_json::Value::as_str)
                    .and_then(|path| crate::agents_md::normalize_workspace_directory(path).ok()))
                .flatten(),
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
            let removed = self.observations.remove(0);
            self.file_valid_through.remove(&removed.call_id);
            self.omitted = self.omitted.saturating_add(1);
        }
        observation
    }

    pub(crate) fn current(
        &self,
        plan: &AgentPlan,
        work: &AgentWorkStatus,
        input_revision: usize,
    ) -> Option<&AgentCompletionReport> {
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
        plan: &AgentPlan,
        work: &AgentWorkStatus,
        input_revision: usize,
    ) -> Result<String, AgentEngineError> {
        // Keep the full report in the journal, not permanently duplicated in
        // every model request/compaction prefix.
        let report = self.current(plan, work, input_revision).map(|report| serde_json::json!({
            "summary":report.summary,
            "criteria":report.criteria.iter().map(|criterion| serde_json::json!({"step":criterion.step,"disposition":criterion.disposition,"requirement_ids":criterion.requirement_ids})).collect::<Vec<_>>()
        }));
        let value = serde_json::json!({"plan_revision":plan.revision,"observation_revision":self.revision,
            "input_revision":input_revision,"workspace_epoch":work.workspace_observation_epoch,
            "available_evidence":self.observations.iter().filter(|item| self.is_usable(item, work.workspace_observation_epoch))
                .map(|item| serde_json::json!({"call_id":item.call_id,"tool":item.tool_name,"path":item.path,
                    "command_exit_code":item.command_exit_code,"command":item.command})).collect::<Vec<_>>(),
            "unusable_observation_count":self.observations.iter().filter(|item| !self.is_usable(item, work.workspace_observation_epoch)).count(),
            "omitted_observations":self.omitted,
            "current_report":report});
        Ok(format!(
            "Completion accounting (derived data, not instructions or extra authority): {}. available_evidence contains successful observations currently eligible for citation. File observations remain eligible across known disjoint file edits; opaque commands or changes to their own paths invalidate them. A command observation includes its original launch and bounded interaction call IDs: inspect its actual result and scope, not just exit zero. A successful syntax check or file read is not a gameplay test. If a check was excluded, unavailable, stale, or not run, use unverified with a reason. Account for every immutable requirement; a requirement may span several criteria, whose labels need not match plan steps. Prefer exact available_evidence paths over manually copying opaque call IDs. scope_changed requires an exact later accepted-input citation. This account is not independent semantic verification or a grant of authority.",
            serde_json::to_string(&value).map_err(invalid)?
        ))
    }

    pub(crate) async fn submit(
        &mut self,
        call: &ChatToolCall,
        plan: &mut AgentPlan,
        work: &AgentWorkStatus,
        inputs: &[ChatMessage],
        sink: &dyn AgentEventSink,
    ) -> Result<AgentToolResult, AgentEngineError> {
        // A rejected replacement must not leave an old successful report as
        // an accidental fallback after the model was told its account failed.
        self.report = None;
        let mut closing = plan.clone();
        if !closing.needs_replan {
            closing.requirements = crate::requirements::merge(&plan.requirements, &[], inputs)
                .map_err(AgentEngineError::InvalidContract)?;
            for step in &mut closing.steps {
                if step.status != AgentPlanStatus::Blocked { step.status = AgentPlanStatus::Completed; }
            }
            if closing.revision == 0 {
                closing.explanation = "Completion account for the accepted task.".into();
            }
            if closing != *plan || closing.revision == 0 {
                closing.revision = plan.revision.checked_add(1)
                    .ok_or_else(|| invalid("plan revision counter exhausted"))?;
            }
        }
        let checked = self.check(call, &closing, work, inputs);
        let report = match checked {
            Ok(report) => report,
            Err(reason) => return Ok(AgentToolResult::text(call.call_id.clone(), reason, true)),
        };
        // Validate the entire account before changing control state. A bad
        // evidence reference cannot accidentally close the current plan.
        if closing != *plan {
            sink.emit(AgentEngineEvent::PlanUpdated { plan: closing.clone() }).await?;
            *plan = closing;
        }
        sink.emit(AgentEngineEvent::CompletionReported {
            report: report.clone(),
        })
        .await?;
        self.report = Some(report);
        Ok(AgentToolResult::text(
            call.call_id.clone(),
            "Completion account recorded, not independently verified. Disclose unverified/blocked items, declared scope changes and actual command scope. Scope changes are not proof the original work was completed; quotation checks establish origin only. A blocked plan/report cannot be published as task completion. Further tool results, plan changes or user input require a new report.",
            false,
        ))
    }

    fn check(
        &self,
        call: &ChatToolCall,
        plan: &AgentPlan,
        work: &AgentWorkStatus,
        inputs: &[ChatMessage],
    ) -> Result<AgentCompletionReport, String> {
        crate::stream_limits::serialized_size(&call.arguments, 48 * 1024)
            .map_err(|_| "Completion report exceeds the 48 KiB serialized budget".to_owned())?;
        if plan.revision == 0 || plan.needs_replan {
            return Err("Call update_plan alone first; report_completion cannot close a missing or stale plan".into());
        }
        crate::requirements::require_input_coverage(&plan.requirements, inputs.len())?;
        let submission: Submission = serde_json::from_value(self.resolve_submission(call, plan, work)?)
            .map_err(|error| format!("Invalid completion report: {error}"))?;
        if submission.summary.trim().is_empty()
            || submission.summary.chars().count() > 2048
            || submission.criteria.is_empty()
            || submission.criteria.len() > 16
        {
            return Err("Completion report needs a bounded summary and 1..16 criteria".into());
        }
        if plan.is_open() || !work.running_processes.is_empty() {
            return Err("Do not retry report_completion yet. Call update_plan ALONE first with every current step completed or explicitly blocked, and settle any running process. After that result is recorded, call report_completion ALONE in a later model step.".into());
        }
        let mut covered_requirements = BTreeSet::new();
        let mut changed_requirements = BTreeSet::new();
        let mut unchanged_requirements = BTreeSet::new();
        if plan.steps.iter().any(|step| step.status == AgentPlanStatus::Blocked)
            && !submission.criteria.iter().any(|item| item.disposition == AgentCriterionDisposition::Blocked) {
            return Err("An explicitly blocked plan still needs a blocked completion criterion or an explicit plan revision.".into());
        }
        for criterion in &submission.criteria {
            if criterion.step.trim().is_empty() || criterion.step.chars().count() > 512
                || criterion.rationale.trim().is_empty()
                || criterion.rationale.chars().count() > 1024
                || criterion.evidence_call_ids.len() > 8
                || criterion.requirement_ids.len() > 32
            {
                return Err("Each criterion needs a nonempty rationale of at most 1024 characters, at most 8 evidence call IDs, and at most 32 requirement IDs".into());
            }
            let supported = criterion.disposition == AgentCriterionDisposition::Supported;
            let scope_changed = criterion.disposition == AgentCriterionDisposition::ScopeChanged;
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
                covered_requirements.insert(id.as_str());
                if scope_changed { changed_requirements.insert(id.as_str()); }
                else { unchanged_requirements.insert(id.as_str()); }
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
                    && !self.is_usable(observation, work.workspace_observation_epoch)
                {
                    let guidance = self.stale_evidence_guidance(work.workspace_observation_epoch);
                    return Err(format!("Evidence is failed, unsettled, overlapping or stale for the current workspace. Do not repeat this report unchanged. {guidance}"));
                }
            }
        }
        if !changed_requirements.is_disjoint(&unchanged_requirements) {
            return Err("A requirement cannot be both scope_changed and claimed under its original scope in the same account.".into());
        }
        if covered_requirements.len() != plan.requirements.len() {
            let missing = plan.requirements.iter()
                .filter(|item| !covered_requirements.contains(item.id.as_str()))
                .map(|item| item.id.as_str()).take(32).collect::<Vec<_>>();
            return Err(format!("Completion omits recorded requirements; cover these IDs in at least one criterion: {}. Removing or renaming plan steps cannot discard user obligations.",
                serde_json::to_string(&missing).unwrap_or_default()));
        }
        Ok(AgentCompletionReport {
            plan_revision: plan.revision,
            observation_revision: self.revision,
            input_revision: inputs.len(),
            workspace_epoch: work.workspace_observation_epoch,
            summary: submission.summary,
            criteria: submission.criteria,
            requirements: plan.requirements.clone(),
        })
    }

    fn resolve_submission(&self, call: &ChatToolCall, plan: &AgentPlan, work: &AgentWorkStatus) -> Result<serde_json::Value, String> {
        let mut value = call.arguments.0.clone();
        if let Some(criteria) = value.get_mut("criteria").and_then(serde_json::Value::as_array_mut) {
            for (index,criterion) in criteria.iter_mut().enumerate() {
                let Some(fields) = criterion.as_object_mut() else { continue; };
                fields.entry("step").or_insert_with(|| serde_json::json!((index + 1).to_string()));
                fields.entry("requirement_ids").or_insert_with(|| serde_json::json!(
                    plan.requirements.iter().map(|requirement| &requirement.id).collect::<Vec<_>>()));
                let paths = fields.remove("evidence_paths").unwrap_or_else(|| serde_json::json!([]));
                let paths = paths.as_array().ok_or("evidence_paths must be an array")?;
                let ids = fields.entry("evidence_call_ids").or_insert_with(|| serde_json::json!([]))
                    .as_array_mut().ok_or("evidence_call_ids must be an array")?;
                for path in paths {
                    let path = path.as_str().ok_or("evidence_paths entries must be strings")?;
                    let normalized = crate::agents_md::normalize_workspace_directory(path.strip_prefix("./").unwrap_or(path)).map_err(|error| error.to_string())?;
                    let observation = self.observations.iter().rev().find(|item|
                        item.path.as_deref() == Some(&normalized) && self.is_usable(item, work.workspace_observation_epoch))
                        .ok_or_else(|| format!("No current successful observation for workspace path {}. Read it if authorized, or mark the claim unverified.", serde_json::to_string(path).unwrap_or_default()))?;
                    let id = serde_json::Value::String(observation.call_id.clone());
                    if !ids.contains(&id) { ids.push(id); }
                }
            }
        }
        Ok(value)
    }

    fn is_usable(&self, observation: &AgentCompletionObservation, epoch: u32) -> bool {
        observation.invocation_attempted && observation.successful && observation.usable_at_observation
            && (observation.workspace_epoch == epoch
                || (observation.path.is_some() && self.file_valid_through.get(&observation.call_id) == Some(&epoch)))
    }

fn stale_evidence_guidance(&self, epoch: u32) -> String {
    let usable = self.observations.iter().rev()
        .filter(|item| self.is_usable(item,epoch))
        .take(8).map(|item| serde_json::json!({"call_id":item.call_id,"path":item.path,"tool":item.tool_name})).collect::<Vec<_>>();
    if usable.is_empty() {
        "No current usable observation exists. If verification is authorized, reopen one plan step as in_progress with update_plan, run the final check, close the plan, then report without any further command; otherwise mark unverified with a reason.".to_owned()
    } else {
        format!("Current usable observation call IDs (not semantic proof): {}. Inspect the actual result and cite the matching successful call ID. A blocked/rejected call is not evidence; do not launch another command merely to refresh an already usable observation.",
            serde_json::to_string(&usable).unwrap_or_default())
    }
}

}

// macOS/Windows volumes commonly alias case and can alias Unicode spellings.
// Without an owner-provided filesystem identity, use conservative ASCII case
// comparison and do not carry evidence across non-ASCII file mutations.
fn file_paths_may_overlap(observed: &str, target: &str) -> bool {
    if target.is_empty() || !observed.is_ascii() || !target.is_ascii() { return true; }
    let observed = observed.to_ascii_lowercase();
    let target = target.to_ascii_lowercase();
    observed == target || observed.starts_with(&format!("{target}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_observation(id: &str, path: &str, epoch: u32) -> AgentCompletionObservation {
        AgentCompletionObservation { call_id:id.into(), tool_name:"read_file".into(), path:Some(path.into()),
            workspace_epoch:epoch, invocation_attempted:true, successful:true,
            usable_at_observation:true, command_exit_code:None, command:None }
    }

    fn file_binding(action: &str) -> AgentToolBinding {
        let schema = StrictJsonValue(serde_json::json!({"type":"object"}));
        AgentToolBinding { model_name:"file_action".into(),
            definition:ChatToolDefinition { name:"file_action".into(),description:"fixture".into(),input_schema:schema.clone(),deferred:false },
            schema_digest:crate::input_schema_digest(&schema).unwrap(),
            canonical_input_schema_ref:"schema://fixture/file".into(),capability_contract_digest:"a".repeat(64).into(),
            capability_id:"workspace.files".into(), action_id:action.into(),resource_binding_ids:Default::default(),
            effect_class:crate::AgentEffectClass::ManagedEffect,parallel_safe:false }
    }

    #[test]
    fn disjoint_file_edits_preserve_file_evidence_without_relabeling_history_or_refreshing_commands() {
        assert!(file_paths_may_overlap("Assets/Game.js", "assets"));
        assert!(file_paths_may_overlap("index.html", "INDEX.HTML"));
        assert!(file_paths_may_overlap("café.js", "cafe\u{301}.js"));
        assert!(!file_paths_may_overlap("assets-other.css", "assets"));
        let mut tracker = CompletionTracker { observations:vec![file_observation("html","index.html",1),
            file_observation("nested","assets/style.css",1),file_observation("sibling","assets-other.css",1)], ..Default::default() };
        let mutate = |tracker: &mut CompletionTracker, action: &str, path: &str, epoch| {
            let call = ChatToolCall { call_id:format!("effect-{epoch}").into(),name:"file_action".into(),
                arguments:StrictJsonValue(serde_json::json!({"path":path,"content":"updated"})),provider_metadata:None };
            tracker.observe(&AgentWorkStatus { workspace_observation_epoch:epoch,..Default::default() },
                &file_binding(action),&call,&AgentToolResult::text(call.call_id.clone(),"ok",false),true);
        };
        mutate(&mut tracker,"workspace.files/write","game.js",2);
        assert!(tracker.is_usable(&tracker.observations[0],2));
        assert_eq!(tracker.observations[0].workspace_epoch,1,"historical epoch is immutable");
        mutate(&mut tracker,"workspace.files/delete","assets",3);
        assert!(tracker.is_usable(&tracker.observations[0],3));
        assert!(!tracker.is_usable(&tracker.observations[1],3));
        assert!(tracker.is_usable(&tracker.observations[2],3),"directory prefix must include slash");
        assert!(!tracker.is_usable(&tracker.observations[0],4),"an unaccounted command/resource epoch is a global barrier");
        mutate(&mut tracker,"workspace.files/write","after-command.txt",5);
        assert!(!tracker.is_usable(&tracker.observations[0],5),"later disjoint edits cannot rehabilitate stale evidence");

        let mut tracker = CompletionTracker { observations: vec![file_observation("html", "index.html", 0)], ..Default::default() };
        let call = ChatToolCall { call_id: "hooked-write".into(), name: "file_action".into(),
            arguments: StrictJsonValue(serde_json::json!({"path":"style.css","content":"body{}"})), provider_metadata: None };
        tracker.observe_with_effect_scope(&AgentWorkStatus { workspace_observation_epoch: 1, ..Default::default() },
            &file_binding("workspace.files/write"), &call, &AgentToolResult::text(call.call_id.clone(), "ok", false), true, false);
        assert!(!tracker.is_usable(&tracker.observations[0], 1), "mutating tool middleware prevents path-scoped evidence reuse");
    }

    #[tokio::test]
    async fn completion_resolves_paths_and_closes_optional_plan_without_label_or_requirement_bookkeeping() {
        let inputs = vec![crate::context_lifecycle::text_message(
            nomifun_chat_model_broker::ChatRole::User, "Build an HTML game with styles".into())];
        let mut plan = AgentPlan::default();
        let mut tracker = CompletionTracker { observations: vec![
            file_observation("html-old","index.html",0), file_observation("html-new","index.html",2),
            file_observation("css-new","style.css",2),
        ], ..Default::default() };
        let work = AgentWorkStatus { workspace_observation_epoch:2, ..Default::default() };
        let call = ChatToolCall { call_id:"done".into(), name:TOOL_NAME.into(), provider_metadata:None,
            arguments:StrictJsonValue(serde_json::json!({"summary":"Created files; browser behavior is unverified", "criteria":[
                {"step":"HTML source","disposition":"supported","evidence_paths":["index.html"],"rationale":"Fresh file read"},
                {"step":"Styles","disposition":"supported","evidence_paths":["./style.css"],"rationale":"Fresh file read"},
                {"step":"Gameplay","disposition":"unverified","rationale":"No interactive test was run"}
            ]})) };
        let result = tracker.submit(&call,&mut plan,&work,&inputs,&crate::NoopAgentEventSink).await.unwrap();
        assert!(!result.is_error,"{}",result.output_text());
        let report = tracker.current(&plan,&work,1).unwrap();
        assert_eq!(report.criteria[0].evidence_call_ids,["html-new"]);
        assert_eq!(report.criteria[1].evidence_call_ids,["css-new"]);
        assert!(report.criteria.iter().all(|criterion| criterion.requirement_ids == ["input_0"]));
        assert!(report.unverified_disclosure().unwrap().contains("No interactive test"));
        assert!(!plan.is_open());
    }

    #[tokio::test]
    async fn stale_or_missing_path_cannot_close_an_open_plan_or_be_replaced_with_unrelated_success() {
        let inputs = vec![crate::context_lifecycle::text_message(
            nomifun_chat_model_broker::ChatRole::User,"Update the game".into())];
        let mut plan = AgentPlan { revision:1, explanation:"Implement requested work".into(),
            steps:vec![crate::AgentPlanStep { step:"Work in progress".into(),status:AgentPlanStatus::InProgress }],
            requirements:crate::requirements::merge(&[],&[],&inputs).unwrap(), needs_replan:false };
        let original = plan.clone();
        let mut tracker = CompletionTracker { observations:vec![file_observation("old-game","game.js",1),
            file_observation("fresh-readme","README.md",2)], ..Default::default() };
        let work = AgentWorkStatus { workspace_observation_epoch:2, ..Default::default() };
        for path in ["game.js","missing.js"] {
            let call = ChatToolCall { call_id:"bad-report".into(), name:TOOL_NAME.into(), provider_metadata:None,
                arguments:StrictJsonValue(serde_json::json!({"summary":"Done","criteria":[
                    {"step":"Game","disposition":"supported","evidence_paths":[path],"rationale":"Claimed verification"}
                ]})) };
            assert!(tracker.submit(&call,&mut plan,&work,&inputs,&crate::NoopAgentEventSink).await.unwrap().is_error);
            assert_eq!(plan,original);
            assert!(tracker.current(&plan,&work,1).is_none());
        }
    }

    #[test]
    fn stale_evidence_feedback_prefers_current_success_over_another_command() {
        let observations = vec![
            AgentCompletionObservation { call_id: "verified".into(), tool_name: "exec_command".into(), path: None,
                workspace_epoch: 2, invocation_attempted: true, successful: true,
                usable_at_observation: true, command_exit_code: Some(0), command: None },
            AgentCompletionObservation { call_id: "blocked".into(), tool_name: "exec_command".into(), path: None,
                workspace_epoch: 2, invocation_attempted: false, successful: false,
                usable_at_observation: false, command_exit_code: None, command: None },
        ];
        let tracker = CompletionTracker { observations, ..Default::default() };
        let guidance = tracker.stale_evidence_guidance(2);
        assert!(guidance.contains("verified"));
        assert!(!guidance.contains("\"blocked\""));
        assert!(guidance.contains("do not launch another command"));
        assert!(tracker.stale_evidence_guidance(3).contains("reopen one plan step"));
    }
}

impl AgentCompletionReport {
    pub(crate) fn is_blocked(&self) -> bool {
        self.criteria
            .iter()
            .any(|criterion| criterion.disposition == AgentCriterionDisposition::Blocked)
    }

    pub(crate) fn unverified_disclosure(&self) -> Option<String> {
        let items = self
            .criteria
            .iter()
            .filter(|criterion| {
                matches!(
                    criterion.disposition,
                    AgentCriterionDisposition::Unverified
                        | AgentCriterionDisposition::ScopeChanged | AgentCriterionDisposition::Blocked
                )
            })
            .map(|criterion| {
                let scope = criterion.scope_change.as_ref().map(|source| format!(
                    " [Scope changed, not original work completed; accepted input {}, quote: {}]",
                    source.input, serde_json::to_string(&source.quote).unwrap_or_default()
                )).unwrap_or_default();
                format!("- ⚠ {}: {}{}",criterion.step,criterion.rationale,scope)
            })
            .collect::<Vec<_>>();
        (!items.is_empty()).then(|| {
            format!("\n\n{}",items.join("\n"))
        })
    }
}

fn invalid(error: impl std::fmt::Display) -> AgentEngineError {
    AgentEngineError::InvalidContract(error.to_string())
}
