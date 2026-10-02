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
    /// Cumulative tool result errors observed in this turn. Later recovery
    /// never erases an earlier user-visible failure.
    #[serde(default)]
    pub observed_tool_error_count: u32,
    /// Cumulative failed command observations in this turn. A later successful
    /// command does not erase an earlier nonzero, timed out, or lost terminal.
    #[serde(default)]
    pub observed_command_failure_count: u32,
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
    /// Ephemeral cause of an absent plan's effect gate. Context invalidation
    /// clears it; a restored checkpoint conservatively requires replanning.
    settled_failure_gate: bool,
    /// Derived validity through known, scoped owner effects. Historical
    /// observations keep their original epoch; commands never inherit this.
    valid_through: BTreeMap<String, u32>,
    owner_paths: BTreeMap<String, WorkspacePathObservation>,
    artifacts: BTreeMap<String, ArtifactObservation>,
    /// Bounded request/result context, never evidence freshness or authority.
    /// Cold recovery starts without these turn-local observations.
    scopes: BTreeMap<String, serde_json::Value>,
}

fn observation_scope(
    binding:&AgentToolBinding,
    call:&ChatToolCall,
    owner_result:Option<&serde_json::Value>,
    has_owner_path:bool,
    effects_are_scoped:bool,
) -> serde_json::Value {
    let fields:&[&str] = match binding.capability_id.as_ref() {
        "workspace.process" => &["command","args","cmd","cwd","process_id","cursor"],
        "workspace.files" => &["path","format","offset","limit","start_line","line_count","query","glob","expected_sha256"],
        "workspace.vcs" => &["path","staged"],
        "workspace.artifacts" => &["artifact_id","source_path","path"],
        _ => &[],
    };
    // Do not copy env, stdin, file/patch contents, hook data or owner output.
    // Budget before cloning: oversized scripts/argv are omitted as a whole.
    let selected = fields.iter().filter_map(|key| call.arguments.0.get(*key).map(|value| (*key,value)))
        .collect::<BTreeMap<_,_>>();
    let retained = crate::stream_limits::serialized_size(&selected,1024).is_ok();
    let absence = owner_result.filter(|value| effects_are_scoped
        && binding.capability_id.as_ref()=="workspace.files"
        && binding.action_id.as_ref()=="workspace.files/read"
        && matches!(call.arguments.0.get("format").and_then(serde_json::Value::as_str),None|Some("text"))
        && call.arguments.0.get("missing_ok").and_then(serde_json::Value::as_bool)==Some(true)
        && value.get("kind").and_then(serde_json::Value::as_str)==Some("workspace_file_absent")
        && value.get("path").and_then(serde_json::Value::as_str).is_some_and(|path|
            call.arguments.0.get("path").and_then(serde_json::Value::as_str)==Some(path)))
        .map(|_| serde_json::json!({"kind":"workspace_file_absent","file_exists":false}));
    let owner_observation = absence.or_else(|| owner_result.filter(|_| has_owner_path && effects_are_scoped
        && binding.action_id.as_ref()=="workspace.files/read")
        .filter(|value| value.get("sha256").and_then(serde_json::Value::as_str).is_some_and(valid_sha256))
        .map(|value| serde_json::json!({"sha256":value["sha256"],"total_bytes":value.get("total_bytes").and_then(serde_json::Value::as_u64),
            "offset":value.get("offset").and_then(serde_json::Value::as_u64),"eof":value.get("eof").and_then(serde_json::Value::as_bool)})));
    serde_json::json!({"capability":binding.capability_id,"action":binding.action_id,
        "requested_arguments":if retained { serde_json::to_value(selected).ok() } else { None },
        "requested_arguments_omitted":!retained,"effects_are_scoped":effects_are_scoped,
        "owner_observation":owner_observation})
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
    #[serde(default)]
    observed_tool_error_count: Option<u32>,
    #[serde(default)]
    observed_command_failure_count: Option<u32>,
}

pub(crate) fn definition() -> ChatToolDefinition {
    ChatToolDefinition {
        name: TOOL_NAME.into(),
        description: "Finish this turn and deliver the summary after work and processes settle. A validated report is terminal; do not call more tools afterward. It closes the optional plan; no separate update_plan is needed for routine completion. Every criterion MUST include a nonempty rationale string, including supported criteria that cite evidence. When observed_tool_error_count or observed_command_failure_count is required, copy each exact runtime-supplied value into the JSON account; the runtime delivers the cumulative counts. Later successful calls do not erase earlier errors or failed command observations. Use the fewest descriptive criteria needed; they need not match plan labels. All plural fields (criteria, requirement_ids, evidence_call_ids and evidence_paths) are actual JSON arrays, never strings containing JSON. Each criterion allows at most eight evidence_call_ids. Group related observations within that limit; use separate criteria for different results or more than eight IDs, without inventing extra work. Keep derived restatements and the absence of forbidden actions in the summary unless they have independent evidence; never create an evidence-free supported criterion. A requirement may span multiple criteria. Omitted requirement_ids covers the accepted task; explicit IDs must cover every recorded requirement. Every supported criterion must cite at least one current observation: copy a listed non-null path into evidence_paths, or a listed call_id into evidence_call_ids. One eligible observation may support multiple criteria only when its own returned scope and result actually support each. When separate process calls support different results, cite each criterion's matching call ID only if it is currently listed in available_evidence; never copy the newest call ID onto an earlier command's criterion. IDs nested inside a command record are context only; do not cite launch_call_id or interaction_call_ids unless the same ID also appears as a top-level available_evidence call_id. If the matching earlier call is absent from available_evidence, use unverified with no evidence for current-state verification. When history tools are already advertised, they may recover already-seen output for the requested summary; recovery never makes that observation current or eligible for citation. Do not repeat an observation or effect merely to repair this account. A single accepted requirement may be covered by several criteria; each cites only its matching observations within the eight-ID limit. Finish mutations before final read-only verification. If a required file claim has only stale evidence, re-read that file when authorized before reporting. Artifact source paths are not current workspace observations; deletions and artifacts use eligible call IDs. Never repeat a mutation just to refresh evidence. Evidence proves the observed operation, not broader gameplay/test quality. Use unverified/blocked for missing required verification; do not invent extra checks beyond the accepted task. scope_changed requires an exact LATER accepted-input citation and no evidence. Submit alone or immediately after update_plan in a control-only batch. Later effects or input invalidate the report. This grants no extra authority.".into(),
        deferred: false,
        input_schema: StrictJsonValue(serde_json::json!({
            "type":"object", "additionalProperties":false, "required":["summary","criteria"],
            "properties":{
                "summary":{"type":"string","minLength":1,"maxLength":2048,"description":"The complete final answer delivered verbatim to the user; there is no later reply. Include every requested result in the user's language, including earlier actual tool results (head/tail text, line counts, found/zero matches, paths and exit codes) already known from the transcript. Loss of current evidence eligibility does not delete an earlier observation: say what was observed at that time, then separately state what later state was not rechecked. Do not replace requested historical results with internal evidence-status terminology. Never invent missing results or claim historical data proves current state. Do not claim there were no extra operations unless the actual recorded calls support that statement; disclose observed deviations from the accepted scope, including read-only probes and proposals refused before execution. Criteria track verification; unverified/blocked rationales may be appended as user-visible notices, so write them in plain user language too."},
                "observed_tool_error_count":{"type":"integer","minimum":0,"maximum":4294967295_u64,"description":"Cumulative Runtime count of tool result errors in this turn, including calls rejected before dispatch. When required, copy the exact const value. Later successful calls do not reduce this count, and the summary must disclose it."},
                "observed_command_failure_count":{"type":"integer","minimum":0,"maximum":4294967295_u64,"description":"Cumulative Runtime count of failed command observations in this turn, including nonzero exits, timeouts and lost terminals. When required, copy the exact const value. Later successful commands do not reduce this count, and the summary must disclose it."},
                "criteria":{"type":"array","minItems":1,"maxItems":16,"description":"A flat JSON array of criterion objects, never a JSON-encoded string. summary and the observed count fields are top-level siblings of criteria, not criteria entries. Never nest another array inside criteria.","items":{
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
                        "disposition":{"type":"string","enum":["supported","unverified","blocked","scope_changed"],"description":"supported cites matching currently available evidence. After work settles, earlier actual observations without current evidence belong in the summary and use this minimal criterion shape: {\"disposition\":\"unverified\",\"rationale\":\"Earlier results are reported in the summary; later state was not rechecked.\"}. Omit evidence_call_ids/evidence_paths; do not attach an old or unrelated ID. Adapt the rationale to the user's language and actual remaining uncertainty. This does not mean the earlier tool never ran. Include the requested concrete earlier results in summary; the example rationale does not replace them. Missing required work or unresolved effects need blocked, not this historical-result example."},
                        "requirement_ids":{"type":"array","minItems":1,"maxItems":32,"items":{"type":"string","minLength":1,"maxLength":64},"description":"Optional actual JSON array value; never a JSON-encoded string. Omit this field entirely to address all accepted requirements; an explicit empty array is invalid. Requirements can be shared across criteria."},
                        "scope_change":crate::requirements::citation_schema(),
                        "evidence_call_ids":{"type":"array","maxItems":8,"items":{"type":"string","minLength":1,"maxLength":256},"description":"Actual JSON array of exact call_id entries currently listed in available_evidence, never a JSON-encoded string. For separate process results, cite the matching call ID only when listed; if it is absent, use unverified with no evidence for current-state verification instead of copying the newest command ID. When history tools are already advertised, they may recover already-seen output for the requested summary; recovery never makes that observation current or eligible for citation. Do not repeat work without authorization. This includes deletion or artifact observations. A call remembered from an earlier step may no longer be eligible."},
                        "evidence_paths":{"type":"array","maxItems":8,"items":{"type":"string","minLength":1,"maxLength":4096},"description":"Actual JSON array of non-null path entries currently listed in available_evidence, never a JSON-encoded string. Do not guess a path from prior writes, deletions or artifact source_path. Re-read a needed stale file when authorized before reporting. This does not claim functional verification."},
                        "rationale":{"type":"string","minLength":1,"maxLength":1024,"description":"REQUIRED for every criterion. Briefly explain what the cited evidence supports or why the item is unverified/blocked."}
                    }
                }}
            }
        })),
    }
}

