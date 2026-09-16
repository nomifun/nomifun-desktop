//! Owner-authored barriers for a turn-local process scope. A barrier proves
//! only retained process-tree cleanup, never command success or effect rollback.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(tag = "event", deny_unknown_fields)]
pub(super) enum ProcessWitness {
    #[serde(rename = "host_process_dispatch")]
    Dispatch { operation_id: String, ordinal: u16 },
    #[serde(rename = "host_process_quiescent")]
    Quiescent {
        operation_id: String,
        ordinal: u16,
        process_count: usize,
    },
}

/// Only usable with exact-build, exact-turn, contiguous canonical journal
/// evidence and a boot authority proving the previous backend is gone.
#[derive(Default)]
pub(super) struct ProcessRecoveryAudit {
    calls: BTreeSet<String>,
    dispatched_calls: BTreeSet<String>,
    dispatches: BTreeMap<String, String>,
    owner_operations: BTreeSet<String>,
    latest: Option<String>,
    ordinal: u16,
    quiescent_ordinal: u16,
    process_count: usize,
}

fn identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
}

impl ProcessRecoveryAudit {
    pub(super) fn admit(&mut self, call: &str, action: &str) -> Result<(), String> {
        if action != "process.exec.invoke"
            || !identity(call)
            || self.calls.len() >= 512
            || !self.calls.insert(call.into())
        {
            return Err("invalid or duplicate process tool admission".into());
        }
        Ok(())
    }

    pub(super) fn dispatch(
        &mut self,
        record: &super::engine_tool_host::EngineToolDispatchRecord,
    ) -> Result<(), String> {
        if record.capability_id != "process.exec"
            || record.action_id != "process.exec.invoke"
            || record.model_name != "exec_command"
            || !identity(&record.operation_id)
            || !self.calls.contains(&record.call_id)
            || self.dispatches.len() >= 512
            || !self.dispatched_calls.insert(record.call_id.clone())
            || self
                .dispatches
                .insert(record.operation_id.clone(), record.call_id.clone())
                .is_some()
        {
            return Err("process dispatch differs from its tool admission".into());
        }
        Ok(())
    }

    pub(super) fn observe(&mut self, witness: ProcessWitness) -> Result<(), String> {
        match witness {
            ProcessWitness::Dispatch {
                operation_id,
                ordinal,
            } => {
                if !self.dispatches.contains_key(&operation_id)
                    || self.ordinal >= 512
                    || ordinal != self.ordinal + 1
                    || !self.owner_operations.insert(operation_id.clone())
                {
                    return Err(
                        "process owner dispatch lacks exact prior host dispatch or sequence".into(),
                    );
                }
                self.ordinal = ordinal;
                self.latest = Some(operation_id);
            }
            ProcessWitness::Quiescent {
                operation_id,
                ordinal,
                process_count,
            } => {
                if self.latest.as_deref() != Some(operation_id.as_str())
                    || ordinal != self.ordinal
                    || ordinal <= self.quiescent_ordinal
                    || process_count > 64
                    || process_count < self.process_count
                {
                    return Err(
                        "process cleanup barrier is stale, duplicated or inconsistent".into(),
                    );
                }
                self.quiescent_ordinal = ordinal;
                self.process_count = process_count;
            }
        }
        Ok(())
    }

    pub(super) fn is_quiescent(&self) -> bool {
        // Host/engine intent without an owner dispatch did not start a
        // process in this exact compiled implementation. A later owner
        // dispatch always invalidates every earlier barrier.
        self.ordinal == self.quiescent_ordinal
    }
}
