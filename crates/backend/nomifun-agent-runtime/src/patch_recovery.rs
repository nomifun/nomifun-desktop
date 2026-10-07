//! Engine policy only: a failed dispatched patch requires fresh observations.
//! No filesystem bypass, automatic undo, test execution or diagnostic-string heuristics.
//! Only the Kernel's exact bounded `workspace_patch_failed` JSON projection is decoded.
use crate::{AgentEffectClass, AgentToolBinding, AgentToolResult};
use nomifun_chat_model_broker::ChatToolCall;
use std::collections::BTreeSet;

/// Engine-derived recovery obligation, not a checkpoint, permission grant or
/// filesystem proof. Hosts restore this independently of conversational history.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPatchRecoveryState {
    pub version: u8,
    /// Targets that must be freshly observed before another effect.
    pub targets: Vec<String>,
    /// Failed patch targets whose requested mutation still lacks a successful
    /// owner receipt. Fresh reads alone do not settle this task obligation.
    #[serde(default)]
    pub unresolved_targets: Vec<String>,
    /// Accepted input count when the unresolved mutation was admitted. A later
    /// scope change can settle it only by citing inputs after this boundary.
    #[serde(default)]
    pub unresolved_before_input: Option<usize>,
    pub target_budget_exceeded: bool,
}

impl Default for AgentPatchRecoveryState {
    fn default() -> Self {
        Self {
            version: 2,
            targets: Vec::new(),
            unresolved_targets: Vec::new(),
            unresolved_before_input: None,
            target_budget_exceeded: false,
        }
    }
}

impl AgentPatchRecoveryState {
    pub fn has_pending(&self) -> bool {
        self.target_budget_exceeded || !self.targets.is_empty() || !self.unresolved_targets.is_empty()
    }

