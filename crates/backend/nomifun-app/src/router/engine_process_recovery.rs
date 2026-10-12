//! Owner-authored barriers for a turn-local process scope. A barrier proves
//! only retained process-tree cleanup, never command success or effect rollback.
pub(super) use nomifun_agent_contracts::NativeProcessWitness as ProcessWitness;
