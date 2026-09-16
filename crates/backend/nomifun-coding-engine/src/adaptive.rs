//! Per-turn activation of execution mechanisms. These modules never grant
//! Capability authority; the frozen ToolPlan remains the only action surface.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{CodingEngineError, CodingEngineEvent, CodingEventSink};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingRuntimeModule {
    ToolLoop,
    ToolHistory,
    TaskLedger,
    CompletionEvidence,
    TaskContinuation,
    PatchRecovery,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingRuntimeActivationReason {
    HistoricalTaskCandidate,
    PendingPatchRecovery,
    ToolCall,
    EffectfulToolCall,
    MultiStepToolUse,
    ExplicitPlan,
    Steering,
    ExplicitTaskContinuation,
}

#[derive(Default)]
pub(crate) struct AdaptiveExecution {
    active: BTreeSet<CodingRuntimeModule>,
    external_tool_batches: u16,
}

impl AdaptiveExecution {
    pub(crate) async fn activate(
        &mut self,
        modules: impl IntoIterator<Item = CodingRuntimeModule>,
        reason: CodingRuntimeActivationReason,
        sink: &dyn CodingEventSink,
    ) -> Result<bool, CodingEngineError> {
        let activated = modules
            .into_iter()
            .filter(|module| self.active.insert(*module))
            .collect::<Vec<_>>();
        if activated.is_empty() {
            return Ok(false);
        }
        sink.emit(CodingEngineEvent::RuntimeModulesActivated {
            modules: activated,
            reason,
        })
        .await?;
        Ok(true)
    }

    pub(crate) fn task_ledger(&self) -> bool {
        self.active.contains(&CodingRuntimeModule::TaskLedger)
    }

    pub(crate) fn tool_history(&self) -> bool {
        self.active.contains(&CodingRuntimeModule::ToolHistory)
    }

    /// Returns true once read-only work spans more than one call in a batch or
    /// more than one model-proposed platform batch. Effectful work activates
    /// the ledger independently before crossing the owner port.
    pub(crate) fn observe_external_batch(&mut self, call_count: usize) -> bool {
        if call_count == 0 {
            return false;
        }
        let multi_step = call_count > 1 || self.external_tool_batches > 0;
        self.external_tool_batches = self.external_tool_batches.saturating_add(1);
        multi_step
    }
}

pub(crate) const TOOL_MODULES: [CodingRuntimeModule; 2] = [
    CodingRuntimeModule::ToolLoop,
    CodingRuntimeModule::ToolHistory,
];

pub(crate) const LEDGER_MODULES: [CodingRuntimeModule; 2] = [
    CodingRuntimeModule::TaskLedger,
    CodingRuntimeModule::CompletionEvidence,
];

pub(crate) const LONG_HORIZON_MODULES: [CodingRuntimeModule; 4] = [
    CodingRuntimeModule::ToolLoop,
    CodingRuntimeModule::ToolHistory,
    CodingRuntimeModule::TaskLedger,
    CodingRuntimeModule::CompletionEvidence,
];
