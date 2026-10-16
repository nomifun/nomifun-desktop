//! Bounded Runtime decisions for tool-process presentation.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One proposal did not reach its owner port. This is neither an authority
/// decision nor an Effect/Turn outcome. A write-ahead admission intent may
/// already exist; earlier calls and their receipts remain unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export, export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum RuntimeToolHoldReason {
    ToolNotExposed,
    ArgumentsRejected,
    BatchRejected,
    PhaseRestricted,
    InputSuperseded,
    PriorCallFailed,
    RuntimeGate,
}
