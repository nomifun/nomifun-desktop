//! Execution windows are not new tasks or new authority. Only an acknowledged
//! quiescent checkpoint can renew a window; cumulative limits never reset.
use nomifun_agent_contracts::{DigestHex, digest_payload};
use serde::{Deserialize, Serialize};

use crate::{AgentEngineError, AgentToolResult};
use nomifun_chat_model_broker::ChatToolCall;

const MAX_SEGMENTS: u16 = 32;
const MAX_TOTAL_STEPS: u16 = 4096;
const MAX_PROGRESS_FINGERPRINTS: usize = 1024;

/// Compiled/host-selected policy, never deserialized from model arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSegmentPolicy {
    pub max_segments: u16,
    pub max_no_progress_segments: u16,
}

impl Default for AgentSegmentPolicy {
    fn default() -> Self {
        Self { max_segments: 16, max_no_progress_segments: 2 }
    }
}

impl AgentSegmentPolicy {
    pub(crate) fn validate(self, steps: u16) -> Result<Self, AgentEngineError> {
        if steps == 0 || self.max_segments == 0 || self.max_segments > MAX_SEGMENTS
            || self.max_no_progress_segments == 0 || self.max_no_progress_segments > self.max_segments
            || steps.checked_mul(self.max_segments).is_none_or(|total| total > MAX_TOTAL_STEPS)
        {
            return Err(AgentEngineError::InvalidContract("invalid bounded execution segment policy".into()));
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSegmentReason { ModelWindow, JournalWindow }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionStopReason {
    UserRequested,
    DispatchWindow,
    TotalModelBudget,
    SegmentBudget,
    NoProgress,
    ProgressLedgerFull,
    CheckpointUnavailable,
    NonQuiescentBoundary,
    TurnJournalBudget,
    SessionPayloadBudget,
}

impl AgentExecutionStopReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::UserRequested => "EXECUTION_USER_REQUESTED",
            Self::DispatchWindow => "EXECUTION_DISPATCH_WINDOW",
            Self::TotalModelBudget => "EXECUTION_TOTAL_MODEL_BUDGET",
            Self::SegmentBudget => "EXECUTION_SEGMENT_BUDGET",
            Self::NoProgress => "EXECUTION_NO_PROGRESS",
            Self::ProgressLedgerFull => "EXECUTION_PROGRESS_LEDGER_FULL",
            Self::CheckpointUnavailable => "EXECUTION_CHECKPOINT_UNAVAILABLE",
            Self::NonQuiescentBoundary => "EXECUTION_NON_QUIESCENT_BOUNDARY",
            Self::TurnJournalBudget => "EXECUTION_TURN_JOURNAL_BUDGET",
            Self::SessionPayloadBudget => "EXECUTION_SESSION_PAYLOAD_BUDGET",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentExecutionPressure {
    pub renew_window: bool,
    pub stop: Option<AgentExecutionStopReason>,
}

/// Digests contain no raw arguments, outputs, process handles or credentials.
/// No eviction: exhausting this bounded ledger cannot make an old repeated
/// result look novel and thereby buy further automatic execution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentExecutionSegmentState {
    pub policy: AgentSegmentPolicy,
    pub model_steps_per_segment: u16,
    pub segment: u16,
    pub segment_start_step: u16,
    pub no_progress_segments: u16,
    pub progress_at_segment_start: usize,
    pub progress_fingerprints: Vec<DigestHex>,
}

impl AgentExecutionSegmentState {
    pub(crate) fn authorize_resume(&mut self, grant: &nomifun_agent_contracts::NativeBudgetIncrease, model_steps: u16) -> Result<(), AgentEngineError> {
        self.policy.max_segments = self.policy.max_segments.checked_add(grant.additional_segments)
            .ok_or_else(|| AgentEngineError::InvalidContract("authorized model allowance overflow".into()))?;
        self.policy.validate(self.model_steps_per_segment)?;
        if grant.retry_stall_guards { self.no_progress_segments = 0; }
        if model_steps >= self.total_model_limit() { return Err(AgentEngineError::InvalidContract("resume needs an explicit additional model allowance".into())); }
        if model_steps >= self.window_end() {
            *self = self.renewed(model_steps).map_err(|reason| AgentEngineError::InvalidContract(format!("resume remains blocked: {}", reason.code())))?;
        }
        self.validate(model_steps)
    }

    pub(crate) fn new(steps: u16, policy: AgentSegmentPolicy) -> Result<Self, AgentEngineError> {
        Ok(Self { policy: policy.validate(steps)?, model_steps_per_segment: steps,
            segment: 1, segment_start_step: 0, no_progress_segments: 0,
            progress_at_segment_start: 0, progress_fingerprints: Vec::new() })
    }

    pub(crate) fn validate(&self, model_steps: u16) -> Result<(), AgentEngineError> {
        self.policy.validate(self.model_steps_per_segment)?;
        let unique: std::collections::BTreeSet<_> = self.progress_fingerprints.iter().collect();
        if self.segment == 0 || self.segment > self.policy.max_segments
            || self.segment_start_step > model_steps || model_steps > self.total_model_limit()
            || model_steps > self.window_end()
            || self.no_progress_segments >= self.policy.max_no_progress_segments
            || self.progress_at_segment_start > self.progress_fingerprints.len()
            || self.progress_fingerprints.len() > MAX_PROGRESS_FINGERPRINTS
            || unique.len() != self.progress_fingerprints.len()
            || self.progress_fingerprints.iter().any(|digest| digest.as_ref().len() != 64
                || !digest.as_ref().bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err(AgentEngineError::InvalidContract("invalid persisted execution segment state".into()));
        }
        Ok(())
    }

    pub(crate) fn total_model_limit(&self) -> u16 {
        self.model_steps_per_segment.saturating_mul(self.policy.max_segments)
    }

    pub(crate) fn window_end(&self) -> u16 {
        self.segment_start_step.saturating_add(self.model_steps_per_segment).min(self.total_model_limit())
    }

    pub(crate) fn observe(&mut self, call: &ChatToolCall, result: &AgentToolResult) -> Result<(), AgentEngineError> {
        if result.is_error || self.progress_fingerprints.len() >= MAX_PROGRESS_FINGERPRINTS { return Ok(()); }
        // IDs, invocation counters and plan explanations are not progress.
        // Call this only for an actually attempted platform observation.
        let digest = digest_payload(&serde_json::json!({
            "tool": call.name, "arguments": call.arguments, "output": result.output,
        })).map_err(|error| AgentEngineError::InvalidContract(error.to_string()))?;
        if !self.progress_fingerprints.contains(&digest) { self.progress_fingerprints.push(digest); }
        Ok(())
    }

    pub(crate) fn renewed(&self, model_steps: u16) -> Result<Self, AgentExecutionStopReason> {
        if model_steps >= self.total_model_limit() { return Err(AgentExecutionStopReason::TotalModelBudget); }
        if self.segment >= self.policy.max_segments { return Err(AgentExecutionStopReason::SegmentBudget); }
        if self.progress_fingerprints.len() >= MAX_PROGRESS_FINGERPRINTS { return Err(AgentExecutionStopReason::ProgressLedgerFull); }
        let mut next = self.clone();
        next.no_progress_segments = if self.progress_fingerprints.len() > self.progress_at_segment_start {
            0
        } else { self.no_progress_segments.saturating_add(1) };
        if next.no_progress_segments >= self.policy.max_no_progress_segments { return Err(AgentExecutionStopReason::NoProgress); }
        next.segment += 1;
        next.segment_start_step = model_steps;
        next.progress_at_segment_start = next.progress_fingerprints.len();
        Ok(next)
    }

    pub(crate) fn context(&self, model_steps: u16) -> String {
        format!("Execution window {}/{}; {} model steps remain in this window, {} remain in the cumulative budget. A window may renew only after a durable quiescent checkpoint and bounded progress accounting. This is the SAME accepted task, not new instructions or permission. Preserve all original requirements, plan and results; do not redo completed effects. Budget exhaustion or no-progress stop is not task completion.",
            self.segment, self.policy.max_segments, self.window_end().saturating_sub(model_steps),
            self.total_model_limit().saturating_sub(model_steps))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renewal_preserves_cumulative_limit_and_blocks_repeated_observations() {
        let mut state = AgentExecutionSegmentState::new(2, AgentSegmentPolicy::default()).unwrap();
        let call = ChatToolCall { call_id: "read-1".into(), name: "read_file".into(),
            arguments: nomifun_agent_contracts::StrictJsonValue(serde_json::json!({"path":"a"})), provider_metadata: None };
        let result = AgentToolResult::text(call.call_id.clone(), "unchanged", false);
        state.observe(&call, &result).unwrap();
        let mut state = state.renewed(2).unwrap();
        assert_eq!(state.total_model_limit(), 32);
        state.observe(&ChatToolCall { call_id: "read-2".into(), ..call.clone() }, &result).unwrap();
        let state = state.renewed(4).unwrap();
        assert_eq!(state.renewed(6), Err(AgentExecutionStopReason::NoProgress));
        state.validate(6).unwrap();
    }

    #[test]
    fn restored_state_cannot_expand_limits_or_hide_duplicate_progress() {
        let mut state = AgentExecutionSegmentState::new(2, AgentSegmentPolicy::default()).unwrap();
        assert!(state.validate(3).is_err());
        state.progress_fingerprints = vec!["a".repeat(64).into(), "a".repeat(64).into()];
        assert!(state.validate(1).is_err());
    }
}
