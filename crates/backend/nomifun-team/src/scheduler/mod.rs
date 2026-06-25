use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::{DashMap, DashSet};
use nomifun_realtime::EventBroadcaster;
use tokio::sync::Mutex;

use crate::error::TeamError;
use crate::events::TeamEventEmitter;
use crate::mailbox::Mailbox;
use crate::task_board::TaskBoard;
use crate::types::{MailboxMessage, TeamAgent, TeamTask, TeammateRole, TeammateStatus};

mod actions;
mod agent_lifecycle;
mod crash_recovery;
mod dedup;
mod state;
mod wake;

#[cfg(test)]
mod tests;

pub use actions::SchedulerAction;
pub use crash_recovery::format_crash_testament;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const WAKE_TIMEOUT_MS: u64 = 60_000;

pub(crate) const FINALIZE_DEDUP_WINDOW: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// normalize_name — canonical form for agent-name conflict checks
// ---------------------------------------------------------------------------

/// Normalize an agent name to its canonical form for conflict detection.
///
/// Rules (see interface-contracts 15.1):
/// 1. Trim leading/trailing whitespace.
/// 2. Drop control characters (`char::is_control`).
/// 3. Lowercase (Unicode-aware via `to_lowercase`).
pub fn normalize_name(name: &str) -> String {
    name.trim()
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .to_lowercase()
}

// ---------------------------------------------------------------------------
// is_settled — helper for "all teammates settled" transitions
// ---------------------------------------------------------------------------

/// Status set that counts as "settled" for the purpose of
/// "all teammates settled -> wake leader" transitions.
///
/// Expanded beyond `Idle` to match the Nomi reference implementation
/// (TeammateManager.ts:440-452): `Completed` and `Error` teammates are
/// terminal and should not block the leader from being woken up.
/// `Pending` is not in the set because the backend currently serde-aliases
/// `"pending"` to `Idle`; it will be reintroduced when the variant is split.
pub(crate) fn is_settled(status: TeammateStatus) -> bool {
    matches!(
        status,
        TeammateStatus::Idle | TeammateStatus::Completed | TeammateStatus::Error
    )
}

// ---------------------------------------------------------------------------
// WakeTimeoutHandler type alias
// ---------------------------------------------------------------------------

/// Callback invoked when the wake-timeout watchdog elapses without seeing
/// any stream activity for a slot.
///
/// Reason: `arm_wake_timeout` is written against `origin/main`, where
/// `handle_inactivity_timeout` (W4-D22, PR #99) does not yet exist. Taking
/// the recovery action as an injected closure keeps this module decoupled --
/// once D22 lands, callers just pass `mgr.handle_inactivity_timeout(...)`
/// through this slot without touching `arm_wake_timeout` itself.
pub type WakeTimeoutHandler = Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

// ---------------------------------------------------------------------------
// WakePayload — context assembled for an agent when it is woken up
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct WakePayload {
    pub agent: TeamAgent,
    pub tasks: Vec<TeamTask>,
    pub unread_messages: Vec<MailboxMessage>,
}

// ---------------------------------------------------------------------------
// AgentSlot — per-agent runtime state tracked by the scheduler
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct AgentSlot {
    pub(crate) agent: TeamAgent,
    pub(crate) status: TeammateStatus,
    /// True until the first wake completes — used to inject role prompt on cold start.
    pub(crate) needs_role_prompt: bool,
}

// ---------------------------------------------------------------------------
// TeammateManager
// ---------------------------------------------------------------------------

pub struct TeammateManager {
    pub(crate) team_id: String,
    pub(crate) slots: Mutex<HashMap<String, AgentSlot>>,
    pub(crate) mailbox: Arc<Mailbox>,
    pub(crate) task_board: Arc<TaskBoard>,
    pub(crate) events: TeamEventEmitter,
    pub(crate) active_wakes: DashSet<String>,
    // Reason: Finish / Error events may fire back-to-back for the same
    // conversation; without this dedup window, finalize_turn would run twice
    // and double-write the IdleNotification (nomifun-audit 4.3, 8 #3).
    pub(crate) finalized_turns: Arc<DashMap<String, Instant>>,
    pub(crate) wake_timeouts: Arc<DashMap<String, tokio::task::JoinHandle<()>>>,
}

impl TeammateManager {
    pub fn new(
        team_id: String,
        agents: &[TeamAgent],
        mailbox: Arc<Mailbox>,
        task_board: Arc<TaskBoard>,
        broadcaster: Arc<dyn EventBroadcaster>,
    ) -> Self {
        let mut slots = HashMap::new();
        for agent in agents {
            let mut a = agent.clone();
            a.status = Some(TeammateStatus::Idle);
            slots.insert(
                a.slot_id.clone(),
                AgentSlot {
                    agent: a,
                    status: TeammateStatus::Idle,
                    needs_role_prompt: true,
                },
            );
        }
        let events = TeamEventEmitter::new(team_id.clone(), broadcaster);
        Self {
            team_id,
            slots: Mutex::new(slots),
            mailbox,
            task_board,
            events,
            active_wakes: DashSet::new(),
            finalized_turns: Arc::new(DashMap::new()),
            wake_timeouts: Arc::new(DashMap::new()),
        }
    }

    pub async fn get_agent(&self, slot_id: &str) -> Result<TeamAgent, TeamError> {
        let slots = self.slots.lock().await;
        let slot = slots
            .get(slot_id)
            .ok_or_else(|| TeamError::AgentNotFound(slot_id.to_owned()))?;
        Ok(slot.agent.clone())
    }

    pub async fn list_agents(&self) -> Vec<TeamAgent> {
        let slots = self.slots.lock().await;
        slots.values().map(|s| s.agent.clone()).collect()
    }

    pub async fn list_tasks(&self) -> Result<Vec<TeamTask>, TeamError> {
        self.task_board.list_tasks(&self.team_id).await
    }

    pub async fn find_lead_slot_id(&self) -> Option<String> {
        let slots = self.slots.lock().await;
        slots
            .values()
            .find(|s| s.agent.role == TeammateRole::Lead)
            .map(|s| s.agent.slot_id.clone())
    }
}
