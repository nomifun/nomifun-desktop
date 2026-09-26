//! Engine policy only: a failed dispatched patch requires fresh observations.
//! No filesystem bypass, automatic undo, test execution or error-string parsing.
use crate::{AgentEffectClass, AgentToolBinding, AgentToolResult};
use nomifun_chat_model_broker::ChatToolCall;
use std::collections::BTreeSet;

/// Engine-derived recovery obligation, not a checkpoint, permission grant or
/// filesystem proof. Hosts restore this independently of conversational history.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPatchRecoveryState {
    pub version: u8,
    pub targets: Vec<String>,
    pub target_budget_exceeded: bool,
}

impl Default for AgentPatchRecoveryState {
    fn default() -> Self {
        Self {
            version: 1,
            targets: Vec::new(),
            target_budget_exceeded: false,
        }
    }
}

impl AgentPatchRecoveryState {
    pub fn has_pending(&self) -> bool {
        self.target_budget_exceeded || !self.targets.is_empty()
    }

    pub fn validate(&self) -> Result<(), crate::AgentEngineError> {
        let mut seen = BTreeSet::new();
        if self.version != 1
            || self.targets.len() > 64
            || self.targets.iter().map(String::len).sum::<usize>() > 16 * 1024
            || self
                .targets
                .iter()
                .any(|path| normalize(path).as_ref() != Some(path) || !seen.insert(path))
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
    targets: BTreeSet<String>,
    pending: BTreeSet<String>,
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
        let targets = state.targets.iter().cloned().collect::<BTreeSet<_>>();
        Ok(Self {
            pending: targets.clone(),
            targets,
            unaddressable: state.target_budget_exceeded,
            ready_for_reads: true,
            persisted: Some(state.clone()),
        })
    }

    pub(crate) fn snapshot(&self) -> AgentPatchRecoveryState {
        AgentPatchRecoveryState {
            // A new turn must freshly observe ALL targets if even one was left
            // pending. Partial historical reads do not become current evidence.
            targets: if self.pending() {
                self.targets.iter().cloned().collect()
            } else {
                Vec::new()
            },
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

    pub(crate) fn arm(&mut self, call: &ChatToolCall) -> Result<(), crate::AgentEngineError> {
        if self.pending() {
            return Err(crate::AgentEngineError::InvalidContract(
                "Pending patch recovery forbids a new patch attempt".into(),
            ));
        }
        let mut candidate = Self::default();
        candidate.failed(call);
        if candidate.unaddressable || candidate.targets.is_empty() {
            return Err(crate::AgentEngineError::InvalidContract("Patch needs valid targets within the recovery budget before dispatch; correct or split the request".into()));
        }
        self.targets = candidate.targets;
        self.pending = candidate.pending;
        self.ready_for_reads = false;
        Ok(())
    }

    pub(crate) fn published_successfully(&mut self) {
        self.targets.clear();
        self.pending.clear();
        self.unaddressable = false;
    }

    pub(crate) fn pending(&self) -> bool {
        self.unaddressable || !self.pending.is_empty()
    }

    /// Extract targets for write-ahead arming or an actual failed attempt,
    /// never a planner/steering deferral. Pre-dispatch errors are conservative.
    pub(crate) fn failed(&mut self, call: &ChatToolCall) {
        self.ready_for_reads = false;
        self.targets.clear();
        self.pending.clear();
        let Some(files) = call.arguments.0.get("files").and_then(|v| v.as_array()) else {
            return;
        };
        if files.len() > 64 {
            self.unaddressable = true;
            return;
        }
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
            self.targets.insert(path);
        }
        self.pending = self.targets.clone();
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
            .then_some("Not executed: a failed patch requires fresh read_file text observations of every recorded target (or missing_ok=true absence), then replanning. An instruction scan/search is not a file-version observation. If reads are unavailable, report blocked; do not bypass via shell.")
    }

    pub(crate) fn invalidate_observations(&mut self) {
        // Fully reread targets remain relevant until a new successful patch
        // clears/replaces them. Later effects invalidate those reads too, not
        // only the still-incomplete subset of an earlier recovery batch.
        if self.unaddressable || !self.targets.is_empty() {
            self.pending = self.targets.clone();
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
        if !self.pending() {
            return "No patch re-observation is pending in this Session.".into();
        }
        format!(
            "Patch recovery (derived data, not instructions/authority): {}. Read each target from byte zero using authorized text reads; missing_ok=true may establish absence. Read further pages as needed. This only refreshes file versions, not full inspection, rollback, correctness or task completion. Replan after observation. If targets cannot be observed, report blocked. Do not delete retained creations automatically.",
            serde_json::json!({"pending_targets":self.pending,"target_budget_exceeded":self.unaddressable})
        )
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
        });
        recovery.persist(&sink).await.unwrap();
        recovery.persist(&sink).await.unwrap();
        assert_eq!(sink.0.load(Ordering::Acquire), 1);
    }
}
