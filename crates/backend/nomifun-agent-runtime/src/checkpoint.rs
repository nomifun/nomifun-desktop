//! Portable progress, not a memory dump or a capability grant. Transcript,
//! accepted input and observations remain in the canonical event journal.
//! Does not copy provider-private reasoning, raw tool/file bodies, provider
//! credentials or live handles; plan text retains its ordinary data policy.
use nomifun_agent_contracts::{DigestHex, OperationId};
use serde::{Deserialize, Serialize};

use crate::{AgentEngineError, AgentPatchRecoveryState, AgentPlan, AgentWorkStatus, EngineBinding};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentExecutionCheckpoint {
    pub version: u8,
    pub binding: EngineBinding,
    pub turn_operation_id: OperationId,
    pub active_set_generation: u64,
    pub model_steps: u16,
    pub tool_call_count: u32,
    pub accepted_input_count: usize,
    /// Application order corresponds to input indices 1.., never ID sort order.
    pub applied_steering_receipts: Vec<OperationId>,
    pub plan: AgentPlan,
    pub work: AgentWorkStatus,
    pub patch_recovery: AgentPatchRecoveryState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segments: Option<crate::AgentExecutionSegmentState>,
    #[serde(default)]
    pub control_rejections: crate::AgentControlRejectionState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentCheckpointReceipt {
    pub revision: u64,
    pub through_seq: u64,
    pub digest: DigestHex,
}

impl AgentExecutionCheckpoint {
    pub fn validate(&self) -> Result<(), AgentEngineError> {
        self.binding.validate()?;
        self.patch_recovery.validate()?;
        if let Some(segments) = &self.segments { segments.validate(self.model_steps)?; }
        self.control_rejections.validate()?;
        crate::requirements::validate_ledger_budget(&self.plan.requirements)
            .map_err(AgentEngineError::InvalidContract)?;
        let operation = self.turn_operation_id.as_ref();
        let mut receipts = std::collections::BTreeSet::new();
        let mut steps = std::collections::BTreeSet::new();
        let mut requirements = std::collections::BTreeSet::new();
        if self.version != 1 || operation.trim().is_empty() || operation != operation.trim() || operation.len() > 1024
            || self.accepted_input_count == 0 || self.accepted_input_count > 17
            || self.applied_steering_receipts.len() > 16
            || self.applied_steering_receipts.len() + 1 != self.accepted_input_count
            || self.applied_steering_receipts.iter().any(|id| id.as_ref().trim().is_empty()
                || id.as_ref().len() > 1024 || !receipts.insert(id))
            || (self.model_steps == 0 && self.tool_call_count != 0)
            || self.plan.steps.len() > 16 || self.plan.explanation.chars().count() > 2048
            || self.plan.steps.iter().any(|step| step.step.trim().is_empty()
                || step.step.chars().count() > 512 || !steps.insert(step.step.trim()))
            || self.plan.steps.iter().filter(|step| step.status == crate::AgentPlanStatus::InProgress).count() > 1
            || (self.plan.revision == 0 && (!self.plan.steps.is_empty() || !self.plan.requirements.is_empty()))
            || self.plan.requirements.iter().any(|item| item.id.is_empty() || item.id.len() > 64
                || !item.id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                || !requirements.insert(&item.id) || item.description.trim().is_empty()
                || item.description.chars().count() > 512 || item.source.quote.chars().count() > 512
                || item.source.input >= self.accepted_input_count)
            || !self.work.running_processes.is_empty()
        {
            return Err(AgentEngineError::InvalidContract("checkpoint is not a bounded quiescent progress boundary".into()));
        }
        crate::stream_limits::serialized_size(self, nomifun_agent_contracts::MAX_NATIVE_EXECUTION_CHECKPOINT_BYTES)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_process_handles_cannot_be_declared_recoverable_state() {
        let checkpoint = AgentExecutionCheckpoint {
            version: 1,
            binding: EngineBinding::new("session".into(), "binding".into(), "build".into(), "a".repeat(64).into(),
                nomifun_agent_contracts::ResolvedSnapshotRef { snapshot_id: "snapshot".into(), snapshot_digest: "b".repeat(64).into() }).unwrap(),
            turn_operation_id: "turn".into(), active_set_generation: 0, model_steps: 1, tool_call_count: 1,
            accepted_input_count: 1, applied_steering_receipts: vec![], plan: AgentPlan::default(),
            work: AgentWorkStatus { running_processes: std::collections::BTreeSet::from(["live-process".into()]), ..Default::default() },
            patch_recovery: Default::default(), segments: None, control_rejections: Default::default(),
        };
        assert!(checkpoint.validate().is_err());
    }
}