impl CompletionTracker {
    /// Runtime control citations are turn-local data, not Kernel grants. The
    /// same exposed schema is used by the whole-batch argument preflight.
    pub(crate) fn definition_with_evidence(
        &self,
        plan: &AgentPlan,
        work: &AgentWorkStatus,
        unresolved_patch: bool,
    ) -> ChatToolDefinition {
        let mut tool = definition();
        tool.description = format!("Use the user language and plain words in public summaries and rationales. Do not expose available_evidence, ineligible_observations, eligible evidence, unverified, call IDs or schema fields as user-facing diagnoses unless the user explicitly requests those internals. Explain what was actually observed and what later state remains unchecked; loss of current evidence eligibility does not mean the tool never ran or the application failed. Copy exact count fields in the JSON account; the runtime separately delivers the cumulative outcome counts. Describe actual exit codes and errors instead of repeating generic tool-error statistics as application-fault notices. {}",tool.description);
        tool.description = format!("available_evidence is the current-support citation set, not a list of everything that ran. Match each actual scope; a directory listing never proves file text or a digest. If an earlier read/search/Git ID is ineligible, use unverified for current-state verification with no evidence; still deliver the earlier actual results requested by the user when known from the transcript, clearly as earlier observations. Do not claim later state is unchanged, borrow another ID, or rerun settled work only to fix the account. {}",tool.description);
        tool.input_schema.0["properties"]["observed_tool_error_count"]["const"] =
            serde_json::json!(work.failed_tools);
        tool.input_schema.0["properties"]["observed_command_failure_count"]["const"] =
            serde_json::json!(work.failed_commands);
        for (field,count,meaning) in [
            ("observed_tool_error_count",work.failed_tools,"All unsuccessful tool results, including ordinary command nonzero outcomes marked is_error, even expected diagnostics, and any validation/admission failures. This is not a count of application faults."),
            ("observed_command_failure_count",work.failed_commands,"All unsuccessful command observations. An expected nonzero diagnostic still belongs to this recorded total."),
        ] {
            let property=&mut tool.input_schema.0["properties"][field];
            // Native callers commonly follow enum/default annotations more
            // reliably than const alone. They do not supply missing input:
            // whole-batch validation still requires the exact recorded value.
            property["enum"]=serde_json::json!([count]);
            property["default"]=serde_json::json!(count);
            property["description"]=serde_json::json!(format!("Fixed host-owned integer: {count}. Copy this value verbatim; do not recompute it from whether the result was expected. {meaning}"));
        }
        let mut required_failure_counts = serde_json::Map::new();
        let mut failure_descriptions = Vec::new();
        if work.failed_tools > 0 {
            tool.input_schema.0["required"].as_array_mut()
                .expect("completion required fields are an array")
                .push(serde_json::json!("observed_tool_error_count"));
            required_failure_counts.insert(
                "observed_tool_error_count".into(),
                serde_json::json!(work.failed_tools),
            );
            failure_descriptions.push(format!(
                "exactly {} unsuccessful tool result(s), including returned nonzero command outcomes even when expected, and any validation or admission failures",
                work.failed_tools
            ));
        }
        if work.failed_commands > 0 {
            tool.input_schema.0["required"].as_array_mut()
                .expect("completion required fields are an array")
                .push(serde_json::json!("observed_command_failure_count"));
            required_failure_counts.insert(
                "observed_command_failure_count".into(),
                serde_json::json!(work.failed_commands),
            );
            failure_descriptions.push(format!(
                "exactly {} failed command observation(s)",
                work.failed_commands
            ));
        }
        if !required_failure_counts.is_empty() {
            tool.description = format!(
                "This turn has recorded {}. Copy EVERY field from this exact JSON object into report_completion: {}. The runtime delivers every count with the accepted report. Explain the observed outcome in the user summary; later successful calls or commands do not erase recorded outcomes. {}",
                failure_descriptions.join(" and "),
                serde_json::Value::Object(required_failure_counts),
                tool.description
            );
        }
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
        let fields = tool.input_schema.0["properties"]["criteria"]["items"]["properties"]
            .as_object_mut()
            .expect("report_completion criterion properties are an object");
        if plan.requirements.is_empty() {
            // The engine will create the implicit full-input requirement while
            // closing the plan. Do not advertise an optional array for which
            // the model has no valid IDs yet: weak tool callers otherwise tend
            // to emit `requirement_ids: []`, which is neither an ID selection
            // nor the documented omission meaning "all accepted input".
            fields.remove("requirement_ids");
            tool.description = format!(
                "No requirement IDs are currently advertised. Omit requirement_ids entirely; never send an empty array. {}",
                tool.description
            );
        } else {
            fields["requirement_ids"]["items"]["enum"] = serde_json::json!(
                plan.requirements
                    .iter()
                    .map(|requirement| &requirement.id)
                    .collect::<Vec<_>>()
            );
        }
        for (name, values) in [("evidence_paths", paths), ("evidence_call_ids", calls)] {
            if values.is_empty() {
                // Empty enum is invalid JSON Schema. Only omission or an
                // empty array is permitted until a usable observation exists.
                fields[name]["maxItems"] = serde_json::json!(0);
                if name == "evidence_paths" {
                    fields[name]["description"] = serde_json::json!(
                        "No current file paths are available; omit this field or use []. If available_evidence lists a matching eligible call_id, use evidence_call_ids only for that observation's actual scope. A path remembered from an earlier read is not current-state evidence; do not repeat settled effects to repair a report.");
                }
            } else {
                fields[name]["items"]["enum"] = serde_json::json!(values);
            }
        }
        tool
    }

    pub(crate) fn invalidate(&mut self) {
        self.invalidate_report();
        self.settled_failure_gate = false;
    }

    pub(crate) fn invalidate_report(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.report = None;
    }

