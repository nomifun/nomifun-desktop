//! Owner-authored barriers for a turn-local process scope. A barrier proves
//! only retained process-tree cleanup, never command success or effect rollback.
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
