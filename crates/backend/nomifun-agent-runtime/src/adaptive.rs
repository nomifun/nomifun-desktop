//! Per-turn activation of execution mechanisms. These modules never grant
//! Capability authority; the frozen ToolPlan remains the only action surface.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{AgentEngineError, AgentEngineEvent, AgentEventSink};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRuntimeModule {
    ToolLoop,
    ToolHistory,
    TaskLedger,
    CompletionEvidence,
    TaskContinuation,
    PatchRecovery,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRuntimeActivationReason {
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
    active: BTreeSet<AgentRuntimeModule>,
    external_tool_batches: u16,
}

impl AdaptiveExecution {
    pub(crate) async fn activate(
        &mut self,
        modules: impl IntoIterator<Item = AgentRuntimeModule>,
        reason: AgentRuntimeActivationReason,
        sink: &dyn AgentEventSink,
    ) -> Result<bool, AgentEngineError> {
        let activated = modules
            .into_iter()
            .filter(|module| self.active.insert(*module))
            .collect::<Vec<_>>();
        if activated.is_empty() {
            return Ok(false);
        }
        sink.emit(AgentEngineEvent::RuntimeModulesActivated {
            modules: activated,
            reason,
        })
        .await?;
        Ok(true)
    }

    pub(crate) fn task_ledger(&self) -> bool {
        self.active.contains(&AgentRuntimeModule::TaskLedger)
    }

    pub(crate) fn tool_history(&self) -> bool {
        self.active.contains(&AgentRuntimeModule::ToolHistory)
    }

    /// Returns true when workspace work spans more than one call in a batch or
    /// more than one model-proposed platform batch. A single call can execute
    /// before the plan tool is exposed; later effects need an active plan.
    pub(crate) fn observe_external_batch(&mut self, call_count: usize) -> bool {
        if call_count == 0 {
            return false;
        }
        let multi_step = call_count > 1 || self.external_tool_batches > 0;
        self.external_tool_batches = self.external_tool_batches.saturating_add(1);
        multi_step
    }
}

pub(crate) const TOOL_MODULES: [AgentRuntimeModule; 2] = [
    AgentRuntimeModule::ToolLoop,
    AgentRuntimeModule::ToolHistory,
];

pub(crate) const LEDGER_MODULES: [AgentRuntimeModule; 2] = [
    AgentRuntimeModule::TaskLedger,
    AgentRuntimeModule::CompletionEvidence,
];

pub(crate) const LONG_HORIZON_MODULES: [AgentRuntimeModule; 4] = [
    AgentRuntimeModule::ToolLoop,
    AgentRuntimeModule::ToolHistory,
    AgentRuntimeModule::TaskLedger,
    AgentRuntimeModule::CompletionEvidence,
];