    pub(crate) fn settled_failure_gate(&self) -> bool { self.settled_failure_gate }
    pub(crate) fn set_settled_failure_gate(&mut self, allowed: bool) { self.settled_failure_gate = allowed; }
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
        // A new observation invalidates the report, not an already proven
        // gate cause. Steering/instruction/unknown-effect invalidations use
        // invalidate() and clear that cause explicitly.
        self.invalidate_report();
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
        if let Some(command) = command.filter(|command| {
            command.cleanup_proven
                && command.omitted_interactions == 0
                && ((command.was_current_at_observation
                    && command.state == "exited"
                    && command.exit_code == Some(0))
                    || (binding.action_id.as_ref() == "workspace.process/cancel"
                        && command.state == "cancelled"
                        && command.provenance_workspace_epoch
                            == Some(work.workspace_observation_epoch)))
        }) {
            let chain = command
                .launch_call_id
                .iter()
                .chain(command.interaction_call_ids.iter())
                .collect::<BTreeSet<_>>();
            for observation in &self.observations {
                if chain.contains(&observation.call_id)
                    && observation.invocation_attempted
                    && observation.successful
                    && command.launch_workspace_epoch.is_some_and(|launch_epoch| {
                        observation.workspace_epoch >= launch_epoch
                    })
                    && command.provenance_workspace_epoch.is_some_and(
                        |provenance_epoch| observation.workspace_epoch <= provenance_epoch,
                    )
                {
                    // The command tracker only marks this terminal current when every
                    // interaction advanced the same owned process lineage without a
                    // gap. Preserve those exact calls across their expected epoch
                    // advances; do not revive other observations from those epochs.
                    self.valid_through.insert(
                        observation.call_id.clone(),
                        work.workspace_observation_epoch,
                    );
                }
            }
        }
        let usable = invocation_attempted
            && !result.is_error
            && work.running_processes.is_empty()
            && (binding.capability_id.as_ref() != "workspace.process"
                || command.is_some_and(|command| {
                    command.cleanup_proven
                        && ((command.was_current_at_observation
                            && command.state == "exited"
                            && command.exit_code == Some(0))
                            || (binding.action_id.as_ref() == "workspace.process/cancel"
                                && command.state == "cancelled"))
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
        self.scopes.insert(observation.call_id.clone(),observation_scope(binding,call,owner_result.as_ref(),owner_path.is_some(),effects_are_scoped));
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
                        .map_or(0, |artifact| serde_json::to_vec(artifact).map_or(usize::MAX, |value| value.len())))
                    .saturating_add(self.scopes.get(&item.call_id)
                        .map_or(0, |scope| crate::stream_limits::serialized_size(scope,2048).unwrap_or(usize::MAX))))
                .fold(0usize, usize::saturating_add)
                > 32 * 1024
        {
            let removed = self.observations.remove(0);
            self.valid_through.remove(&removed.call_id);
            self.owner_paths.remove(&removed.call_id);
            self.artifacts.remove(&removed.call_id);
            self.scopes.remove(&removed.call_id);
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
                && report.observed_tool_error_count == work.failed_tools
                && report.observed_command_failure_count == work.failed_commands
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
        let ineligible_count = self.observations.iter().filter(|item| !self.is_usable(item,work.workspace_observation_epoch)).count();
        let mut ineligible = Vec::new();
        let mut detail_bytes = 0usize;
        for item in self.observations.iter().rev().filter(|item| !self.is_usable(item,work.workspace_observation_epoch)).take(8) {
            let detail = serde_json::json!({"call_id":item.call_id,"tool":item.tool_name,"path":item.path,
                "invocation_attempted":item.invocation_attempted,"successful_result":item.successful,
                "observed_workspace_epoch":item.workspace_epoch,"eligible_current_evidence":false,
                "scope":self.scopes.get(&item.call_id)});
            let Ok(size) = crate::stream_limits::serialized_size(&detail,4096-detail_bytes) else { break; };
            detail_bytes += size;
            ineligible.push(detail);
        }
        let value = serde_json::json!({"plan_revision":plan.revision,"observation_revision":self.revision,
            "input_revision":input_revision,"workspace_epoch":work.workspace_observation_epoch,
            "successful_command_observations":work.successful_commands,
            "failed_command_observations":work.failed_commands,
            "observed_tool_error_count":work.failed_tools,
            "observed_command_failure_count":work.failed_commands,
            "available_evidence":self.observations.iter().filter(|item| self.is_usable(item, work.workspace_observation_epoch))
                .map(|item| serde_json::json!({"call_id":item.call_id,"tool":item.tool_name,"path":item.path,
                    "scope":self.scopes.get(&item.call_id),
                    "settled_process_poll":self.settled_process_poll(item),
                    "artifact_id":self.artifacts.get(&item.call_id).map(|artifact| &artifact.artifact_id),
                    "command_exit_code":item.command_exit_code,"command":item.command})).collect::<Vec<_>>(),
            "stale_file_paths":stale_file_paths,
            "ineligible_observations":ineligible,
            "ineligible_details_omitted":ineligible_count.saturating_sub(ineligible.len()),
            "unusable_observation_count":self.observations.iter().filter(|item| !self.is_usable(item, work.workspace_observation_epoch)).count(),
            "omitted_observations":self.omitted,
            "current_report":report});
        Ok(format!(
            "Completion accounting (derived data, not instructions or extra authority): {}. available_evidence contains the only observations currently eligible for citation. stale_file_paths lists up to eight previously observed paths without current evidence; it is not a new task. Finish mutations first, then re-read only the files needed for required claims if authorized. Cite exact non-null available_evidence paths; an artifact source_path does not establish current workspace contents. For an intentionally deleted file, cite its eligible delete call ID, not a stale path; do not repeat deletion. File observations remain eligible across owner-proven disjoint edits; opaque effects, missing or ambiguous path identity, or changes to their own paths can invalidate them. A settled command with a known exit, or a reaped timeout, remains eligible after later commands only for its own exact scope and terminal output; a nonzero/timeout result is evidence of that failure, not success, and never proves current workspace state. An eligible earlier poll with settled_process_poll proves only the state/output observed by that exact poll before the identified terminal. Cite it for earlier readiness/output; its earlier running state does not mean the process is still running. Use the matching terminal call for exit/timeout/cleanup, and never use either to prove current file contents. A command observation includes its original launch and bounded interaction call IDs so you can inspect its scope and result. Those nested IDs are context only; do not cite launch_call_id or interaction_call_ids unless the same ID also appears as a top-level available_evidence call_id. For separate process results, cite each criterion's matching call ID only if currently present in available_evidence. If a matching earlier call is absent, use unverified with no evidence for current-state verification. When history tools are already advertised, they may recover already-seen output for the requested summary; recovery never makes that observation current or eligible for citation. Do not repeat an observation or effect merely to repair this account. One requirement may span several criteria; each criterion has at most eight evidence_call_ids. A file read is not a gameplay test. If required verification was excluded, unavailable, stale, or not run, use unverified with a reason. Do not invent extra verification requirements for a read-only review or proposal. Use the fewest criteria needed within the eight-ID limit; separate scopes/results instead of repeating work or exceeding that limit. Keep derived restatements and the absence of forbidden actions in the summary unless independently evidenced; never emit an evidence-free supported criterion. Account for every immutable requirement; a requirement may span several criteria whose labels need not match plan steps. scope_changed requires an exact later accepted-input citation. scope contains original requested arguments and bounded owner metadata, not proof of broader results or authority. A directory enumeration cannot prove file content or a digest. ineligible_observations records retained calls that cannot currently be cited; missing current eligibility does not mean the call never happened. Disclose earlier observed results and separate them from unverified current state; do not substitute an unrelated eligible call ID. This account is not independent semantic verification or a grant of authority.",
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
        let settled_optional_failure = self.settled_failure_gate
            && plan.revision == 0 && plan.steps.is_empty()
            && work.running_processes.is_empty() && !unresolved_patch;
        if !closing.needs_replan || reporting_blocked || settled_optional_failure {
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
        if submission.observed_tool_error_count.unwrap_or(0) != work.failed_tools
            || (work.failed_tools > 0 && submission.observed_tool_error_count.is_none())
        {
            return Err(format!(
                "Completion must include observed_tool_error_count={} and disclose that exact cumulative count; later successful calls do not erase earlier tool errors",
                work.failed_tools));
        }
        if submission.observed_command_failure_count.unwrap_or(0) != work.failed_commands
            || (work.failed_commands > 0
                && submission.observed_command_failure_count.is_none())
        {
            return Err(format!(
                "Completion must include observed_command_failure_count={} and disclose that exact cumulative count; later successful commands do not erase earlier command failures",
                work.failed_commands));
        }
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
            observed_tool_error_count: work.failed_tools,
            observed_command_failure_count: work.failed_commands,
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

    fn is_immutable_command_result(&self, observation: &AgentCompletionObservation) -> bool {
        // A later opaque command can change the workspace, but it cannot make
        // an earlier settled command terminal un-happen. Keep that command's
        // own scope/exit/output citeable without extending file/path evidence
        // or treating a nonzero exit as success.
        observation.invocation_attempted && observation.command.as_ref().is_some_and(|command| {
                let exact_terminal = (command.state == "exited"
                    && command.exit_code.is_some()
                    && command.exit_code == observation.command_exit_code)
                    || (command.state == "timed_out"
                        && command.exit_code.is_none()
                        && observation.command_exit_code.is_none());
                command.observation_call_id == observation.call_id
                    && exact_terminal
                    && command.cleanup_proven
                    && command.omitted_interactions == 0
                    && command.launch_call_id.as_deref().is_some_and(|launch| {
                        // A terminal poll has its own call ID. Preserve only
                        // that exact result with a retained, bound launch and
                        // matching owner process, never current workspace
                        // contents. Earlier polls require their own matching
                        // observation below. Missing old scope
                        // metadata remains conservative.
                        launch == observation.call_id
                            || (!launch.is_empty()
                                && command.interaction_call_ids.last() == Some(&observation.call_id)
                                && self.observations.iter().any(|item| item.call_id == launch && item.invocation_attempted)
                                && self.scopes.get(launch).is_some_and(|scope|
                                    scope["capability"] == "workspace.process"
                                        && matches!(scope["action"].as_str(), Some("workspace.process/start" | "workspace.process/exec")))
                                && self.scopes.get(&observation.call_id).is_some_and(|scope|
                                    scope["capability"] == "workspace.process"
                                        && scope["action"] == "workspace.process/poll"
                                        && scope["requested_arguments"]["process_id"].as_str() == Some(command.process_id.as_str())))
                    })
                    && command.observed_workspace_epoch == observation.workspace_epoch
            })
    }

    fn settled_process_poll(&self, observation: &AgentCompletionObservation) -> Option<&str> {
        if !observation.invocation_attempted || !observation.successful { return None; }
        let scope = self.scopes.get(&observation.call_id)?;
        if scope["capability"] != "workspace.process" || scope["action"] != "workspace.process/poll" {
            return None;
        }
        let process_id = scope["requested_arguments"]["process_id"].as_str()?;
        // Only a retained, exact settled owner chain qualifies a prior poll.
        // This preserves that poll's past output/state, never file freshness,
        // current running state, or a launch/stdin action's effect claim.
        self.observations.iter().find_map(|terminal| {
            let command = terminal.command.as_ref()?;
            if terminal.call_id == observation.call_id || command.process_id != process_id
                || !command.interaction_call_ids.contains(&observation.call_id)
                || !self.is_immutable_command_result(terminal)
                || observation.workspace_epoch > terminal.workspace_epoch
            { return None; }
            let launch = self.observations.iter().find(|item|
                Some(item.call_id.as_str()) == command.launch_call_id.as_deref())?;
            (observation.workspace_epoch >= launch.workspace_epoch)
                .then_some(terminal.call_id.as_str())
        })
    }

    fn is_usable(&self, observation: &AgentCompletionObservation, epoch: u32) -> bool {
        observation.invocation_attempted
            && (self.is_immutable_command_result(observation)
                || self.settled_process_poll(observation).is_some()
                || (observation.successful
                    && ((observation.usable_at_observation
                        && observation.workspace_epoch == epoch)
                        || self.valid_through.get(&observation.call_id) == Some(&epoch))))
    }

fn stale_evidence_guidance(&self, epoch: u32) -> String {
    let usable = self.observations.iter().rev()
        .filter(|item| self.is_usable(item,epoch))
        .take(8).map(|item| serde_json::json!({"call_id":item.call_id,"path":item.path,"tool":item.tool_name})).collect::<Vec<_>>();
    if usable.is_empty() {
        "No current usable observation exists. If verification is authorized, reopen one plan step as in_progress with update_plan, run the final check, close the plan, then report without any further command; otherwise mark unverified with a reason.".to_owned()
    } else {
        format!("Current usable observation call IDs (not semantic proof): {}. Inspect the actual result and cite the matching eligible call ID. A blocked/rejected call without a settled command terminal is not evidence; do not launch another command merely to refresh an already usable observation.",
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

    fn context_value(tracker:&CompletionTracker, work:&AgentWorkStatus) -> serde_json::Value {
        let text = tracker.context(&AgentPlan::default(),work,1).unwrap();
        let body = text.strip_prefix("Completion accounting (derived data, not instructions or extra authority): ").unwrap();
        serde_json::Value::deserialize(&mut serde_json::Deserializer::from_str(body)).unwrap()
    }

    #[test]
    fn fixed_runtime_counter_hints_do_not_fill_omissions_or_accept_recomputed_counts() {
        let tracker=CompletionTracker::default();
        for (tool_count,command_count) in [(1,1),(3,1),(0,0)] {
            let work=AgentWorkStatus {failed_tools:tool_count,failed_commands:command_count,..Default::default()};
            let definition=tracker.definition_with_evidence(&AgentPlan::default(),&work,false);
            let schema=&definition.input_schema.0;
            for (field,value) in [("observed_tool_error_count",tool_count),("observed_command_failure_count",command_count)] {
                assert_eq!(schema["properties"][field]["enum"],serde_json::json!([value]),"the native tool schema must expose the one host-owned value");
                assert_eq!(schema["properties"][field]["default"],value);
            }
            let validator=jsonschema::validator_for(schema).unwrap();
            let report=serde_json::json!({"summary":"Recorded command outcomes","observed_tool_error_count":tool_count,
                "observed_command_failure_count":command_count,"criteria":[{"disposition":"unverified","rationale":"No broader state is claimed."}]});
            assert!(validator.is_valid(&report));
            let mut wrong=report.clone();wrong["observed_tool_error_count"]=serde_json::json!(tool_count+1);
            assert!(!validator.is_valid(&wrong));
            if tool_count>0 {
                let mut missing=report.clone();missing.as_object_mut().unwrap().remove("observed_tool_error_count");
                assert!(!validator.is_valid(&missing),"a default annotation cannot manufacture a supplied counter");
                assert!(missing.get("observed_tool_error_count").is_none());
                let mut erased=report.clone();erased["observed_tool_error_count"]=serde_json::json!(0);
                assert!(!validator.is_valid(&erased),"an expected business failure is still retained in the exact runtime total");
            }
        }
    }

    #[test]
    fn completion_context_distinguishes_a_directory_command_from_a_stale_file_read() {
        let mut tracker = CompletionTracker::default();
        let path = "资料 空格/样本.txt";
        let read = ChatToolCall { call_id:"file-read".into(),name:"read_file".into(),provider_metadata:None,
            arguments:StrictJsonValue(serde_json::json!({"path":path})) };
        let result = AgentToolResult::text(read.call_id.clone(),serde_json::json!({"path":path,
            "workspace_path":{"root_sha256":"a".repeat(64),"path":path,"case_resolved":true},
            "sha256":"b".repeat(64),"total_bytes":43,"offset":0,"eof":true,"content":"PRIVATE_FILE_CONTENT"}).to_string(),false);
        let old = tracker.observe(&AgentWorkStatus::default(),&file_binding("workspace.files/read"),&read,&result,true);
        let work = AgentWorkStatus { workspace_observation_epoch:1,
            recent_commands:vec![settled_command("directory-list","owned",1,0)],..Default::default() };
        let listing = ChatToolCall { call_id:"directory-list".into(),name:"exec_command".into(),provider_metadata:None,
            arguments:StrictJsonValue(serde_json::json!({"cmd":"Get-ChildItem -LiteralPath . -Force","env":{"TOKEN":"PRIVATE_ENV_SECRET"}})) };
        tracker.observe(&work,&process_binding("workspace.process/exec"),&listing,
            &AgentToolResult::text(listing.call_id.clone(),"directory names",false),true);
        let context = context_value(&tracker,&work);
        let current = &context["available_evidence"][0];
        assert_eq!(current["call_id"],"directory-list");
        assert_eq!(current["scope"]["requested_arguments"]["cmd"],"Get-ChildItem -LiteralPath . -Force");
        assert!(current["scope"]["owner_observation"].is_null(),"directory enumeration must not acquire file content/hash metadata");
        let stale = &context["ineligible_observations"][0];
        assert_eq!(stale["tool"],"read_file");
        assert_eq!(stale["scope"]["owner_observation"]["sha256"],"b".repeat(64));
        assert_eq!(stale["eligible_current_evidence"],false);
        assert!(!tracker.is_usable(&old,work.workspace_observation_epoch));
        let schema = tracker.definition_with_evidence(&AgentPlan::default(),&work,false).input_schema.0;
        assert_eq!(schema["properties"]["criteria"]["items"]["properties"]["evidence_call_ids"]["items"]["enum"],serde_json::json!(["directory-list"]));
        let serialized = context.to_string();
        assert!(!serialized.contains("PRIVATE_FILE_CONTENT") && !serialized.contains("PRIVATE_ENV_SECRET"));
    }

    #[test]
    fn missing_file_scope_retains_absence_without_claiming_content_or_freshness() {
        let mut tracker=CompletionTracker::default();
        let call=ChatToolCall {call_id:"absent-copy".into(),name:"read_file".into(),provider_metadata:None,
            arguments:StrictJsonValue(serde_json::json!({"path":"副本 结果.txt","missing_ok":true}))};
        let result=AgentToolResult::text(call.call_id.clone(),
            serde_json::json!({"kind":"workspace_file_absent","path":"副本 结果.txt"}).to_string(),false);
        let work=AgentWorkStatus::default();
        let observation=tracker.observe(&work,&file_binding("workspace.files/read"),&call,&result,true);
        let current=context_value(&tracker,&work);
        assert_eq!(current["available_evidence"][0]["scope"]["owner_observation"],
            serde_json::json!({"kind":"workspace_file_absent","file_exists":false}));
        assert!(tracker.is_usable(&observation,0));
        let later=AgentWorkStatus {workspace_observation_epoch:1,..Default::default()};
        let stale=context_value(&tracker,&later);
        assert_eq!(stale["available_evidence"],serde_json::json!([]));
        assert_eq!(stale["ineligible_observations"][0]["scope"]["owner_observation"]["file_exists"],false);
        assert!(!tracker.is_usable(&observation,1));
        for invalid in [
            serde_json::json!({"kind":"workspace_file_absent","path":"unrelated.txt"}),
            serde_json::json!({"content":"{\"kind\":\"workspace_file_absent\"}"}),
        ] {
            assert!(observation_scope(&file_binding("workspace.files/read"),&call,Some(&invalid),false,true)["owner_observation"].is_null());
        }
        let marker=serde_json::json!({"kind":"workspace_file_absent","path":"副本 结果.txt"});
        assert!(observation_scope(&file_binding("workspace.files/read"),&call,Some(&marker),false,false)["owner_observation"].is_null());
        for arguments in [
            serde_json::json!({"path":"副本 结果.txt"}),
            serde_json::json!({"path":"副本 结果.txt","missing_ok":true,"format":"image"}),
            serde_json::json!({"path":"副本 结果.txt","missing_ok":true,"format":"instruction_scope"}),
        ] {
            let mut wrong=call.clone();wrong.arguments=StrictJsonValue(arguments);
            assert!(observation_scope(&file_binding("workspace.files/read"),&wrong,Some(&marker),false,true)["owner_observation"].is_null());
        }
        for (attempted,is_error) in [(false,false),(true,true)] {
            let mut rejected=CompletionTracker::default();
            let output=AgentToolResult::text(call.call_id.clone(),marker.to_string(),is_error);
            rejected.observe(&work,&file_binding("workspace.files/read"),&call,&output,attempted);
            let value=context_value(&rejected,&work);
            assert!(value["available_evidence"].as_array().unwrap().is_empty());
            assert!(value["ineligible_observations"][0]["scope"]["owner_observation"].is_null());
        }
    }

    #[test]
    fn operation_scope_metadata_is_bounded_and_excludes_effect_payloads() {
        let call = ChatToolCall { call_id:"large".into(),name:"exec_command".into(),provider_metadata:None,
            arguments:StrictJsonValue(serde_json::json!({"cmd":"UNBOUNDED_SCRIPT_MARKER".repeat(3000),
                "env":{"TOKEN":"ENV_PRIVATE"},"input":"STDIN_PRIVATE","content":"FILE_PRIVATE"})) };
        let scope = observation_scope(&process_binding("workspace.process/exec"),&call,None,false,true);
        assert_eq!(scope["requested_arguments_omitted"],true);
        assert!(scope["requested_arguments"].is_null(),"an excerpt must not pretend to be complete executable arguments");
        let encoded = scope.to_string();
        assert!(encoded.len()<1024);
        for private in ["UNBOUNDED_SCRIPT_MARKER","ENV_PRIVATE","STDIN_PRIVATE","FILE_PRIVATE"] {
            assert!(!encoded.contains(private));
        }
        let mut tracker = CompletionTracker::default();
        for index in 0..100 {
            let mut call = call.clone(); call.call_id=format!("scope-{index}").into();
            tracker.observe(&AgentWorkStatus::default(),&process_binding("workspace.process/exec"),&call,
                &AgentToolResult::text(call.call_id.clone(),"held before execution",true),false);
        }
        assert_eq!(tracker.scopes.len(),tracker.observations.len());
        assert!(tracker.scopes.len()<=64 && !tracker.scopes.contains_key("scope-0"));
        let context = context_value(&tracker,&AgentWorkStatus::default());
        assert!(serde_json::to_vec(&context["ineligible_observations"]).unwrap().len()<=4096+18);
        assert!(context["ineligible_details_omitted"].as_u64().unwrap()>0);
        assert_eq!(context["available_evidence"],serde_json::json!([]));
    }

    #[test]
    fn stale_search_and_git_calls_remain_visible_without_becoming_current_evidence() {
        let mut tracker = CompletionTracker::default();
        for (name,capability,action,args) in [
            ("search_files","workspace.files","workspace.files/search",serde_json::json!({"query":"needle","path":"."})),
            ("git_status","workspace.vcs","workspace.vcs/status",serde_json::json!({})),
            ("git_diff","workspace.vcs","workspace.vcs/diff",serde_json::json!({"path":"."})),
        ] {
            let mut binding = file_binding(action); binding.capability_id=capability.into();
            let call = ChatToolCall {call_id:name.into(),name:name.into(),arguments:StrictJsonValue(args),provider_metadata:None};
            tracker.observe(&AgentWorkStatus::default(),&binding,&call,&AgentToolResult::text(call.call_id.clone(),"observed result",false),true);
        }
        let work = AgentWorkStatus { workspace_observation_epoch:1,..Default::default() };
        let context = context_value(&tracker,&work);
        assert_eq!(context["available_evidence"],serde_json::json!([]));
        let old = context["ineligible_observations"].as_array().unwrap();
        assert_eq!(old.len(),3);
        assert!(old.iter().all(|item| item["invocation_attempted"]==true && item["successful_result"]==true
            && item["eligible_current_evidence"]==false));
        let search = old.iter().find(|item| item["tool"]=="search_files").unwrap();
        assert_eq!(search["scope"]["requested_arguments"]["query"],"needle");
        assert!(tracker.definition_with_evidence(&AgentPlan::default(),&work,false).input_schema.0
            ["properties"]["criteria"]["items"]["properties"]["evidence_call_ids"]["maxItems"]==0);
    }

    #[test]
    fn context_invalidation_cannot_reuse_a_settled_failure_gate() {
        let mut tracker = CompletionTracker::default();
        tracker.set_settled_failure_gate(true);
        tracker.invalidate();
        assert!(!tracker.settled_failure_gate());
        let plan = AgentPlan { needs_replan:true, ..Default::default() };
        assert!(plan.effect_gate().is_some(), "context changes retain the effect/recovery gate");
    }

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

    fn process_binding(action: &str) -> AgentToolBinding {
        let mut binding = file_binding(action);
        binding.model_name = "cancel_process".into();
        binding.definition.name = "cancel_process".into();
        binding.capability_id = "workspace.process".into();
        binding
    }

    fn settled_command(
        call_id: &str,
        process_id: &str,
        epoch: u32,
        exit_code: i32,
    ) -> crate::AgentCommandObservation {
        crate::AgentCommandObservation {
            process_id: process_id.into(),
            launch_call_id: Some(call_id.into()),
            observation_call_id: call_id.into(),
            state: "exited".into(),
            exit_code: Some(exit_code),
            cleanup_proven: true,
            launch_workspace_epoch: Some(epoch),
            provenance_workspace_epoch: Some(epoch),
            interaction_call_ids: Vec::new(),
            omitted_interactions: 0,
            observed_workspace_epoch: epoch,
            was_current_at_observation: true,
        }
    }

    #[test]
    fn explicit_reaped_cancel_is_eligible_completion_evidence() {
        let work = AgentWorkStatus {
            workspace_observation_epoch: 1,
            recent_commands: vec![crate::AgentCommandObservation {
                process_id: "process-1".into(),
                launch_call_id: Some("start-1".into()),
                observation_call_id: "cancel-1".into(),
                state: "cancelled".into(),
                exit_code: None,
                cleanup_proven: true,
                launch_workspace_epoch: Some(1),
                provenance_workspace_epoch: Some(1),
                interaction_call_ids: vec!["poll-1".into()],
                omitted_interactions: 0,
                observed_workspace_epoch: 1,
                was_current_at_observation: false,
            }],
            ..Default::default()
        };
        let call = ChatToolCall {
            call_id: "cancel-1".into(),
            name: "cancel_process".into(),
            arguments: StrictJsonValue(serde_json::json!({"process_id":"process-1"})),
            provider_metadata: None,
        };
        let result = AgentToolResult::text(
            call.call_id.clone(),
            serde_json::json!({
                "state":"cancelled", "process_id":"process-1", "success":true,
                "cleanup":{"reaped":true,"errors":["interrupt unavailable"]}
            })
            .to_string(),
            false,
        );
        let mut tracker = CompletionTracker::default();
        let observation = tracker.observe(&work, &process_binding("workspace.process/cancel"),
            &call, &result, true);
        assert!(observation.successful);
        assert!(observation.usable_at_observation);
        assert_eq!(observation.command_exit_code, None);
        assert_eq!(observation.command.as_ref().unwrap().state, "cancelled");
        assert_eq!(
            observation
                .command
                .as_ref()
                .unwrap()
                .interaction_call_ids,
            ["poll-1"]
        );

        let schema = tracker
            .definition_with_evidence(&AgentPlan::default(), &work, false)
            .input_schema
            .0;
        let validator = jsonschema::options().build(&schema).unwrap();
        assert!(validator.is_valid(&serde_json::json!({
            "summary":"Cancelled and reaped the managed process",
            "criteria":[{
                "disposition":"supported", "evidence_call_ids":["cancel-1"],
                "rationale":"The explicit cancellation returned cancelled with cleanup.reaped=true"
            }]
        })));
    }

    #[test]
    fn terminal_stdin_chain_keeps_every_exact_interaction_evidence() {
        let process_id = "stdin-process";
        let call = |id: &str, name: &str, arguments| ChatToolCall {
            call_id: id.into(),
            name: name.into(),
            arguments: StrictJsonValue(arguments),
            provider_metadata: None,
        };
        let running = |call: &ChatToolCall, text: &str, cursor: u64| {
            AgentToolResult::text(
                call.call_id.clone(),
                serde_json::json!({
                    "state":"running","pid":1234,"process_id":process_id,"success":null,
                    "output":{"text":text,"next_cursor":cursor,"retained_bytes":cursor,"dropped_bytes":0,
                        "source_encoding":"utf-8","decode_errors":0}
                })
                .to_string(),
                false,
            )
        };
        let mut tracker = CompletionTracker::default();
        let mut observe_running = |id: &str,
                                   name: &str,
                                   action: &str,
                                   arguments: serde_json::Value,
                                   epoch: u32,
                                   text: &str,
                                   cursor: u64| {
            let call = call(id, name, arguments);
            let mut binding = process_binding(action);
            binding.model_name = name.into();
            binding.definition.name = name.into();
            tracker.observe(
                &AgentWorkStatus {
                    running_processes: [process_id.to_owned()].into_iter().collect(),
                    workspace_observation_epoch: epoch,
                    ..Default::default()
                },
                &binding,
                &call,
                &running(&call, text, cursor),
                true,
            );
        };
        observe_running(
            "start-call",
            "start_process",
            "workspace.process/start",
            serde_json::json!({"command":"reader"}),
            1,
            "",
            0,
        );
        observe_running(
            "input-call",
            "write_process_stdin",
            "workspace.process/input",
            serde_json::json!({"process_id":process_id,"input":"payload"}),
            2,
            "",
            0,
        );
        observe_running(
            "close-call",
            "close_process_stdin",
            "workspace.process/close_stdin",
            serde_json::json!({"process_id":process_id}),
            3,
            "ECHO:payload\n",
            13,
        );
        drop(observe_running);
        let poll = call(
            "poll-call",
            "poll_process",
            serde_json::json!({"process_id":process_id,"cursor":0,"wait_ms":5000}),
        );
        let command = crate::AgentCommandObservation {
            process_id: process_id.into(),
            launch_call_id: Some("start-call".into()),
            observation_call_id: "poll-call".into(),
            state: "exited".into(),
            exit_code: Some(0),
            cleanup_proven: true,
            launch_workspace_epoch: Some(1),
            provenance_workspace_epoch: Some(3),
            interaction_call_ids: vec![
                "input-call".into(),
                "close-call".into(),
                "poll-call".into(),
            ],
            omitted_interactions: 0,
            observed_workspace_epoch: 3,
            was_current_at_observation: true,
        };
        let work = AgentWorkStatus {
            workspace_observation_epoch: 3,
            recent_commands: vec![command],
            ..Default::default()
        };
        let mut poll_binding = process_binding("workspace.process/poll");
        poll_binding.model_name = "poll_process".into();
        poll_binding.definition.name = "poll_process".into();
        tracker.observe(
            &work,
            &poll_binding,
            &poll,
            &AgentToolResult::text(
                poll.call_id.clone(),
                serde_json::json!({
                    "state":"exited","exit_code":0,"signal":null,"process_id":process_id,"success":true,
                    "output":{"text":"ECHO:payload\n","next_cursor":13,"retained_bytes":13,"dropped_bytes":0,
                        "source_encoding":"utf-8","decode_errors":0},
                    "cleanup":{"interrupt_attempted":false,"terminate_attempted":false,
                        "force_kill_attempted":false,"reaped":true,"elapsed_ms":0,"errors":[]}
                })
                .to_string(),
                false,
            ),
            true,
        );
        assert!(
            tracker
                .observations
                .iter()
                .all(|observation| tracker.is_usable(observation, 3)),
            "terminal cleanup must preserve the exact start/input/close/poll chain"
        );
        let definition = tracker.definition_with_evidence(&AgentPlan::default(), &work, false);
        let actual = definition.input_schema.0["properties"]["criteria"]["items"]
            ["properties"]["evidence_call_ids"]["items"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let expected = ["start-call", "input-call", "close-call", "poll-call"]
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn settled_successful_commands_keep_distinct_evidence_after_later_commands() {
        let mut binding = process_binding("workspace.process/exec");
        binding.model_name = "exec_command".into();
        binding.definition.name = "exec_command".into();
        let alpha = ChatToolCall {
            call_id: "alpha-call".into(),
            name: "exec_command".into(),
            arguments: StrictJsonValue(serde_json::json!({"command":"echo-alpha"})),
            provider_metadata: None,
        };
        let beta = ChatToolCall {
            call_id: "beta-call".into(),
            name: "exec_command".into(),
            arguments: StrictJsonValue(serde_json::json!({"command":"echo-beta"})),
            provider_metadata: None,
        };
        let alpha_command = settled_command("alpha-call", "process-alpha", 1, 0);
        let beta_command = settled_command("beta-call", "process-beta", 2, 0);
        let result = |call: &ChatToolCall, command: &crate::AgentCommandObservation| {
            AgentToolResult::text(
                call.call_id.clone(),
                serde_json::json!({
                    "process_id":command.process_id,
                    "state":"exited",
                    "exit_code":0,
                    "cleanup":{"reaped":true},
                    "success":true
                })
                .to_string(),
                false,
            )
        };
        let mut tracker = CompletionTracker::default();
        tracker.observe(
            &AgentWorkStatus {
                workspace_observation_epoch: 1,
                recent_commands: vec![alpha_command.clone()],
                ..Default::default()
            },
            &binding,
            &alpha,
            &result(&alpha, &alpha_command),
            true,
        );
        assert!(tracker.is_usable(&tracker.observations[0], 1));
        let work = AgentWorkStatus {
            workspace_observation_epoch: 2,
            recent_commands: vec![alpha_command, beta_command.clone()],
            ..Default::default()
        };
        tracker.observe(
            &work,
            &binding,
            &beta,
            &result(&beta, &beta_command),
            true,
        );
        assert!(
            tracker.is_usable(&tracker.observations[0], 2),
            "a later command must not erase the settled ALPHA exit/output fact"
        );
        assert!(tracker.is_usable(&tracker.observations[1], 2));
        let definition = tracker.definition_with_evidence(&AgentPlan::default(), &work, false);
        let ids = definition.input_schema.0["properties"]["criteria"]["items"]
            ["properties"]["evidence_call_ids"]["items"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(ids, &[serde_json::json!("alpha-call"), serde_json::json!("beta-call")]);
        let validator = jsonschema::options()
            .build(&definition.input_schema.0)
            .unwrap();
        assert!(validator.is_valid(&serde_json::json!({
            "summary":"ALPHA and BETA each use their matching command observation",
            "criteria":[
                {"disposition":"supported","evidence_call_ids":["alpha-call"],"rationale":"ALPHA command output"},
                {"disposition":"supported","evidence_call_ids":["beta-call"],"rationale":"BETA command output"}
            ]
        })));
        let context = tracker.context(&AgentPlan::default(), &work, 1).unwrap();
        assert!(context.contains("\"call_id\":\"alpha-call\""));
        assert!(context.contains("\"call_id\":\"beta-call\""));
    }

    fn settled_poll_fixture(state: &str, exit_code: Option<i32>) -> (CompletionTracker, AgentWorkStatus) {
        process_poll_fixture(state, exit_code, false)
    }

    fn process_poll_fixture(state: &str, exit_code: Option<i32>, include_ready: bool) -> (CompletionTracker, AgentWorkStatus) {
        let mut tracker = CompletionTracker::default();
        let mut work = AgentWorkStatus::default();
        let mut commands = crate::workflow::CommandTracker::default();
        let read = ChatToolCall {
            call_id: "before-read".into(), name: "read_file".into(),
            arguments: StrictJsonValue(serde_json::json!({"path":"sample.txt"})), provider_metadata: None,
        };
        tracker.observe(&work, &file_binding("workspace.files/read"), &read,
            &AgentToolResult::text(read.call_id.clone(), "earlier sample", false), true);
        let mut calls = vec![
            ("start-call", "start_process", "workspace.process/start",
                serde_json::json!({"command":"diagnostic","args":["sample.txt"]}),
                serde_json::json!({"process_id":"owned-process","state":"running","success":null}), false),
            ("terminal-poll", "poll_process", "workspace.process/poll",
                serde_json::json!({"process_id":"owned-process","cursor":0,"wait_ms":30000}),
                serde_json::json!({"process_id":"owned-process","state":state,"exit_code":exit_code,
                    "signal":null,"cleanup":{"reaped":true,"errors":[]},"success":exit_code==Some(0)}),
                exit_code != Some(0)),
            ("later-command", "exec_command", "workspace.process/exec",
                serde_json::json!({"command":"later-effect"}),
                serde_json::json!({"process_id":"later-process","state":"exited","exit_code":0,
                    "cleanup":{"reaped":true},"success":true}), false),
        ];
        if include_ready {
            calls.insert(1, ("ready-poll", "poll_process", "workspace.process/poll",
                serde_json::json!({"process_id":"owned-process","cursor":0,"wait_ms":30000}),
                serde_json::json!({"process_id":"owned-process","state":"running","success":null,
                    "output":{"text":"READY_PARENT\nREADY_CHILD\n","next_cursor":25}}), false));
            calls[2].3["cursor"] = serde_json::json!(25);
        }
        for (id, name, action, arguments, receipt, error) in calls {
            let call = ChatToolCall {call_id:id.into(), name:name.into(),
                arguments:StrictJsonValue(arguments), provider_metadata:None};
            let mut binding = process_binding(action);
            binding.model_name = name.into();
            binding.definition.name = name.into();
            let result = AgentToolResult::text(call.call_id.clone(), receipt.to_string(), error);
            work.observe(&binding, &call, &result, &mut commands);
            tracker.observe(&work, &binding, &call, &result, true);
        }
        (tracker, work)
    }

    #[tokio::test]
    async fn earlier_successful_polls_of_a_settled_process_remain_historical_evidence() {
        for (state, exit_code) in [("exited", Some(0)), ("exited", Some(1)), ("timed_out", None)] {
            let (mut tracker, work) = process_poll_fixture(state, exit_code, true);
            assert!(work.running_processes.is_empty());
            assert_eq!(work.workspace_observation_epoch, 2);
            let ready = &tracker.observations[2];
            assert!(ready.successful && !ready.usable_at_observation);
            assert!(ready.command.is_none());
            assert!(tracker.is_usable(ready, 2),
                "the successful READY poll is evidence of its earlier output after exact {state}/{exit_code:?} cleanup");
            assert!(!tracker.is_usable(&tracker.observations[0], 2));
            assert!(!tracker.is_usable(&tracker.observations[1], 2));
            assert!(!tracker.valid_through.contains_key("before-read"));
            assert_ne!(tracker.valid_through.get("ready-poll"), Some(&2));
            let definition = tracker.definition_with_evidence(&AgentPlan::default(), &work, false);
            let validator = jsonschema::validator_for(&definition.input_schema.0).unwrap();
            let report = serde_json::json!({
                "summary":"READY_PARENT and READY_CHILD were observed earlier; the process has now ended and been reaped. No current workspace claim.",
                "observed_tool_error_count":work.failed_tools,"observed_command_failure_count":work.failed_commands,
                "criteria":[
                    {"disposition":"supported","evidence_call_ids":["ready-poll"],"rationale":"Earlier output from the exact owned process."},
                    {"disposition":"supported","evidence_call_ids":["terminal-poll"],"rationale":"The exact process terminal and cleanup."}
                ]});
            assert!(validator.is_valid(&report));
            let context = context_value(&tracker, &work);
            let entry = context["available_evidence"].as_array().unwrap().iter()
                .find(|entry| entry["call_id"] == "ready-poll").unwrap();
            assert_eq!(entry["settled_process_poll"], "terminal-poll");
            assert_eq!(entry["scope"]["requested_arguments"]["process_id"], "owned-process");
            assert_eq!(entry["command"], serde_json::Value::Null);
            assert_eq!(entry["path"], serde_json::Value::Null);
            let call = ChatToolCall {call_id:"report".into(), name:TOOL_NAME.into(),
                arguments:StrictJsonValue(report), provider_metadata:None};
            let inputs = vec![crate::context_lifecycle::text_message(
                nomifun_chat_model_broker::ChatRole::User, "Report readiness and the settled diagnostic.".into())];
            let result = tracker.submit(&call, &mut AgentPlan::default(), &work, &inputs,
                false, None, &crate::NoopAgentEventSink).await.unwrap();
            assert!(!result.is_error, "{}", result.output_text());
        }
    }

    #[tokio::test]
    async fn settled_poll_results_survive_later_commands_without_refreshing_workspace_evidence() {
        for (state, exit_code) in [("exited", Some(0)), ("exited", Some(1)), ("timed_out", None)] {
            let (mut tracker, work) = settled_poll_fixture(state, exit_code);
            assert_eq!(work.workspace_observation_epoch, 2);
            assert!(work.running_processes.is_empty());
            let terminal = &tracker.observations[2];
            assert_eq!(terminal.command.as_ref().unwrap().launch_call_id.as_deref(), Some("start-call"));
            assert!(tracker.is_usable(terminal, 2),
                "the original reaped {state}/{exit_code:?} poll remains evidence of its own terminal result");
            assert!(!tracker.is_usable(&tracker.observations[0], 2), "later opaque effects still invalidate file contents");
            assert!(!tracker.is_usable(&tracker.observations[1], 2), "the launch is not a later terminal observation");
            let definition = tracker.definition_with_evidence(&AgentPlan::default(), &work, false);
            let validator = jsonschema::validator_for(&definition.input_schema.0).unwrap();
            let summary = format!("The diagnostic {state} with exit {exit_code:?}; this describes its earlier terminal, not current file state.");
            let report = serde_json::json!({"summary":summary,
                "observed_tool_error_count":work.failed_tools,"observed_command_failure_count":work.failed_commands,
                "criteria":[{"disposition":"supported","evidence_call_ids":["terminal-poll"],
                    "rationale":"The matching owned-process poll records this settled terminal."}]});
            assert!(validator.is_valid(&report));
            for stale in ["before-read", "start-call"] {
                let mut wrong = report.clone();
                wrong["criteria"][0]["evidence_call_ids"] = serde_json::json!([stale]);
                assert!(!validator.is_valid(&wrong));
            }
            let context = context_value(&tracker, &work);
            let entry = context["available_evidence"].as_array().unwrap().iter()
                .find(|entry| entry["call_id"] == "terminal-poll").unwrap();
            assert_eq!(entry["scope"]["action"], "workspace.process/poll");
            assert_eq!(entry["scope"]["requested_arguments"]["process_id"], "owned-process");
            assert_eq!(entry["command"]["state"], state);
            assert_eq!(entry["command_exit_code"], serde_json::json!(exit_code));
            assert_eq!(entry["path"], serde_json::Value::Null);
            let call = ChatToolCall {call_id:"report".into(), name:TOOL_NAME.into(),
                arguments:StrictJsonValue(report), provider_metadata:None};
            let inputs = vec![crate::context_lifecycle::text_message(
                nomifun_chat_model_broker::ChatRole::User, "Report the diagnostic terminal and later command.".into())];
            let mut plan = AgentPlan::default();
            let result = tracker.submit(&call, &mut plan, &work, &inputs, false, None, &crate::NoopAgentEventSink)
                .await.unwrap();
            assert!(!result.is_error, "{}", result.output_text());
            let accepted = tracker.current(&plan, &work, 1).unwrap();
            assert_eq!(accepted.criteria[0].evidence_call_ids, ["terminal-poll"]);
            assert_eq!(accepted.observed_command_failure_count, u32::from(exit_code != Some(0)));
            assert_eq!(accepted.observed_tool_error_count, u32::from(exit_code != Some(0)));
        }
    }

    #[test]
    fn earlier_polls_require_a_successful_observation_and_exact_retained_settlement() {
        for defect in ["failed-poll", "not-dispatched", "wrong-capability", "wrong-action", "wrong-process",
            "missing-scope", "before-launch", "after-terminal", "unreaped", "lost", "wrong-exit",
            "wrong-terminal-call", "wrong-terminal-epoch", "missing-launch", "unknown-launch",
            "undispatched-launch", "omitted-interaction", "missing-interaction", "wrong-last-interaction",
            "missing-terminal", "missing-terminal-scope"] {
            let (mut tracker, _) = process_poll_fixture("timed_out", None, true);
            match defect {
                "failed-poll" => tracker.observations[2].successful = false,
                "not-dispatched" => tracker.observations[2].invocation_attempted = false,
                "wrong-capability" => tracker.scopes.get_mut("ready-poll").unwrap()["capability"] = serde_json::json!("workspace.files"),
                "wrong-action" => tracker.scopes.get_mut("ready-poll").unwrap()["action"] = serde_json::json!("workspace.process/input"),
                "wrong-process" => tracker.scopes.get_mut("ready-poll").unwrap()["requested_arguments"]["process_id"] = serde_json::json!("other-process"),
                "missing-scope" => { tracker.scopes.remove("ready-poll"); }
                "before-launch" => tracker.observations[2].workspace_epoch = 0,
                "after-terminal" => tracker.observations[2].workspace_epoch = 99,
                "undispatched-launch" => tracker.observations[1].invocation_attempted = false,
                "missing-terminal" => { tracker.observations.remove(3); }
                "missing-terminal-scope" => { tracker.scopes.remove("terminal-poll"); }
                _ => {
                    let command = tracker.observations[3].command.as_mut().unwrap();
                    match defect {
                        "unreaped" => command.cleanup_proven = false,
                        "lost" => command.state = "lost".into(),
                        "wrong-exit" => command.exit_code = Some(0),
                        "wrong-terminal-call" => command.observation_call_id = "other-poll".into(),
                        "wrong-terminal-epoch" => command.observed_workspace_epoch = 99,
                        "missing-launch" => command.launch_call_id = None,
                        "unknown-launch" => command.launch_call_id = Some("other-start".into()),
                        "omitted-interaction" => command.omitted_interactions = 1,
                        "missing-interaction" => command.interaction_call_ids = vec!["terminal-poll".into()],
                        "wrong-last-interaction" => command.interaction_call_ids.push("other-poll".into()),
                        _ => unreachable!(),
                    }
                }
            }
            assert!(!tracker.is_usable(&tracker.observations[2], 2), "{defect} cannot qualify an earlier poll");
        }
    }

    #[test]
    fn incomplete_or_unbound_polls_do_not_become_immutable_completion_evidence() {
        for defect in ["unreaped", "lost", "wrong-exit", "wrong-call", "wrong-epoch", "no-launch",
            "empty-launch", "unknown-launch", "omitted-interaction", "wrong-interaction", "wrong-process",
            "wrong-action", "wrong-capability", "missing-scope", "not-dispatched"] {
            let (mut tracker, _) = settled_poll_fixture("exited", Some(1));
            let observation = &mut tracker.observations[2];
            let command = observation.command.as_mut().unwrap();
            match defect {
                "unreaped" => command.cleanup_proven = false,
                "lost" => command.state = "lost".into(),
                "wrong-exit" => command.exit_code = Some(7),
                "wrong-call" => command.observation_call_id = "other-poll".into(),
                "wrong-epoch" => command.observed_workspace_epoch = 99,
                "no-launch" => command.launch_call_id = None,
                "empty-launch" => command.launch_call_id = Some(String::new()),
                "unknown-launch" => command.launch_call_id = Some("other-start".into()),
                "omitted-interaction" => command.omitted_interactions = 1,
                "wrong-interaction" => command.interaction_call_ids = vec!["other-poll".into()],
                "wrong-process" => command.process_id = "other-process".into(),
                "wrong-action" => tracker.scopes.get_mut("terminal-poll").unwrap()["action"] = serde_json::json!("workspace.process/input"),
                "wrong-capability" => tracker.scopes.get_mut("terminal-poll").unwrap()["capability"] = serde_json::json!("workspace.files"),
                "missing-scope" => { tracker.scopes.remove("terminal-poll"); }
                "not-dispatched" => observation.invocation_attempted = false,
                _ => unreachable!(),
            }
            assert!(!tracker.is_usable(&tracker.observations[2], 2), "{defect} is not a usable terminal receipt");
        }
    }

    #[tokio::test]
    async fn settled_failed_command_is_citable_for_its_exact_terminal_result() {
        let mut binding = process_binding("workspace.process/exec");
        binding.model_name = "exec_command".into();
        binding.definition.name = "exec_command".into();
        let failed = ChatToolCall {
            call_id: "expected-failure".into(),
            name: "exec_command".into(),
            arguments: StrictJsonValue(serde_json::json!({"command":"diagnostic"})),
            provider_metadata: None,
        };
        let mut command = settled_command("expected-failure", "process-failed", 1, 7);
        // Production command tracking does not grant a current workspace
        // provenance epoch to an is_error terminal, even though the exact
        // launch identity, exit and cleanup receipt are trustworthy.
        command.launch_workspace_epoch = None;
        command.provenance_workspace_epoch = None;
        command.was_current_at_observation = false;
        let work = AgentWorkStatus {
            failed_commands: 1,
            failed_tools: 1,
            workspace_observation_epoch: 1,
            recent_commands: vec![command.clone()],
            ..Default::default()
        };
        let mut tracker = CompletionTracker::default();
        tracker.observe(
            &work,
            &binding,
            &failed,
            &AgentToolResult::text(
                failed.call_id.clone(),
                serde_json::json!({
                    "process_id":command.process_id,
                    "state":"exited",
                    "exit_code":7,
                    "cleanup":{"reaped":true},
                    "success":false
                })
                .to_string(),
                true,
            ),
            true,
        );
        assert!(
            tracker.is_usable(&tracker.observations[0], 1),
            "a trusted nonzero terminal is evidence of that failure"
        );
        let inputs = vec![crate::context_lifecycle::text_message(
            nomifun_chat_model_broker::ChatRole::User,
            "Run the expected failing diagnostic and explain its exit".into(),
        )];
        let mut plan = AgentPlan::default();
        let report = ChatToolCall {
            call_id: "report".into(),
            name: TOOL_NAME.into(),
            arguments: StrictJsonValue(serde_json::json!({
                "summary":"The diagnostic ran and exited 7 as expected",
                "observed_tool_error_count":1,
                "observed_command_failure_count":1,
                "criteria":[{
                    "disposition":"supported",
                    "evidence_call_ids":["expected-failure"],
                    "rationale":"The settled command observation records exit code 7"
                }]
            })),
            provider_metadata: None,
        };
        let result = tracker
            .submit(
                &report,
                &mut plan,
                &work,
                &inputs,
                false,
                None,
                &crate::NoopAgentEventSink,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output_text());
        assert_eq!(
            tracker.current(&plan, &work, 1).unwrap().criteria[0].evidence_call_ids,
            ["expected-failure"]
        );
    }

    #[tokio::test]
    async fn reaped_timeout_is_citable_for_its_exact_terminal_result() {
        let mut binding = process_binding("workspace.process/exec");
        binding.model_name = "exec_command".into();
        binding.definition.name = "exec_command".into();
        let timed_out = ChatToolCall {
            call_id: "timed-out".into(),
            name: "exec_command".into(),
            arguments: StrictJsonValue(serde_json::json!({"command":"slow","timeout_ms":250})),
            provider_metadata: None,
        };
        let mut command = settled_command("timed-out", "process-timeout", 1, 0);
        command.state = "timed_out".into();
        command.exit_code = None;
        command.launch_workspace_epoch = None;
        command.provenance_workspace_epoch = None;
        command.was_current_at_observation = false;
        let work = AgentWorkStatus {
            failed_commands: 1,
            failed_tools: 1,
            workspace_observation_epoch: 1,
            recent_commands: vec![command.clone()],
            ..Default::default()
        };
        let mut tracker = CompletionTracker::default();
        tracker.observe(
            &work,
            &binding,
            &timed_out,
            &AgentToolResult::text(
                timed_out.call_id.clone(),
                serde_json::json!({
                    "process_id":command.process_id,
                    "state":"timed_out",
                    "cleanup":{"reaped":true},
                    "success":false
                })
                .to_string(),
                true,
            ),
            true,
        );
        assert!(
            tracker.is_usable(&tracker.observations[0], 1),
            "a reaped timeout is evidence of that terminal result"
        );
        let inputs = vec![crate::context_lifecycle::text_message(
            nomifun_chat_model_broker::ChatRole::User,
            "Run the expected timeout diagnostic and explain its terminal".into(),
        )];
        let mut plan = AgentPlan::default();
        let report = ChatToolCall {
            call_id: "report-timeout".into(),
            name: TOOL_NAME.into(),
            arguments: StrictJsonValue(serde_json::json!({
                "summary":"The diagnostic timed out and was reaped as expected",
                "observed_tool_error_count":1,
                "observed_command_failure_count":1,
                "criteria":[{
                    "disposition":"supported",
                    "evidence_call_ids":["timed-out"],
                    "rationale":"The settled command observation records timed_out with cleanup.reaped=true"
                }]
            })),
            provider_metadata: None,
        };
        let result = tracker
            .submit(
                &report,
                &mut plan,
                &work,
                &inputs,
                false,
                None,
                &crate::NoopAgentEventSink,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output_text());
        assert_eq!(
            tracker.current(&plan, &work, 1).unwrap().criteria[0].evidence_call_ids,
            ["timed-out"]
        );
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
        let plan = AgentPlan {
            requirements: vec![crate::AgentTaskRequirement {
                id: "input_0".into(),
                description: "Complete the accepted input".into(),
                source: crate::AgentInputCitation {
                    input: 0,
                    quote: "accepted input".into(),
                },
                origin: None,
            }],
            ..Default::default()
        };
        let definition = tracker.definition_with_evidence(&plan, &work, false);
        let criteria_description=definition.input_schema.0["properties"]["criteria"]["description"].as_str().unwrap();
        assert!(criteria_description.contains("flat JSON array of criterion objects"));
        assert!(criteria_description.contains("top-level siblings"));
        assert!(criteria_description.contains("Never nest another array"));
        assert!(definition.description.contains("Each criterion allows at most eight evidence_call_ids"));
        assert!(definition.description.contains("separate criteria for different results or more than eight IDs"));
        assert!(definition.description.contains("never create an evidence-free supported criterion"));
        assert!(definition.description.contains("separate process calls"));
        assert!(definition.description.contains("matching call ID"));
        assert!(definition.description.contains("absent from available_evidence"));
        assert!(definition.description.contains("use unverified"));
        assert!(definition.description.contains("do not cite launch_call_id"));
        assert!(definition.input_schema.0["properties"]["criteria"]["items"]["properties"]
            ["rationale"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("REQUIRED")));
        let schema = definition.input_schema.0;
        assert!(schema["properties"]["criteria"]["items"]["properties"]["requirement_ids"]
            ["description"]
            .as_str()
            .is_some_and(|description| description.contains("JSON array value")
                && description.contains("never a JSON-encoded string")
                && description.contains("empty array is invalid")));
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
        assert!(!validator.is_valid(&serde_json::json!({"criteria":[
            [{"disposition":"supported","rationale":"Observed work","evidence_call_ids":["current"]}],
            {"summary":"Finished","observed_tool_error_count":0,"observed_command_failure_count":0}
        ]})),"nested criteria and root fields remain invalid");
        let mut too_many_calls = report("evidence_call_ids", "current");
        too_many_calls["criteria"][0]["evidence_call_ids"] = serde_json::json!(vec!["current"; 9]);
        assert!(!validator.is_valid(&too_many_calls));
        let mut empty_requirements = report("evidence_call_ids", "current");
        empty_requirements["criteria"][0]["requirement_ids"] = serde_json::json!([]);
        assert!(!validator.is_valid(&empty_requirements));
        let mut unknown_requirement = report("evidence_call_ids", "current");
        unknown_requirement["criteria"][0]["requirement_ids"] = serde_json::json!(["unknown"]);
        assert!(!validator.is_valid(&unknown_requirement));
        for (path, call) in [("old.txt", "stale"), ("missing.txt", "missing"), ("failed.txt", "failed")] {
            assert!(!validator.is_valid(&report("evidence_paths", path)));
            assert!(!validator.is_valid(&report("evidence_call_ids", call)));
        }
        assert!(!validator.is_valid(&serde_json::json!({"summary":"Unsupported claim","criteria":[
            {"disposition":"supported","rationale":"No observation was cited"}
        ]})));
        let empty_definition = CompletionTracker::default()
            .definition_with_evidence(&AgentPlan::default(), &work, false);
        assert!(empty_definition
            .description
            .contains("Omit requirement_ids entirely"));
        assert!(empty_definition.input_schema.0["properties"]["criteria"]["items"]
            ["properties"]
            .get("requirement_ids")
            .is_none());
        assert!(empty_definition.input_schema.0["properties"]["criteria"]["items"]
            ["properties"]["evidence_paths"]["description"].as_str().unwrap()
            .contains("No current file paths are available; omit this field"));
        let empty = empty_definition.input_schema.0;
        let validator = jsonschema::options().build(&empty).unwrap();
        assert!(!validator.is_valid(&report("evidence_paths", "current.txt")));
        assert!(!validator.is_valid(&report("evidence_call_ids", "current")));
        assert!(validator.is_valid(&serde_json::json!({"summary":"Verification unavailable","criteria":[
            {"disposition":"unverified","rationale":"No authorized observation available"}
        ]})));
        assert!(!validator.is_valid(&serde_json::json!({"summary":"Unsupported claim","criteria":[
            {"disposition":"supported","rationale":"No observation was cited"}
        ]})));
        assert!(!validator.is_valid(&serde_json::json!({"summary":"No IDs advertised","criteria":[
            {"disposition":"unverified","rationale":"No observation available","requirement_ids":[]}
        ]})));
    }

    #[test]
    fn completion_schema_requires_the_exact_cumulative_tool_error_count() {
        let tracker = CompletionTracker::default();
        let work = AgentWorkStatus {
            successful_commands: 50,
            failed_commands: 0,
            failed_tools: 5,
            ..Default::default()
        };
        let definition = tracker.definition_with_evidence(&AgentPlan::default(), &work, false);
        assert!(definition.description.contains("exactly 5 unsuccessful tool result(s)"));
        assert!(definition.description.contains("including returned nonzero command outcomes even when expected"));
        let schema = definition.input_schema.0;
        assert_eq!(schema["properties"]["observed_tool_error_count"]["const"], 5);
        assert!(schema["required"].as_array().unwrap().contains(
            &serde_json::json!("observed_tool_error_count")));
        let validator = jsonschema::options().build(&schema).unwrap();
        let report = |count: Option<u32>| {
            let mut value = serde_json::json!({
                "summary":"Recovered after five visible tool errors",
                "criteria":[{"disposition":"unverified","rationale":"Earlier command evidence is outside the bounded window"}]
            });
            if let Some(count) = count {
                value["observed_tool_error_count"] = serde_json::json!(count);
            }
            value
        };
        assert!(!validator.is_valid(&report(None)));
        assert!(!validator.is_valid(&report(Some(0))));
        assert!(validator.is_valid(&report(Some(5))));

        let context = tracker.context(&AgentPlan::default(), &work, 1).unwrap();
        assert!(context.contains("\"successful_command_observations\":50"));
        assert!(context.contains("\"failed_command_observations\":0"));
        assert!(context.contains("\"observed_tool_error_count\":5"));
    }

    #[test]
    fn completion_schema_requires_the_exact_cumulative_command_failure_count() {
        let tracker = CompletionTracker::default();
        let work = AgentWorkStatus {
            successful_commands: 1,
            failed_commands: 2,
            ..Default::default()
        };
        let definition =
            tracker.definition_with_evidence(&AgentPlan::default(), &work, false);
        assert!(
            definition
                .description
                .contains("exactly 2 failed command observation(s)")
        );
        let schema = definition.input_schema.0;
        assert_eq!(
            schema["properties"]["observed_command_failure_count"]["const"],
            2
        );
        assert!(schema["required"].as_array().unwrap().contains(
            &serde_json::json!("observed_command_failure_count")
        ));
        let validator = jsonschema::options().build(&schema).unwrap();
        let report = |count: Option<u32>| {
            let mut value = serde_json::json!({
                "summary":"Recovered after two command failures",
                "criteria":[{"disposition":"unverified","rationale":"Earlier command evidence is outside the bounded window"}]
            });
            if let Some(count) = count {
                value["observed_command_failure_count"] = serde_json::json!(count);
            }
            value
        };
        assert!(!validator.is_valid(&report(None)));
        assert!(!validator.is_valid(&report(Some(0))));
        assert!(validator.is_valid(&report(Some(2))));

        let context = tracker.context(&AgentPlan::default(), &work, 1).unwrap();
        assert!(context.contains("\"observed_command_failure_count\":2"));
    }

    #[test]
    fn completion_description_groups_every_required_failure_count_in_one_json_object() {
        let tracker = CompletionTracker::default();
        let work = AgentWorkStatus {
            failed_tools: 1,
            failed_commands: 1,
            ..Default::default()
        };
        let definition =
            tracker.definition_with_evidence(&AgentPlan::default(), &work, false);
        assert!(
            definition
                .description
                .contains("Copy EVERY field from this exact JSON object")
        );
        assert!(
            definition
                .description
                .contains("\"observed_tool_error_count\":1")
        );
        assert!(
            definition
                .description
                .contains("\"observed_command_failure_count\":1")
        );
        let validator = jsonschema::options()
            .build(&definition.input_schema.0)
            .unwrap();
        let report = |tool: Option<u32>, command: Option<u32>| {
            let mut value = serde_json::json!({
                "summary":"One command failed before recovery",
                "criteria":[{"disposition":"unverified","rationale":"The failure is outside current evidence"}]
            });
            if let Some(count) = tool {
                value["observed_tool_error_count"] = serde_json::json!(count);
            }
            if let Some(count) = command {
                value["observed_command_failure_count"] = serde_json::json!(count);
            }
            value
        };
        assert!(!validator.is_valid(&report(Some(1), None)));
        assert!(!validator.is_valid(&report(None, Some(1))));
        assert!(validator.is_valid(&report(Some(1), Some(1))));
    }

    #[test]
    fn legacy_completion_reports_default_failure_counts_to_zero() {
        let report: AgentCompletionReport = serde_json::from_value(serde_json::json!({
            "plan_revision":1,
            "observation_revision":2,
            "input_revision":1,
            "workspace_epoch":3,
            "summary":"legacy report",
            "criteria":[],
            "requirements":[]
        })).unwrap();
        assert_eq!(report.observed_tool_error_count, 0);
        assert_eq!(report.observed_command_failure_count, 0);
    }

    #[tokio::test]
    async fn completion_rejects_erasing_recovered_tool_errors_and_discloses_the_exact_count() {
        let inputs = vec![crate::context_lifecycle::text_message(
            nomifun_chat_model_broker::ChatRole::User,
            "Run a sequence and report visible failures".into(),
        )];
        let mut plan = AgentPlan::default();
        let mut tracker = CompletionTracker::default();
        let work = AgentWorkStatus { failed_tools: 2, ..Default::default() };
        let call = |id: &str, count: Option<u32>| {
            let mut arguments = serde_json::json!({
                "summary":"Two tool calls failed before recovery",
                "criteria":[{"disposition":"unverified","rationale":"No current observation proves the whole sequence"}]
            });
            if let Some(count) = count {
                arguments["observed_tool_error_count"] = serde_json::json!(count);
            }
            ChatToolCall { call_id:id.into(), name:TOOL_NAME.into(),
                arguments:StrictJsonValue(arguments), provider_metadata:None }
        };

        for (id, count) in [("missing", None), ("wrong", Some(0))] {
            let result = tracker.submit(&call(id, count), &mut plan, &work, &inputs,
                false, None, &crate::NoopAgentEventSink).await.unwrap();
            assert!(result.is_error);
            assert!(result.output_text().contains("observed_tool_error_count=2"));
            assert!(tracker.current(&plan, &work, 1).is_none());
        }

        let result = tracker.submit(&call("correct", Some(2)), &mut plan, &work, &inputs,
            false, None, &crate::NoopAgentEventSink).await.unwrap();
        assert!(!result.is_error, "{}", result.output_text());
        let report = tracker.current(&plan, &work, 1).unwrap();
        assert_eq!(report.observed_tool_error_count, 2);
        assert!(report.tool_error_disclosure().unwrap().contains("turn: 2"));

        let mut changed = work.clone();
        changed.failed_tools = 3;
        assert!(tracker.current(&plan, &changed, 1).is_none(),
            "a later tool error must invalidate an older completion account");
    }

    #[tokio::test]
    async fn completion_rejects_erasing_recovered_command_failures_and_discloses_the_exact_count() {
        let inputs = vec![crate::context_lifecycle::text_message(
            nomifun_chat_model_broker::ChatRole::User,
            "Run a command sequence and report visible failures".into(),
        )];
        let mut plan = AgentPlan::default();
        let mut tracker = CompletionTracker::default();
        let work = AgentWorkStatus {
            successful_commands: 1,
            failed_commands: 2,
            ..Default::default()
        };
        let call = |id: &str, count: Option<u32>| {
            let mut arguments = serde_json::json!({
                "summary":"Two commands failed before recovery",
                "criteria":[{"disposition":"unverified","rationale":"No current observation proves the whole sequence"}]
            });
            if let Some(count) = count {
                arguments["observed_command_failure_count"] = serde_json::json!(count);
            }
            ChatToolCall {
                call_id: id.into(),
                name: TOOL_NAME.into(),
                arguments: StrictJsonValue(arguments),
                provider_metadata: None,
            }
        };

        for (id, count) in [("missing", None), ("wrong", Some(0))] {
            let result = tracker
                .submit(
                    &call(id, count),
                    &mut plan,
                    &work,
                    &inputs,
                    false,
                    None,
                    &crate::NoopAgentEventSink,
                )
                .await
                .unwrap();
            assert!(result.is_error);
            assert!(
                result
                    .output_text()
                    .contains("observed_command_failure_count=2")
            );
            assert!(tracker.current(&plan, &work, 1).is_none());
        }

        let result = tracker
            .submit(
                &call("correct", Some(2)),
                &mut plan,
                &work,
                &inputs,
                false,
                None,
                &crate::NoopAgentEventSink,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output_text());
        let report = tracker.current(&plan, &work, 1).unwrap();
        assert_eq!(report.observed_command_failure_count, 2);
        assert!(
            report
                .command_failure_disclosure()
                .unwrap()
                .contains("turn: 2")
        );

        let mut changed = work.clone();
        changed.failed_commands = 3;
        assert!(
            tracker.current(&plan, &changed, 1).is_none(),
            "a later command failure must invalidate an older completion account"
        );
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
    async fn earlier_results_can_be_delivered_without_promoting_stale_current_evidence() {
        let inputs=vec![crate::context_lifecycle::text_message(nomifun_chat_model_broker::ChatRole::User,
            "Report the observed file head and search result.".into())];
        let mut tracker=CompletionTracker {observations:vec![file_observation("old-read","sample.txt",0)],..Default::default()};
        let work=AgentWorkStatus {workspace_observation_epoch:1,..Default::default()};
        let mut plan=AgentPlan::default();
        let definition=tracker.definition_with_evidence(&plan,&work,false);
        let validator=jsonschema::validator_for(&definition.input_schema.0).unwrap();
        let summary="At the earlier read, sample.txt began with alpha; the earlier search found one match. The file's later state was not rechecked.";
        let args=serde_json::json!({"summary":summary,"criteria":[{
            "disposition":"unverified","rationale":"The earlier results are reported separately from unverified current state."}]});
        assert!(validator.is_valid(&args));
        let mut false_current=args.clone();
        false_current["criteria"][0]["disposition"]=serde_json::json!("supported");
        false_current["criteria"][0]["evidence_call_ids"]=serde_json::json!(["old-read"]);
        assert!(!validator.is_valid(&false_current),"historical reporting cannot make an old call eligible for current support");
        let call=ChatToolCall {call_id:"report".into(),name:TOOL_NAME.into(),arguments:StrictJsonValue(args),provider_metadata:None};
        let result=tracker.submit(&call,&mut plan,&work,&inputs,false,None,&crate::NoopAgentEventSink).await.unwrap();
        assert!(!result.is_error,"{}",result.output_text());
        let report=tracker.current(&plan,&work,1).unwrap();
        assert_eq!(report.summary,summary);
        assert_eq!(report.criteria[0].disposition,AgentCriterionDisposition::Unverified);
        assert!(report.criteria[0].evidence_call_ids.is_empty());
        assert_eq!(tracker.observations[0].workspace_epoch,0);
        assert!(!tracker.is_usable(&tracker.observations[0],1));
        let description=definition.input_schema.0["properties"]["summary"]["description"].as_str().unwrap();
        assert!(description.contains("earlier actual tool results"));
        assert!(description.contains("does not delete an earlier observation"));
        assert!(definition.description.contains("unverified for current-state verification"));
        assert!(definition.description.contains("recover already-seen output for the requested summary"));
        assert!(definition.description.contains("never makes that observation current or eligible"));
        let call_description=definition.input_schema.0["properties"]["criteria"]["items"]
            ["properties"]["evidence_call_ids"]["description"].as_str().unwrap();
        assert!(call_description.contains("recover already-seen output for the requested summary"));
        assert!(!call_description.contains("loading history or repeating work"));
        let context=tracker.context(&plan,&work,1).unwrap();
        assert!(context.contains("When history tools are already advertised"));
        assert!(context.contains("never makes that observation current or eligible"));
        assert!(!context.contains("do not load history or repeat work unless authorized"));
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
    pub(crate) fn delivery_text(&self) -> String {
        format!("{}{}{}{}", self.summary,
            self.unverified_disclosure().unwrap_or_default(),
            self.tool_error_disclosure().unwrap_or_default(),
            self.command_failure_disclosure().unwrap_or_default())
    }

    pub(crate) fn matches_delivery(&self, text: &str) -> bool {
        let current = self.delivery_text();
        if text == current || text == format!("\n\n{current}") { return true; }
        // Preserve immutable deliveries from the earlier formatter. Both known
        // formats contain the exact accepted report and cumulative counts.
        let mut legacy = format!("{}{}", self.summary, self.unverified_disclosure().unwrap_or_default());
        if self.observed_tool_error_count > 0 {
            legacy.push_str(&format!("\n\n- ⚠ Tool-call errors observed in this turn: {}. Later successful calls did not erase these errors.", self.observed_tool_error_count));
        }
        if self.observed_command_failure_count > 0 {
            legacy.push_str(&format!("\n\n- ⚠ Command failures observed in this turn: {}. Later successful commands did not erase these failures.", self.observed_command_failure_count));
        }
        text == legacy || text == format!("\n\n{legacy}")
    }

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

    pub(crate) fn tool_error_disclosure(&self) -> Option<String> {
        (self.observed_tool_error_count > 0).then(|| format!(
            "\n\nUnsuccessful tool attempts in this turn: {} (including argument checks and command outcomes). Details remain available in the execution steps.",
            self.observed_tool_error_count
        ))
    }

    pub(crate) fn command_failure_disclosure(&self) -> Option<String> {
        (self.observed_command_failure_count > 0).then(|| format!(
            "\n\nUnsuccessful command attempts in this turn: {}. Each command's exit status and output explain the result.",
            self.observed_command_failure_count
        ))
    }
}

fn invalid(error: impl std::fmt::Display) -> AgentEngineError {
    AgentEngineError::InvalidContract(error.to_string())
}
