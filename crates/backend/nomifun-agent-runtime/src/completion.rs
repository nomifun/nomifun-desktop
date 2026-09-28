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
    /// Derived validity through known, scoped owner effects. Historical
    /// observations keep their original epoch; commands never inherit this.
    valid_through: BTreeMap<String, u32>,
    owner_paths: BTreeMap<String, WorkspacePathObservation>,
    artifacts: BTreeMap<String, ArtifactObservation>,
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Clone, Debug, Serialize)]
struct ArtifactObservation {
    root_sha256: String,
    artifact_id: String,
}

impl ArtifactObservation {
    fn from_owner(binding: &AgentToolBinding, call: &ChatToolCall, value: &serde_json::Value) -> Option<Self> {
        if binding.capability_id.as_ref() != "workspace.artifacts" { return None; }
        let root = value.get("workspace_root_sha256")?.as_str()?;
        let id = value.get("artifact_id")?.as_str()?;
        if !valid_sha256(root) || !valid_sha256(id) || value.get("sha256")?.as_str()? != id { return None; }
        match binding.action_id.as_ref() {
            "workspace.artifacts/publish" if value.get("relative_path")?.as_str()? == format!(".nomifun/artifacts/{id}") => {}
            "workspace.artifacts/read" if call.arguments.0.get("artifact_id")?.as_str()? == id => {}
            _ => return None,
        }
        Some(Self {root_sha256:root.to_owned(),artifact_id:id.to_owned()})
    }
}

/// Data from the scoped workspace owner result, never from model arguments.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspacePathObservation {
    root_sha256: String,
    path: String,
    case_resolved: bool,
}

impl WorkspacePathObservation {
    fn parse(value: &serde_json::Value) -> Option<Self> {
        let value: Self = serde_json::from_value(value.clone()).ok()?;
        if !valid_sha256(&value.root_sha256)
            || value.path.is_empty() || value.path.len() > 4096 || value.path.chars().any(char::is_control)
            || crate::agents_md::normalize_workspace_directory(&value.path).ok().as_deref() != Some(value.path.as_str())
        { return None; }
        Some(value)
    }

    fn may_overlap(&self, target: &Self) -> bool {
        // A changed binding/root is not evidence of a disjoint mutation.
        if self.root_sha256 != target.root_sha256 { return true; }
        if self.case_resolved && target.case_resolved {
            self.path == target.path || self.path.starts_with(&format!("{}/", target.path))
        } else {
            file_paths_may_overlap(&self.path, &target.path)
        }
    }
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
        description: "Finish this turn and deliver the summary after work and processes settle. A validated report is terminal; do not call more tools afterward. It closes the optional plan; no separate update_plan is needed for routine completion. Use the fewest descriptive criteria needed; they need not match plan labels. For a read-only verification jointly proved by the same observations, prefer one supported criterion citing all relevant paths/call IDs. Keep derived restatements and the absence of forbidden actions in the summary unless they have independent evidence; never create an evidence-free supported criterion. A requirement may span multiple criteria. Omitted requirement_ids covers the accepted task; explicit IDs must cover every recorded requirement. Every supported criterion must cite at least one current observation: copy a listed non-null path into evidence_paths, or a listed call_id into evidence_call_ids. The same eligible evidence may support multiple criteria. Finish mutations before final read-only verification. If a required file claim has only stale evidence, re-read that file when authorized before reporting. Artifact source paths are not current workspace observations; deletions and artifacts use eligible call IDs. Never repeat a mutation just to refresh evidence. Evidence proves the observed operation, not broader gameplay/test quality. Use unverified/blocked for missing required verification; do not invent extra checks beyond the accepted task. scope_changed requires an exact LATER accepted-input citation and no evidence. Submit alone or immediately after update_plan in a control-only batch. Later effects or input invalidate the report. This grants no extra authority.".into(),
        deferred: false,
        input_schema: StrictJsonValue(serde_json::json!({
            "type":"object", "additionalProperties":false, "required":["summary","criteria"],
            "properties":{
                "summary":{"type":"string","minLength":1,"maxLength":2048,"description":"The complete final answer delivered verbatim to the user. This is the ONLY final reply: criteria rationales are internal and are not shown. Include every requested delivery detail, such as paths, artifact IDs, readback contents and deletion results, while following the user's requested language and output format. There is no later assistant reply after an accepted report."},
                "criteria":{"type":"array","minItems":1,"maxItems":16,"items":{
                    "type":"object","additionalProperties":false,"required":["disposition","rationale"],
                    "allOf":[{
                        "if":{"properties":{"disposition":{"const":"supported"}},"required":["disposition"]},
                        "then":{"anyOf":[
                            {"required":["evidence_call_ids"],"properties":{"evidence_call_ids":{"minItems":1}}},
                            {"required":["evidence_paths"],"properties":{"evidence_paths":{"minItems":1}}}
                        ]}
                    }],
                    "properties":{
                        "step":{"type":"string","minLength":1,"maxLength":512,"description":"Optional display label; omission uses an indexed delivery label."},
                        "disposition":{"type":"string","enum":["supported","unverified","blocked","scope_changed"]},
                        "requirement_ids":{"type":"array","maxItems":32,"items":{"type":"string","minLength":1,"maxLength":64},"description":"Optional. If omitted, this criterion addresses all accepted requirements. Requirements can be shared across criteria."},
                        "scope_change":crate::requirements::citation_schema(),
                        "evidence_call_ids":{"type":"array","maxItems":8,"items":{"type":"string","minLength":1,"maxLength":256},"description":"Exact call_id entries currently listed in available_evidence, including deletion or artifact observations. A call remembered from an earlier step may no longer be eligible."},
                        "evidence_paths":{"type":"array","maxItems":8,"items":{"type":"string","minLength":1,"maxLength":4096},"description":"Copy only non-null path entries currently listed in available_evidence. Do not guess a path from prior writes, deletions or artifact source_path. Re-read a needed stale file when authorized before reporting. This does not claim functional verification."},
                        "rationale":{"type":"string","minLength":1,"maxLength":1024}
                    }
                }}
            }
        })),
    }
}

