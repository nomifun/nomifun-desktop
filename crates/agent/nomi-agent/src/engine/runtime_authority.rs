//! Hidden in-memory runtime authority owned by one accepted Agent turn.
//!
//! [`AcceptedTurnRoot`](crate::session::AcceptedTurnRoot) restores the durable
//! half of a rejected turn: transcript, editable checkpoint, host context, and
//! deferred tool activations. None of that covers the authority a turn can grant
//! itself in memory. The reproducible shape is:
//!
//! 1. a failing turn runs an inline Skill successfully;
//! 2. the Skill's context modifier grants `Bash`, switches model / effort /
//!    plan mode, or merges hooks into the live [`HookEngine`];
//! 3. the model then claims completion with no machine evidence, so A2 rejects
//!    the turn and the transcript is retracted;
//! 4. without this snapshot the same runtime keeps the auto-approval, hook, and
//!    plan state with no visible provenance, while a fresh reload does not —
//!    a live/reload fork in exactly the security-relevant direction.
//!
//! What is deliberately **not** captured:
//!
//! - `total_usage`: provider cost and telemetry must stay true even when the
//!   provisional transcript is rejected.
//! - workspace and external tool side effects: the rollback restores authority
//!   and session truth, not the world.
//! - [`ToolApprovalManager`](nomi_protocol::ToolApprovalManager): every mutation
//!   is an explicit operator decision (an "always" click in the approval dialog,
//!   or a session-mode switch). No skill or model path reaches it, so it is a
//!   human preference rather than turn authority. The same reasoning splits
//!   `ToolConfirmer`: its skill-granted allow list rolls back, its interactive
//!   `[a]lways` grants do not.
//! - `thinking` and `compaction_level` are only ever written by
//!   `apply_config_update`, whose sole caller queues `set_config` and applies it
//!   *after* the turn future returns ("set_config: queued, will apply after
//!   current response"). They are still captured here because they are request
//!   authority; the audit above proves no operator decision can be interleaved
//!   with a mid-turn rollback, so restoring them cannot revoke a human choice.

use nomi_config::hooks::HooksConfig;
use nomi_types::llm::ThinkingConfig;

use crate::cache_diagnostics::CacheBreakDetector;
use crate::compact::state::CompactState;
use crate::confirm::ToolConfirmerAuthority;
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
    pub(crate) allow_list: Vec<String>,
    pub(crate) confirmer: ToolConfirmerAuthority,
    /// `None` for an engine with no hook engine installed. Only the config is
    /// captured — the supervised shell and process supervisor stay live.
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
