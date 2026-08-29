//! Hidden in-memory runtime authority owned by one accepted Agent turn.
//!
//! The accepted-turn root restores durable transcript/session state. This
//! companion snapshot restores the mutable in-memory fields that skills,
//! planning, hooks, and continuation may change before a turn is adjudicated.

use nomi_config::hooks::HooksConfig;
use nomi_types::llm::ThinkingConfig;

use crate::cache_diagnostics::CacheBreakDetector;
use crate::compact::state::CompactState;
use crate::goal::state::GoalState;
use crate::loop_guard::StagnationGuard;
use crate::plan::state::PlanState;

/// Exact pre-turn value of every in-memory authority field an accepted turn may
/// mutate. Captured once, before the first provider await, and reused unchanged
/// by every host race-tail pass of the same accepted turn.
#[derive(Debug, Clone)]
pub(crate) struct AcceptedTurnRuntimeAuthority {
    pub(crate) model: String,
    pub(crate) thinking: Option<ThinkingConfig>,
    pub(crate) current_reasoning_effort: Option<String>,
    pub(crate) compaction_level: nomi_compact::CompactionLevel,
    pub(crate) compact_state: CompactState,
    /// `None` for an engine with no hook engine installed. Only the config is
    /// captured; the supervised shell and process supervisor stay live.
    pub(crate) hooks: Option<HooksConfig>,
    pub(crate) plan_state: PlanState,
    /// Value of the shared plan-mode flag, or `None` when no flag is installed.
    /// The `Arc` itself is shared with the plan tools and is never replaced.
    pub(crate) plan_active: Option<bool>,
    /// Shared goal state, or `None` when goal-driven continuation is off.
    pub(crate) goal: Option<GoalState>,
    pub(crate) cache_detector: CacheBreakDetector,
    pub(crate) stagnation_guard: StagnationGuard,
}