    pub fn validate(&self) -> Result<(), crate::AgentEngineError> {
        let pending = self.targets.iter().collect::<BTreeSet<_>>();
        let unresolved = self.unresolved_targets.iter().collect::<BTreeSet<_>>();
        if !matches!(self.version, 1 | 2)
            || (self.version == 1 && !self.unresolved_targets.is_empty())
            || self.targets.len() > 64
            || self.unresolved_targets.len() > 64
            || self.targets.iter().chain(&self.unresolved_targets).map(String::len).sum::<usize>() > 32 * 1024
            || self.unresolved_before_input.is_some_and(|count| count == 0 || count > 32)
            || (self.unresolved_targets.is_empty() && self.unresolved_before_input.is_some())
            || pending.len() != self.targets.len() || unresolved.len() != self.unresolved_targets.len()
            || self.targets.iter().chain(&self.unresolved_targets).any(|path| normalize(path).as_ref() != Some(path))
        {
            return Err(crate::AgentEngineError::InvalidContract(
                "Invalid bounded patch recovery state".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct PatchRecovery {
    unresolved: BTreeSet<String>,
    pending: BTreeSet<String>,
    active: BTreeSet<String>,
    unresolved_before_input: Option<usize>,
    active_input_count: Option<usize>,
    origin_before_active: Option<usize>,
    unaddressable: bool,
    // Results are folded only after an entire batch. Reads preceding a failed
    // patch (or process cleanup) in that batch cannot refresh its aftermath.
    ready_for_reads: bool,
    persisted: Option<AgentPatchRecoveryState>,
}

impl PatchRecovery {
    pub(crate) fn restore(
        state: &AgentPatchRecoveryState,
    ) -> Result<Self, crate::AgentEngineError> {
        state.validate()?;
        let mut pending = state.targets.iter().cloned().collect::<BTreeSet<_>>();
        let mut unresolved = state.unresolved_targets.iter().cloned().collect::<BTreeSet<_>>();
        // Historical v1 checkpoints only carried pending targets. Treat them
        // as unresolved instead of losing a failed mutation during upgrade.
        if state.version == 1 { unresolved.extend(pending.iter().cloned()); }
        // A resumed turn must independently refresh every unresolved target;
        // prior reads are historical after a process/generation boundary.
        pending.extend(unresolved.iter().cloned());
        Ok(Self {
            pending,
            unresolved,
            active: BTreeSet::new(),
            unresolved_before_input: state.unresolved_before_input,
            active_input_count: None,
            origin_before_active: None,
            unaddressable: state.target_budget_exceeded,
            ready_for_reads: true,
            persisted: Some(state.clone()),
        })
    }

    pub(crate) fn snapshot(&self) -> AgentPatchRecoveryState {
        AgentPatchRecoveryState {
            // Pending reads and unresolved mutation obligations are distinct;
            // restore() makes every unresolved target pending in a new turn.
            targets: self.pending.iter().cloned().collect(),
            unresolved_targets: self.unresolved.iter().cloned().collect(),
            unresolved_before_input: self.unresolved_before_input,
            target_budget_exceeded: self.unaddressable,
            ..Default::default()
        }
    }

    pub(crate) async fn persist(
        &mut self,
        sink: &dyn crate::AgentEventSink,
    ) -> Result<(), crate::AgentEngineError> {
        let state = self.snapshot();
        if self.persisted.as_ref() != Some(&state) {
            sink.emit(crate::AgentEngineEvent::PatchRecoveryUpdated {
                state: state.clone(),
            })
            .await?;
            self.persisted = Some(state);
        }
        Ok(())
    }

    pub(crate) fn arm(&mut self, call: &ChatToolCall, accepted_input_count: usize) -> Result<(), crate::AgentEngineError> {
        if self.pending() {
            return Err(crate::AgentEngineError::InvalidContract(
                "Pending patch recovery forbids a new patch attempt".into(),
            ));
        }
        let Some(candidate) = patch_paths(call) else {
            return Err(crate::AgentEngineError::InvalidContract("Patch needs valid targets within the recovery budget before dispatch; correct or split the request".into()));
        };
        self.active = candidate;
        self.origin_before_active = self.unresolved_before_input;
        self.active_input_count = Some(accepted_input_count);
        self.unresolved_before_input = Some(self.unresolved_before_input.map_or(accepted_input_count, |count| count.max(accepted_input_count)));
        self.unresolved.extend(self.active.iter().cloned());
        self.pending.extend(self.active.iter().cloned());
        self.ready_for_reads = false;
        Ok(())
    }

    pub(crate) fn published_successfully(&mut self) {
        for path in std::mem::take(&mut self.active) {
            self.unresolved.remove(&path);
            self.pending.remove(&path);
        }
        self.unresolved_before_input = if self.unresolved.is_empty() { None } else { self.origin_before_active };
        self.active_input_count = None;
        self.origin_before_active = None;
        if self.unresolved.is_empty() { self.unaddressable = false; }
    }

    pub(crate) fn observe_successful_file_repair(
        &mut self,
        binding: &AgentToolBinding,
        result: &AgentToolResult,
    ) {
        if result.is_error || binding.capability_id.as_ref() != "workspace.files"
            || binding.action_id.as_ref() != "workspace.files/write"
        { return; }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&result.output_text()) else { return; };
        let Some(path) = value.get("workspace_path").and_then(|path| path.get("path"))
            .and_then(serde_json::Value::as_str).and_then(normalize) else { return; };
        self.unresolved.remove(&path);
        self.pending.remove(&path);
        if self.unresolved.is_empty() {
            self.unresolved_before_input = None;
            self.unaddressable = false;
        }
    }

    pub(crate) fn accept_scope_change(&mut self) {
        self.unresolved.clear();
        self.pending.clear();
        self.active.clear();
        self.unresolved_before_input = None;
        self.active_input_count = None;
        self.origin_before_active = None;
        self.unaddressable = false;
    }

    pub(crate) fn pending(&self) -> bool {
        self.unaddressable || !self.pending.is_empty()
    }

    pub(crate) fn unresolved(&self) -> bool {
        self.unaddressable || !self.unresolved.is_empty()
    }

    pub(crate) fn unresolved_before_input(&self) -> Option<usize> {
        self.unresolved().then_some(self.unresolved_before_input).flatten()
    }

    /// Extract targets for write-ahead arming or an actual failed attempt,
    /// never a planner/steering deferral. Pre-dispatch errors are conservative.
    pub(crate) fn failed(&mut self, call: &ChatToolCall, outcome: Option<&PatchFailureOutcome>) {
        self.ready_for_reads = false;
        let Some(files) = call.arguments.0.get("files").and_then(|v| v.as_array()) else {
            return;
        };
        if files.len() > 64 {
            self.unaddressable = true;
            return;
        }
        let mut paths = Vec::with_capacity(files.len());
        let mut bytes = 0usize;
        for file in files {
            let Some(path) = file
                .get("path")
                .and_then(|v| v.as_str())
                .and_then(normalize)
            else {
                continue;
            };
            bytes = bytes.saturating_add(path.len());
            if bytes > 16 * 1024 {
                self.unaddressable = true;
                break;
            }
            paths.push(path);
        }
        if self.unaddressable || paths.len() != files.len() {
            self.unresolved.extend(paths.iter().cloned());
            self.pending.extend(paths);
            self.active.clear();
            return;
        }
        self.pending.extend(paths.iter().cloned());
        let unresolved_indices = outcome
            .and_then(|outcome| outcome.unresolved_indices(paths.len()))
            .unwrap_or_else(|| (0..paths.len()).collect());
        let current_failed = !unresolved_indices.is_empty();
        for path in &paths { self.unresolved.remove(path); }
        self.unresolved.extend(unresolved_indices.into_iter().map(|index| paths[index].clone()));
        if self.unresolved.is_empty() { self.unresolved_before_input = None; }
        else if current_failed && let Some(count) = self.active_input_count {
            self.unresolved_before_input = Some(self.origin_before_active.map_or(count, |before| before.max(count)));
        } else { self.unresolved_before_input = self.origin_before_active; }
        self.active.clear();
        self.active_input_count = None;
        self.origin_before_active = None;
    }

    pub(crate) fn gate(
        &self,
        binding: &AgentToolBinding,
        _call: &ChatToolCall,
    ) -> Option<&'static str> {
        let cleanup = matches!(
            binding.action_id.as_ref(),
            "workspace.process/poll"
                | "workspace.process/cancel"
                | "workspace.process/close_stdin"
        );
        (self.pending() && !cleanup
            && (!matches!(binding.effect_class, AgentEffectClass::ReadOnly)
                || binding.capability_id.as_ref() == "workspace.process"))
            .then_some("Not executed: further effects after a failed patch require fresh read_file text observations of every recorded target (or missing_ok=true absence), then replanning. An instruction scan/search is not a file-version observation. If further work is forbidden or unavailable, call report_completion with blocked disposition to stop; no recovery read is required for that failure report. Do not bypass via shell.")
    }

    pub(crate) fn invalidate_observations(&mut self) {
        // Fully reread targets remain relevant until a new successful patch
        // clears/replaces them. Later effects invalidate those reads too, not
        // only the still-incomplete subset of an earlier recovery batch.
        if self.unaddressable || !self.unresolved.is_empty() {
            self.pending.extend(self.unresolved.iter().cloned());
            self.ready_for_reads = false;
        }
    }

    pub(crate) fn end_batch(&mut self) {
        self.ready_for_reads = true;
    }

    pub(crate) fn observe_read(&mut self, call: &ChatToolCall, result: &AgentToolResult) {
        if !self.pending() || !self.ready_for_reads || result.is_error {
            return;
        }
        let args = &call.arguments.0;
        if args
            .get("format")
            .and_then(|v| v.as_str())
            .is_some_and(|format| format != "text")
            || args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) != 0
        {
            return;
        }
        let Some(path) = args.get("path").and_then(|v| v.as_str()) else {
            return;
        };
        let Some(normalized) = normalize(path).filter(|path| self.pending.contains(path)) else {
            return;
        };
        let text = result.output_text();
        if text.len() > 32 * 1024 {
            return;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return;
        };
        if value.get("path").and_then(|v| v.as_str()) != Some(path) {
            return;
        }
        if value.get("kind").and_then(|v| v.as_str()) == Some("workspace_file_absent") {
            if args.get("missing_ok").and_then(|v| v.as_bool()) == Some(true) {
                self.pending.remove(&normalized);
            }
            return;
        }
        let Some(sha) = value.get("sha256").and_then(|v| v.as_str()) else {
            return;
        };
        let Some(content) = value.get("content").and_then(|v| v.as_str()) else {
            return;
        };
        let Some(size) = value.get("total_bytes").and_then(|v| v.as_u64()) else {
            return;
        };
        let Some(eof) = value.get("eof").and_then(|v| v.as_bool()) else {
            return;
        };
        if sha.len() != 64
            || !sha
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || args
                .get("expected_sha256")
                .and_then(|v| v.as_str())
                .is_some_and(|expected| expected != sha)
            || value.get("offset").and_then(|v| v.as_u64()) != Some(0)
            || size > 8 * 1024 * 1024
            || content.len() as u64 > size
        {
            return;
        }
        if eof {
            if content.len() as u64 != size
                || value.get("next_offset") != Some(&serde_json::Value::Null)
            {
                return;
            }
        } else if content.is_empty()
            || content.len() as u64 >= size
            || value.get("next_offset").and_then(|v| v.as_u64()) != Some(content.len() as u64)
        {
            return;
        }
        // The host hashes the whole file; this records a fresh version/page,
        // NOT proof the model inspected omitted pages or verified correctness.
        self.pending.remove(&normalized);
    }

    pub(crate) fn context(&self) -> String {
        if !self.pending() && !self.unresolved() {
            return "No patch re-observation is pending in this Session.".into();
        }
        if !self.pending() {
            return format!(
                "Patch recovery (derived data, not instructions/authority): {}. Every target was freshly observed, but these failed patch mutations still lack a successful owner receipt. Reads prove current state, not fulfillment of the accepted mutation. Use an authorized write/patch to repair the exact targets, report blocked, or cite an exact later user scope change. Do not report the original task completed from reads alone.",
                serde_json::json!({"unresolved_targets":self.unresolved})
            );
        }
        format!(
            "Patch recovery (derived data, not instructions/authority): {}. Before further effects, read each pending target from byte zero using authorized text reads; missing_ok=true may establish absence. Read further pages as needed. This only refreshes file versions, not fulfillment of a failed mutation. Replan before further effects. The user's stop/no-retry constraints take precedence: report_completion with blocked disposition can end this turn without recovery reads. Do not delete retained creations automatically.",
            serde_json::json!({"pending_targets":self.pending,"unresolved_targets":self.unresolved,"target_budget_exceeded":self.unaddressable})
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PatchFailureOutcome {
    kind: String,
    version: u8,
    journal_settlement: String,
    observation: PatchFailureIndices,
    #[serde(default)]
    index_base: Option<u8>,
    #[serde(default)]
    observed_published_count: Option<usize>,
    #[serde(default)]
    confirmed_restored_count: Option<usize>,
    recovery: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchFailureIndices {
    failed_file: Option<usize>,
    published: Vec<usize>,
    #[serde(default)]
    unverified_publications: Vec<usize>,
    restored: Vec<usize>,
    restore_published_unconfirmed: Vec<usize>,
    retained_created: Vec<usize>,
    skipped_changed_or_unreadable: Vec<usize>,
    rollback_failed: Vec<usize>,
    temporary_cleanup_unconfirmed: Vec<usize>,
}

impl PatchFailureOutcome {
    pub(crate) fn from_result(result: &Result<AgentToolResult, crate::AgentEngineError>) -> Option<Self> {
        let text = match result {
            Err(crate::AgentEngineError::CapabilityKernel { message, .. }) => message.as_str(),
            Ok(result) if result.is_error => return serde_json::from_str(&result.output_text()).ok(),
            _ => return None,
        };
        serde_json::from_str(text).ok()
    }

    fn unresolved_indices(&self, file_count: usize) -> Option<BTreeSet<usize>> {
        if self.kind != "workspace_patch_failed" || self.version != 1
            || self.journal_settlement != "settled" || self.index_base.is_some_and(|base| base != 0)
            || self.observed_published_count.is_some_and(|count| count != self.observation.published.len())
            || self.confirmed_restored_count.is_some_and(|count| count != self.observation.restored.len())
            || self.recovery.len() > 4096
        { return None; }
        let groups = [
            &self.observation.published, &self.observation.unverified_publications,
            &self.observation.restored, &self.observation.restore_published_unconfirmed,
            &self.observation.retained_created, &self.observation.skipped_changed_or_unreadable,
            &self.observation.rollback_failed, &self.observation.temporary_cleanup_unconfirmed,
        ];
        if self.observation.failed_file.is_some_and(|index| index >= file_count)
            || groups.iter().any(|indices| indices.len() > file_count
                || indices.iter().any(|index| *index >= file_count)
                || indices.iter().copied().collect::<BTreeSet<_>>().len() != indices.len())
        { return None; }
        let published = self.observation.published.iter().copied().collect::<BTreeSet<_>>();
        // A settled zero-publication rejection may itself be the requested
        // negative check (for example a stale source guard). Preserve existing
        // behavior after mandatory rereads. Once any file published, however,
        // every unpublished peer is a concrete incomplete mutation.
        let mut unresolved = if published.is_empty() { BTreeSet::new() }
            else { (0..file_count).filter(|index| !published.contains(index)).collect::<BTreeSet<_>>() };
        unresolved.extend(self.observation.unverified_publications.iter().copied());
        unresolved.extend(self.observation.restored.iter().copied());
        unresolved.extend(self.observation.restore_published_unconfirmed.iter().copied());
        Some(unresolved)
    }
}

fn normalize(path: &str) -> Option<String> {
    if path.is_empty() || path.len() > 4096 || path.chars().any(char::is_control) {
        return None;
    }
    crate::agents_md::normalize_workspace_directory(path)
        .ok()
        .filter(|path| !path.is_empty())
}

fn patch_paths(call: &ChatToolCall) -> Option<BTreeSet<String>> {
    let files = call.arguments.0.get("files")?.as_array()?;
    if files.is_empty() || files.len() > 64 { return None; }
    let mut bytes = 0usize;
    let mut paths = BTreeSet::new();
    for file in files {
        let path = file.get("path")?.as_str().and_then(normalize)?;
        bytes = bytes.checked_add(path.len())?;
        if bytes > 16 * 1024 || !paths.insert(path) { return None; }
    }
    Some(paths)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use nomifun_agent_contracts::StrictJsonValue;
    use nomifun_chat_model_broker::{ChatToolCall, ToolCallId};

    use super::*;

    #[derive(Default)]
    struct Sink(AtomicUsize);

    #[async_trait]
    impl crate::AgentEventSink for Sink {
        async fn emit(
            &self,
            event: crate::AgentEngineEvent,
        ) -> Result<(), crate::AgentEngineError> {
            if matches!(event, crate::AgentEngineEvent::PatchRecoveryUpdated { .. }) {
                self.0.fetch_add(1, Ordering::AcqRel);
            }
            Ok(())
        }
    }

    fn patch_call(paths: &[&str]) -> ChatToolCall {
        ChatToolCall {
            call_id: ToolCallId::from("patch"),
            name: "apply_patch".into(),
            arguments: StrictJsonValue(serde_json::json!({
                "files": paths.iter().map(|path| serde_json::json!({"path":path,"hunks":[]})).collect::<Vec<_>>()
            })),
            provider_metadata: None,
        }
    }

    fn failure(published: &[usize], restored: &[usize], settlement: &str) -> PatchFailureOutcome {
        serde_json::from_value(serde_json::json!({
            "kind":"workspace_patch_failed","version":1,"journal_settlement":settlement,
            "index_base":0,"observed_published_count":published.len(),"confirmed_restored_count":restored.len(),
            "observation":{"failed_file":1,"published":published,"restored":restored,
                "restore_published_unconfirmed":[],"retained_created":[],"skipped_changed_or_unreadable":[],
                "rollback_failed":[],"temporary_cleanup_unconfirmed":[]},
            "recovery":"re-read targets"
        })).unwrap()
    }

    fn read_result(path: &str) -> AgentToolResult {
        let content = "current";
        AgentToolResult::text(ToolCallId::from(format!("read-{path}")), serde_json::json!({
            "path":path,"content":content,"sha256":"a".repeat(64),"total_bytes":content.len(),
            "offset":0,"next_offset":null,"eof":true
        }).to_string(), false)
    }

    fn read_call(path: &str) -> ChatToolCall {
        ChatToolCall {call_id:ToolCallId::from(format!("read-{path}")),name:"read_file".into(),
            arguments:StrictJsonValue(serde_json::json!({"path":path})),provider_metadata:None}
    }

    #[tokio::test]
    async fn restored_recovery_state_is_written_only_after_a_transition() {
        let sink = Sink::default();
        let mut recovery = PatchRecovery::restore(&AgentPatchRecoveryState::default()).unwrap();
        recovery.persist(&sink).await.unwrap();
        assert_eq!(sink.0.load(Ordering::Acquire), 0);

        recovery.failed(&ChatToolCall {
            call_id: ToolCallId::from("patch"),
            name: "apply_patch".into(),
            arguments: StrictJsonValue(serde_json::json!({
                "files":[{"path":"src/lib.rs","hunks":[]}]
            })),
            provider_metadata: None,
        }, None);
        recovery.persist(&sink).await.unwrap();
        recovery.persist(&sink).await.unwrap();
        assert_eq!(sink.0.load(Ordering::Acquire), 1);
    }

    #[test]
    fn fresh_reads_do_not_settle_an_unpublished_patch_target() {
        let call = patch_call(&["a", "b"]);
        let mut recovery = PatchRecovery::default();
        recovery.arm(&call, 1).unwrap();
        recovery.failed(&call, Some(&failure(&[0], &[], "settled")));
        recovery.end_batch();
        for path in ["a", "b"] { recovery.observe_read(&read_call(path), &read_result(path)); }
        assert!(!recovery.pending(), "both current versions were observed");
        assert!(recovery.unresolved(), "b still lacks a successful mutation receipt");
        assert_eq!(recovery.snapshot().unresolved_targets, ["b"]);
        assert_eq!(recovery.snapshot().unresolved_before_input, Some(1));
        assert!(recovery.context().contains("Reads prove current state, not fulfillment"));
        let restored = PatchRecovery::restore(&recovery.snapshot()).unwrap();
        assert!(restored.pending(), "a resumed generation must refresh unresolved targets again");
        assert_eq!(restored.pending, BTreeSet::from(["b".to_owned()]));
    }

    #[test]
    fn legacy_pending_state_migrates_to_a_conservative_unresolved_obligation() {
        let legacy:AgentPatchRecoveryState=serde_json::from_value(serde_json::json!({
            "version":1,"targets":["legacy.txt"],"target_budget_exceeded":false
        })).unwrap();
        let recovery=PatchRecovery::restore(&legacy).unwrap();
        assert!(recovery.pending()&&recovery.unresolved());
        assert_eq!(recovery.snapshot().version,2);
        assert_eq!(recovery.snapshot().unresolved_targets,["legacy.txt"]);
        assert_eq!(recovery.unresolved_before_input(),None,"unknown historical scope cannot be auto-cleared");
    }

    #[test]
    fn successful_exact_write_resolves_only_its_failed_target() {
        let call = patch_call(&["a", "b"]);
        let mut recovery = PatchRecovery::default();
        recovery.arm(&call, 1).unwrap();
        recovery.failed(&call, Some(&failure(&[0], &[], "settled")));
        recovery.end_batch();
        for path in ["a", "b"] { recovery.observe_read(&read_call(path), &read_result(path)); }
        let binding = AgentToolBinding {
            model_name:"write_file".into(),definition:nomifun_chat_model_broker::ChatToolDefinition {
                name:"write_file".into(),description:"write".into(),input_schema:StrictJsonValue(serde_json::json!({"type":"object"})),deferred:false},
            schema_digest:crate::input_schema_digest(&StrictJsonValue(serde_json::json!({"type":"object"}))).unwrap(),
            canonical_input_schema_ref:"schema://write".into(),capability_contract_digest:"c".repeat(64).into(),
            capability_id:"workspace.files".into(),action_id:"workspace.files/write".into(),
            resource_binding_ids:Default::default(),effect_class:AgentEffectClass::ManagedEffect,parallel_safe:false,
        };
        recovery.observe_successful_file_repair(&binding,&AgentToolResult::text("write-other".into(),serde_json::json!({
            "written":true,"workspace_path":{"root_sha256":"d".repeat(64),"path":"other","case_resolved":true}
        }).to_string(),false));
        assert!(recovery.unresolved(),"an unrelated successful write cannot settle b");
        recovery.observe_successful_file_repair(&binding,&AgentToolResult::text("write-b".into(),serde_json::json!({
            "written":true,"workspace_path":{"root_sha256":"d".repeat(64),"path":"b","case_resolved":true}
        }).to_string(),false));
        assert!(!recovery.unresolved());
        assert!(recovery.snapshot().unresolved_targets.is_empty());
    }

    #[test]
    fn unconfirmed_or_malformed_patch_observation_keeps_every_target_unresolved() {
        for outcome in [Some(failure(&[0,1], &[], "unconfirmed")), None] {
            let call = patch_call(&["a", "b"]);
            let mut recovery = PatchRecovery::default();
            recovery.arm(&call, 1).unwrap();
            recovery.failed(&call, outcome.as_ref());
            assert_eq!(recovery.unresolved, BTreeSet::from(["a".to_owned(),"b".to_owned()]));
        }
    }

    #[test]
    fn verified_late_failure_requires_reads_but_has_no_unpublished_obligation() {
        let call=patch_call(&["a","b"]);
        let mut recovery=PatchRecovery::default();
        recovery.arm(&call,1).unwrap();
        recovery.failed(&call,Some(&failure(&[0,1],&[],"settled")));
        assert!(recovery.pending());
        assert!(!recovery.unresolved());
        recovery.end_batch();
        for path in ["a","b"] { recovery.observe_read(&read_call(path),&read_result(path)); }
        assert!(!recovery.pending()&&!recovery.unresolved());
        assert!(recovery.snapshot().targets.is_empty());
    }

    #[test]
    fn settled_zero_publication_rejection_does_not_create_a_permanent_mutation_obligation() {
        let call=patch_call(&["a","b"]);
        let mut recovery=PatchRecovery::default();
        recovery.arm(&call,1).unwrap();
        recovery.failed(&call,Some(&failure(&[],&[],"settled")));
        assert!(recovery.pending(),"current state must still be refreshed");
        assert!(!recovery.unresolved(),"a proven zero-effect negative check remains completable after reads");
    }

    #[test]
    fn accepted_scope_change_clears_observation_and_fulfillment_obligations() {
        let call=patch_call(&["a"]);
        let mut recovery=PatchRecovery::default();
        recovery.arm(&call,1).unwrap();
        recovery.failed(&call,None);
        recovery.accept_scope_change();
        assert!(!recovery.pending()&&!recovery.unresolved());
        assert!(!recovery.snapshot().has_pending());
    }
}