impl CompletionTracker {
    /// Runtime control citations are turn-local data, not Kernel grants. The
    /// same exposed schema is used by the whole-batch argument preflight.
    pub(crate) fn definition_with_evidence(&self, work: &AgentWorkStatus, unresolved_patch: bool) -> ChatToolDefinition {
        let mut tool = definition();
        if unresolved_patch {
            tool.description = format!(
                "A failed patch has targets that still require observation or a successful owner receipt. Fresh reads report current state but do not by themselves prove an unpublished requested mutation completed. To honor an error-stop/no-retry request, provide the actual partial-result summary and a blocked criterion. Repair exact unresolved targets only when authorized, or use scope_changed with an exact later accepted-input citation. Do not report the original task supported from reads alone. {}",
                tool.description);
        }
        let usable = self.observations.iter()
            .filter(|item| self.is_usable(item, work.workspace_observation_epoch))
            .collect::<Vec<_>>();
        let paths = usable.iter().filter_map(|item| item.path.as_ref())
            .cloned().collect::<BTreeSet<_>>();
        let calls = usable.iter().map(|item| item.call_id.clone()).collect::<BTreeSet<_>>();
        let fields = &mut tool.input_schema.0["properties"]["criteria"]["items"]["properties"];
        for (name, values) in [("evidence_paths", paths), ("evidence_call_ids", calls)] {
            if values.is_empty() {
                // Empty enum is invalid JSON Schema. Only omission or an
                // empty array is permitted until a usable observation exists.
                fields[name]["maxItems"] = serde_json::json!(0);
            } else {
                fields[name]["items"]["enum"] = serde_json::json!(values);
            }
        }
        tool
    }

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
        let owner_result = (matches!(binding.capability_id.as_ref(), "workspace.files" | "workspace.artifacts")
            && invocation_attempted && !result.is_error)
            .then(|| serde_json::from_str::<serde_json::Value>(&result.output_text()).ok()).flatten();
        let owner_path = owner_result.as_ref().filter(|_| binding.capability_id.as_ref() == "workspace.files"
            && matches!(binding.action_id.as_ref(), "workspace.files/read" | "workspace.files/write" | "workspace.files/delete")
            && call.arguments.0.get("format").and_then(serde_json::Value::as_str) != Some("instruction_scope"))
            .and_then(|value| value.get("workspace_path"))
            .and_then(WorkspacePathObservation::parse);
        let artifact = owner_result.as_ref().and_then(|value| ArtifactObservation::from_owner(binding, call, value));
        // Writing style.css does not erase the observed index.html content.
        // Advance only owner evidence unaffected by this exact confined file
        // action. Opaque commands, VCS and resources remain global barriers.
        if effects_are_scoped && invocation_attempted && !result.is_error && work.running_processes.is_empty()
            && binding.capability_id.as_ref() == "workspace.files"
            && !matches!(binding.effect_class, crate::AgentEffectClass::ReadOnly)
            && let Some(previous_epoch) = work.workspace_observation_epoch.checked_sub(1)
        {
            let targets: Option<Vec<WorkspacePathObservation>> = match binding.action_id.as_ref() {
                "workspace.files/write" | "workspace.files/delete" => owner_path.clone().map(|path| vec![path]),
                "workspace.files/patch" => owner_result.as_ref().and_then(|value| value["files"].as_array())
                    .filter(|files| call.arguments.0["files"].as_array().is_some_and(|requested| requested.len() == files.len()))
                    .and_then(|files| files.iter().map(|file| WorkspacePathObservation::parse(&file["workspace_path"])).collect()),
                _ => None,
            };
            if let Some(targets) = targets.filter(|targets| !targets.is_empty()) {
                for observation in &self.observations {
                    if (self.owner_paths.get(&observation.call_id).is_some_and(|path|
                        !targets.iter().any(|target| path.may_overlap(target)))
                        || self.artifacts.get(&observation.call_id).is_some_and(|artifact|
                            targets.iter().all(|target| target.root_sha256 == artifact.root_sha256)))
                        && self.is_usable(observation, previous_epoch)
                    {
                        self.valid_through.insert(observation.call_id.clone(), work.workspace_observation_epoch);
                    }
                }
            }
        }
        if effects_are_scoped && invocation_attempted && !result.is_error && work.running_processes.is_empty()
            && binding.action_id.as_ref() == "workspace.artifacts/publish"
            && let Some(artifact) = &artifact
            && let Some(previous_epoch) = work.workspace_observation_epoch.checked_sub(1)
        {
            // Publication writes only into this owner's protected artifact
            // namespace. It cannot change user files or prior addressed blobs.
            for observation in &self.observations {
                let same_root = self.owner_paths.get(&observation.call_id)
                    .is_some_and(|path| path.root_sha256 == artifact.root_sha256)
                    || self.artifacts.get(&observation.call_id)
                        .is_some_and(|prior| prior.root_sha256 == artifact.root_sha256);
                if same_root && self.is_usable(observation, previous_epoch) {
                    self.valid_through.insert(observation.call_id.clone(), work.workspace_observation_epoch);
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
        if let Some(path) = owner_path {
            self.owner_paths.insert(observation.call_id.clone(), path);
        }
        if let Some(artifact) = artifact { self.artifacts.insert(observation.call_id.clone(), artifact); }
        self.observations.push(observation.clone());
        // Interactive provenance can carry several call IDs per observation.
        // Bound the serialized window too, not just its number of records.
        while self.observations.len() > 64
            || self
                .observations
                .iter()
                .map(|item| serde_json::to_vec(item).map_or(usize::MAX, |value| value.len())
                    .saturating_add(self.owner_paths.get(&item.call_id)
                        .map_or(0, |path| serde_json::to_vec(path).map_or(usize::MAX, |value| value.len())))
                    .saturating_add(self.artifacts.get(&item.call_id)
                        .map_or(0, |artifact| serde_json::to_vec(artifact).map_or(usize::MAX, |value| value.len()))))
                .fold(0usize, usize::saturating_add)
                > 32 * 1024
        {
            let removed = self.observations.remove(0);
            self.valid_through.remove(&removed.call_id);
            self.owner_paths.remove(&removed.call_id);
            self.artifacts.remove(&removed.call_id);
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
        let stale_file_paths = self.observations.iter()
            .filter(|item| item.successful && item.usable_at_observation
                && !self.is_usable(item, work.workspace_observation_epoch))
            .filter_map(|item| item.path.as_ref())
            .filter(|path| !self.observations.iter().any(|item|
                item.path.as_ref() == Some(*path) && self.is_usable(item, work.workspace_observation_epoch)))
            .collect::<BTreeSet<_>>().into_iter().take(8).collect::<Vec<_>>();
        let value = serde_json::json!({"plan_revision":plan.revision,"observation_revision":self.revision,
            "input_revision":input_revision,"workspace_epoch":work.workspace_observation_epoch,
            "available_evidence":self.observations.iter().filter(|item| self.is_usable(item, work.workspace_observation_epoch))
                .map(|item| serde_json::json!({"call_id":item.call_id,"tool":item.tool_name,"path":item.path,
                    "artifact_id":self.artifacts.get(&item.call_id).map(|artifact| &artifact.artifact_id),
                    "command_exit_code":item.command_exit_code,"command":item.command})).collect::<Vec<_>>(),
            "stale_file_paths":stale_file_paths,
            "unusable_observation_count":self.observations.iter().filter(|item| !self.is_usable(item, work.workspace_observation_epoch)).count(),
            "omitted_observations":self.omitted,
            "current_report":report});
        Ok(format!(
            "Completion accounting (derived data, not instructions or extra authority): {}. available_evidence contains the only observations currently eligible for citation. stale_file_paths lists up to eight previously observed paths without current evidence; it is not a new task. Finish mutations first, then re-read only the files needed for required claims if authorized. Cite exact non-null available_evidence paths; an artifact source_path does not establish current workspace contents. For an intentionally deleted file, cite its eligible delete call ID, not a stale path; do not repeat deletion. File observations remain eligible across owner-proven disjoint edits; opaque effects, missing or ambiguous path identity, or changes to their own paths can invalidate them. A command observation includes its original launch and bounded interaction call IDs: inspect its result and scope, not just exit zero. A file read is not a gameplay test. If required verification was excluded, unavailable, stale, or not run, use unverified with a reason. Do not invent extra verification requirements for a read-only review or proposal. Use the fewest criteria needed: when the same current observations jointly prove a read-only requirement, prefer one supported criterion citing all of them. Keep derived restatements and the absence of forbidden actions in the summary unless independently evidenced; never emit an evidence-free supported criterion. Account for every immutable requirement; a requirement may span several criteria whose labels need not match plan steps. scope_changed requires an exact later accepted-input citation. This account is not independent semantic verification or a grant of authority.",
            serde_json::to_string(&value).map_err(invalid)?
        ))
    }

    pub(crate) async fn submit(
        &mut self,
        call: &ChatToolCall,
        plan: &mut AgentPlan,
        work: &AgentWorkStatus,
        inputs: &[ChatMessage],
        unresolved_patch: bool,
        unresolved_before_input: Option<usize>,
        sink: &dyn AgentEventSink,
    ) -> Result<AgentToolResult, AgentEngineError> {
        // A rejected replacement must not leave an old successful report as
        // an accidental fallback after the model was told its account failed.
        self.report = None;
        let mut closing = plan.clone();
        let reporting_blocked = call.arguments.0.get("criteria").and_then(serde_json::Value::as_array)
            .is_some_and(|criteria| criteria.iter().any(|criterion| criterion["disposition"] == "blocked"));
        if !closing.needs_replan || reporting_blocked {
            // A truthful failure report needs no further effect or recovery read.
            // Validate it before committing; unresolved work stays blocked.
            closing.needs_replan = false;
            closing.requirements = crate::requirements::merge(&plan.requirements, &[], inputs)
                .map_err(AgentEngineError::InvalidContract)?;
            for step in &mut closing.steps {
                if matches!(step.status, AgentPlanStatus::Pending | AgentPlanStatus::InProgress) {
                    step.status = if reporting_blocked { AgentPlanStatus::Blocked } else { AgentPlanStatus::Completed };
                }
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
        let scoped_out = unresolved_before_input.is_some_and(|boundary| {
            report.scopes_out_requirements_before(boundary)
        });
        if unresolved_patch && !report.is_blocked() && !scoped_out {
            return Ok(AgentToolResult::text(call.call_id.clone(),
                "A failed patch still has targets requiring observation or successful repair. Current-state reads do not prove an unpublished requested mutation completed. Report blocked, repair exact unresolved targets when authorized, or cite an exact later accepted-input scope change; the original task cannot be completed yet.", true));
        }
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
                || ((self.owner_paths.contains_key(&observation.call_id) || self.artifacts.contains_key(&observation.call_id))
                    && self.valid_through.get(&observation.call_id) == Some(&epoch)))
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
// Hosts without verified canonical case spelling use conservative ASCII case
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
    fn advertised_evidence_schema_rejects_stale_missing_and_failed_citations() {
        let mut failed = file_observation("failed", "failed.txt", 2);
        failed.successful = false;
        let tracker = CompletionTracker { observations: vec![
            file_observation("stale", "old.txt", 1),
            file_observation("current", "current.txt", 2), failed,
        ], ..Default::default() };
        let work = AgentWorkStatus { workspace_observation_epoch: 2, ..Default::default() };
        let definition = tracker.definition_with_evidence(&work,false);
        assert!(definition.description.contains("prefer one supported criterion"));
        assert!(definition.description.contains("never create an evidence-free supported criterion"));
        let schema = definition.input_schema.0;
        let validator = jsonschema::options().build(&schema).unwrap();
        let report = |field: &str, reference: &str| {
            let mut value = serde_json::json!({"summary":"Finished","criteria":[
                {"disposition":"supported","rationale":"Observed the requested work"}
            ]});
            value["criteria"][0][field] = serde_json::json!([reference]);
            value
        };
        assert!(validator.is_valid(&report("evidence_paths", "current.txt")));
        assert!(validator.is_valid(&report("evidence_call_ids", "current")));
        for (path, call) in [("old.txt", "stale"), ("missing.txt", "missing"), ("failed.txt", "failed")] {
            assert!(!validator.is_valid(&report("evidence_paths", path)));
            assert!(!validator.is_valid(&report("evidence_call_ids", call)));
        }
        assert!(!validator.is_valid(&serde_json::json!({"summary":"Unsupported claim","criteria":[
            {"disposition":"supported","rationale":"No observation was cited"}
        ]})));
        let empty = CompletionTracker::default().definition_with_evidence(&work,false).input_schema.0;
        let validator = jsonschema::options().build(&empty).unwrap();
        assert!(!validator.is_valid(&report("evidence_paths", "current.txt")));
        assert!(!validator.is_valid(&report("evidence_call_ids", "current")));
        assert!(validator.is_valid(&serde_json::json!({"summary":"Verification unavailable","criteria":[
            {"disposition":"unverified","rationale":"No authorized observation available"}
        ]})));
        assert!(!validator.is_valid(&serde_json::json!({"summary":"Unsupported claim","criteria":[
            {"disposition":"supported","rationale":"No observation was cited"}
        ]})));
    }

    #[test]
    fn file_evidence_uses_owner_paths_for_unicode_siblings_and_junction_aliases() {
        for (read_path, read_canonical, write_path, write_canonical, remains_current) in [
            ("验收/回执.txt", "验收/回执.txt", "验收/临时.txt", "验收/临时.txt", true),
            ("shortcut/index.html", "real/index.html", "real/index.html", "real/index.html", false),
        ] {
            let owner_result = |path| serde_json::json!({"workspace_path":{
                "root_sha256":"a".repeat(64),"path":path,"case_resolved":true
            }}).to_string();
            let mut tracker = CompletionTracker::default();
            let read = ChatToolCall { call_id:"observed".into(),name:"read_file".into(),
                arguments:StrictJsonValue(serde_json::json!({"path":read_path})),provider_metadata:None };
            let mut binding = file_binding("workspace.files/read");
            binding.effect_class = crate::AgentEffectClass::ReadOnly;
            tracker.observe(&AgentWorkStatus::default(), &binding, &read,
                &AgentToolResult::text(read.call_id.clone(), owner_result(read_canonical), false), true);
            let write = ChatToolCall {call_id:"mutation".into(),name:"write_file".into(),
                arguments:StrictJsonValue(serde_json::json!({"path":write_path,"content":"changed"})),provider_metadata:None};
            tracker.observe(&AgentWorkStatus {workspace_observation_epoch:1,..Default::default()},
                &file_binding("workspace.files/write"), &write,
                &AgentToolResult::text(write.call_id.clone(), owner_result(write_canonical), false),true);
            assert_eq!(tracker.is_usable(&tracker.observations[0],1), remains_current,
                "owner paths must distinguish unrelated Unicode names and identify a real alias");
        }
    }

    #[test]
    fn disjoint_file_edits_preserve_file_evidence_without_relabeling_history_or_refreshing_commands() {
        assert!(file_paths_may_overlap("Assets/Game.js", "assets"));
        assert!(file_paths_may_overlap("index.html", "INDEX.HTML"));
        assert!(file_paths_may_overlap("café.js", "cafe\u{301}.js"));
        assert!(!file_paths_may_overlap("assets-other.css", "assets"));
        let mut tracker = CompletionTracker { observations:vec![file_observation("html","index.html",1),
            file_observation("nested","assets/style.css",1),file_observation("sibling","assets-other.css",1)], ..Default::default() };
        for item in &tracker.observations {
            tracker.owner_paths.insert(item.call_id.clone(), WorkspacePathObservation {
                root_sha256:"a".repeat(64),path:item.path.clone().unwrap(),case_resolved:false,
            });
        }
        let mutate = |tracker: &mut CompletionTracker, action: &str, path: &str, epoch| {
            let call = ChatToolCall { call_id:format!("effect-{epoch}").into(),name:"file_action".into(),
                arguments:StrictJsonValue(serde_json::json!({"path":path,"content":"updated"})),provider_metadata:None };
            tracker.observe(&AgentWorkStatus { workspace_observation_epoch:epoch,..Default::default() },
                &file_binding(action),&call,&AgentToolResult::text(call.call_id.clone(),serde_json::json!({
                    "workspace_path":{"root_sha256":"a".repeat(64),"path":path,"case_resolved":false}
                }).to_string(),false),true);
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

    #[test]
    fn artifact_store_effects_preserve_file_and_artifact_observations_but_not_across_commands() {
        let root = "a".repeat(64);
        let artifact = "b".repeat(64);
        let file_result = |path| serde_json::json!({"workspace_path":{
            "root_sha256":root,"path":path,"case_resolved":true
        }}).to_string();
        let call = |id: &str, args| ChatToolCall {call_id:id.into(),name:id.into(),arguments:StrictJsonValue(args),provider_metadata:None};
        let mut tracker = CompletionTracker::default();
        let read = call("read-main",serde_json::json!({"path":"验收/回执.txt"}));
        let mut read_binding = file_binding("workspace.files/read");
        read_binding.effect_class = crate::AgentEffectClass::ReadOnly;
        tracker.observe(&AgentWorkStatus::default(),&read_binding,&read,
            &AgentToolResult::text(read.call_id.clone(),file_result("验收/回执.txt"),false),true);
        let published = call("published",serde_json::json!({"path":"验收/回执.txt"}));
        let mut publish_binding = file_binding("workspace.artifacts/publish");
        publish_binding.capability_id = "workspace.artifacts".into();
        let artifact_result = serde_json::json!({"artifact_id":artifact,"sha256":artifact,
            "workspace_root_sha256":root,"relative_path":format!(".nomifun/artifacts/{artifact}")}).to_string();
        tracker.observe(&AgentWorkStatus {workspace_observation_epoch:1,..Default::default()},&publish_binding,&published,
            &AgentToolResult::text(published.call_id.clone(),artifact_result.clone(),false),true);
        assert!(tracker.is_usable(&tracker.observations[0],1),"publishing into the protected store does not modify its source");
        let mut artifact_read_binding = publish_binding.clone();
        artifact_read_binding.action_id = "workspace.artifacts/read".into();
        artifact_read_binding.effect_class = crate::AgentEffectClass::ReadOnly;
        let artifact_read = call("artifact-read",serde_json::json!({"artifact_id":artifact}));
        tracker.observe(&AgentWorkStatus {workspace_observation_epoch:1,..Default::default()},&artifact_read_binding,&artifact_read,
            &AgentToolResult::text(artifact_read.call_id.clone(),artifact_result,false),true);
        for (epoch,path) in [(2,"验收/临时.txt"),(3,"验收/回执.txt")] {
            let write = call(&format!("write-{epoch}"),serde_json::json!({"path":path}));
            tracker.observe(&AgentWorkStatus {workspace_observation_epoch:epoch,..Default::default()},&file_binding("workspace.files/write"),&write,
                &AgentToolResult::text(write.call_id.clone(),file_result(path),false),true);
            assert_eq!(tracker.is_usable(&tracker.observations[0],epoch),epoch==2);
            assert!(tracker.is_usable(&tracker.observations[1],epoch));
            assert!(tracker.is_usable(&tracker.observations[2],epoch));
        }
        assert!(!tracker.is_usable(&tracker.observations[2],4),"an unaccounted command epoch still invalidates artifact evidence");
        let later = call("later-write",serde_json::json!({"path":"later.txt"}));
        tracker.observe(&AgentWorkStatus {workspace_observation_epoch:5,..Default::default()},&file_binding("workspace.files/write"),&later,
            &AgentToolResult::text(later.call_id.clone(),file_result("later.txt"),false),true);
        assert!(!tracker.is_usable(&tracker.observations[2],5),"a later file edit cannot rehabilitate stale artifact evidence");
    }

    #[test]
    fn only_same_root_successful_protected_artifact_publication_preserves_file_evidence() {
        for (root, relative, sha, error, scoped) in [
            (Some("b".repeat(64)), format!(".nomifun/artifacts/{}","c".repeat(64)), "c".repeat(64), false, true),
            (None, format!(".nomifun/artifacts/{}","c".repeat(64)), "c".repeat(64), false, true),
            (Some("a".repeat(64)), "user/output.txt".into(), "c".repeat(64), false, true),
            (Some("a".repeat(64)), format!(".nomifun/artifacts/{}","c".repeat(64)), "d".repeat(64), false, true),
            (Some("a".repeat(64)), format!(".nomifun/artifacts/{}","c".repeat(64)), "c".repeat(64), true, true),
            (Some("a".repeat(64)), format!(".nomifun/artifacts/{}","c".repeat(64)), "c".repeat(64), false, false),
        ] {
            let mut tracker = CompletionTracker {observations:vec![file_observation("file","result.txt",0)],..Default::default()};
            tracker.owner_paths.insert("file".into(),WorkspacePathObservation {root_sha256:"a".repeat(64),path:"result.txt".into(),case_resolved:true});
            let mut binding=file_binding("workspace.artifacts/publish");
            binding.capability_id="workspace.artifacts".into();
            let call=ChatToolCall {call_id:"publish".into(),name:"publish_artifact".into(),
                arguments:StrictJsonValue(serde_json::json!({"path":"result.txt"})),provider_metadata:None};
            tracker.observe_with_effect_scope(&AgentWorkStatus {workspace_observation_epoch:1,..Default::default()},&binding,&call,
                &AgentToolResult::text(call.call_id.clone(),serde_json::json!({"artifact_id":"c".repeat(64),"sha256":sha,
                    "workspace_root_sha256":root,"relative_path":relative}).to_string(),error),true,scoped);
            assert!(!tracker.is_usable(&tracker.observations[0],1));
        }
    }

    #[test]
    fn missing_malformed_cross_root_and_failed_owner_receipts_do_not_preserve_evidence() {
        for (read_owner, write_owner, is_error) in [
            (serde_json::Value::Null, serde_json::json!({"root_sha256":"a".repeat(64),"path":"other.txt","case_resolved":true}), false),
            (serde_json::json!({"root_sha256":"a".repeat(64),"path":"index.html","case_resolved":true}), serde_json::Value::Null, false),
            (serde_json::json!({"root_sha256":"a".repeat(64),"path":"index.html","case_resolved":true}), serde_json::json!({"root_sha256":"b".repeat(64),"path":"other.txt","case_resolved":true}), false),
            (serde_json::json!({"root_sha256":"a".repeat(64),"path":"index.html","case_resolved":true}), serde_json::json!({"root_sha256":"a".repeat(64),"path":"../other.txt","case_resolved":true}), false),
            (serde_json::json!({"root_sha256":"a".repeat(64),"path":"index.html","case_resolved":true}), serde_json::json!({"root_sha256":"a".repeat(64),"path":"other.txt","case_resolved":true}), true),
        ] {
            let mut tracker = CompletionTracker::default();
            let read = ChatToolCall {call_id:"read".into(),name:"read_file".into(),
                arguments:StrictJsonValue(serde_json::json!({"path":"index.html"})),provider_metadata:None};
            let mut binding = file_binding("workspace.files/read");
            binding.effect_class = crate::AgentEffectClass::ReadOnly;
            tracker.observe(&AgentWorkStatus::default(), &binding, &read,
                &AgentToolResult::text(read.call_id.clone(), serde_json::json!({"workspace_path":read_owner}).to_string(),false),true);
            let write = ChatToolCall {call_id:"write".into(),name:"write_file".into(),
                arguments:StrictJsonValue(serde_json::json!({"path":"other.txt"})),provider_metadata:None};
            tracker.observe(&AgentWorkStatus {workspace_observation_epoch:1,..Default::default()},
                &file_binding("workspace.files/write"), &write,
                &AgentToolResult::text(write.call_id.clone(),serde_json::json!({"workspace_path":write_owner}).to_string(),is_error),true);
            assert!(!tracker.is_usable(&tracker.observations[0],1));
        }
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
        let result = tracker.submit(&call,&mut plan,&work,&inputs,false,None,&crate::NoopAgentEventSink).await.unwrap();
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
            assert!(tracker.submit(&call,&mut plan,&work,&inputs,false,None,&crate::NoopAgentEventSink).await.unwrap().is_error);
            assert_eq!(plan,original);
            assert!(tracker.current(&plan,&work,1).is_none());
        }
    }

    #[tokio::test]
    async fn pending_patch_requires_blocked_disposition_and_preserves_rejected_state() {
        let inputs = vec![crate::context_lifecycle::text_message(nomifun_chat_model_broker::ChatRole::User,
            "Stop after the patch error; do not retry".into())];
        for disposition in ["supported", "unverified", "blocked"] {
            let mut plan = AgentPlan::default();
            let before = plan.clone();
            let work = AgentWorkStatus::default();
            let mut tracker = CompletionTracker { observations:vec![file_observation("read-a","a",0)], ..Default::default() };
            let call = ChatToolCall { call_id:"report".into(),name:TOOL_NAME.into(),provider_metadata:None,
                arguments:StrictJsonValue(serde_json::json!({"summary":"Partial result", "criteria":[{
                    "disposition":disposition,"evidence_call_ids":["read-a"],"rationale":"The remaining target is unobserved"
                }]})) };
            let result = tracker.submit(&call,&mut plan,&work,&inputs,true,Some(1),&crate::NoopAgentEventSink).await.unwrap();
            assert_eq!(result.is_error,disposition != "blocked");
            if result.is_error {
                assert_eq!(plan,before,"rejected success or unverified completion cannot close the plan");
                assert!(tracker.current(&plan,&work,1).is_none());
            } else {
                assert!(tracker.current(&plan,&work,1).unwrap().is_blocked());
            }
        }
    }

    #[tokio::test]
    async fn exact_later_scope_change_can_settle_the_original_patch_obligation() {
        let inputs = vec![
            crate::context_lifecycle::text_message(nomifun_chat_model_broker::ChatRole::User,"Patch both files".into()),
            crate::context_lifecycle::text_message(nomifun_chat_model_broker::ChatRole::User,"Only keep the first file change".into()),
        ];
        let mut plan=AgentPlan {revision:1,explanation:"Updated scope".into(),steps:vec![
            crate::AgentPlanStep {step:"Honor revised scope".into(),status:AgentPlanStatus::Completed}],
            requirements:crate::requirements::merge(&[],&[],&inputs).unwrap(),needs_replan:false};
        let mut tracker=CompletionTracker {observations:vec![file_observation("fresh-a","a",0)],..Default::default()};
        let work=AgentWorkStatus::default();
        let call=ChatToolCall {call_id:"scope".into(),name:TOOL_NAME.into(),provider_metadata:None,
            arguments:StrictJsonValue(serde_json::json!({"summary":"Revised scope delivered","criteria":[
                {"disposition":"scope_changed","requirement_ids":["input_0"],"scope_change":{
                    "input":1,"quote":"Only keep the first file change"},"rationale":"Later user input removed the second mutation"},
                {"disposition":"supported","requirement_ids":["input_1"],"evidence_call_ids":["fresh-a"],
                    "rationale":"Fresh owner read supports the retained first-file result"}
            ]}))};
        let result=tracker.submit(&call,&mut plan,&work,&inputs,true,Some(1),&crate::NoopAgentEventSink).await.unwrap();
        assert!(!result.is_error,"{}",result.output_text());
        let report=tracker.current(&plan,&work,2).unwrap();
        assert!(report.scopes_out_requirements_before(1));
        assert!(!report.is_blocked());
    }

    #[tokio::test]
    async fn blocked_patch_report_closes_pending_steps_without_discarding_new_input() {
        let inputs = vec![
            crate::context_lifecycle::text_message(nomifun_chat_model_broker::ChatRole::User,"Patch files".into()),
            crate::context_lifecycle::text_message(nomifun_chat_model_broker::ChatRole::User,"Stop after an error".into()),
        ];
        let mut plan = AgentPlan { revision:1,needs_replan:true,
            steps:vec![crate::AgentPlanStep {step:"Patch files".into(),status:AgentPlanStatus::InProgress}],
            requirements:crate::requirements::merge(&[],&[],&inputs[..1]).unwrap(),..Default::default() };
        let call = ChatToolCall {call_id:"blocked".into(),name:TOOL_NAME.into(),provider_metadata:None,
            arguments:StrictJsonValue(serde_json::json!({"summary":"Stopped with partial effects", "criteria":[{
                "disposition":"blocked","rationale":"The patch failed and further operations are forbidden"
            }]})) };
        let mut tracker = CompletionTracker::default();
        let work = AgentWorkStatus::default();
        assert!(!tracker.submit(&call,&mut plan,&work,&inputs,true,Some(1),&crate::NoopAgentEventSink).await.unwrap().is_error);
        assert_eq!(plan.steps[0].status,AgentPlanStatus::Blocked);
        assert!(!plan.needs_replan);
        let report = tracker.current(&plan,&work,2).unwrap();
        assert_eq!(report.requirements.len(),2);
        assert_eq!(report.criteria[0].requirement_ids.len(),2);
        assert!(report.is_blocked());
    }

    #[tokio::test]
    async fn blocked_patch_report_does_not_bypass_evidence_or_process_checks() {
        let inputs = vec![crate::context_lifecycle::text_message(nomifun_chat_model_broker::ChatRole::User,"Patch files".into())];
        for running in [false,true] {
            let mut plan = AgentPlan {needs_replan:true,..Default::default()};
            let before = plan.clone();
            let work = AgentWorkStatus {workspace_observation_epoch:1,
                running_processes:if running {std::collections::BTreeSet::from(["live".into()])} else {Default::default()},
                ..Default::default()};
            let mut tracker = CompletionTracker {observations:vec![file_observation("old-a","a",0)],..Default::default()};
            let mut criteria = vec![serde_json::json!({"disposition":"blocked","rationale":"Patch failed"})];
            if !running { criteria.push(serde_json::json!({"disposition":"supported","evidence_call_ids":["old-a"],"rationale":"Stale claim"})); }
            let call = ChatToolCall {call_id:"bad".into(),name:TOOL_NAME.into(),provider_metadata:None,
                arguments:StrictJsonValue(serde_json::json!({"summary":"Partial result","criteria":criteria}))};
            assert!(tracker.submit(&call,&mut plan,&work,&inputs,true,Some(1),&crate::NoopAgentEventSink).await.unwrap().is_error);
            assert_eq!(plan,before);
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

    #[tokio::test]
    async fn completion_context_exposes_stale_file_paths_without_rehabilitating_evidence() {
        let inputs = vec![crate::context_lifecycle::text_message(
            nomifun_chat_model_broker::ChatRole::User, "Create and verify the requested files".into())];
        let mut plan = AgentPlan::default();
        let mut tracker = CompletionTracker { observations: vec![
            file_observation("receipt-old", "验收/回执.txt", 1),
            file_observation("other-old", "验收/其他.txt", 0),
        ], ..Default::default() };
        let work = AgentWorkStatus { workspace_observation_epoch: 2, ..Default::default() };
        let edit = ChatToolCall { call_id: "other-write".into(), name: "write_file".into(),
            arguments: StrictJsonValue(serde_json::json!({"path":"验收/其他.txt","content":"other"})), provider_metadata: None };
        tracker.observe(&work, &file_binding("workspace.files/write"), &edit,
            &AgentToolResult::text(edit.call_id.clone(), "written", false), true);
        let account = |tracker: &CompletionTracker, plan: &AgentPlan| {
            let text = tracker.context(plan, &work, 1).unwrap();
            serde_json::Deserializer::from_str(text.split_once(": ").unwrap().1)
                .into_iter::<serde_json::Value>().next().unwrap().unwrap()
        };
        assert_eq!(account(&tracker, &plan)["stale_file_paths"], serde_json::json!(["验收/回执.txt"]));
        let report = ChatToolCall { call_id: "finish".into(), name: TOOL_NAME.into(), provider_metadata: None,
            arguments: StrictJsonValue(serde_json::json!({"summary":"Verified the receipt","criteria":[
                {"disposition":"supported","evidence_paths":["验收/回执.txt"],"rationale":"Read the file"}
            ]})) };
        assert!(tracker.submit(&report, &mut plan, &work, &inputs, false, None, &crate::NoopAgentEventSink).await.unwrap().is_error);
        let read = ChatToolCall { call_id: "receipt-current".into(), name: "read_file".into(),
            arguments: StrictJsonValue(serde_json::json!({"path":"验收/回执.txt"})), provider_metadata: None };
        let mut binding = file_binding("workspace.files/read");
        binding.effect_class = crate::AgentEffectClass::ReadOnly;
        tracker.observe(&work, &binding, &read, &AgentToolResult::text(read.call_id.clone(), "receipt", false), true);
        assert_eq!(account(&tracker, &plan)["stale_file_paths"], serde_json::json!([]));
        assert!(!tracker.submit(&report, &mut plan, &work, &inputs, false, None, &crate::NoopAgentEventSink).await.unwrap().is_error);
        assert_eq!(tracker.observations[0].workspace_epoch, 1, "history is not relabeled");
        assert_eq!(tracker.current(&plan, &work, 1).unwrap().criteria[0].evidence_call_ids, ["receipt-current"]);
    }
}

impl AgentCompletionReport {
    pub(crate) fn is_blocked(&self) -> bool {
        self.criteria
            .iter()
            .any(|criterion| criterion.disposition == AgentCriterionDisposition::Blocked)
    }

    pub(crate) fn scopes_out_requirements_before(&self, input_count: usize) -> bool {
        let affected = self.requirements.iter()
            .filter(|requirement| requirement.source.input < input_count)
            .collect::<Vec<_>>();
        !affected.is_empty() && affected.into_iter().all(|requirement| {
            self.criteria.iter().any(|criterion| {
                criterion.disposition == AgentCriterionDisposition::ScopeChanged
                    && criterion.requirement_ids.contains(&requirement.id)
            })
        })
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
